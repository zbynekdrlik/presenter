//! Per-consumer GStreamer ELEMENT construction (leaf helpers, no
//! `ConsumerBranch` state): build the `appsrc → [vp8enc →] payloader →
//! webrtcbin` elements, add + link them into the consumer pipeline, adopt the
//! encoder timeline, connect the webrtcbin signals, and run the per-pipeline
//! `Latency` bus watch.
//!
//! Split out of `consumers.rs` (file-size cap) as a cohesive group of pure,
//! self-contained builders. The orchestrator
//! `consumers::build_consumer_pipeline_blocking` calls these and owns all the
//! `ConsumerBranch` lifecycle / negotiation logic, which STAYS in `consumers`.

use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_utils::StreamProducer;
use gstreamer_webrtc as gst_webrtc;
use tokio::sync::mpsc::UnboundedSender;

use super::adaptive::START_BPS;
use super::build::{compat_raw_caps, consumer_h264_caps};
use super::StreamProfile;
use crate::whep_session::{IceCandidate, WhepConnectionState};

/// Add the per-consumer elements to the consumer pipeline: `appsrc`,
/// `[vp8enc]` (COMPAT only — #387), `payloader`, `webrtcbin`.
pub(super) fn add_consumer_elements(
    pipeline: &gst::Pipeline,
    appsrc: &gst_app::AppSrc,
    encoder: Option<&gst::Element>,
    payloader: &gst::Element,
    webrtcbin: &gst::Element,
) -> Result<()> {
    pipeline
        .add_many([appsrc.upcast_ref::<gst::Element>(), payloader, webrtcbin])
        .context("add appsrc+payloader+webrtcbin to consumer pipeline")?;
    if let Some(encoder) = encoder {
        pipeline
            .add(encoder)
            .context("add per-consumer vp8enc to consumer pipeline")?;
    }
    Ok(())
}

/// Link the per-consumer chain: `appsrc → [vp8enc →] payloader → webrtcbin`.
/// For COMPAT the appsrc carries RAW I420 and the per-consumer `vp8enc`
/// encodes it (#387); for DEFAULT the appsrc carries encoded H264 straight
/// into the payloader. The pay→webrtc link is filtered to the codec's
/// application/x-rtp caps (payload OMITTED — re-aligned later in
/// `align_payload_type`).
pub(super) fn link_consumer_elements(
    appsrc: &gst_app::AppSrc,
    encoder: Option<&gst::Element>,
    payloader: &gst::Element,
    webrtcbin: &gst::Element,
    encoding_name: &str,
) -> Result<()> {
    let appsrc_el = appsrc.upcast_ref::<gst::Element>();
    match encoder {
        Some(encoder) => {
            gst::Element::link_many([appsrc_el, encoder, payloader])
                .context("link appsrc -> vp8enc -> payloader")?;
        }
        None => {
            appsrc_el
                .link(payloader)
                .context("link appsrc -> payloader")?;
        }
    }
    let rtp_caps = gst::Caps::builder("application/x-rtp")
        .field("media", "video")
        .field("encoding-name", encoding_name)
        .field("clock-rate", 90_000i32)
        .build();
    payloader
        .link_filtered(webrtcbin, &rtp_caps)
        .context("link payloader -> webrtcbin (codec caps)")
}

/// Put a consumer pipeline on the ENCODER pipeline's clock + base-time so the
/// producer's forwarded buffer timestamps (PTS + segment, preserved by
/// StreamProducer's push_sample) are valid on this pipeline's timeline.
/// set_start_time(NONE) stops the PLAYING transition from re-selecting a
/// base-time. This mirrors webrtcsink's session-pipeline setup verbatim.
pub(super) fn adopt_encoder_timeline(
    consumer_pipeline: &gst::Pipeline,
    enc_clock: Option<gst::Clock>,
    enc_base_time: Option<gst::ClockTime>,
    session_id: &str,
) {
    if let Some(clock) = &enc_clock {
        consumer_pipeline.use_clock(Some(clock));
    }
    consumer_pipeline.set_start_time(gst::ClockTime::NONE);
    match enc_base_time {
        Some(base) => consumer_pipeline.set_base_time(base),
        None => {
            // Defensive: should not happen (WHEP POSTs are gated on the encoder
            // pipeline being Streaming). Surface it loudly — timestamps would
            // be on the wrong timeline.
            tracing::warn!(
                session_id = %session_id,
                "encoder pipeline has no base-time; consumer timeline may be wrong"
            );
        }
    }
}

