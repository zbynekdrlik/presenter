//! Soniox real-time WebSocket JSON (#852 C3-3; docs.soniox.com
//! `stt/api-reference/websocket-api`, `stt/rt/manual-finalization`,
//! `stt/rt/connection-keepalive`, read 2026-11-20): the config the first TEXT
//! frame carries (the measured one, spec §22.3), the two control messages,
//! the responses (tokens, processed ms, `finished`, the error fields) and the
//! error classes the session machine (C3-4) decides on. The API key never
//! appears here: it travels in the `Authorization` header of the upgrade
//! (the config's `api_key` field is deprecated), and [`redact`] removes it
//! from any text that is logged.

use presenter_core::listen::context::ContextTerms;
use presenter_core::listen::transcript::AsrToken;
use serde::{Deserialize, Serialize};

pub const DEFAULT_URL: &str = "wss://stt-rt.soniox.com/transcribe-websocket";
pub const DEFAULT_MODEL: &str = "stt-rt-v6";
/// The audio the mic chain produces: 16 kHz mono s16 (spec §22.3).
const AUDIO_FORMAT: &str = "pcm_s16le";
const SAMPLE_RATE: u32 = 16_000;
const NUM_CHANNELS: u32 = 1;
/// The guest session is always English (plan Decision 3).
const LANGUAGE_HINTS: [&str; 1] = ["en"];
/// Soniox's documented range of `max_endpoint_delay_ms`.
pub const ENDPOINT_DELAY_MIN_MS: u32 = 500;
pub const ENDPOINT_DELAY_MAX_MS: u32 = 3_000;
/// What [`redact`] puts in place of the key.
const REDACTED: &str = "<redacted>";

/// One connection's config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SonioxConfig {
    pub model: String,
    pub context: ContextTerms,
    /// `None` = no endpoint detection (the measured config); `Some` turns it
    /// on with this delay, clamped to 500–3000 ms.
    pub endpoint_delay_ms: Option<u32>,
}

#[derive(Serialize)]
struct ConfigJson<'a> {
    model: &'a str,
    audio_format: &'a str,
    sample_rate: u32,
    num_channels: u32,
    language_hints: &'a [&'a str],
    context: ContextJson<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    enable_endpoint_detection: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_endpoint_delay_ms: Option<u32>,
}

#[derive(Serialize)]
struct ContextJson<'a> {
    general: Vec<GeneralJson<'a>>,
    terms: &'a [String],
}

#[derive(Serialize)]
struct GeneralJson<'a> {
    key: &'a str,
    value: &'a str,
}

/// The config TEXT frame. Never an `api_key` field.
pub fn config_message(c: &SonioxConfig) -> String {
    let delay = c
        .endpoint_delay_ms
        .map(|ms| ms.clamp(ENDPOINT_DELAY_MIN_MS, ENDPOINT_DELAY_MAX_MS));
    let json = ConfigJson {
        model: &c.model,
        audio_format: AUDIO_FORMAT,
        sample_rate: SAMPLE_RATE,
        num_channels: NUM_CHANNELS,
        language_hints: &LANGUAGE_HINTS,
        context: ContextJson {
            general: c
                .context
                .general
                .iter()
                .map(|(key, value)| GeneralJson { key, value })
                .collect(),
            terms: &c.context.terms,
        },
        enable_endpoint_detection: delay.map(|_| true),
        max_endpoint_delay_ms: delay,
    };
    // Plain strings and numbers only: serialisation cannot fail. Never fall
    // back to "" — an empty TEXT frame ENDS the stream; "{}" is refused by
    // Soniox as a bad request (a visible stop) instead.
    serde_json::to_string(&json).unwrap_or_else(|_| "{}".to_string())
}

/// A control message (a TEXT frame).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    /// Finalize the audio sent so far; answered by finals + `<fin>`.
    Finalize,
    /// Keeps the stream open while no audio is sent (≤ 20 s apart).
    Keepalive,
}

pub fn control_message(c: Control) -> String {
    match c {
        Control::Finalize => r#"{"type":"finalize"}"#.to_string(),
        Control::Keepalive => r#"{"type":"keepalive"}"#.to_string(),
    }
}

/// One token as Soniox sends it; times in ms of the connection.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RawToken {
    pub text: String,
    #[serde(default)]
    pub start_ms: Option<u64>,
    #[serde(default)]
    pub end_ms: Option<u64>,
    #[serde(default)]
    pub confidence: Option<f32>,
    #[serde(default)]
    pub is_final: Option<bool>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub translation_status: Option<String>,
}

/// One response. An error response carries `error`; the server closes the
/// connection right after it.
#[derive(Debug, Clone, PartialEq)]
pub struct SonioxResponse {
    pub tokens: Vec<RawToken>,
    pub final_audio_proc_ms: Option<u64>,
    pub total_audio_proc_ms: Option<u64>,
    pub finished: bool,
    pub error: Option<SonioxError>,
}

/// The error fields of a response (`error_code`, `error_type`,
/// `error_message`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SonioxError {
    pub code: Option<u16>,
    /// `error_type`: stable, machine-readable (the docs branch on it).
    pub kind: Option<String>,
    pub message: String,
}

