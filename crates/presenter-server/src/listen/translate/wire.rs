//! The Gemini Live wire (#999 L1), as the raw probes of 2026-11-27 saw it (no
//! SDK): ONE setup message, then `realtimeInput` audio (16 kHz s16le, base64,
//! ~100 ms per message); the server answers every message as a BINARY frame
//! holding JSON — `setupComplete`, `serverContent` (output audio 24 kHz s16le
//! in `modelTurn.parts[].inlineData`, `outputTranscription` = the guest's
//! text, `inputTranscription` = the Slovak original, `turnComplete`),
//! `sessionResumptionUpdate` (a new handle about every second), `goAway`
//! (the connection ends soon) and `usageMetadata`.
//!
//! A refusal is a CLOSE after the upgrade, never an HTTP status: 1007 = the
//! key, 1008 = an unknown model or the 120 s idle cut ("client inactivity").

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use serde::Deserialize;

/// The output rate the model documents and the probes saw.
pub const OUTPUT_RATE: u32 = 24_000;
/// The input rate we send (the mic frames are 16 kHz).
pub const INPUT_MIME: &str = "audio/pcm;rate=16000";

/// The setup: audio out, both transcriptions, the target language, the
/// translation context as `systemInstruction` (#999 L6c — setup-only; the doc
/// ignores instructions that fight the target language or the output format),
/// a sliding context window (the connection never hits the context limit) and
/// session resumption (`handle` = continue an earlier session after a
/// reconnect / `goAway`).
pub fn setup_message(model: &str, language: &str, context: &str, handle: Option<&str>) -> String {
    let resumption = match handle {
        Some(handle) => serde_json::json!({ "handle": handle }),
        None => serde_json::json!({}),
    };
    serde_json::json!({
        "setup": {
            "model": format!("models/{model}"),
            "generationConfig": {
                "responseModalities": ["AUDIO"],
                "translationConfig": { "targetLanguageCode": language }
            },
            "systemInstruction": { "parts": [{ "text": context }] },
            "inputAudioTranscription": {},
            "outputAudioTranscription": {},
            "contextWindowCompression": { "slidingWindow": {} },
            "sessionResumption": resumption
        }
    })
    .to_string()
}

/// One mic frame as `realtimeInput` audio.
pub fn audio_message(pcm: &[i16]) -> String {
    let bytes: Vec<u8> = pcm.iter().flat_map(|sample| sample.to_le_bytes()).collect();
    serde_json::json!({
        "realtimeInput": { "audio": { "data": BASE64.encode(bytes), "mimeType": INPUT_MIME } }
    })
    .to_string()
}