/// Spawn the per-consumer-pipeline bus watch: services `Latency` messages with
/// `recalculate_latency()` (via `call_async`, exactly like webrtcsink) and
/// logs pipeline errors. The task never ends on its own (it holds the bus
/// alive); the explicit abort — by `ConsumerBranch`/`WhepSession` Drop — is
/// its ONLY exit.
pub(super) fn spawn_consumer_bus_watch(
    consumer_pipeline: &gst::Pipeline,
    session_id: &str,
    rt: &tokio::runtime::Handle,
) -> Result<tokio::task::JoinHandle<()>> {
    let bus = consumer_pipeline
        .bus()
        .ok_or_else(|| anyhow!("consumer pipeline has no bus"))?;
    let pipeline_weak = consumer_pipeline.downgrade();
    let sid = session_id.to_string();
    Ok(rt.spawn(async move {
        use futures_util::StreamExt;
        let mut stream = bus.stream();
        while let Some(msg) = stream.next().await {
            match msg.view() {
                gst::MessageView::Latency(_) => {
                    if let Some(pipeline) = pipeline_weak.upgrade() {
                        // call_async: run on a GStreamer worker thread, never
                        // blocking this task or risking a state-lock deadlock.
                        pipeline.call_async(|p| {
                            let _ = p.recalculate_latency();
                        });
                    }
                }
                gst::MessageView::Error(err) => {
                    tracing::warn!(
                        session_id = %sid,
                        error = %err.error(),
                        debug = ?err.debug(),
                        "consumer pipeline error (client watchdog will reconnect)"
                    );
                }
                _ => {}
            }
        }
    }))
}

/// Build the per-consumer elements: an `appsrc` (caps matching the requested
/// profile branch's producer appsink — byte-stream/AU H264 for Default, RAW
/// I420 854×480 for Compat), an OPTIONAL per-consumer `vp8enc`
/// (`venc_<session>`) for Compat ONLY (#387 — DEFAULT shares the H264
/// encoder), a per-consumer payloader (`rtph264pay` / `rtpvp8pay`) pre-seated
/// on the offer's dynamic payload type `pt`, and a `webrtcbin`.
///
/// Returns `(appsrc, Option<encoder>, payloader, webrtcbin)`. The encoder is
/// `Some` for COMPAT (its `target-bitrate` is driven per-TV by
/// `CompatBitrateController`) and `None` for DEFAULT.
#[allow(clippy::type_complexity)]
pub(super) fn build_consumer_elements(
    session_id: &str,
    profile: StreamProfile,
    pt: u32,
) -> Result<(
    gst_app::AppSrc,
    Option<gst::Element>,
    gst::Element,
    gst::Element,
)> {
    // Initial caps match the profile branch's producer appsink caps filter so
    // the very first forwarded sample agrees (`consumer_h264_caps` /
    // `compat_raw_caps` pin BOTH sides of each bridge). Compat now carries RAW
    // I420 (the consumer encodes VP8 itself — #387).
    let bridge_caps = match profile {
        StreamProfile::Default => consumer_h264_caps(),
        StreamProfile::Compat => compat_raw_caps(),
    };
    let appsrc = gst_app::AppSrc::builder()
        .name(format!("src_{session_id}"))
        .caps(&bridge_caps)
        .build();
    // CRITICAL ORDER: apply the consumer configuration (is-live=true,
    // format=time, leaky downstream, 500ms queue bound) NOW — BEFORE the
    // pipeline transitions to PLAYING. basesrc latches `live_running` only at
    // the PAUSED→PLAYING transition and ONLY if the source is already live;
    // if `is_live` flips to true afterwards (which is what happened when
    // StreamProducer::add_consumer — which calls this internally — ran after
    // PLAYING), the appsrc's task blocks in "live source waiting for running
    // state" FOREVER and not a single buffer is ever pushed downstream —
    // connected, but black. add_consumer re-applies the same configuration
    // later, which is a harmless no-op. Holds identically for the RAW compat
    // appsrc.
    StreamProducer::configure_consumer(&appsrc);

    // COMPAT: this consumer's OWN vp8enc (#387) so its bitrate adapts per-TV.
    let encoder = match profile {
        StreamProfile::Default => None,
        StreamProfile::Compat => Some(build_compat_vp8_encoder(session_id)?),
    };

    // Per-consumer payloader so each webrtcbin negotiates its own dynamic
    // payload type with its browser (#336); pre-seated on the browser's
    // offered pt for the profile's codec.
    let payloader = match profile {
        StreamProfile::Default => build_h264_payloader(session_id, pt)?,
        StreamProfile::Compat => build_vp8_payloader(session_id, pt)?,
    };

    let webrtcbin = gst::ElementFactory::make("webrtcbin")
        .name(session_id)
        // max-bundle: audio + video on ONE ICE/DTLS transport, matching the
        // browser's `a=group:BUNDLE` offer (default `none` hangs the 2nd DTLS
        // handshake → connecting forever → black).
        .property_from_str("bundle-policy", "max-bundle")
        // Explicit jitterbuffer/session latency (200 ms is webrtcbin's own
        // default; set explicitly so the value is visible and stable).
        .property("latency", 200u32)
        .build()
        .context("build webrtcbin")?;

    Ok((appsrc, encoder, payloader, webrtcbin))
}

