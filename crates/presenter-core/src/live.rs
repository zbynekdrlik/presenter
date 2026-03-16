use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::stage_appearance::StageAppearance;
use crate::stage_client::StageClientSnapshot;
use crate::{BibleBroadcast, BibleSlideOutput, StageDesign, StageDisplaySnapshot, TimersOverview};

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
    StageAppearance {
        layout: String,
        appearance: StageAppearance,
    },
    StageDesign {
        layout: String,
        design: StageDesign,
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
}

/// Messages sent from a WebSocket client to the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InboundMessage {
    StagePresence {
        client_id: String,
        layout_code: String,
    },
    StageHeartbeatAck {
        client_id: String,
        #[serde(default)]
        heartbeat_id: Option<String>,
    },
    StageDisconnect {
        client_id: String,
    },
    #[serde(other)]
    Unknown,
}
