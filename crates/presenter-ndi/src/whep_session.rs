//! Per-WHEP-consumer state.
//!
//! One `WhepSession` is created when a browser POSTs a WHEP offer to
//! `/ndi/whep/:source_id`. It owns:
//!   - one `webrtcbin` GStreamer element (the WebRTC peer)
//!   - the `tee` request-pad src that feeds it from the shared encoder
//!   - an async channel of pending ICE candidates flowing server→browser
//!   - the last-seen connection state (updated by the signal subscriber)
//!   - the session UUID used as the WHEP HTTP Location path segment
//!
//! Lifetime ends when:
//!   - The HTTP DELETE `/ndi/whep/:source_id/:session_id` route fires
//!     `remove_consumer(session_id)` on the pipeline.
//!   - webrtcbin emits `connection-state-change` to `Failed` or
//!     `Disconnected` (handled by a signal subscriber that calls
//!     `remove_consumer`).
//!   - The owning pipeline is torn down (Drop on the pipeline cascades
//!     through `tee.remove_pad` + `pipeline.remove(webrtcbin)`).
//!
//! Non-Send constraint: `webrtcbin` and `gst::Pad` are non-Send glib
//! types. All signal connections and pad linking happen on the GStreamer
//! main loop thread; tokio code talks to the session via Send channels
//! and Send-able UUIDs.

use gstreamer as gst;
use gstreamer::prelude::*;
use std::sync::{Arc, Mutex};
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

/// One WHEP consumer. Owned by the pipeline's session map.
pub struct WhepSession {
    /// UUID used as the WHEP HTTP Location path segment.
    pub session_id: String,
    /// The webrtcbin element for this consumer.
    pub webrtcbin: gst::Element,
    /// The queue element buffering the tee branch for this consumer.
    /// Added to the pipeline alongside webrtcbin; removed in remove_consumer.
    pub queue: gst::Element,
    /// The src pad on `tee` feeding this consumer's webrtcbin (via a
    /// queue). Released back to the tee on Drop.
    pub tee_src_pad: gst::Pad,
    /// Holds the latest reported connection state, updated by the
    /// `connection-state-change` signal subscriber.
    ///
    /// Uses `std::sync::Mutex` (not `tokio::sync::Mutex`) because the
    /// `notify::connection-state` signal fires from GStreamer streaming
    /// threads (raw `std::thread`, spawned by GLib) — NOT from within a
    /// tokio async context. Holding a tokio Mutex across a blocking
    /// GStreamer callback risks deadlock.
    pub connection_state: Arc<Mutex<WhepConnectionState>>,
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
        // Best-effort teardown of the per-consumer state. The pipeline's
        // remove_consumer method is the canonical path; Drop is the
        // backstop for unexpected drops (pipeline Drop, panic unwind).
        let _ = self.webrtcbin.set_state(gst::State::Null);
        let _ = self.queue.set_state(gst::State::Null);
        // tee_src_pad release is the parent tee's responsibility — we
        // can't release a request-pad without holding the tee. The
        // pipeline-level teardown() iterates sessions and calls
        // tee.release_request_pad(&session.tee_src_pad) after removing
        // both queue and webrtcbin from the pipeline.
        tracing::debug!(
            session_id = %self.session_id,
            "WhepSession dropped (webrtcbin + queue set to Null; pad release handled by pipeline)"
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
