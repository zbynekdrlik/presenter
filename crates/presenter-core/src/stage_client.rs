use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageClientStatus {
    Connecting,
    Connected,
    Reconnecting,
    Disconnected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageClientSnapshot {
    pub id: Uuid,
    pub layout_code: String,
    pub last_heartbeat: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u32>,
    pub status: StageClientStatus,
    /// #732 diagnostics: the connecting client's `navigator.userAgent`
    /// (carries the TV WebView's Chromium version), captured on connect.
    /// `None` for a client that predates the diagnostics protocol.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
    /// #732 diagnostics: the latest NDI `<video>` state snapshot this client
    /// reported (on heartbeat + on change). `None` until the first snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ndi_video: Option<NdiVideoDiag>,
    /// #732 diagnostics: when the latest `ndi_video` snapshot was received.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_diag_at: Option<DateTime<Utc>>,
}

/// #732 diagnostics — a point-in-time snapshot of the stage display's NDI
/// `<video data-role="ndi-video">` runtime state, reported by the stage page
/// over the presence/heartbeat socket. The real Vestel/TCL vendor WebViews at
/// events are dark between events and the grey play-arrow never reproduced on
/// the emulated WebViews, so the product must self-report this from every TV.
///
/// EVERY field is `Option` + `#[serde(default)]` so an OLD client (no
/// diagnostics) still deserializes, a NEW client talking to an OLD server is
/// simply ignored, and any single field a given WebView cannot expose degrades
/// to `null` rather than dropping the whole snapshot. Field naming is camelCase
/// on the wire (consistent with `StageClientSnapshot`), symmetric across the
/// client's serialize and the server's deserialize.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NdiVideoDiag {
    /// `HTMLMediaElement.paused`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused: Option<bool>,
    /// `HTMLMediaElement.readyState` (0=HAVE_NOTHING … 4=HAVE_ENOUGH_DATA).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready_state: Option<u8>,
    /// `HTMLVideoElement.videoWidth` — 0 while no frame has decoded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_width: Option<u32>,
    /// `HTMLVideoElement.videoHeight`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_height: Option<u32>,
    /// `HTMLMediaElement.currentTime` (seconds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_time: Option<f64>,
    /// `HTMLMediaElement.error.code` (1..=4) or `None` when `error` is null.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<u16>,
    /// Whether `HTMLMediaElement.srcObject` is a non-null MediaStream.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_src_object: Option<bool>,
    /// `HTMLMediaElement.muted`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub muted: Option<bool>,
    /// `HTMLMediaElement.controls` — should be false; a true here would draw
    /// the native control chrome incl. the play button.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controls: Option<bool>,
    /// `getVideoPlaybackQuality().totalVideoFrames` — `None` when the WebView
    /// lacks the API.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frames_decoded: Option<f64>,
    /// `getVideoPlaybackQuality().droppedVideoFrames`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frames_dropped: Option<f64>,
    /// Milliseconds since the last frame the rVFC observer presented; `None`
    /// when rVFC never fired (no observer / no frame yet).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_frame_age_ms: Option<f64>,
    /// The #568 playback guard's cumulative replay attempt count; `None` when
    /// no guard is installed on the element.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playback_guard_replays: Option<u32>,
    /// Whether the neutral "waiting/connecting" cover is currently mounted
    /// over the video (`ndi_fullscreen.rs` `<Show>`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_visible: Option<bool>,
    /// The stage page's active layout code (from `body[data-layout-code]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout_code: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::InboundMessage;

    /// #732 wire back-compat: an OLD `StageClientSnapshot` JSON (no
    /// user_agent / ndi_video / last_diag_at) must still deserialize — the
    /// server may hold connections registered by clients that predate the
    /// diagnostics protocol.
    #[test]
    fn old_stage_client_snapshot_without_diag_fields_deserializes() {
        let json = r#"{
            "id": "00000000-0000-0000-0000-000000000001",
            "layoutCode": "ndi-fullscreen",
            "lastHeartbeat": "2026-08-23T12:00:00Z",
            "status": "connected"
        }"#;
        let snap: StageClientSnapshot = serde_json::from_str(json).expect("deserialize");
        assert_eq!(snap.layout_code, "ndi-fullscreen");
        assert_eq!(snap.user_agent, None);
        assert_eq!(snap.ndi_video, None);
        assert_eq!(snap.last_diag_at, None);
    }

    /// #732 wire back-compat: an OLD `stage_presence` message (no
    /// user_agent) must still deserialize on a NEW server.
    #[test]
    fn old_stage_presence_without_user_agent_deserializes() {
        let json = r#"{"type":"stage_presence","client_id":"c","layout_code":"timer"}"#;
        match serde_json::from_str::<InboundMessage>(json).expect("deserialize") {
            InboundMessage::StagePresence {
                layout_code,
                user_agent,
                ..
            } => {
                assert_eq!(layout_code, "timer");
                assert_eq!(user_agent, None);
            }
            other => panic!("expected StagePresence, got {other:?}"),
        }
    }

    /// #732 wire back-compat: an OLD `stage_heartbeat_ack` (no ndi_video)
    /// must still deserialize on a NEW server.
    #[test]
    fn old_heartbeat_ack_without_ndi_video_deserializes() {
        let json = r#"{"type":"stage_heartbeat_ack","client_id":"c","heartbeat_id":"h"}"#;
        match serde_json::from_str::<InboundMessage>(json).expect("deserialize") {
            InboundMessage::StageHeartbeatAck { ndi_video, .. } => assert_eq!(ndi_video, None),
            other => panic!("expected StageHeartbeatAck, got {other:?}"),
        }
    }

    /// #732: the new `StageDiag` message round-trips with a `stage_diag` tag.
    #[test]
    fn stage_diag_message_roundtrips() {
        let msg = InboundMessage::StageDiag {
            client_id: "c".to_string(),
            ndi_video: Some(NdiVideoDiag {
                paused: Some(false),
                video_width: Some(1280),
                cover_visible: Some(false),
                ..Default::default()
            }),
        };
        let json = serde_json::to_string(&msg).expect("serialize");
        assert!(json.contains(r#""type":"stage_diag""#));
        match serde_json::from_str::<InboundMessage>(&json).expect("deserialize") {
            InboundMessage::StageDiag { ndi_video, .. } => {
                let d = ndi_video.expect("ndi_video present");
                assert_eq!(d.paused, Some(false));
                assert_eq!(d.video_width, Some(1280));
            }
            other => panic!("expected StageDiag, got {other:?}"),
        }
    }

    /// #732: a PARTIAL `NdiVideoDiag` (a WebView that can only expose some
    /// fields) deserializes — every absent field degrades to `None`, never
    /// dropping the whole snapshot.
    #[test]
    fn partial_ndi_video_diag_deserializes_absent_fields_as_none() {
        let json = r#"{"paused":true,"errorCode":3}"#;
        let d: NdiVideoDiag = serde_json::from_str(json).expect("deserialize");
        assert_eq!(d.paused, Some(true));
        assert_eq!(d.error_code, Some(3));
        assert_eq!(d.video_width, None);
        assert_eq!(d.frames_decoded, None);
        assert_eq!(d.layout_code, None);
    }

    /// An OLD client parsing a NEW `StageClientSnapshot` broadcast: the new
    /// fields serialize under camelCase and round-trip cleanly.
    #[test]
    fn stage_client_snapshot_with_diag_fields_roundtrips() {
        let json = serde_json::json!({
            "id": "00000000-0000-0000-0000-000000000002",
            "layoutCode": "ndi-fullscreen",
            "lastHeartbeat": "2026-08-23T12:00:00Z",
            "status": "connected",
            "userAgent": "Chrome/90",
            "ndiVideo": {"paused": false, "videoWidth": 1280, "coverVisible": false},
            "lastDiagAt": "2026-08-23T12:00:01Z"
        })
        .to_string();
        let snap: StageClientSnapshot = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(snap.user_agent.as_deref(), Some("Chrome/90"));
        let diag = snap.ndi_video.expect("ndi_video present");
        assert_eq!(diag.video_width, Some(1280));
        assert!(snap.last_diag_at.is_some());
        // Re-serialize + re-parse to confirm the camelCase wire is symmetric.
        let back = serde_json::to_string(&snap).expect("serialize");
        let _: StageClientSnapshot = serde_json::from_str(&back).expect("re-deserialize");
    }
}
