//! `GET /listen/status` (#852 C1a, spec §5.4 "Health", §14): the socket, each
//! configured mic's counters, the streams seen but not used (they help the
//! first setup), the senders whose datagrams were refused and the recorder
//! (C1b). Read by
//! the C1b Settings card and the box acceptance (§5.8: counters read before
//! and after a capture and diffed).

use crate::audio::vban::VbanAudioFormat;
use crate::listen::mics::MicConfig;
use crate::listen::recorder::RecorderStatus;
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
    /// Packets arrive, but the converter refused their format.
    Error,
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
    pub packets_per_sec: f32,
    pub last_packet_age_ms: Option<u64>,
    pub lost_packets: u64,
    pub late_packets: u64,
    pub resyncs: u64,
    pub dropped_blocks: u64,
    pub other_sender_packets: u64,
    /// 100 ms frames of 16 kHz produced.
    pub frames_out: u64,
    /// The last frame's RMS level; only while receiving.
    pub level_dbfs: Option<f32>,
    pub voiced: bool,
    /// Slovak, for the operator (e.g. an unsupported sample rate).
    pub error: Option<String>,
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

/// A sender whose datagrams were refused, counted by reject kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RejectedSender {
    /// The sender's IP.
    pub sender: String,
    pub not_vban: u64,
    pub non_audio: u64,
    pub unsupported_codec: u64,
    pub unsupported_type: u64,
    pub corrupt: u64,
    pub last_packet_age_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListenStatus {
    pub socket: SocketStatus,
    pub mics: Vec<MicStatus>,
    pub unmatched: Vec<UnmatchedStream>,
    pub rejected: Vec<RejectedSender>,
    /// The recorder (#852 C1b): the per-day toggle, the folder, each mic's file.
    pub recorder: RecorderStatus,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::vban::VbanSampleType;
    use crate::listen::mics::{MicChannel, MicLanguage, MicRole};
    use serde_json::json;

    /// The guest mic of the wire test.
    fn guest_status() -> MicStatus {
        MicStatus {
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
            packets_per_sec: 0.0,
            last_packet_age_ms: Some(3_200),
            lost_packets: 1,
            late_packets: 0,
            resyncs: 0,
            dropped_blocks: 0,
            other_sender_packets: 0,
            frames_out: 49,
            level_dbfs: None,
            voiced: false,
            error: None,
        }
    }

    #[test]
    fn the_status_wire_shape_is_exact() {
        let status = ListenStatus {
            socket: SocketStatus {
                state: SocketState::Bound,
                bind_addr: Some("0.0.0.0:6980".into()),
                local_addr: Some("0.0.0.0:6980".into()),
                error: None,
            },
            mics: vec![guest_status()],
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
            rejected: vec![RejectedSender {
                sender: "198.51.100.7".into(),
                not_vban: 1,
                non_audio: 2,
                unsupported_codec: 0,
                unsupported_type: 0,
                corrupt: 0,
                last_packet_age_ms: 10,
            }],
            recorder: RecorderStatus::default(),
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
                    "framesPerPacket": 256, "packets": 936, "packetsPerSec": 0.0,
                    "lastPacketAgeMs": 3200, "lostPackets": 1, "latePackets": 0,
                    "resyncs": 0, "droppedBlocks": 0, "otherSenderPackets": 0,
                    "framesOut": 49, "levelDbfs": null, "voiced": false, "error": null
                }],
                "unmatched": [{"streamName": "Stream1", "sender": "198.51.100.7:6980",
                    "format": {"rateHz": 48000, "channels": 2, "sampleType": "int24"},
                    "packets": 3, "lastPacketAgeMs": 40}],
                "rejected": [{"sender": "198.51.100.7", "notVban": 1, "nonAudio": 2,
                    "unsupportedCodec": 0, "unsupportedType": 0, "corrupt": 0,
                    "lastPacketAgeMs": 10}],
                "recorder": {"enabled": false, "enabledFor": null, "root": "", "mics": []}
            })
        );
        let back: ListenStatus = serde_json::from_value(wire).unwrap();
        assert_eq!(back, status);
        assert_eq!(
            serde_json::to_value(SocketState::NotStarted).unwrap(),
            json!("not_started")
        );
        assert_eq!(
            serde_json::to_value(MicState::Error).unwrap(),
            json!("error")
        );
    }
}
