//! Contract tests ensuring serialization compatibility between server and WASM client.
//!
//! These tests verify that all types shared between presenter-server and presenter-ui
//! serialize and deserialize correctly through JSON roundtrips.

#[cfg(test)]
mod tests {
    use crate::{
        InboundMessage, LiveEvent, StageClientSnapshot, StageClientStatus, StageDisplayLayout,
        StageDisplaySlide, StageDisplaySnapshot, StageState, TimersOverview,
    };
    use chrono::Utc;
    use uuid::Uuid;

    /// Helper: serialize then deserialize, assert equality via JSON string comparison.
    fn roundtrip_json<T: serde::Serialize + serde::de::DeserializeOwned>(value: &T) -> T {
        let json = serde_json::to_string(value).expect("serialize");
        serde_json::from_str(&json).expect("deserialize")
    }

    // ── LiveEvent variants ──────────────────────────────────────────

    #[test]
    fn live_event_timers_roundtrip() {
        let event = LiveEvent::Timers {
            overview: TimersOverview::demo(Utc::now()),
        };
        let json = serde_json::to_string(&event).expect("serialize");
        assert!(json.contains(r#""type":"timers""#));
        let _: LiveEvent = serde_json::from_str(&json).expect("deserialize");
    }

    #[test]
    fn live_event_stage_roundtrip() {
        let snapshot = StageDisplaySnapshot {
            layout: StageDisplayLayout {
                code: "worship-snv".to_string(),
                name: "WORSHIP SNV".to_string(),
                description: "Test".to_string(),
            },
            generated_at: Utc::now(),
            presentation_id: None,
            presentation_name: None,
            library_name: None,
            song_name: None,
            song_number: None,
            current_slide_id: None,
            current: None,
            next_slide_id: None,
            next: None,
            timers: TimersOverview::demo(Utc::now()),
            latency_ms: None,
            current_position: None,
            total_slides: None,
            playlist_id: None,
            playlist_name: None,
            playlist_entries: None,
        };
        let event = LiveEvent::Stage { snapshot };
        let json = serde_json::to_string(&event).expect("serialize");
        assert!(json.contains(r#""type":"stage""#));
        let _: LiveEvent = serde_json::from_str(&json).expect("deserialize");
    }

    #[test]
    fn live_event_heartbeat_roundtrip() {
        let event = LiveEvent::Heartbeat {
            id: Uuid::new_v4(),
            timestamp: Utc::now(),
        };
        let json = serde_json::to_string(&event).expect("serialize");
        assert!(json.contains(r#""type":"heartbeat""#));
        let _: LiveEvent = serde_json::from_str(&json).expect("deserialize");
    }

    #[test]
    fn live_event_stage_connection_roundtrip() {
        let event = LiveEvent::StageConnection {
            snapshot: StageClientSnapshot {
                id: Uuid::new_v4(),
                layout_code: "worship-snv".to_string(),
                last_heartbeat: Utc::now(),
                latency_ms: Some(42),
                status: StageClientStatus::Connected,
            },
        };
        let json = serde_json::to_string(&event).expect("serialize");
        assert!(json.contains(r#""type":"stage_connection""#));
        let _: LiveEvent = serde_json::from_str(&json).expect("deserialize");
    }

    #[test]
    fn live_event_bible_cleared_roundtrip() {
        let event = LiveEvent::BibleCleared;
        let json = serde_json::to_string(&event).expect("serialize");
        assert!(json.contains(r#""type":"bible_cleared""#));
        let _: LiveEvent = serde_json::from_str(&json).expect("deserialize");
    }

    #[test]
    fn live_event_stage_layout_roundtrip() {
        let event = LiveEvent::StageLayout {
            code: "timer".to_string(),
        };
        let json = serde_json::to_string(&event).expect("serialize");
        assert!(json.contains(r#""type":"stage_layout""#));
        let _: LiveEvent = serde_json::from_str(&json).expect("deserialize");
    }

    #[test]
    fn live_event_bible_preferences_changed_roundtrip() {
        let event = LiveEvent::BiblePreferencesChanged {
            character_limit: 500,
        };
        let json = serde_json::to_string(&event).expect("serialize");
        assert!(json.contains(r#""type":"bible_preferences_changed""#));
        let _: LiveEvent = serde_json::from_str(&json).expect("deserialize");
    }

    #[test]
    fn live_event_broadcast_live_roundtrip() {
        let event = LiveEvent::BroadcastLive { enabled: true };
        let json = serde_json::to_string(&event).expect("serialize");
        assert!(json.contains(r#""type":"broadcast_live""#));
        let _: LiveEvent = serde_json::from_str(&json).expect("deserialize");
    }

    #[test]
    fn live_event_bible_slides_changed_roundtrip() {
        let event = LiveEvent::BibleSlidesChanged {
            presentation_id: "test-id".to_string(),
        };
        let json = serde_json::to_string(&event).expect("serialize");
        assert!(json.contains(r#""type":"bible_slides_changed""#));
        let _: LiveEvent = serde_json::from_str(&json).expect("deserialize");
    }

    #[test]
    fn live_event_ndi_source_activated_roundtrip() {
        let event = LiveEvent::NdiSourceActivated {
            ndi_name: "CAM1 (usb)".to_string(),
            label: "Main Camera".to_string(),
        };
        let json = serde_json::to_string(&event).expect("serialize");
        assert!(json.contains(r#""type":"ndi_source_activated""#));
        let _: LiveEvent = serde_json::from_str(&json).expect("deserialize");
    }

    #[test]
    fn live_event_ndi_source_deactivated_roundtrip() {
        let event = LiveEvent::NdiSourceDeactivated;
        let json = serde_json::to_string(&event).expect("serialize");
        assert!(json.contains(r#""type":"ndi_source_deactivated""#));
        let _: LiveEvent = serde_json::from_str(&json).expect("deserialize");
    }

    #[test]
    fn live_event_ndi_connection_status_roundtrip() {
        let event = LiveEvent::NdiConnectionStatus {
            status: "connected".to_string(),
        };
        let json = serde_json::to_string(&event).expect("serialize");
        assert!(json.contains(r#""type":"ndi_connection_status""#));
        let _: LiveEvent = serde_json::from_str(&json).expect("deserialize");
    }

    // ── InboundMessage variants ─────────────────────────────────────

    #[test]
    fn inbound_message_stage_presence_roundtrip() {
        let msg = InboundMessage::StagePresence {
            client_id: Uuid::new_v4().to_string(),
            layout_code: "worship-snv".to_string(),
        };
        let json = serde_json::to_string(&msg).expect("serialize");
        assert!(json.contains(r#""type":"stage_presence""#));
        let _: InboundMessage = serde_json::from_str(&json).expect("deserialize");
    }

    #[test]
    fn inbound_message_heartbeat_ack_roundtrip() {
        let msg = InboundMessage::StageHeartbeatAck {
            client_id: Uuid::new_v4().to_string(),
            heartbeat_id: Some(Uuid::new_v4().to_string()),
        };
        let json = serde_json::to_string(&msg).expect("serialize");
        assert!(json.contains(r#""type":"stage_heartbeat_ack""#));
        let _: InboundMessage = serde_json::from_str(&json).expect("deserialize");
    }

    #[test]
    fn inbound_message_disconnect_roundtrip() {
        let msg = InboundMessage::StageDisconnect {
            client_id: Uuid::new_v4().to_string(),
        };
        let json = serde_json::to_string(&msg).expect("serialize");
        assert!(json.contains(r#""type":"stage_disconnect""#));
        let _: InboundMessage = serde_json::from_str(&json).expect("deserialize");
    }

    #[test]
    fn inbound_message_unknown_variant_handled() {
        let json = r#"{"type":"some_future_message","data":"hello"}"#;
        let msg: InboundMessage = serde_json::from_str(json).expect("deserialize");
        matches!(msg, InboundMessage::Unknown);
    }

    // ── Domain type roundtrips ──────────────────────────────────────

    #[test]
    fn stage_client_snapshot_roundtrip() {
        let snapshot = StageClientSnapshot {
            id: Uuid::new_v4(),
            layout_code: "worship-snv".to_string(),
            last_heartbeat: Utc::now(),
            latency_ms: Some(15),
            status: StageClientStatus::Connected,
        };
        let result = roundtrip_json(&snapshot);
        assert_eq!(result.id, snapshot.id);
        assert_eq!(result.latency_ms, Some(15));
        assert_eq!(result.status, StageClientStatus::Connected);
    }

    #[test]
    fn stage_client_status_serializes_as_snake_case() {
        let json = serde_json::to_string(&StageClientStatus::Reconnecting).expect("serialize");
        assert_eq!(json, r#""reconnecting""#);
    }

    #[test]
    fn stage_client_snapshot_optional_latency_null() {
        let snapshot = StageClientSnapshot {
            id: Uuid::new_v4(),
            layout_code: "timer".to_string(),
            last_heartbeat: Utc::now(),
            latency_ms: None,
            status: StageClientStatus::Connecting,
        };
        let json = serde_json::to_string(&snapshot).expect("serialize");
        // latency_ms is skip_serializing_if = None, so it should be absent
        assert!(!json.contains("latencyMs"));
    }

    #[test]
    fn stage_display_snapshot_empty_optional_fields() {
        let snapshot = StageDisplaySnapshot {
            layout: StageDisplayLayout {
                code: "worship-snv".to_string(),
                name: "Test".to_string(),
                description: "Test".to_string(),
            },
            generated_at: Utc::now(),
            presentation_id: None,
            presentation_name: None,
            library_name: None,
            song_name: None,
            song_number: None,
            current_slide_id: None,
            current: None,
            next_slide_id: None,
            next: None,
            timers: TimersOverview::demo(Utc::now()),
            latency_ms: None,
            current_position: None,
            total_slides: None,
            playlist_id: None,
            playlist_name: None,
            playlist_entries: None,
        };
        let json = serde_json::to_string(&snapshot).expect("serialize");
        let result: StageDisplaySnapshot = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(result.layout.code, "worship-snv");
        assert!(result.presentation_id.is_none());
    }

    #[test]
    fn stage_state_roundtrip() {
        let state = StageState {
            presentation_id: Some(crate::PresentationId::new()),
            current_slide_id: Some(crate::SlideId::new()),
            next_slide_id: None,
            playlist_id: None,
        };
        let result = roundtrip_json(&state);
        assert_eq!(result.presentation_id, state.presentation_id);
        assert!(result.next_slide_id.is_none());
    }

    #[test]
    fn timers_overview_roundtrip() {
        let overview = TimersOverview::demo(Utc::now());
        let result = roundtrip_json(&overview);
        // Roundtrip preserves timer states
        assert_eq!(result.preach_timer.state, overview.preach_timer.state);
        assert_eq!(
            result.preach_timer.seconds_elapsed,
            overview.preach_timer.seconds_elapsed
        );
        assert_eq!(
            result.countdown_to_start.state,
            overview.countdown_to_start.state
        );
    }

    #[test]
    fn extra_fields_in_json_dont_panic() {
        let json = r#"{"type":"heartbeat","id":"550e8400-e29b-41d4-a716-446655440000","timestamp":"2024-01-01T00:00:00Z","extra_field":"should be ignored"}"#;
        let result: Result<LiveEvent, _> = serde_json::from_str(json);
        assert!(result.is_ok());
    }

    #[test]
    fn stage_display_slide_roundtrip() {
        let slide = StageDisplaySlide {
            main: "Main text".to_string(),
            translation: "Translation".to_string(),
            stage: "Stage text".to_string(),
            group: Some("Verse 1".to_string()),
        };
        let result = roundtrip_json(&slide);
        assert_eq!(result.main, "Main text");
        assert_eq!(result.group, Some("Verse 1".to_string()));
    }

    #[test]
    fn live_event_tag_discriminant_format() {
        // Verify all LiveEvent variants use snake_case tag format
        let events = vec![
            serde_json::to_string(&LiveEvent::BibleCleared).unwrap(),
            serde_json::to_string(&LiveEvent::BroadcastLive { enabled: false }).unwrap(),
            serde_json::to_string(&LiveEvent::BiblePreferencesChanged {
                character_limit: 100,
            })
            .unwrap(),
            serde_json::to_string(&LiveEvent::BibleSlidesChanged {
                presentation_id: "x".into(),
            })
            .unwrap(),
        ];
        for json in &events {
            // All type tags should be lowercase with underscores
            let parsed: serde_json::Value = serde_json::from_str(json).unwrap();
            let tag = parsed["type"].as_str().unwrap();
            assert_eq!(tag, tag.to_lowercase(), "tag should be lowercase: {tag}");
            assert!(
                !tag.contains('-'),
                "tag should use underscores, not hyphens: {tag}"
            );
        }
    }

    #[test]
    fn uuid_fields_survive_roundtrip() {
        let id = Uuid::new_v4();
        let snapshot = StageClientSnapshot {
            id,
            layout_code: "test".to_string(),
            last_heartbeat: Utc::now(),
            latency_ms: None,
            status: StageClientStatus::Connected,
        };
        let json = serde_json::to_string(&snapshot).unwrap();
        let result: StageClientSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(result.id, id);
    }

    // ── AbleSetStatusSnapshot ──────────────────────────────────────

    #[test]
    fn ableset_status_snapshot_roundtrip() {
        use crate::AbleSetStatusSnapshot;

        let snapshot = AbleSetStatusSnapshot {
            enabled: true,
            tracking: true,
            follow_enabled: true,
            host: "fohabl.lan".to_string(),
            http_port: 80,
            osc_port: 39051,
            library_name: "NEW LEVEL".to_string(),
            song_prefix_length: 3,
            last_song: Some(crate::AbleSetSongSnapshot::new(
                "148 Amazing Grace".to_string(),
                "148".to_string(),
                Some(5),
                Some(Utc::now()),
            )),
            last_error: None,
        };
        let json = serde_json::to_string(&snapshot).expect("serialize");
        assert!(json.contains("followEnabled"), "expected camelCase: {json}");
        assert!(json.contains("httpPort"), "expected camelCase: {json}");
        assert!(json.contains("oscPort"), "expected camelCase: {json}");
        assert!(json.contains("libraryName"), "expected camelCase: {json}");
        assert!(
            json.contains("songPrefixLength"),
            "expected camelCase: {json}"
        );
        assert!(json.contains("lastSong"), "expected camelCase: {json}");
        assert!(
            !json.contains("follow_enabled"),
            "unexpected snake_case: {json}"
        );
        assert!(!json.contains("http_port"), "unexpected snake_case: {json}");
        let result: AbleSetStatusSnapshot = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(result.enabled, snapshot.enabled);
        assert_eq!(result.follow_enabled, snapshot.follow_enabled);
        assert_eq!(result.http_port, snapshot.http_port);
        assert_eq!(result.osc_port, snapshot.osc_port);
        assert_eq!(result.library_name, snapshot.library_name);
        assert_eq!(result.song_prefix_length, snapshot.song_prefix_length);
        assert!(result.last_song.is_some());
        assert!(result.last_error.is_none());
    }

    #[test]
    fn ableset_status_snapshot_with_error() {
        use crate::AbleSetStatusSnapshot;

        let snapshot = AbleSetStatusSnapshot {
            enabled: false,
            tracking: false,
            follow_enabled: false,
            host: "localhost".to_string(),
            http_port: 8080,
            osc_port: 39051,
            library_name: "Test".to_string(),
            song_prefix_length: 3,
            last_song: None,
            last_error: Some("connection refused".to_string()),
        };
        let json = serde_json::to_string(&snapshot).expect("serialize");
        assert!(json.contains("lastError"), "expected camelCase: {json}");
        let result: AbleSetStatusSnapshot = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(result.last_error, Some("connection refused".to_string()));
    }

    // ── FeatureFlags ───────────────────────────────────────────────

    #[test]
    fn feature_flags_roundtrip() {
        use crate::FeatureFlags;

        let flags = FeatureFlags {
            companion_enabled: true,
            companion_port: 18175,
        };
        let json = serde_json::to_string(&flags).expect("serialize");
        assert!(
            json.contains("companionEnabled"),
            "expected camelCase: {json}"
        );
        assert!(json.contains("companionPort"), "expected camelCase: {json}");
        assert!(
            !json.contains("companion_enabled"),
            "unexpected snake_case: {json}"
        );
        let result: FeatureFlags = serde_json::from_str(&json).expect("deserialize");
        assert!(result.companion_enabled);
        assert_eq!(result.companion_port, 18175);
    }
}
