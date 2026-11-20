//! The `GET /listen/ws` wire (#852 C3, spec §4, plan Decision 8): on connect
//! ONE [`ListenWsMessage::Snapshot`], then `sentence` / `tail` / `session`
//! messages. Shared by the server, presenter-ui and the E2E assertions, so the
//! JSON shape lives here once.

use crate::listen::transcription::MicTranscription;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// One closed sentence of a mic's transcript; times are session ms.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptSentence {
    /// Increasing per process, never reused.
    pub id: u64,
    pub mic_id: String,
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub closed_at: DateTime<Utc>,
    pub language: Option<String>,
}

/// The open sentence of a mic: its final words and the live (non-final) rest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptTail {
    pub mic_id: String,
    pub final_text: String,
    pub partial_text: String,
    pub updated_at: DateTime<Utc>,
}

/// What a reader gets first: the newest sentences, every tail and every
/// mic's session state.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptSnapshot {
    pub sentences: Vec<TranscriptSentence>,
    pub tails: Vec<TranscriptTail>,
    pub sessions: Vec<MicTranscription>,
}

/// One message on `/listen/ws`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ListenWsMessage {
    Snapshot { snapshot: TranscriptSnapshot },
    Sentence { sentence: TranscriptSentence },
    Tail { tail: TranscriptTail },
    Session { session: MicTranscription },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::listen::transcription::{OffReason, TranscriptionState};
    use chrono::TimeZone;
    use serde_json::{json, Value};

    fn at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 22, 9, 30, 5)
            .single()
            .unwrap()
    }

    fn sentence() -> TranscriptSentence {
        TranscriptSentence {
            id: 7,
            mic_id: "guest".into(),
            text: "Turn with me to John 3:16.".into(),
            start_ms: 61_200,
            end_ms: 63_450,
            closed_at: at(),
            language: Some("en".into()),
        }
    }

    fn tail() -> TranscriptTail {
        TranscriptTail {
            mic_id: "guest".into(),
            final_text: "For God so".into(),
            partial_text: " loved the".into(),
            updated_at: at(),
        }
    }

    fn session() -> MicTranscription {
        MicTranscription {
            mic_id: "guest".into(),
            state: TranscriptionState::Reconnecting,
            reason: Some(OffReason::TooManyConnections),
            error: Some("Služba prepisu nedostupná".into()),
            audio_ms_today: 120_000,
            connected_ms_today: 180_000,
            cap_ms: 14_400_000,
            cost_usd_today: 0.006,
            connections: 2,
            reconnects: 1,
            latency_last_ms: Some(640),
            latency_p50_ms: Some(700),
            latency_p90_ms: None,
        }
    }

    fn sentence_json() -> Value {
        json!({"id": 7, "micId": "guest", "text": "Turn with me to John 3:16.", "startMs": 61200,
               "endMs": 63450, "closedAt": "2026-11-22T09:30:05Z", "language": "en"})
    }

    fn tail_json() -> Value {
        json!({"micId": "guest", "finalText": "For God so", "partialText": " loved the",
               "updatedAt": "2026-11-22T09:30:05Z"})
    }

    fn session_json() -> Value {
        json!({"micId": "guest", "state": "reconnecting", "reason": "tooManyConnections",
               "error": "Služba prepisu nedostupná", "audioMsToday": 120000,
               "connectedMsToday": 180000, "capMs": 14400000, "costUsdToday": 0.006,
               "connections": 2, "reconnects": 1, "latencyLastMs": 640, "latencyP50Ms": 700,
               "latencyP90Ms": null})
    }

    fn encode(message: &ListenWsMessage) -> Value {
        serde_json::to_value(message).unwrap()
    }

    fn decode(value: Value) -> ListenWsMessage {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn a_sentence_message_has_the_shared_shape() {
        let message = ListenWsMessage::Sentence {
            sentence: sentence(),
        };
        let expected = json!({"type": "sentence", "sentence": sentence_json()});
        assert_eq!(encode(&message), expected);
        assert_eq!(decode(expected), message);
    }

    #[test]
    fn a_tail_message_has_the_shared_shape() {
        let message = ListenWsMessage::Tail { tail: tail() };
        let expected = json!({"type": "tail", "tail": tail_json()});
        assert_eq!(encode(&message), expected);
        assert_eq!(decode(expected), message);
    }

    #[test]
    fn a_session_message_has_the_shared_shape() {
        let message = ListenWsMessage::Session { session: session() };
        let expected = json!({"type": "session", "session": session_json()});
        assert_eq!(encode(&message), expected);
        assert_eq!(decode(expected), message);
    }

    #[test]
    fn a_snapshot_message_has_the_shared_shape() {
        let snapshot = TranscriptSnapshot {
            sentences: vec![sentence()],
            tails: vec![tail()],
            sessions: vec![session()],
        };
        let message = ListenWsMessage::Snapshot { snapshot };
        let expected = json!({"type": "snapshot", "snapshot": {"sentences": [sentence_json()], "tails": [tail_json()], "sessions": [session_json()]}});
        assert_eq!(encode(&message), expected);
        assert_eq!(decode(expected), message);
    }

    #[test]
    fn an_empty_snapshot_and_an_off_session_serialise_plainly() {
        let message = ListenWsMessage::Snapshot {
            snapshot: TranscriptSnapshot::default(),
        };
        let expected =
            json!({"type": "snapshot", "snapshot": {"sentences": [], "tails": [], "sessions": []}});
        assert_eq!(encode(&message), expected);
        assert_eq!(decode(expected), message);
        let off = MicTranscription {
            mic_id: "guest".into(),
            reason: Some(OffReason::NoKey),
            ..MicTranscription::default()
        };
        let value = serde_json::to_value(&off).unwrap();
        assert_eq!(value["state"], "off");
        assert_eq!(value["reason"], "noKey");
    }

    #[test]
    fn every_state_and_reason_has_a_camel_case_name() {
        let states = [
            (TranscriptionState::Off, "off"),
            (TranscriptionState::Armed, "armed"),
            (TranscriptionState::Connecting, "connecting"),
            (TranscriptionState::Streaming, "streaming"),
            (TranscriptionState::Gated, "gated"),
            (TranscriptionState::Parked, "parked"),
            (TranscriptionState::Reconnecting, "reconnecting"),
            (TranscriptionState::Stopped, "stopped"),
        ];
        for (state, name) in states {
            assert_eq!(serde_json::to_value(state).unwrap(), name);
        }
        let reasons = [
            (OffReason::Disabled, "disabled"),
            (OffReason::NoGuestMic, "noGuestMic"),
            (OffReason::GuestMicMissing, "guestMicMissing"),
            (OffReason::NoKey, "noKey"),
            (OffReason::CapReached, "capReached"),
            (OffReason::NotPreach, "notPreach"),
            (OffReason::OverrideOff, "overrideOff"),
            (OffReason::Auth, "auth"),
            (OffReason::Payment, "payment"),
            (OffReason::Refused, "refused"),
            (OffReason::TooManyConnections, "tooManyConnections"),
        ];
        for (reason, name) in reasons {
            assert_eq!(serde_json::to_value(reason).unwrap(), name);
            let back: OffReason = serde_json::from_value(Value::from(name)).unwrap();
            assert_eq!(back, reason);
        }
    }
}
