//! Per-WHEP-consumer state.
//!
//! One `WhepSession` is created when a browser POSTs a WHEP offer to
//! `/ndi/whep/:source_id`. It owns:
//!   - the consumer's OWN `gst::Pipeline` (`appsrc → rtph264pay → webrtcbin`)
//!   - the `webrtcbin` element inside it (for trickle ICE forwarding)
//!   - the `ConsumptionLink` connecting its appsrc to the encoder's
//!     `StreamProducer` (drop = disconnect; carries delivery counters)
//!   - the per-pipeline bus-watch task (Latency → recalculate_latency)
//!   - an async channel of pending ICE candidates flowing server→browser
//!   - the last-seen connection state (updated by the signal subscriber)
//!   - the session UUID used as the WHEP HTTP Location path segment
//!
//! Lifetime ends when:
//!   - The HTTP DELETE `/ndi/whep/:source_id/:session_id` route fires
//!     `remove_consumer(session_id)` on the pipeline.
//!   - The owning pipeline is torn down (teardown drains the session map;
//!     each session's Drop tears down its own consumer pipeline).

use gstreamer as gst;
use gstreamer::prelude::*;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::mpsc;
use uuid::Uuid;

/// ICE candidate delivered to the browser via WHEP PATCH or WHEP
/// half-trickle response body.
#[derive(Debug, Clone)]
pub struct IceCandidate {
    pub sdp_mline_index: u32,
    pub candidate: String,
}

/// Connection state for the diagnostic snapshot route (#336 spec
/// "Diagnostic surface"). Mirrors `GstWebRTCPeerConnectionState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WhepConnectionState {
    New,
    Connecting,
    Connected,
    Disconnected,
    Failed,
    Closed,
}

impl WhepConnectionState {
    /// Map GStreamer's `GstWebRTCPeerConnectionState` GEnum value into
    /// our serializable enum. GStreamer values: 0=new, 1=connecting,
    /// 2=connected, 3=disconnected, 4=failed, 5=closed. See
    /// gstreamer-webrtc-0.25 docs for `WebRTCPeerConnectionState`.
    pub fn from_gst_value(value: i32) -> Self {
        match value {
            0 => Self::New,
            1 => Self::Connecting,
            2 => Self::Connected,
            3 => Self::Disconnected,
            4 => Self::Failed,
            5 => Self::Closed,
            _ => {
                tracing::warn!(
                    value,
                    "Unknown GstWebRTCPeerConnectionState integer — treating as New"
                );
                Self::New
            }
        }
    }
}

impl From<gstreamer_webrtc::WebRTCPeerConnectionState> for WhepConnectionState {
    fn from(value: gstreamer_webrtc::WebRTCPeerConnectionState) -> Self {
        use gstreamer_webrtc::WebRTCPeerConnectionState as GstState;
        match value {
            GstState::New => Self::New,
            GstState::Connecting => Self::Connecting,
            GstState::Connected => Self::Connected,
            GstState::Disconnected => Self::Disconnected,
            GstState::Failed => Self::Failed,
            GstState::Closed => Self::Closed,
            // gstreamer-webrtc 0.25 enums are `#[non_exhaustive]` so future
            // GStreamer additions (or the `__Unknown(i32)` variant) must be
            // handled. Treat unknown as `New` and warn so the production log
            // surfaces if it ever fires — same pattern as from_gst_value.
            other => {
                tracing::warn!(
                    state = ?other,
                    "Unknown GstWebRTCPeerConnectionState variant — treating as New"
                );
                Self::New
            }
        }
    }
}

/// RTCP-liveness tracker for one WHEP consumer (#388).
///
/// gst webrtcbin NEVER flips `connection-state` for a peer that vanished
/// without closing the connection (a stage TV that rebooted/reloaded without a
/// WHEP DELETE) — so the state-based zombie reaper is INERT in production. The
/// only reliable server-side liveness signal is the webrtcbin TRANSPORT's
/// `bytes-received` counter: a live WHEP consumer continuously sends RTCP
/// receiver reports, so bytes keep arriving FROM the peer; a vanished peer's
/// counter goes flat. The stale reaper samples this counter and tears a session
/// down when it has not advanced for longer than the stale window.
#[derive(Debug, Clone, Copy)]
pub struct LivenessState {
    /// Last transport `bytes-received` value observed for this session.
    pub last_bytes: u64,
    /// When `last_bytes` last INCREASED (i.e. the peer was last seen sending
    /// RTCP/RTP back to us). A session whose bytes have not advanced since
    /// `now - stale_after` is a vanished-peer zombie.
    pub last_progress: Instant,
}

impl LivenessState {
    /// Fresh tracker at consumer-add time: no bytes seen yet, progress clock
    /// started NOW so a brand-new consumer gets a full `stale_after` grace
    /// window before it can ever be considered stale.
    pub fn new() -> Self {
        Self {
            last_bytes: 0,
            last_progress: Instant::now(),
        }
    }
}

impl Default for LivenessState {
    fn default() -> Self {
        Self::new()
    }
}

