use std::env;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gstreamer::{element_error, ElementFactory, Pipeline, ReferenceTimestampMeta, State};
use gstreamer::prelude::{Cast, ElementExt, ElementExtManual, FormattedValue, GstBinExtManual, GstObjectExt};
use gstreamer_app::{AppSink, AppSinkCallbacks, gst};
use livekit::options::{AudioCaptureOptions, TrackPublishOptions, VideoCaptureOptions};
use livekit::prelude::*;
use livekit::webrtc::{
    audio_frame::AudioFrame,
    audio_source::native::NativeAudioSource,
    video_frame::{I420Buffer, VideoFrame, VideoRotation},
    video_frame::native::I420BufferExt, video_source::native::NativeVideoSource,
};
use livekit_api::access_token::{AccessToken, VideoGrants};

use byteorder::{BigEndian, ReadBytesExt};

#[tokio::main]
async fn main() {
    let livekit_url = env::var("LIVEKIT_WSS_URL").unwrap();
    let livekit_token = AccessToken::new()
        .unwrap()
        .with_ttl(Duration::from_secs(3600))
        .with_identity("gstreamer")
        .with_name("gstreamer")
        .with_grants(VideoGrants {
            room_join: true,
            can_publish: true,
            can_publish_data: true,
            can_publish_sources: vec!["Camera".to_string(), "Microphone".to_string()],
            can_update_own_metadata: true,
            room: "Default".to_string(),
            ..Default::default()
        })
        .to_jwt().unwrap();

    let (room, _event_ch) = Room::connect(
        livekit_url.as_str(),
        livekit_token.as_str(),
    ).await.unwrap();
    let video_source = NativeVideoSource::default();
    let video_track = LocalVideoTrack::create_video_track(
        "gstreamer-test",
        VideoCaptureOptions::default(),
        video_source.clone(),
    );
    let audio_source = NativeAudioSource::default();
    let audio_track = LocalAudioTrack::create_audio_track("gstreamer-test", AudioCaptureOptions {
        echo_cancellation: true,
        auto_gain_control: true,
        noise_suppression: true,
    }, audio_source.clone());
    let session = room.session();
    session.local_participant()
        .publish_track(
            LocalTrack::Video(video_track.clone()),
            TrackPublishOptions { source: TrackSource::Camera, ..Default::default() },
        )
        .await.unwrap();
    session
        .local_participant()
        .publish_track(
            LocalTrack::Audio(audio_track.clone()),
            TrackPublishOptions { source: TrackSource::Microphone, ..Default::default() },
        )
        .await.unwrap();
    tokio::task::spawn_blocking(|| unsafe {
        track_task(video_source, audio_source)
    }).await.unwrap().await;
}