/// What one server message carries (everything optional; several can come
/// in one message).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ServerMessage {
    pub setup_complete: bool,
    /// Output audio chunks, s16le at [`OUTPUT_RATE`], in order.
    pub audio: Vec<Vec<u8>>,
    /// A chunk whose mime type named another rate (logged, still played).
    pub unexpected_rate: Option<String>,
    pub output_text: Option<String>,
    pub input_text: Option<String>,
    pub turn_complete: bool,
    /// A new resumption handle (only when the server says it is resumable).
    pub handle: Option<String>,
    /// `goAway.timeLeft` (e.g. "10s"): the connection ends soon.
    pub go_away: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum WireError {
    #[error("not JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("audio is not base64: {0}")]
    Base64(#[from] base64::DecodeError),
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Raw {
    setup_complete: Option<serde_json::Value>,
    server_content: Option<RawContent>,
    session_resumption_update: Option<RawResumption>,
    go_away: Option<RawGoAway>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawContent {
    model_turn: Option<RawTurn>,
    output_transcription: Option<RawText>,
    input_transcription: Option<RawText>,
    #[serde(default)]
    turn_complete: bool,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawTurn {
    #[serde(default)]
    parts: Vec<RawPart>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawPart {
    inline_data: Option<RawBlob>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawBlob {
    #[serde(default)]
    mime_type: String,
    #[serde(default)]
    data: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawText {
    #[serde(default)]
    text: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawResumption {
    #[serde(default)]
    new_handle: String,
    #[serde(default)]
    resumable: bool,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawGoAway {
    #[serde(default)]
    time_left: String,
}

/// Reads one server message (TEXT or BINARY frame, both JSON).
pub fn parse_server(bytes: &[u8]) -> Result<ServerMessage, WireError> {
    let raw: Raw = serde_json::from_slice(bytes)?;
    let mut message = ServerMessage {
        setup_complete: raw.setup_complete.is_some(),
        ..ServerMessage::default()
    };
    if let Some(content) = raw.server_content {
        for part in content
            .model_turn
            .map(|turn| turn.parts)
            .unwrap_or_default()
        {
            let Some(blob) = part.inline_data else {
                continue;
            };
            if !blob.mime_type.starts_with("audio/") || blob.data.is_empty() {
                continue;
            }
            if !blob.mime_type.contains(&format!("rate={OUTPUT_RATE}")) {
                message.unexpected_rate = Some(blob.mime_type.clone());
            }
            message.audio.push(BASE64.decode(blob.data.as_bytes())?);
        }
        message.output_text = content
            .output_transcription
            .map(|t| t.text)
            .filter(|t| !t.is_empty());
        message.input_text = content
            .input_transcription
            .map(|t| t.text)
            .filter(|t| !t.is_empty());
        message.turn_complete = content.turn_complete;
    }
    if let Some(update) = raw.session_resumption_update {
        if update.resumable && !update.new_handle.is_empty() {
            message.handle = Some(update.new_handle);
        }
    }
    message.go_away = raw.go_away.map(|g| g.time_left);
    Ok(message)
}

/// How a connection that closed is treated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseClass {
    /// The 120 s idle cut: reconnect at once with the handle, no back-off.
    Idle,
    /// The key or the model is wrong: stop, never retry.
    Fatal,
    /// Anything else (a server error, a network drop): back off and retry.
    Retry,
}

pub fn classify_close(code: Option<u16>, reason: &str) -> CloseClass {
    match code {
        Some(1008) if reason.to_ascii_lowercase().contains("inactivity") => CloseClass::Idle,
        Some(1007 | 1008) => CloseClass::Fatal,
        _ => CloseClass::Retry,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_names_the_model_language_compression_and_resumption() {
        let fresh: serde_json::Value = serde_json::from_str(&setup_message(
            "gemini-x",
            "de",
            "A sermon at Example Chapel.",
            None,
        ))
        .expect("json");
        let setup = &fresh["setup"];
        assert_eq!(setup["model"], "models/gemini-x");
        assert_eq!(setup["generationConfig"]["responseModalities"][0], "AUDIO");
        assert_eq!(
            setup["generationConfig"]["translationConfig"]["targetLanguageCode"],
            "de"
        );
        assert!(setup["inputAudioTranscription"].is_object());
        assert!(setup["outputAudioTranscription"].is_object());
        assert!(setup["contextWindowCompression"]["slidingWindow"].is_object());
        assert_eq!(setup["sessionResumption"], serde_json::json!({}));
        assert_eq!(
            setup["systemInstruction"],
            serde_json::json!({"parts": [{"text": "A sermon at Example Chapel."}]}),
            "the translation context rides the setup"
        );

        let resumed: serde_json::Value =
            serde_json::from_str(&setup_message("gemini-x", "de", "ctx", Some("h-1")))
                .expect("json");
        assert_eq!(resumed["setup"]["sessionResumption"]["handle"], "h-1");
    }

    #[test]
    fn audio_is_base64_s16le_at_16_khz() {
        let message: serde_json::Value =
            serde_json::from_str(&audio_message(&[1, -2, 0x1234])).expect("json");
        let audio = &message["realtimeInput"]["audio"];
        assert_eq!(audio["mimeType"], "audio/pcm;rate=16000");
        let bytes = BASE64
            .decode(audio["data"].as_str().expect("data"))
            .expect("b64");
        assert_eq!(bytes, vec![1, 0, 0xFE, 0xFF, 0x34, 0x12]);
    }

    #[test]
    fn reads_the_messages_the_live_probe_saw() {
        assert!(
            parse_server(b"{\n  \"setupComplete\": {}\n}\n")
                .expect("ok")
                .setup_complete
        );

        let audio = BASE64.encode([1_u8, 0, 2, 0]);
        let turn = format!(
            r#"{{"serverContent":{{"modelTurn":{{"parts":[{{"inlineData":{{"mimeType":"audio/pcm;rate=24000","data":"{audio}"}}}}]}}}}}}"#
        );
        let parsed = parse_server(turn.as_bytes()).expect("ok");
        assert_eq!(parsed.audio, vec![vec![1, 0, 2, 0]]);
        assert_eq!(parsed.unexpected_rate, None);
        assert!(!parsed.setup_complete);

        let text = parse_server(
            br#"{"serverContent":{"outputTranscription":{"text":" they crossed","languageCode":"en"},"turnComplete":true}}"#,
        )
        .expect("ok");
        assert_eq!(text.output_text.as_deref(), Some(" they crossed"));
        assert!(text.turn_complete);
        let original =
            parse_server(br#"{"serverContent":{"inputTranscription":{"text":"mali to"}}}"#)
                .expect("ok");
        assert_eq!(original.input_text.as_deref(), Some("mali to"));
        assert_eq!(original.output_text, None);

        let handle = parse_server(
            br#"{"sessionResumptionUpdate":{"newHandle":"8417cbac","resumable":true}}"#,
        )
        .expect("ok");
        assert_eq!(handle.handle.as_deref(), Some("8417cbac"));
        let not_resumable =
            parse_server(br#"{"sessionResumptionUpdate":{"newHandle":"","resumable":false}}"#)
                .expect("ok");
        assert_eq!(not_resumable.handle, None);

        let away = parse_server(br#"{"goAway":{"timeLeft":"10s"}}"#).expect("ok");
        assert_eq!(away.go_away.as_deref(), Some("10s"));
        let empty = parse_server(br#"{"serverContent":{},"usageMetadata":{"totalTokenCount":5}}"#)
            .expect("ok");
        assert_eq!(empty, ServerMessage::default());
    }

    #[test]
    fn another_rate_is_flagged_and_bad_input_is_an_error() {
        let audio = BASE64.encode([0_u8, 0]);
        let turn = format!(
            r#"{{"serverContent":{{"modelTurn":{{"parts":[{{"inlineData":{{"mimeType":"audio/pcm;rate=16000","data":"{audio}"}}}},{{"text":"x"}}]}}}}}}"#
        );
        let parsed = parse_server(turn.as_bytes()).expect("ok");
        assert_eq!(parsed.audio.len(), 1);
        assert_eq!(
            parsed.unexpected_rate.as_deref(),
            Some("audio/pcm;rate=16000")
        );
        assert!(matches!(parse_server(b"not json"), Err(WireError::Json(_))));
        let bad = br#"{"serverContent":{"modelTurn":{"parts":[{"inlineData":{"mimeType":"audio/pcm;rate=24000","data":"@@@"}}]}}}"#;
        assert!(matches!(parse_server(bad), Err(WireError::Base64(_))));
    }

    #[test]
    fn closes_are_classified_as_the_probes_saw_them() {
        assert_eq!(
            classify_close(Some(1008), "Session closed due to client inactivity."),
            CloseClass::Idle
        );
        assert_eq!(
            classify_close(
                Some(1007),
                "API key not valid. Please pass a valid API key."
            ),
            CloseClass::Fatal
        );
        assert_eq!(
            classify_close(Some(1008), "models/x is not found for API version v1beta"),
            CloseClass::Fatal
        );
        assert_eq!(
            classify_close(Some(1011), "Internal error"),
            CloseClass::Retry
        );
        assert_eq!(classify_close(Some(1000), ""), CloseClass::Retry);
        assert_eq!(classify_close(None, "the stream ended"), CloseClass::Retry);
    }
}