/// Build ONE compat consumer's `vp8enc` ("venc_<session>") with libwebrtc-
/// parity realtime tuning (#387). Each weak TV gets its OWN encoder so its
/// `target-bitrate` is driven independently by that consumer's
/// `CompatBitrateController` from its OWN RTCP loss/RTT — a SHARED encoder
/// could only serve the worst TV's bitrate to all of them. STARTS HIGH at
/// `START_BPS` (900k) per the quality policy (reduce only on measured loss).
/// Property types verified via gst-inspect-1.0 and locked by
/// `compat_consumer_has_per_consumer_adaptive_vp8enc`:
///
/// - `deadline=1` (µs/frame, Integer64): libvpx realtime mode.
/// - `cpu-used=8`: fastest realtime encode preset (quality ↓, speed ↑).
/// - `end-usage=cbr` + `target-bitrate=START_BPS` (bits/sec): constant-bitrate
///   like libwebrtc's rate controller; the controller sets it live thereafter.
/// - `keyframe-max-dist=240`: GOP parity with the H264 branch; joins are
///   served by `request_keyframe`, not scheduled keyframe pulses.
/// - `token-partitions="4"`: partitioned token coding lets the TV's libvpx
///   decoder spread entropy decode across its 4 cores (the VDO.Ninja delta).
/// - `threads=4`: one encode thread per partition on the server.
/// - `error-resilient=default` (flags): frames stay decodable after loss.
/// - `lag-in-frames=0`: zero lookahead — no encoder-side frame delay.
fn build_compat_vp8_encoder(session_id: &str) -> Result<gst::Element> {
    gst::ElementFactory::make("vp8enc")
        .name(format!("venc_{session_id}"))
        .property("deadline", 1i64)
        .property("cpu-used", 8i32)
        .property_from_str("end-usage", "cbr")
        .property("target-bitrate", START_BPS)
        .property("keyframe-max-dist", 240i32)
        .property_from_str("token-partitions", "4")
        .property("threads", 4i32)
        .property_from_str("error-resilient", "default")
        .property("lag-in-frames", 0i32)
        .build()
        .with_context(|| format!("build per-consumer vp8enc venc_{session_id}"))
}

/// Per-consumer `rtph264pay`. config-interval=-1 resends SPS/PPS before every
/// IDR; aggregate-mode=zero-latency is webrtcsink parity (aggregate NALs only
/// until a VCL unit is complete — never hold a frame's data back for packing
/// efficiency).
fn build_h264_payloader(session_id: &str, pt: u32) -> Result<gst::Element> {
    gst::ElementFactory::make("rtph264pay")
        .name(format!("pay_{session_id}"))
        .property("config-interval", -1i32)
        .property("pt", pt)
        .property_from_str("aggregate-mode", "zero-latency")
        .build()
        .context("build rtph264pay")
}

/// Per-consumer `rtpvp8pay` for compat consumers. Only the pt needs seating
/// — VP8 has no SPS/PPS-style config to re-insert and the payloader
/// fragments each frame (including its token partitions) per RFC 7741 as-is.
fn build_vp8_payloader(session_id: &str, pt: u32) -> Result<gst::Element> {
    gst::ElementFactory::make("rtpvp8pay")
        .name(format!("pay_{session_id}"))
        .property("pt", pt)
        .build()
        .context("build rtpvp8pay")
}

/// Connect the per-consumer webrtcbin signals: on-ice-candidate (forwards
/// candidates to `ice_tx`) and notify::connection-state (updates the shared
/// `connection_state`). Both fire from a GStreamer streaming thread.
pub(super) fn connect_branch_signals(
    webrtcbin: &gst::Element,
    ice_tx: UnboundedSender<IceCandidate>,
    connection_state: Arc<std::sync::Mutex<WhepConnectionState>>,
    session_id: String,
) {
    // on-ice-candidate signature: void(webrtcbin, sdp_mline_index: u32, candidate: &str)
    webrtcbin.connect("on-ice-candidate", false, move |args| {
        let sdp_mline_index = args.get(1).and_then(|v| v.get::<u32>().ok()).unwrap_or(0);
        let candidate = args
            .get(2)
            .and_then(|v| v.get::<String>().ok())
            .unwrap_or_default();
        let _ = ice_tx.send(IceCandidate {
            sdp_mline_index,
            candidate,
        });
        None
    });

    // notify::connection-state fires from a GStreamer streaming thread (raw
    // std::thread) — use std::sync::Mutex directly. On poison recover the guard.
    webrtcbin.connect_notify(Some("connection-state"), move |webrtcbin, _pspec| {
        let gst_state =
            webrtcbin.property::<gst_webrtc::WebRTCPeerConnectionState>("connection-state");
        let our_state = WhepConnectionState::from(gst_state);
        *connection_state.lock().unwrap_or_else(|p| p.into_inner()) = our_state;
        tracing::debug!(
            session_id = %session_id,
            state = ?our_state,
            "WHEP consumer connection-state changed"
        );
    });
}
