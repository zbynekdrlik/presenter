//! Per-source GStreamer pipeline owning ndisrc + shared encoder + fanout tee.
//!
//! Each `NdiPipeline` instance corresponds to ONE active NDI source. The
//! pipeline builds a shared-encoder topology:
//!
//! ```text
//! ndisrc → ndisrcdemux → videoconvert → vah264enc → rtph264pay → tee
//!                audio ↘ fakesink     (one encoder)              |
//!                                                    ┌───────────┘
//!                                                    ├─ src_0 → queue → webrtcbin (consumer #1)
//!                                                    ├─ src_1 → queue → webrtcbin (consumer #2)
//!                                                    └─ src_N → queue → webrtcbin (consumer #N)
//! ```
//!
//! Per-consumer state lives in `WhepSession` (`whep_session.rs`). The pipeline
//! owns the shared encoder + tee and a `tokio::sync::Mutex<HashMap<String,
//! WhepSession>>` of active sessions.
//!
//! Structure: the type definitions live here in the module root; the
//! `impl NdiPipeline` methods are split across focused submodules
//! (`build`, `lifecycle`, `consumers`) to keep each file well under the
//! project's file-size cap. Private struct fields stay accessible to those
//! submodules because they are descendants of this module.

use std::collections::HashMap;
use std::sync::Arc;

use gstreamer as gst;
use tokio::sync::watch;

use crate::whep_session::{IceCandidate, WhepConnectionState, WhepSession};

mod build;
mod consumers;
mod lifecycle;

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
}

/// Owns one GStreamer pipeline for one NDI source.
pub struct NdiPipeline {
    /// Underlying GStreamer pipeline.
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
    /// Active per-consumer sessions.
    sessions: Arc<tokio::sync::Mutex<HashMap<String, WhepSession>>>,
    /// Tee element — `add_consumer` / `remove_consumer` request/release pads.
    tee: Arc<gst::Element>,
}

impl Drop for NdiPipeline {
    fn drop(&mut self) {
        self.teardown();
    }
}

#[cfg(test)]
mod tests;
