use std::env;

use gstreamer::{Element, parse_bin_from_description, parse_bin_from_description_full, ParseFlags, Pipeline, State};
use gstreamer::prelude::{Cast, ElementExt, ElementExtManual, GstBinExt, GstObjectExt};
use gstreamer::prelude::PadExt;
use gstreamer::State::Null;
use gstreamer_rtp::gst;

#[tokio::main]
async fn main() {
    let url = env::var("LIVEKIT_URL").unwrap();
    let api_key = env::var("LIVEKIT_API_KEY").unwrap();
    let api_secret = env::var("LIVEKIT_API_SECRET").unwrap();
    gst::init().unwrap();

    let pipeline = Pipeline::new();
    pipeline.set_state(State::Playing).unwrap();

    let sink0 = parse_bin_from_description_full(
        format!(r#"livekitwebrtcsink
            name=lk0
            signaller::ws-url={url}
            signaller::api-key={api_key}
            signaller::secret-key={api_secret}
            signaller::room-name=Default
            signaller::identity=Gstreamer"#
        ).as_str(),
        false,
        None,
        ParseFlags::NO_SINGLE_ELEMENT_BINS,
    ).expect("Unable to parse sink bin");

    let sink1 = parse_bin_from_description_full(
        format!(r#"livekitwebrtcsink
            name=lk1
            signaller::ws-url={url}
            signaller::api-key={api_key}
            signaller::secret-key={api_secret}
            signaller::room-name=Default
            signaller::identity=Gstreamer"#
        ).as_str(),
        false,
        None,
        ParseFlags::NO_SINGLE_ELEMENT_BINS,
    ).expect("Unable to parse sink bin");

    let pad_template0 = sink0.pad_template("video_%u")
        .expect("Unable to get sink pad template");
    let sink_pad0 = sink0.request_pad(&pad_template0, Some("video_0"), None)
        .expect("Unable to request sink_pad video_0");

    let pad_template1 = sink1.pad_template("video_%u")
        .expect("Unable to get sink pad template");
    let _sink_pad1 = sink1.request_pad(&pad_template1, Some("video_0"), None)
        .expect("Unable to request sink_pad");

    pipeline.add(&sink0).unwrap();
    pipeline.add(&sink1).unwrap();
    sink0.sync_state_with_parent().unwrap();
    sink1.sync_state_with_parent().unwrap();

    let src_text = r#"videotestsrc num-buffers=200000000 ! tee name = t
            t. ! queue name=q0
            t. ! queue name=q1"#;
    let src_bin = parse_bin_from_description(
        &src_text,
        true,
    ).expect("Unable to parse src bin");

    let src_el: Element = src_bin.upcast();
    pipeline.add(&src_el).expect("Unable to add src bin");

    let src_pad = src_el.static_pad("src").expect("No matching static pad");
    src_pad.link(&sink_pad0).expect("Unable to link pads");

    src_el.sync_state_with_parent().expect("Unable to sync src state with parent");

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
                pipeline.set_state(Null).unwrap();
            }
            _ => (),
        }
    }

    pipeline.set_state(Null).unwrap();
}