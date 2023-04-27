use std::sync::Arc;
use std::time::Duration;

use gstreamer::{Element, ReferenceTimestampMeta};
use gstreamer::glib::getenv;
use gstreamer::prelude::*;
use gstreamer_app::gst;
use livekit::options::{TrackPublishOptions, VideoCaptureOptions};
use livekit::prelude::*;
use livekit::webrtc::{
    video_frame::{I420Buffer, VideoFrame, VideoRotation},
    video_source::native::NativeVideoSource,
};
use livekit::webrtc::video_frame::native::I420BufferExt;
use livekit_api::access_token::{AccessToken, VideoGrants};
use tokio::sync::Mutex;

use {
    gstreamer::{ElementFactory, Pipeline, State},
    gstreamer_app::AppSink,
};

#[tokio::main]
async fn main() {
    // let livekit_url = getenv("LIVEKIT_URL").unwrap();
    let livekit_url = getenv("LIVEKIT_WSS_URL").unwrap();
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
        }).to_jwt().unwrap();

    let (room, _event_ch) = Room::connect(
        livekit_url.to_str().unwrap(),
        livekit_token.as_str(),
    ).await.unwrap();
    let source = NativeVideoSource::default();
    let track = LocalVideoTrack::create_video_track(
        "gstreamer-test",
        VideoCaptureOptions::default(),
        source.clone(),
    );
    room.session()
        .local_participant()
        .publish_track(
            LocalTrack::Video(track.clone()),
            TrackPublishOptions {
                source: TrackSource::Camera,
                ..Default::default()
            },
        )
        .await.unwrap();
    tokio::task::spawn_blocking(|| unsafe {
        track_task(source)
    }).await.unwrap().await;
}

async unsafe fn track_task(rtc_source: NativeVideoSource) {
    let (height, width) = (720, 1280);
    gst::init().unwrap();
    let videotestsrc = ElementFactory::make("videotestsrc")
        .build().unwrap();
    let audiotestsrc = ElementFactory::make("audiotestsrc")
        .property("wave", 4)
        .build().unwrap();
    let caps_filter = ElementFactory::make("capsfilter")
        .property("caps", gst::Caps::builder("video/x-raw")
            .field("format", gstreamer_video::VideoFormat::I420.to_str())
            .field("width", width as i32)
            .field("height", height as i32)
            .build(),
        ).build().unwrap();

    let appsink = ElementFactory::make("appsink").build().unwrap();

    let pipeline = Pipeline::new(None);
    pipeline.add_many(&[&videotestsrc, &caps_filter, &appsink]).unwrap();
    Element::link_many(&[&videotestsrc, &videotestsrc, &caps_filter, &appsink]).unwrap();

    let app_sink = appsink.dynamic_cast::<AppSink>().unwrap();
    pipeline.set_state(State::Playing).unwrap();

    let frame = Arc::new(Mutex::new(VideoFrame {
        rotation: VideoRotation::VideoRotation0,
        timestamp: 0,
        buffer: I420Buffer::new(1280, 720),
    }));

    while let Ok(sample) = app_sink.pull_sample() {
        if let Some(sample_buffer) = sample.buffer() {
            let mut frame = frame.lock().await;
            let i420_buffer = &mut frame.buffer;

            let (dst_y, dst_u, dst_v) = i420_buffer.data_mut();

            let caps = sample.caps().unwrap();
            let video_info = gstreamer_video::VideoInfo::from_caps(caps).unwrap();
            let mut ts = 0;
            if let Some(meta) = sample_buffer.meta::<ReferenceTimestampMeta>() {
                ts = meta.timestamp().into_raw_value();
            }
            let sample_frame =
                gstreamer_video::video_frame::VideoFrame::from_buffer_readable(
                    sample_buffer.copy(),
                    &video_info,
                ).unwrap();

            let src_y = sample_frame.plane_data(0).unwrap();
            let src_u = sample_frame.plane_data(1).unwrap();
            let src_v = sample_frame.plane_data(2).unwrap();

            dst_y.copy_from_slice(src_y);
            dst_u.copy_from_slice(src_u);
            dst_v.copy_from_slice(src_v);
            frame.timestamp = ts;

            rtc_source.capture_frame(&*frame);
        }
    }
}