/// One WHEP consumer. Owned by the pipeline's session map.
///
/// Each consumer runs in its OWN fresh `gst::Pipeline`
/// (`appsrc → rtph264pay → webrtcbin`), fed by the encoder pipeline's appsink
/// via the shared software fanout. Running each webrtcbin from-zero in its own
/// pipeline is the #373 straggler fix: a webrtcbin added to a long-running
/// pipeline never gets its rtpsession latency configured and drops all RTP.
pub struct WhepSession {
    /// UUID used as the WHEP HTTP Location path segment.
    pub session_id: String,
    /// This consumer's OWN pipeline: `appsrc → rtph264pay → webrtcbin`.
    /// Set to Null on remove/teardown/Drop.
    pub consumer_pipeline: gst::Pipeline,
    /// The webrtcbin element inside `consumer_pipeline` (kept for
    /// `add_ice_candidate`).
    pub webrtcbin: gst::Element,
    /// The StreamProducer link feeding this consumer's appsrc from the encoder
    /// appsink. Dropping it disconnects the consumer from the producer; it also
    /// carries pushed/dropped delivery counters for diagnostics.
    pub link: gstreamer_utils::ConsumptionLink,
    /// The per-pipeline bus watch (services `Latency` messages with
    /// `recalculate_latency()`, logs errors). Aborted on Drop.
    pub bus_task: tokio::task::JoinHandle<()>,
    /// Holds the latest reported connection state, updated by the
    /// `connection-state-change` signal subscriber.
    ///
    /// Uses `std::sync::Mutex` (not `tokio::sync::Mutex`) because the
    /// `notify::connection-state` signal fires from GStreamer streaming
    /// threads (raw `std::thread`, spawned by GLib) — NOT from within a
    /// tokio async context. Holding a tokio Mutex across a blocking
    /// GStreamer callback risks deadlock.
    pub connection_state: Arc<Mutex<WhepConnectionState>>,
    /// RTCP-liveness tracker (#388): the last transport `bytes-received`
    /// value and when it last advanced. Sampled by `reap_stale_sessions` to
    /// detect peers that vanished WITHOUT a connection-state transition — the
    /// only zombies the state-based reaper cannot see.
    ///
    /// `std::sync::Mutex` (not tokio) for parity with `connection_state`: the
    /// reaper updates it from a `spawn_blocking` get-stats read, and the
    /// critical section is a trivial compare-and-swap — never held across an
    /// await.
    pub liveness: Arc<Mutex<LivenessState>>,
    /// ICE candidates flowing server→browser (sender). The receiver
    /// half lives in the pipeline's add_consumer path and is drained
    /// while building the WHEP answer body OR delivered via subsequent
    /// PATCH responses (trickle).
    pub ice_tx: mpsc::UnboundedSender<IceCandidate>,
}

impl WhepSession {
    /// Generate a UUIDv4 session id. Public so unit tests can assert
    /// the format without spawning a real GStreamer element.
    pub fn new_session_id() -> String {
        Uuid::new_v4().to_string()
    }
}

impl Drop for WhepSession {
    fn drop(&mut self) {
        // Full per-consumer teardown — this IS the canonical cleanup path
        // (remove_consumer just drops the session off the async thread):
        //   1. the ConsumptionLink field's own Drop disconnects this consumer's
        //      appsrc from the encoder's StreamProducer (no more samples in);
        //   2. abort the bus-watch task;
        //   3. set the whole consumer pipeline to Null (tears down appsrc +
        //      rtph264pay + webrtcbin together).
        self.bus_task.abort();
        let _ = self.consumer_pipeline.set_state(gst::State::Null);
        tracing::debug!(
            session_id = %self.session_id,
            "WhepSession dropped (producer link disconnected, bus task aborted, \
             consumer pipeline set to Null)"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_session_id_is_a_valid_uuid_v4() {
        let id = WhepSession::new_session_id();
        let parsed = Uuid::parse_str(&id).expect("session id must parse as UUID");
        assert_eq!(
            parsed.get_version(),
            Some(uuid::Version::Random),
            "session id must be UUIDv4 (Version::Random), got {:?}",
            parsed.get_version(),
        );
    }

    #[test]
    fn new_session_id_is_unique_per_call() {
        let a = WhepSession::new_session_id();
        let b = WhepSession::new_session_id();
        assert_ne!(a, b, "two calls must return distinct UUIDs");
    }

    #[test]
    fn connection_state_maps_gst_values_correctly() {
        assert_eq!(
            WhepConnectionState::from_gst_value(0),
            WhepConnectionState::New
        );
        assert_eq!(
            WhepConnectionState::from_gst_value(1),
            WhepConnectionState::Connecting
        );
        assert_eq!(
            WhepConnectionState::from_gst_value(2),
            WhepConnectionState::Connected
        );
        assert_eq!(
            WhepConnectionState::from_gst_value(3),
            WhepConnectionState::Disconnected
        );
        assert_eq!(
            WhepConnectionState::from_gst_value(4),
            WhepConnectionState::Failed
        );
        assert_eq!(
            WhepConnectionState::from_gst_value(5),
            WhepConnectionState::Closed
        );
        assert_eq!(
            WhepConnectionState::from_gst_value(99),
            WhepConnectionState::New
        );
    }

    #[test]
    fn from_gst_typed_enum_maps_correctly() {
        use gstreamer_webrtc::WebRTCPeerConnectionState as GstState;
        assert_eq!(
            WhepConnectionState::from(GstState::New),
            WhepConnectionState::New
        );
        assert_eq!(
            WhepConnectionState::from(GstState::Connecting),
            WhepConnectionState::Connecting
        );
        assert_eq!(
            WhepConnectionState::from(GstState::Connected),
            WhepConnectionState::Connected
        );
        assert_eq!(
            WhepConnectionState::from(GstState::Disconnected),
            WhepConnectionState::Disconnected
        );
        assert_eq!(
            WhepConnectionState::from(GstState::Failed),
            WhepConnectionState::Failed
        );
        assert_eq!(
            WhepConnectionState::from(GstState::Closed),
            WhepConnectionState::Closed
        );
    }

    #[test]
    fn connection_state_serializes_as_lowercase_string() {
        let s = serde_json::to_string(&WhepConnectionState::Connected).unwrap();
        assert_eq!(s, "\"connected\"");
    }
}
