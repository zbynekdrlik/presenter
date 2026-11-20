//! The guest's live transcription (#852 C3, spec §22.5): the per-mic session
//! state carried by `GET /listen/status` and the `/listen/ws` wire
//! ([`crate::listen::ws`]). C3-1 holds the types the wire needs; the setting
//! rows, the start decision and the cost follow in C3-2.

use serde::{Deserialize, Serialize};

/// A transcription session's life cycle for one mic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TranscriptionState {
    /// Not running; [`MicTranscription::reason`] says why.
    #[default]
    Off,
    /// Allowed to run; connects on the first voiced frame (lazy connect).
    Armed,
    Connecting,
    Streaming,
    /// Connected, no voice for 10 s: no audio sent, keepalives only.
    Gated,
    /// No voice for 5 min: the connection ended, the next voice reconnects.
    Parked,
    /// The connection was lost; retried with a growing pause (the banner).
    Reconnecting,
    /// The provider refused for good (key, account, request); a person must
    /// change something before it runs again.
    Stopped,
}

/// Why a mic's transcription is not running (spec §22.5, plan Decision 9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OffReason {
    /// The paid switch is off.
    Disabled,
    /// No guest mic is chosen.
    NoGuestMic,
    /// The chosen guest mic is not in the published list.
    GuestMicMissing,
    /// No Soniox key on this box.
    NoKey,
    /// Today's cap of connected minutes is reached.
    CapReached,
    /// Automatic mode and the stage layout is not Kázeň.
    NotPreach,
    /// Today's choice is "stopped".
    OverrideOff,
    /// The key was refused (401 / 403).
    Auth,
    /// The account cannot pay (402).
    Payment,
    /// The request was refused (400).
    Refused,
    /// More than 20 connections in 10 minutes.
    TooManyConnections,
}

/// One mic's transcription as `GET /listen/status` and `/listen/ws` show it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MicTranscription {
    pub mic_id: String,
    pub state: TranscriptionState,
    pub reason: Option<OffReason>,
    /// The provider's last error, Slovak or quoted, never the key.
    pub error: Option<String>,
    pub audio_ms_today: u64,
    pub connected_ms_today: u64,
    pub cap_ms: u64,
    pub cost_usd_today: f64,
    pub connections: u32,
    pub reconnects: u32,
    pub latency_last_ms: Option<u32>,
    pub latency_p50_ms: Option<u32>,
    pub latency_p90_ms: Option<u32>,
}
