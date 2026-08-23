use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::stage_client::{NdiVideoDiag, StageClientSnapshot};
use crate::{BibleBroadcast, BibleSlideOutput, StageDisplaySnapshot, TimersOverview};

/// Events broadcast over the `/live/ws` WebSocket to all connected clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum LiveEvent {
    Timers {
        overview: TimersOverview,
    },
    Stage {
        snapshot: StageDisplaySnapshot,
    },
    Heartbeat {
        id: Uuid,
        timestamp: DateTime<Utc>,
    },
    StageConnection {
        snapshot: StageClientSnapshot,
    },
    Bible {
        broadcast: BibleBroadcast,
    },
    /// Single-source-of-truth Bible slide event.
    BibleSlide {
        output: BibleSlideOutput,
    },
    BibleCleared,
    StageLayout {
        code: String,
    },
    BiblePreferencesChanged {
        character_limit: u32,
    },
    BroadcastLive {
        enabled: bool,
    },
    /// Bible presentation slides changed (content edit, add, delete, reorder).
    BibleSlidesChanged {
        presentation_id: String,
    },
    NdiSourceActivated {
        source_id: String,
        ndi_name: String,
        label: String,
    },
    NdiSourceDeactivated,
    NdiConnectionStatus {
        status: String,
    },
    /// Stream-graphics show-state for one output (epic #718, ADR 0009 §6).
    /// Fired on every ACTIVATION change (base set/clear, overlay on/off/clear)
    /// — a lightweight "apply directly" event. Activation does NOT bump
    /// `config_revision` (`.claude/rules/stream-graphics.md`), so a client
    /// applies this in place; it refetches the full def only when
    /// `config_revision` advances (see `StreamConfigChanged`).
    StreamState {
        output: String,
        active_scene_id: Option<i64>,
        active_overlay_ids: Vec<i64>,
        config_revision: u64,
    },
    /// Stream-graphics CONFIG change for one output (epic #718, ADR 0009 §6).
    /// Fired on every config write (output / scene / element create / rename /
    /// patch / delete + scene reorder) — all of which bump `config_revision`.
    /// The client refetches `GET /stream/api/outputs/{slug}/def` in response.
    StreamConfigChanged {
        output: String,
        config_revision: u64,
    },
}

/// Messages sent from a WebSocket client to the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InboundMessage {
    StagePresence {
        client_id: String,
        layout_code: String,
        /// #732 diagnostics: `navigator.userAgent` of the connecting TV
        /// WebView, sent on connect. Additive + optional — an old client
        /// omits it and the server records `None`.
        #[serde(default)]
        user_agent: Option<String>,
    },
    StageHeartbeatAck {
        client_id: String,
        #[serde(default)]
        heartbeat_id: Option<String>,
        /// #732 diagnostics: the NDI `<video>` state snapshot at this
        /// heartbeat. Additive + optional — the heartbeat cadence carrier
        /// for the diagnostics snapshot (`StageDiag` carries the on-change
        /// pushes between heartbeats).
        #[serde(default)]
        ndi_video: Option<NdiVideoDiag>,
    },
    /// #732 diagnostics: an out-of-band NDI `<video>` state snapshot pushed
    /// immediately on a change of paused/error/cover between heartbeats. A
    /// server that predates the diagnostics protocol deserializes this as
    /// `Unknown` (via `#[serde(other)]`) and ignores it.
    StageDiag {
        client_id: String,
        #[serde(default)]
        ndi_video: Option<NdiVideoDiag>,
    },
    StageDisconnect {
        client_id: String,
    },
    #[serde(other)]
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn stream_state_wire_shape_is_exact() {
        let event = LiveEvent::StreamState {
            output: "stream".to_string(),
            active_scene_id: Some(7),
            active_overlay_ids: vec![3, 5],
            config_revision: 12,
        };
        // The LiveEvent enum is `tag = "type"`, snake_case — the variant tag
        // is `stream_state`, the fields flat.
        assert_eq!(
            serde_json::to_value(&event).unwrap(),
            json!({
                "type": "stream_state",
                "output": "stream",
                "active_scene_id": 7,
                "active_overlay_ids": [3, 5],
                "config_revision": 12
            })
        );
    }

    #[test]
    fn stream_state_with_no_base_round_trips() {
        let event = LiveEvent::StreamState {
            output: "stream".to_string(),
            active_scene_id: None,
            active_overlay_ids: vec![],
            config_revision: 0,
        };
        let wire = serde_json::to_string(&event).unwrap();
        let back: LiveEvent = serde_json::from_str(&wire).unwrap();
        match back {
            LiveEvent::StreamState {
                output,
                active_scene_id,
                active_overlay_ids,
                config_revision,
            } => {
                assert_eq!(output, "stream");
                assert_eq!(active_scene_id, None);
                assert!(active_overlay_ids.is_empty());
                assert_eq!(config_revision, 0);
            }
            other => panic!("expected StreamState, got {other:?}"),
        }
    }

    #[test]
    fn stream_config_changed_wire_shape_is_exact() {
        let event = LiveEvent::StreamConfigChanged {
            output: "stream".to_string(),
            config_revision: 42,
        };
        assert_eq!(
            serde_json::to_value(&event).unwrap(),
            json!({
                "type": "stream_config_changed",
                "output": "stream",
                "config_revision": 42
            })
        );
        // Round-trips back to the same variant.
        let back: LiveEvent =
            serde_json::from_value(serde_json::to_value(&event).unwrap()).unwrap();
        assert!(matches!(
            back,
            LiveEvent::StreamConfigChanged {
                config_revision: 42,
                ..
            }
        ));
    }
}
