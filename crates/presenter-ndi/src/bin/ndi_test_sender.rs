//! Synthetic NDI source for deterministic E2E testing.
//!
//! Publishes a `videotestsrc` pattern as an NDI sender so CI (and local dev)
//! can exercise the full NDI → encode → WHEP → browser-decode path WITHOUT
//! depending on an external broadcaster (Resolume) being live. Used by the
//! `e2e-ndi` self-hosted CI lane (see `.github/workflows/pipeline.yml`).
//!
//! `ndisink`/`ndisinkcombiner` are part of the statically-registered
//! `gst-plugin-ndi` (default `sink` feature), so they become available after
//! `presenter_ndi::init()` — `gst-inspect-1.0` cannot see them because the
//! plugin is registered in-process, not as a system `.so`.
//!
//! Usage:
//!   PRESENTER_NDI_TEST_NAME="PRESENTER-TEST" \
//!   cargo run -p presenter-ndi --features test-helpers --bin ndi_test_sender
//!
//! Runs until killed (SIGINT/SIGTERM). Logs the published NDI name on startup.

use anyhow::{anyhow, Context, Result};
use gstreamer as gst;
use gstreamer::prelude::*;

fn main() -> Result<()> {
    // Registers gstreamer + the in-process gst-plugin-ndi (with ndisink).
    presenter_ndi::init().context("presenter_ndi::init failed")?;

    let ndi_name =
        std::env::var("PRESENTER_NDI_TEST_NAME").unwrap_or_else(|_| "PRESENTER-TEST".to_string());

    // videotestsrc (live) → UYVY 1280x720@30 → ndisinkcombiner → ndisink
    let src = gst::ElementFactory::make("videotestsrc")
        .property("is-live", true)
        .property_from_str("pattern", "smpte")
        .build()
        .context("build videotestsrc")?;
    let convert = gst::ElementFactory::make("videoconvert")
        .build()
        .context("build videoconvert")?;
    let capsfilter = gst::ElementFactory::make("capsfilter")
        .property(
            "caps",
            gst::Caps::builder("video/x-raw")
                .field("format", "UYVY")
                .field("width", 1280i32)
                .field("height", 720i32)
                .field("framerate", gst::Fraction::new(30, 1))
                .build(),
        )
        .build()
        .context("build capsfilter")?;
    let combiner = gst::ElementFactory::make("ndisinkcombiner")
        .build()
        .context("build ndisinkcombiner")?;
    let sink = gst::ElementFactory::make("ndisink")
        .property("ndi-name", &ndi_name)
        .build()
        .context("build ndisink")?;

    let pipeline = gst::Pipeline::new();
    pipeline
        .add_many([&src, &convert, &capsfilter, &combiner, &sink])
        .context("add elements")?;
    gst::Element::link_many([&src, &convert, &capsfilter]).context("link src→convert→caps")?;
    // ndisinkcombiner has an always "video" sink pad.
    let combiner_video = combiner
        .static_pad("video")
        .ok_or_else(|| anyhow!("ndisinkcombiner has no 'video' pad"))?;
    let caps_src = capsfilter
        .static_pad("src")
        .ok_or_else(|| anyhow!("capsfilter has no src pad"))?;
    caps_src
        .link(&combiner_video)
        .map_err(|e| anyhow!("link capsfilter→ndisinkcombiner.video: {e:?}"))?;
    combiner.link(&sink).context("link combiner→ndisink")?;

    let bus = pipeline
        .bus()
        .ok_or_else(|| anyhow!("pipeline has no bus"))?;
    if let Err(e) = pipeline.set_state(gst::State::Playing) {
        // Surface the real element error from the bus rather than the opaque
        // "Element failed to change its state".
        let detail = bus
            .timed_pop_filtered(gst::ClockTime::from_seconds(3), &[gst::MessageType::Error])
            .and_then(|m| match m.view() {
                gst::MessageView::Error(err) => Some(format!(
                    "{} ({:?}) from {:?}",
                    err.error(),
                    err.debug(),
                    err.src().map(|s| s.path_string())
                )),
                _ => None,
            })
            .unwrap_or_else(|| format!("{e:?}"));
        let _ = pipeline.set_state(gst::State::Null);
        return Err(anyhow!("pipeline failed to start: {detail}"));
    }
    println!("synthetic NDI sender publishing ndi-name=\"{ndi_name}\" (Ctrl-C to stop)");

    // Run until the bus posts EOS/Error or the process is signalled.
    for msg in bus.iter_timed(gst::ClockTime::NONE) {
        use gst::MessageView;
        match msg.view() {
            MessageView::Error(err) => {
                let _ = pipeline.set_state(gst::State::Null);
                return Err(anyhow!(
                    "pipeline error from {:?}: {}",
                    err.src().map(|s| s.path_string()),
                    err.error()
                ));
            }
            MessageView::Eos(_) => break,
            _ => {}
        }
    }
    pipeline
        .set_state(gst::State::Null)
        .context("set pipeline Null")?;
    Ok(())
}
