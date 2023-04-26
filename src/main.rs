use std::collections::HashMap;
use std::ffi::OsStr;
use std::sync::{Arc};

use anyhow::anyhow;
use gstreamer::Element;
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
use livekit_webrtc::native::yuv_helper;
use tokio::sync::Mutex;

use {
    gstreamer::{ElementFactory, Pipeline, State},
    gstreamer_app::AppSink,
};

#[tokio::main]
async fn main() {
    let livekit_url = getenv("LIVEKIT_URL").unwrap();
    let liveki_api_url = getenv("LIVEKIT_API_URL").unwrap();
    let livekit_api_key = getenv("LIVEKIT_API_KEY").unwrap();
    let livekit_api_secret = getenv("LIVEKIT_API_SECRET").unwrap();

    tokio::task::spawn_blocking(|| async move {
        let livekit_token = create_join_token(
            &liveki_api_url,
            &livekit_api_key,
            &livekit_api_secret,
            "Default",
            "gstreamer",
        ).await.unwrap();
        let source = NativeVideoSource::default();
        let track = LocalVideoTrack::create_video_track(
            "gstreamer-test",
            VideoCaptureOptions::default(),
            source.clone(),
        );
        tokio::spawn(track_task(source.clone()));
        let (room, _event_ch) = Room::connect(
            livekit_url.to_str().unwrap(),
            livekit_token.as_str(),
        ).await.unwrap();
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
    }).await.unwrap().await;
}

async fn track_task(rtc_source: NativeVideoSource) {
    let (height, width) = (720, 1280);
    gst::init().unwrap();
    let videotestsrc = ElementFactory::make("videotestsrc")
        .property("num-buffers", &200)
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
    Element::link_many(&[&videotestsrc, &caps_filter, &appsink]).unwrap();

    let app_sink = appsink.dynamic_cast::<AppSink>().unwrap();

    let frame = Arc::new(Mutex::new(VideoFrame {
        rotation: VideoRotation::VideoRotation0,
        timestamp: 0,
        buffer: I420Buffer::new(1280, 720),
    }));

    pipeline.set_state(State::Playing).unwrap();

    while let Ok(sample) = app_sink.pull_sample() {
        if let Some(sample_buffer) = sample.buffer() {
            let mut frame = frame.lock().await;
            let i420_buffer = &mut frame.buffer;

            let dst_stride_y = &i420_buffer.stride_y();
            let dst_stride_u = &i420_buffer.stride_u();
            let dst_stride_v = &i420_buffer.stride_v();
            let (dst_y, dst_u, dst_v) = i420_buffer.data_mut();

            let caps = sample.caps().unwrap();
            let video_info = gstreamer_video::VideoInfo::from_caps(&caps).unwrap();
            let sample_frame =
                gstreamer_video::video_frame::VideoFrame::from_buffer_readable(
                    sample_buffer.copy(),
                    &video_info,
                ).unwrap();

            let src_y = sample_frame.plane_data(0).unwrap();
            let src_u = sample_frame.plane_data(1).unwrap();
            let src_v = sample_frame.plane_data(2).unwrap();
            let src_stride_y = sample_frame.plane_stride()[0];
            let src_stride_u = sample_frame.plane_stride()[1];
            let src_stride_v = sample_frame.plane_stride()[2];

            // dst_stride_y = &src_stride_y;
            // dst_y.copy_from_slice(src_y);
            // dst_stride_u = &src_stride_u;
            // dst_u.copy_from_slice(src_u);
            // dst_stride_v = &src_stride_v;
            // dst_v.copy_from_slice(src_v);

            let mut argb_data = vec![0u8; width * height * 4];
            let argb = argb_data.as_mut_slice();
            let argb_stride: i32 = 0;

            yuv_helper::i420_to_argb(
                src_y,
                src_stride_y,
                src_u,
                src_stride_u,
                src_v,
                src_stride_v,
                argb,
                argb_stride,
                width as i32,
                height as i32,
            ).unwrap();

            yuv_helper::argb_to_i420(
                argb,
                argb_stride,
                dst_y,
                *dst_stride_y,
                dst_u,
                *dst_stride_u,
                dst_v,
                *dst_stride_v,
                width as i32,
                height as i32,
            ).unwrap();

            rtc_source.capture_frame(&*frame);
        }
    }
}

async fn create_join_token(
    server_url: &OsStr,
    api_key: &OsStr,
    api_secret: &OsStr,
    room_name: &str,
    identity: &str,
) -> anyhow::Result<String> {
    let output = tokio::process::Command::new("livekit-cli")
        .envs(HashMap::from([
            (OsStr::new("LIVEKIT_API_KEY"), api_key),
            (
                OsStr::new("LIVEKIT_API_SECRET"),
                api_secret,
            ),
            (
                OsStr::new("LIVEKIT_SERVER_URL"),
                server_url,
            ),
        ]))
        .arg("create-token")
        .arg("--join")
        .arg("--room")
        .arg(room_name)
        .arg("--identity")
        .arg(identity)
        .output()
        .await?;
    match output.status.success() {
        true => {
            let stdout = String::from_utf8(output.stdout).unwrap();
            for line in stdout.lines() {
                if !line.starts_with("access token: ") {
                    continue;
                }
                let token = line
                    .split_whitespace()
                    .nth(2)
                    .ok_or(anyhow!("failed to find access_token in output: {}", line))?;
                return Ok(token.to_string());
            }
            Err(anyhow!("failed to find access_token in output: {}", stdout))
        }
        false => {
            let stderr = String::from_utf8(output.stderr).unwrap();
            Err(anyhow!("failed to create join token: {}", stderr))
        }
    }
}
