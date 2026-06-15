//! Per-source GStreamer pipeline: one shared-encoder ENCODER pipeline plus one
//! FRESH pipeline per WHEP consumer, bridged by `gstreamer_utils::StreamProducer`.
//!
//! Each `NdiPipeline` instance corresponds to ONE active NDI source. The
//! ENCODER pipeline is built once and NEVER modified afterwards:
//!
//! ```text
//! ndisrc → ndisrcdemux → videoconvert → videoscale → caps(NV12,720p) → raw_tee
//!                audio ↘ fakesink
//!  (default) raw_tee → q_default → encoder → profile_caps → h264parse
//!                        → enc_appsink                  (1280×720 H264)
//!  (compat)  raw_tee → q_compat → videorate → videoconvert → videoscale
//!                        → compat_scale_caps(I420 854×480 @20)
//!                        → encoder_compat(vp8enc)
//!                        → enc_appsink_compat   (854×480@20 realtime VP8)
//!                       (one encoder per PROFILE — never per consumer)
//!                                      StreamProducer fanout (one per profile)
//!                                                            ▼
//!   per consumer (its OWN gst::Pipeline): appsrc → rtph264pay|rtpvp8pay
//!                                                            → webrtcbin
//! ```
//!
//! Profile rule (realtime-VP8 pivot): a consumer is served the profile its
//! WHEP POST requested via the `?profile=compat` query ([`StreamProfile`],
//! parsed at the HTTP layer) — compat is the VDO.Ninja-style realtime VP8
//! stream for weak TVs whose H264 OMX decoder is vendor-broken; every other
//! consumer gets the default 720p H264 stream. The profile IMPLIES the codec
//! ([`StreamProfile::encoding_name`]); the WHEP answer dictates it to the
//! browser (every browser offer carries both H264 and VP8).
//!
//! Why per-consumer pipelines: a `webrtcbin` added to an already-running LIVE
//! pipeline never gets its rtpsession's running-time/latency configured, so
//! every straggler/reconnect connected but received ZERO RTP — the #373
//! black-stage bug. Running each consumer in its OWN pipeline (sharing the
//! encoder pipeline's clock + base-time, with a per-pipeline `Latency` bus
//! handler) makes the latency configuration deterministic. This is EXACTLY the
//! architecture of gst-plugin-rs `webrtcsink` (one session pipeline per peer,
//! `StreamProducer` bridge, `Latency` message → `recalculate_latency()`), which
//! is the reference implementation this design follows.
//!
//! The appsink→appsrc bridge is `gstreamer_utils::StreamProducer` — the same
//! battle-tested fanout `webrtcsink` uses. It forwards samples (caps, segment
//! and PTS preserved), propagates the producer's latency to every consumer
//! appsrc, gates each new consumer on a keyframe, and forwards the browser's
//! force-keyunit (PLI) requests upstream to the shared encoder.
//!
//! Per-consumer state lives in `WhepSession` (`whep_session.rs`). The pipeline
//! owns the shared encoder + the producer and a `tokio::sync::Mutex<HashMap<
//! String, WhepSession>>` of active sessions.
//!
//! Structure: the type definitions live here in the module root; the
//! `impl NdiPipeline` methods are split across focused submodules
//! (`build`, `lifecycle`, `consumers`) to keep each file well under the
//! project's file-size cap. Private struct fields stay accessible to those
//! submodules because they are descendants of this module.

use std::collections::HashMap;
use std::sync::Arc;

use gstreamer as gst;
use gstreamer_utils::StreamProducer;
use tokio::sync::watch;

use crate::whep_session::{IceCandidate, WhepConnectionState, WhepSession};

mod build;
mod consumers;
mod lifecycle;
mod negotiation;
mod reaper;

/// Pipeline lifecycle state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineState {
    /// Built but not yet PLAYING (waiting for ASYNC_DONE).
    Starting,
    /// PLAYING — WHEP endpoint is live and accepting subscribers.
    Streaming,
    /// Tearing down or torn down.
    Stopped,
    /// Error state — pipeline failed and must be recreated.
    Errored(String),
}

/// Answer returned by `add_consumer` to the HTTP WHEP shim.
pub struct WhepAnswer {
    pub session_id: String,
    pub sdp_answer: String,
    pub initial_candidates: Vec<IceCandidate>,
}

/// Which encode branch serves a WHEP consumer. Selected by the CLIENT via
/// the `?profile=compat` query parameter on its WHEP POST (parsed at the
/// HTTP layer with [`StreamProfile::from_query`]); `add_consumer` maps it
/// to the matching branch's `StreamProducer`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StreamProfile {
    /// 1280×720 H264 @ 2.5 Mbps — the primary stream every healthy client uses.
    #[default]
    Default,
    /// 854×480@20 realtime VP8 @ 900 kbps — the compat stream for weak TVs
    /// whose MStar H264 OMX decoder is vendor-broken (even an exactly-
    /// 640×480 H264 stream dies after ~5s). VDO.Ninja's libwebrtc VP8 plays
    /// smoothly on the SAME TVs, so this branch mirrors its realtime stream
    /// properties — above all token-partitions=4 for multithreaded software
    /// decode across the TV's 4 cores (see `build::build_compat_vp8_encoder`).
    Compat,
}