async unsafe fn track_task(video_source: NativeVideoSource, audio_source: NativeAudioSource) {
    let (height, width) = (720, 1280);
    gst::init().unwrap();
    let videotestsrc = ElementFactory::make("videotestsrc")
        .build().unwrap();
    let vid_appsink = AppSink::builder()
        .caps(&gst::Caps::builder("video/x-raw")
            .field("format", gstreamer_video::VideoFormat::I420.to_str())
            .field("width", width as i32)
            .field("height", height as i32)
            .build(),
        ).build();

    let vid_frame = Arc::new(Mutex::new(VideoFrame {
        rotation: VideoRotation::VideoRotation0,
        timestamp: 0,
        buffer: I420Buffer::new(1280, 720),
    }));

    vid_appsink.set_callbacks(
        AppSinkCallbacks::builder()
            .new_sample(move |appsink| {
                let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                let buffer = sample.buffer().ok_or_else(|| {
                    element_error!(
                        appsink,
                        gst::ResourceError::Failed,
                        ("Failed to get buffer from appsink")
                    );
                    gst::FlowError::Error
                })?;
                let mut frame = vid_frame.lock().unwrap();
                let i420_buffer = &mut frame.buffer;
                let (dst_y, dst_u, dst_v) = i420_buffer.data_mut();
                let caps = sample.caps().unwrap();
                let mut ts = 0;
                if let Some(meta) = buffer.meta::<ReferenceTimestampMeta>() {
                    ts = meta.timestamp().into_raw_value();
                }
                let video_info = gstreamer_video::VideoInfo::from_caps(caps).unwrap();
                let sample_frame =
                    gstreamer_video::video_frame::VideoFrame::from_buffer_readable(
                        buffer.copy(),
                        &video_info,
                    ).unwrap();
                let src_y = sample_frame.plane_data(0).unwrap();
                let src_u = sample_frame.plane_data(1).unwrap();
                let src_v = sample_frame.plane_data(2).unwrap();
                dst_y.copy_from_slice(src_y);
                dst_u.copy_from_slice(src_u);
                dst_v.copy_from_slice(src_v);
                frame.timestamp = ts;

                video_source.capture_frame(&*frame);
                Ok(gst::FlowSuccess::Ok)
            }).build(),
    );

    let audiotestsrc = ElementFactory::make("audiotestsrc")
        .build().unwrap();
    let aud_appsink = AppSink::builder()
        .caps(&gst::Caps::builder("audio/x-raw")
            .field("format", gstreamer_audio::AudioFormat::S16be.to_str())
            .field("layout", "interleaved")
            .field("rate", 48000)
            .field("channels", 2)
            .build(),
        ).build();


    aud_appsink.set_callbacks(
        AppSinkCallbacks::builder()
            .new_sample(move |appsink| {
                let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                let buffer = sample.buffer().ok_or_else(|| {
                    element_error!(
                        appsink,
                        gst::ResourceError::Failed,
                        ("Failed to get buffer from appsink")
                    );
                    gst::FlowError::Error
                })?;
                let audio_info = gstreamer_audio::AudioInfo::builder(gstreamer_audio::AudioFormat::S16be, 48000, 2)
                    .build().unwrap();
                let audio_buffer = gstreamer_audio::AudioBuffer::from_buffer_readable(
                    buffer.copy(),
                    &audio_info,
                ).unwrap();
                let map = buffer.map_readable().map_err(|_| {
                    element_error!(
                        appsink,
                        gst::ResourceError::Failed,
                        ("Failed to map buffer readable")
                    );
                    gst::FlowError::Error
                })?;
                let mut plane_data = map.as_slice();
                let mut data = vec![0i16; audio_buffer.n_samples() * 2];
                let data_slice = data.as_mut_slice();
                ReadBytesExt::read_i16_into::<BigEndian>(&mut plane_data, data_slice).unwrap();
                audio_source.capture_frame(AudioFrame {
                    data: data_slice.to_vec(),
                    sample_rate_hz: audio_buffer.rate(),
                    num_channels: audio_buffer.channels() as usize,
                    samples_per_channel: audio_buffer.n_samples() / 2,
                });
                Ok(gst::FlowSuccess::Ok)
            }).build(),
    );

    let pipeline = Pipeline::new(None);
    pipeline.add_many(&[
        &videotestsrc,
        vid_appsink.upcast_ref(),
        &audiotestsrc,
        aud_appsink.upcast_ref(),
    ]).unwrap();

    videotestsrc.link(&vid_appsink).unwrap();
    audiotestsrc.link(&aud_appsink).unwrap();

    pipeline.set_state(State::Playing).unwrap();

    let bus = pipeline
        .bus()
        .expect("Pipeline without bus. Shouldn't happen!");

    for msg in bus.iter_timed(gst::ClockTime::NONE) {
        use gst::MessageView;

        match msg.view() {
            MessageView::Eos(..) => break,
            MessageView::Error(err) => {
                println!(
                    "Error from {:?}: {} ({:?})",
                    err.src().map(|s| s.path_string()),
                    err.error(),
                    err.debug()
                );
                pipeline.set_state(gst::State::Null).unwrap();
            }
            _ => (),
        }
    }

    pipeline.set_state(gst::State::Null).unwrap();
}