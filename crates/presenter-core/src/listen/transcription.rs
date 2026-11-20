//! The guest's live transcription (#852 C3, spec §22.5): the per-mic session
//! state carried by `GET /listen/status` and the `/listen/ws` wire
//! ([`crate::listen::ws`]), the three `app_settings` rows, the start decision,
//! the daily cap and the cost (plan Decisions 9–11).
//!
//! Rows (absent = the default, so a box nobody set up runs nothing):
//! `listen.transcription` `{enabled, guestMic, dailyCapMinutes}` (the paid
//! switch, off by default); `listen.session` `{day, mode, extraMinutes}`
//! (today's choice, it overrides the Kázeň automation for the rest of the
//! day); `listen.usage` `{day, mics}` (today's metered time, so the cap
//! survives a restart).

use std::collections::BTreeMap;
use std::ops::RangeInclusive;
use std::time::{Duration, Instant};

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::listen::mics::MicConfig;

/// The `app_settings` key of [`TranscriptionSetting`].
pub const TRANSCRIPTION_KEY: &str = "listen.transcription";
/// The `app_settings` key of [`SessionOverride`].
pub const SESSION_KEY: &str = "listen.session";
/// The `app_settings` key of [`UsageRow`].
pub const USAGE_KEY: &str = "listen.usage";
/// Connected minutes a day before the cap stops the guest session.
pub const DEFAULT_DAILY_CAP_MINUTES: u32 = 240;
/// The daily caps the setting accepts, in minutes.
pub const DAILY_CAP_MINUTES_RANGE: RangeInclusive<u32> = 10..=600;
/// After the stage leaves Kázeň, the automatic session keeps running this long.
pub const PREACH_STOP_DELAY: Duration = Duration::from_secs(60);
/// A start after the cap is reached adds this many minutes to today's cap.
pub const START_EXTRA_MINUTES: u32 = 60;
/// Soniox real-time price (docs, pricing), per connected hour.
pub const SONIOX_USD_PER_HOUR: f64 = 0.12;
/// The stage layout code that starts the automatic session.
pub const PREACH_LAYOUT: &str = "preach";

const MS_PER_MINUTE: u64 = 60_000;
const MS_PER_HOUR: f64 = 3_600_000.0;

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

impl OffReason {
    /// The operator's line for the reason (Settings, the interpreter's banner).
    pub fn slovak(&self) -> &'static str {
        match self {
            Self::Disabled => "Prepis je vypnutý",
            Self::NoGuestMic => "Nie je vybraný mikrofón hosťa",
            Self::GuestMicMissing => "Mikrofón hosťa nie je v zozname mikrofónov",
            Self::NoKey => "Chýba kľúč služby prepisu (Soniox)",
            Self::CapReached => "Denný limit prepisu dosiahnutý",
            Self::NotPreach => "Čaká na rozloženie Kázeň",
            Self::OverrideOff => "Prepis je dnes zastavený",
            Self::Auth => "Prepis vypnutý — služba odmietla kľúč",
            Self::Payment => "Prepis vypnutý — problém s účtom služby",
            Self::Refused => "Prepis vypnutý — služba odmietla požiadavku",
            Self::TooManyConnections => "Prepis zastavený — príliš veľa pripojení za 10 minút",
        }
    }
}

/// The stored `listen.transcription` row; absent = off, no guest mic, 240 min.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TranscriptionSetting {
    /// The paid switch.
    pub enabled: bool,
    /// The id of the mic the guest speaks into (any published mic).
    pub guest_mic: Option<String>,
    pub daily_cap_minutes: u32,
}

impl Default for TranscriptionSetting {
    fn default() -> Self {
        Self {
            enabled: false,
            guest_mic: None,
            daily_cap_minutes: DEFAULT_DAILY_CAP_MINUTES,
        }
    }
}

/// Today's choice: run now, stay stopped, or follow the Kázeň layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionMode {
    On,
    Off,
    #[default]
    Auto,
}

/// The stored `listen.session` row: valid only on its `day`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionOverride {
    pub day: NaiveDate,
    /// Absent = `auto`.
    #[serde(default)]
    pub mode: SessionMode,
    /// Minutes added to the cap by starts after the cap was reached.
    #[serde(default)]
    pub extra_minutes: u32,
}

/// One mic's metered time of a day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MicUsage {
    /// Audio sent to the provider.
    pub audio_ms: u64,
    /// Time a connection was open (what the cap counts).
    pub connected_ms: u64,
}

/// The stored `listen.usage` row: valid only on its `day`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageRow {
    pub day: NaiveDate,
    #[serde(default)]
    pub mics: BTreeMap<String, MicUsage>,
}

/// The transcription part of `GET /listen/status`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TranscriptionStatus {
    /// A Soniox key exists on this box (never the key itself).
    pub key_configured: bool,
    pub model: String,
    pub setting: TranscriptionSetting,
    /// Today's mode (`auto` when no choice was made today).
    pub mode: SessionMode,
    pub mics: Vec<MicTranscription>,
}

/// What [`start_decision`] reads.
#[derive(Debug, Clone, Copy)]
pub struct StartInputs<'a> {
    pub setting: &'a TranscriptionSetting,
    /// The published mic list (saved VBAN mics and the fixed Dante mics).
    pub published: &'a [MicConfig],
    /// A provider key exists.
    pub key: bool,
    pub today: NaiveDate,
    pub over: Option<&'a SessionOverride>,
    /// [`preach_active`] now.
    pub preach: bool,
    /// The guest mic's connected time today.
    pub connected_ms_today: u64,
}

