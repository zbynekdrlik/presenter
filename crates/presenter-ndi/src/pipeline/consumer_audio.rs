//! Consumer-side AUDIO CLOCK ANCHOR media: build the `audio appsrc →
//! rtpopuspay → webrtcbin` branch added alongside the video in each consumer
//! pipeline. Split from `consumers.rs` (which owns the consumer lifecycle) so
//! both files stay under the project's file-size cap.
//!
//! WHY the anchor exists: a video-only WebRTC stream has no clock-drift
//! compensation in Chromium — the only resampler lives in the audio NetEq, and
//! libwebrtc's stream synchronizer early-returns without an audio track — so the
//! receiver jitter buffer drifts unbounded (latency to >1s). Sending a
//! continuous Opus track on the SAME pipeline clock + timeline as the video
//! (the encoder pipeline's `producer_audio`, NDI source audio mixed with
//! silence) makes the browser run its NetEq audio device clock and slave the
//! video to it, bounding the buffer.

use anyhow::{Context, Result};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_utils::StreamProducer;

use super::build::consumer_opus_caps;
use super::negotiation::parse_opus_payload_type;

/// Build the AUDIO clock-anchor media (`audio appsrc → rtpopuspay → webrtcbin`)
/// and add+link it into the consumer's pipeline + webrtcbin — the second media
/// alongside the video. Returns `Some((appsrc, payloader))` so the caller can
/// connect the appsrc to `producer_audio` after PLAYING and align the Opus pt
/// after negotiation; returns `None` when the offer has no Opus rtpmap
/// (video-only fallback — the anchor is best-effort, never a join-blocker).
///
/// The audio appsrc is configured LIVE via `StreamProducer::configure_consumer`
/// BEFORE the pipeline reaches PLAYING — the SAME is-live ordering invariant the
/// video appsrc relies on (flipping is-live after PLAYING strands the appsrc's
/// task "waiting for running state" forever, pushing zero buffers).
pub(super) fn add_audio_media(
    consumer_pipeline: &gst::Pipeline,
    webrtcbin: &gst::Element,
    offer_str: &str,
    session_id: &str,
) -> Result<Option<(gst_app::AppSrc, gst::Element)>> {
    let Some(opus_pt) = parse_opus_payload_type(offer_str) else {
        tracing::warn!(
            session_id = %session_id,
            "WHEP offer carries no Opus rtpmap — audio clock anchor skipped \
             (video-only; the receiver jitter buffer may drift unbounded)"
        );
        return Ok(None);
    };
    let appsrc = gst_app::AppSrc::builder()
        .name(format!("asrc_{session_id}"))
        .caps(&consumer_opus_caps())
        .build();
    // Configure LIVE before any state change (see fn doc / the video appsrc's
    // CRITICAL ORDER note in build_consumer_elements).
    StreamProducer::configure_consumer(&appsrc);
    let payloader = build_opus_payloader(session_id, opus_pt)?;
    consumer_pipeline
        .add_many([appsrc.upcast_ref::<gst::Element>(), &payloader])
        .context("add audio appsrc+rtpopuspay to consumer pipeline")?;
    appsrc
        .upcast_ref::<gst::Element>()
        .link(&payloader)
        .context("link audio appsrc -> rtpopuspay")?;
    // rtpopuspay → webrtcbin, filtered to the Opus application/x-rtp caps
    // (payload OMITTED — re-aligned to the negotiated pt in align_payload_type).
    let rtp_caps = gst::Caps::builder("application/x-rtp")
        .field("media", "audio")
        .field("encoding-name", "OPUS")
        .field("clock-rate", 48_000i32)
        .build();
    payloader
        .link_filtered(webrtcbin, &rtp_caps)
        .context("link rtpopuspay -> webrtcbin (audio caps)")?;
    tracing::info!(session_id = %session_id, opus_pt, "audio clock anchor wired into webrtcbin");
    Ok(Some((appsrc, payloader)))
}

/// Per-consumer `rtpopuspay` for the audio clock anchor, pre-seated on the
/// offer's dynamic Opus pt. Only the pt needs seating — Opus has no
/// config-block to re-insert; the payloader packetizes each Opus frame as-is.
fn build_opus_payloader(session_id: &str, pt: u32) -> Result<gst::Element> {
    gst::ElementFactory::make("rtpopuspay")
        .name(format!("apay_{session_id}"))
        .property("pt", pt)
        .build()
        .context("build rtpopuspay")
}