/// What the session machine does about an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    /// 401 / 403: a person must fix the key — stop, no retry.
    Auth,
    /// 402: balance or budget — stop, no retry.
    Payment,
    /// 400: our request is wrong — stop, no retry.
    BadRequest,
    /// 408: retry.
    Timeout,
    /// 413: the 300-minute limit — reconnect at once.
    TooLong,
    /// 429: retry with back-off.
    RateLimited,
    /// 5xx: retry with back-off.
    Server,
    Unknown,
}

#[derive(Debug, thiserror::Error)]
pub enum WireError {
    #[error("invalid Soniox response: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Deserialize)]
struct ResponseJson {
    #[serde(default)]
    tokens: Vec<RawToken>,
    #[serde(default)]
    final_audio_proc_ms: Option<u64>,
    #[serde(default)]
    total_audio_proc_ms: Option<u64>,
    #[serde(default)]
    finished: bool,
    #[serde(default)]
    error_code: Option<u16>,
    #[serde(default)]
    error_type: Option<String>,
    #[serde(default)]
    error_message: Option<String>,
}

/// Parses one response TEXT frame; unknown fields are ignored.
pub fn parse_response(text: &str) -> Result<SonioxResponse, WireError> {
    let raw: ResponseJson = serde_json::from_str(text)?;
    let has_error =
        raw.error_code.is_some() || raw.error_type.is_some() || raw.error_message.is_some();
    let error = has_error.then(|| SonioxError {
        code: raw.error_code,
        kind: raw.error_type,
        message: raw.error_message.unwrap_or_default(),
    });
    Ok(SonioxResponse {
        tokens: raw.tokens,
        final_audio_proc_ms: raw.final_audio_proc_ms,
        total_audio_proc_ms: raw.total_audio_proc_ms,
        finished: raw.finished,
        error,
    })
}

/// The class of an error: the code first, then the `error_type` text.
pub fn classify(e: &SonioxError) -> ErrorClass {
    match e.code.map(class_of_code) {
        Some(class) if class != ErrorClass::Unknown => class,
        _ => e.kind.as_deref().map_or(ErrorClass::Unknown, class_of_kind),
    }
}

/// The class of a WebSocket close that came without an error response: an
/// `error_type` named in the reason first, then the close code.
pub fn classify_close(code: u16, reason: &str) -> ErrorClass {
    match class_of_kind(reason) {
        ErrorClass::Unknown => match code {
            // Going away: the server closed an idle stream.
            1001 => ErrorClass::Timeout,
            // Protocol error, unsupported or invalid data, too big: ours.
            1002 | 1003 | 1007 | 1009 => ErrorClass::BadRequest,
            // Internal error, restart, try again later, bad gateway.
            1011..=1014 => ErrorClass::Server,
            _ => ErrorClass::Unknown,
        },
        class => class,
    }
}

fn class_of_code(code: u16) -> ErrorClass {
    match code {
        400 => ErrorClass::BadRequest,
        401 | 403 => ErrorClass::Auth,
        402 => ErrorClass::Payment,
        408 => ErrorClass::Timeout,
        413 => ErrorClass::TooLong,
        429 => ErrorClass::RateLimited,
        500..=599 => ErrorClass::Server,
        _ => ErrorClass::Unknown,
    }
}

/// The documented `error_type` values, matched inside a longer text too.
fn class_of_kind(text: &str) -> ErrorClass {
    const KINDS: [(&str, ErrorClass); 13] = [
        ("unauthenticated", ErrorClass::Auth),
        ("permission_denied", ErrorClass::Auth),
        ("temp_api_key_session_expired", ErrorClass::Auth),
        ("balance_exhausted", ErrorClass::Payment),
        ("budget_exhausted", ErrorClass::Payment),
        ("invalid_request", ErrorClass::BadRequest),
        ("model_not_available", ErrorClass::BadRequest),
        ("request_timeout", ErrorClass::Timeout),
        ("max_duration_reached", ErrorClass::TooLong),
        ("limit_exceeded", ErrorClass::RateLimited),
        (
            "max_concurrent_connections_reached",
            ErrorClass::RateLimited,
        ),
        ("internal_error", ErrorClass::Server),
        ("service_unavailable", ErrorClass::Server),
    ];
    KINDS
        .iter()
        .find(|(kind, _)| text.contains(kind))
        .map_or(ErrorClass::Unknown, |(_, class)| *class)
}

/// The response's tokens for the assembler. A token without `confidence`
/// (a marker) gets 1.0: a missing value is not a low one.
pub fn to_tokens(r: &SonioxResponse) -> Vec<AsrToken> {
    r.tokens
        .iter()
        .map(|t| AsrToken {
            text: t.text.clone(),
            start_ms: t.start_ms,
            end_ms: t.end_ms,
            confidence: t.confidence.unwrap_or(1.0),
            is_final: t.is_final.unwrap_or(false),
            language: t.language.clone(),
            is_translation: t.translation_status.as_deref() == Some("translation"),
        })
        .collect()
}

/// `s` with every occurrence of the key replaced; an empty key changes
/// nothing.
pub fn redact(s: &str, key: &str) -> String {
    if key.is_empty() {
        s.to_string()
    } else {
        s.replace(key, REDACTED)
    }
}

#[cfg(test)]
mod tests;