/// Whether the guest session runs, and for which mic. Checked in this order
/// (plan Decision 9): the switch, a chosen guest mic, that mic published, a
/// key, today's cap, today's choice (`on` runs, `off` stops), then the Kázeň
/// automation.
pub fn start_decision(i: &StartInputs) -> Result<String, OffReason> {
    if !i.setting.enabled {
        return Err(OffReason::Disabled);
    }
    let Some(guest) = i.setting.guest_mic.as_deref() else {
        return Err(OffReason::NoGuestMic);
    };
    if !i.published.iter().any(|mic| mic.id == guest) {
        return Err(OffReason::GuestMicMissing);
    }
    if !i.key {
        return Err(OffReason::NoKey);
    }
    if i.connected_ms_today >= cap_ms(i.setting, i.over, i.today) {
        return Err(OffReason::CapReached);
    }
    match today_mode(i.over, i.today) {
        SessionMode::On => Ok(guest.to_string()),
        SessionMode::Off => Err(OffReason::OverrideOff),
        SessionMode::Auto if i.preach => Ok(guest.to_string()),
        SessionMode::Auto => Err(OffReason::NotPreach),
    }
}

/// Today's mode: the stored choice when it was made today, else `auto`.
pub fn today_mode(over: Option<&SessionOverride>, today: NaiveDate) -> SessionMode {
    todays(over, today).map_or(SessionMode::Auto, |o| o.mode)
}

/// The stored choice only when it was made today.
fn todays(over: Option<&SessionOverride>, today: NaiveDate) -> Option<&SessionOverride> {
    over.filter(|o| o.day == today)
}

/// Today's cap of connected time: the setting plus today's extra minutes.
pub fn cap_ms(
    setting: &TranscriptionSetting,
    over: Option<&SessionOverride>,
    today: NaiveDate,
) -> u64 {
    let extra = todays(over, today).map_or(0, |o| o.extra_minutes);
    (u64::from(setting.daily_cap_minutes) + u64::from(extra)) * MS_PER_MINUTE
}

/// The provider's price of `connected_ms` of connected time, in US dollars.
pub fn cost_usd(connected_ms: u64) -> f64 {
    connected_ms as f64 / MS_PER_HOUR * SONIOX_USD_PER_HOUR
}

/// The Kázeň automation: the stage shows `preach`, or left it less than
/// [`PREACH_STOP_DELAY`] ago (`left_preach_at`). `std` instants: a tokio
/// caller passes `tokio::time::Instant::into_std()` (exact under the paused
/// test clock too).
pub fn preach_active(layout: &str, left_preach_at: Option<Instant>, now: Instant) -> bool {
    layout == PREACH_LAYOUT
        || left_preach_at
            .is_some_and(|left| now.saturating_duration_since(left) < PREACH_STOP_DELAY)
}

/// The row stored when the operator picks `mode` today. Today's extra
/// minutes are kept (a choice from another day starts at 0); a start (`on`)
/// while the cap is reached adds [`START_EXTRA_MINUTES`].
pub fn next_override(
    current: Option<&SessionOverride>,
    today: NaiveDate,
    mode: SessionMode,
    cap_reached: bool,
) -> SessionOverride {
    let kept = todays(current, today).map_or(0, |o| o.extra_minutes);
    let added = if mode == SessionMode::On && cap_reached {
        START_EXTRA_MINUTES
    } else {
        0
    };
    SessionOverride {
        day: today,
        mode,
        extra_minutes: kept.saturating_add(added),
    }
}

/// Why a `listen.transcription` row was refused (HTTP 422, shown as it is).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TranscriptionValidationError {
    #[error("Denný limit prepisu môže byť 10 až 600 minút (zadané {0}).")]
    CapOutOfRange(u32),
    #[error("Mikrofón hosťa „{0}“ nie je v zozname mikrofónov.")]
    UnknownMic(String),
}

/// Validate a new row: the cap in 10–600 minutes, the guest mic one of the
/// published mics (or none).
pub fn validate_transcription(
    s: &TranscriptionSetting,
    published: &[MicConfig],
) -> Result<(), TranscriptionValidationError> {
    validate_transcription_change(s, None, published)
}

/// Validate a row replacing `stored`. A guest mic the stored row already
/// names is kept even while it is not published (spec gap 7: it comes back
/// with the mic, status `GuestMicMissing`), so the switch can still be turned
/// off and the cap changed while it is missing; a NEWLY chosen mic must be
/// published.
pub fn validate_transcription_change(
    s: &TranscriptionSetting,
    stored: Option<&TranscriptionSetting>,
    published: &[MicConfig],
) -> Result<(), TranscriptionValidationError> {
    if !DAILY_CAP_MINUTES_RANGE.contains(&s.daily_cap_minutes) {
        return Err(TranscriptionValidationError::CapOutOfRange(
            s.daily_cap_minutes,
        ));
    }
    let kept = |id: &str| stored.and_then(|row| row.guest_mic.as_deref()) == Some(id);
    match s.guest_mic.as_deref() {
        Some(id) if !kept(id) && !published.iter().any(|mic| mic.id == id) => {
            Err(TranscriptionValidationError::UnknownMic(id.to_string()))
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests;
