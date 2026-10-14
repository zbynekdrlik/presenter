//! `GET /listen/status` (#852 C1a, spec §5.4 "Health", §14): the socket, each
//! configured mic's counters, the streams seen but not used (they help the
//! first setup); lane C1a-3 adds the refused senders and its counters. Read by
//! the C1b Settings card and the box acceptance (§5.8: counters read before
//! and after a capture and diffed).

use crate::audio::vban::VbanAudioFormat;
use crate::listen::mics::MicConfig;
use serde::{Deserialize, Serialize};

/// The VBAN receive socket's life cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SocketState {
    /// The receiver was never started (validate mode, tests).
    NotStarted,
    /// No mic configured: no socket (C-S8).
    Idle,
    Bound,
    /// Binding or receiving failed; retried with a growing pause.
    Retrying,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SocketStatus {
    pub state: SocketState,
    pub bind_addr: Option<String>,
    pub local_addr: Option<String>,
    /// Slovak, for the operator.
    pub error: Option<String>,
}

/// One configured mic's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MicState {
    /// Configured, no packet yet.
    Waiting,
    Receiving,
    /// No packet for 3 s ("žiadny zvuk", C-S1).
    NoAudio,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MicStatus {
    #[serde(flatten)]
    pub config: MicConfig,
    pub state: MicState,
    /// `ip:port` of the mic's sender (from C1a-3: the one it is locked to).
    pub sender: Option<String>,
    pub format: Option<VbanAudioFormat>,
    pub frames_per_packet: Option<u16>,
    pub packets: u64,
    pub last_packet_age_ms: Option<u64>,
}

/// A stream seen on the socket that no mic uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnmatchedStream {
    pub stream_name: String,
    pub sender: String,
    pub format: VbanAudioFormat,
    pub packets: u64,
    pub last_packet_age_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListenStatus {
    pub socket: SocketStatus,
    pub mics: Vec<MicStatus>,
    pub unmatched: Vec<UnmatchedStream>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::vban::VbanSampleType;
    use crate::listen::mics::{MicChannel, MicLanguage, MicRole};
    use serde_json::json;

    #[test]
    fn the_status_wire_shape_is_exact() {
        let status = ListenStatus {
            socket: SocketStatus {
                state: SocketState::Bound,
                bind_addr: Some("0.0.0.0:6980".into()),
                local_addr: Some("0.0.0.0:6980".into()),
                error: None,
            },
            mics: vec![MicStatus {
                config: MicConfig {
                    id: "guest".into(),
                    name: "Hosť".into(),
                    stream_name: "GUEST_EN".into(),
                    sender_ip: None,
                    channel: MicChannel::Left,
                    language: MicLanguage::En,
                    role: MicRole::Guest,
                },
                state: MicState::NoAudio,
                sender: Some("192.0.2.10:6980".into()),
                format: Some(VbanAudioFormat {
                    rate_hz: 48_000,
                    channels: 1,
                    sample_type: VbanSampleType::Int16,
                }),
                frames_per_packet: Some(256),
                packets: 936,
                last_packet_age_ms: Some(3_200),
            }],
            unmatched: vec![UnmatchedStream {
                stream_name: "Stream1".into(),
                sender: "198.51.100.7:6980".into(),
                format: VbanAudioFormat {
                    rate_hz: 48_000,
                    channels: 2,
                    sample_type: VbanSampleType::Int24,
                },
                packets: 3,
                last_packet_age_ms: 40,
            }],
        };
        let wire = serde_json::to_value(&status).unwrap();
        assert_eq!(
            wire,
            json!({
                "socket": {"state": "bound", "bindAddr": "0.0.0.0:6980", "localAddr": "0.0.0.0:6980", "error": null},
                "mics": [{
                    "id": "guest", "name": "Hosť", "streamName": "GUEST_EN", "senderIp": null,
                    "channel": "left", "language": "en", "role": "guest",
                    "state": "no_audio", "sender": "192.0.2.10:6980",
                    "format": {"rateHz": 48000, "channels": 1, "sampleType": "int16"},
                    "framesPerPacket": 256, "packets": 936,
                    "lastPacketAgeMs": 3200
                }],
                "unmatched": [{"streamName": "Stream1", "sender": "198.51.100.7:6980",
                    "format": {"rateHz": 48000, "channels": 2, "sampleType": "int24"},
                    "packets": 3, "lastPacketAgeMs": 40}]
            })
        );
        let back: ListenStatus = serde_json::from_value(wire).unwrap();
        assert_eq!(back, status);
        assert_eq!(
            serde_json::to_value(SocketState::NotStarted).unwrap(),
            json!("not_started")
        );
    }
}