impl StreamProfile {
    /// Parse the WHEP `profile` query value: `"compat"` selects the compat
    /// branch; absent or ANY other value selects Default — an unknown
    /// profile string must degrade to the primary stream, never break a
    /// display's join.
    pub fn from_query(value: Option<&str>) -> Self {
        // TEST OVERRIDE (VP8 stutter test, 2026-06-15): the fallback is
        // Compat (software VP8) so EVERY client gets VP8 regardless of its
        // cached `ndiCodecMode` localStorage — the only way a default-pinned
        // TV picks up the VP8 branch without clearing its storage. Explicit
        // `?profile=default` still forces the 720p H264 branch (laptop control).
        // REVERT this fallback to `Self::Default` once the VP8 verdict is in.
        if value == Some("default") {
            Self::Default
        } else {
            Self::Compat
        }
    }

    /// The RTP encoding-name of the codec this profile streams. The profile
    /// IMPLIES the codec since the realtime-VP8 compat pivot: Default → H264
    /// (720p hw encode), Compat → VP8 (854×480 sw encode, token-partitioned).
    /// The consumer pipeline's appsrc caps, payloader, RTP caps and pt
    /// alignment all follow this value (see `consumers`).
    pub(crate) fn encoding_name(self) -> &'static str {
        match self {
            // Default → 720p hardware H264; Compat → software VP8 (854×480@20,
            // token-partitioned). Software-decoded VP8 produces a plain
            // sampleable texture the weak TVs' browser can ALWAYS paint and
            // composite smoothly — hardware-H264 on these TVs decodes to an
            // opaque overlay buffer that renders black (Cromite) or hitches
            // ~every 20s through the system WebView's libhwui compositor. VP8
            // is exactly the codec VDO.Ninja used, visible+smooth, on these TVs.
            Self::Default => "H264",
            Self::Compat => "VP8",
        }
    }
}

/// Soft consumer cap per NDI source. 9th consumer's POST returns 503 with
/// Retry-After: 60. Picked because realistic church setups have ≤6 stage
/// displays per source (choir, drums, vocals, side-screen, OBS browser
/// source, plus headroom). Prevents a buggy kiosk in a reconnect loop from
/// DoSing the encoder's pad fanout.
pub const MAX_CONSUMERS_PER_SOURCE: usize = 8;

/// Error returned by `add_consumer` and `add_consumer_stub`.
///
/// The HTTP shim maps `CapReached` to 503 + Retry-After: 60.
#[derive(Debug, thiserror::Error)]
pub enum AddConsumerError {
    /// The per-source soft cap was hit. The HTTP shim emits 503 + Retry-After.
    #[error("WHEP consumer cap reached ({max} per source) — try again later")]
    CapReached { max: usize },
    /// Any other pipeline or signalling error.
    #[error("{0}")]
    Other(#[from] anyhow::Error),
}

/// Snapshot of the pipeline state for the diagnostic route (Task 8 fills
/// `source_id`).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineSnapshot {
    pub source_id: String,
    pub state: String,
    pub encoder_factory: Option<String>,
    pub encoder_count: usize,
    pub consumer_count: usize,
    pub sessions: Vec<SessionSnapshot>,
}

/// Per-consumer snapshot entry.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSnapshot {
    pub id: String,
    pub connection_state: WhepConnectionState,
    /// Buffers forwarded to this consumer by the StreamProducer link.
    pub buffers_pushed: u64,
    /// Buffers dropped because the consumer appsrc queue was full.
    pub buffers_dropped: u64,
    /// RTCP receiver-report round-trip time (ms) — the display's link RTT.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rtcp_round_trip_ms: Option<f64>,
    /// RTCP receiver-report interarrival jitter (ms).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rtcp_jitter_ms: Option<f64>,
    /// RTCP receiver-report cumulative packets lost.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rtcp_packets_lost: Option<i64>,
}

/// Owns one GStreamer pipeline for one NDI source.
pub struct NdiPipeline {
    /// The ENCODER pipeline (ndisrc → … → encoder → h264parse → appsink).
    /// Built once and never modified when consumers come and go.
    pipeline: gst::Pipeline,
    /// WHEP URL that subscribers (browsers) POST to.
    whep_url: String,
    /// State observer for the manager / WS event emitter.
    state_tx: watch::Sender<PipelineState>,
    state_rx: watch::Receiver<PipelineState>,
    /// Bus watch task handle so we can cancel on stop/drop.
    ///
    /// Wrapped in `std::sync::Mutex` to allow interior-mutability access from
    /// `&self` methods (`start`, `stop`, `teardown`). This is necessary because
    /// `NdiPipeline` is stored inside `Arc<NdiPipeline>` in `ActiveSource`
    /// (the Arc lets WHEP HTTP handlers clone a pipeline reference out of the
    /// active-map mutex guard before calling blocking pipeline methods — the
    /// critical fix for the lock-held-across-await bug). `Arc` requires `&self`
    /// for shared access, so `&mut self` methods are incompatible. The Mutex
    /// critical section is trivially short (take/set a JoinHandle).
    bus_watch: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Active per-consumer sessions (each owns its OWN consumer pipeline).
    sessions: Arc<tokio::sync::Mutex<HashMap<String, WhepSession>>>,
    /// StreamProducer wrapping the default-profile (720p H264) encoder
    /// appsink — the fanout that feeds every default consumer pipeline's
    /// appsrc. Clone-cheap (internally Arc'd).
    producer: StreamProducer,
    /// StreamProducer wrapping the compat-profile (854×480@20 realtime VP8)
    /// vp8enc appsink (`enc_appsink_compat`) — feeds consumers that POSTed
    /// with `?profile=compat` (weak TVs whose H264 OMX decoder is vendor-
    /// broken; they software-decode token-partitioned VP8 across 4 cores).
    producer_compat: StreamProducer,
}

impl Drop for NdiPipeline {
    fn drop(&mut self) {
        self.teardown();
    }
}

#[cfg(test)]
mod tests;
