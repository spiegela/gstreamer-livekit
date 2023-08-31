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

    let sink = parse_bin_from_description_full(
        format!(r#"livekitwebrtcsink
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


    let pad_template = sink.pad_template("video_%u").expect("Unable to get sink pad template");
    let patterns = vec!["smpte", "snow", "ball"];
    let pads = patterns.iter().enumerate().map(|(i, _p)| {
        let pad_name = format!("video_{}", i);
        sink.request_pad(&pad_template, Some(pad_name.as_str()), None).expect("Unable to request sink_pad")
    }).collect::<Vec<_>>();

    pipeline.add(&sink).unwrap();
    sink.sync_state_with_parent().unwrap();

    for (i, p) in patterns.iter().enumerate() {
        let src_text = format!(r#"videotestsrc pattern={} num-buffers=200000000 ! videoconvert ! video/x-raw ! queue"#, p);
        let src_bin = parse_bin_from_description(
            &src_text,
            true,
        ).expect("Unable to parse src bin");

        let src_el: Element = src_bin.upcast();
        pipeline.add(&src_el).expect("Unable to add src bin");

        let src_pad = src_el.static_pad("src").expect("No matching static pad");
        src_pad.link(&pads[i]).expect("Unable to link pads");

        src_el.sync_state_with_parent().expect("Unable to sync src state with parent");
    }

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