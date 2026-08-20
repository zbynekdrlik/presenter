use super::protocol::*;
use super::variables::CompanionVariableState;
use super::*;
use crate::live::LiveEvent;
use chrono::{TimeZone, Utc};
use presenter_core::{bible::BibleIngestionBatch, BiblePassage, BibleTranslation};
use serde_json::json;
use tokio::time::{timeout, Duration};

#[test]
fn token_validation_respects_expected_secret() {
    assert!(validate_token(None, None).is_ok());
    assert!(validate_token(None, Some("abc")).is_ok());
    assert!(validate_token(Some("secret"), Some("secret")).is_ok());
    assert!(validate_token(Some("secret"), Some("wrong")).is_err());
    assert!(validate_token(Some("secret"), None).is_err());
}

#[test]
fn stage_variable_serialisation_populates_defaults() {
    let builder = CompanionVariableState::default().to_variables();
    let map: std::collections::HashMap<_, _> = builder
        .into_iter()
        .map(|var| (var.name, var.value))
        .collect();
    assert_eq!(map.get("stage_current_main").unwrap(), "");
    assert_eq!(map.get("timer_countdown_state").unwrap(), "idle");
    assert_eq!(map.get("timer_countdown_remaining_hhmm").unwrap(), "00:00");
    assert_eq!(map.get("timer_preach_elapsed_hhmm").unwrap(), "00:00");
    assert_eq!(map.get("bible_text").unwrap(), "");
}

#[test]
fn timer_variables_reflect_snapshot() {
    let mut state = CompanionVariableState::default();
    let overview = TimersOverview {
        countdown_to_start: presenter_core::timer::CountdownTimerSnapshot {
            state: TimerState::Running,
            target: Utc.with_ymd_and_hms(2025, 9, 27, 18, 0, 0).unwrap(),
            target_local: chrono::Utc
                .with_ymd_and_hms(2025, 9, 27, 18, 0, 0)
                .unwrap()
                .with_timezone(&chrono::Local)
                .format("%H:%M:%S")
                .to_string(),
            seconds_remaining: 120,
        },
        preach_timer: presenter_core::timer::PreachTimerSnapshot {
            state: TimerState::Paused,
            seconds_elapsed: 30,
            limit_seconds: None,
        },
    };
    state.apply_timers(overview);
    let variables = state.to_variables();
    let map: std::collections::HashMap<_, _> = variables
        .into_iter()
        .map(|var| (var.name, var.value))
        .collect();
    assert_eq!(map.get("timer_countdown_state").unwrap(), "running");
    assert_eq!(map.get("timer_preach_state").unwrap(), "paused");
    assert_eq!(map.get("timer_countdown_remaining_seconds").unwrap(), "120");
    assert_eq!(map.get("timer_countdown_remaining_hhmm").unwrap(), "00:02");
    assert_eq!(map.get("timer_preach_elapsed_hhmm").unwrap(), "00:00");
}

#[test]
fn stage_variables_update_across_layouts() {
    use std::collections::HashMap;

    let mut state = CompanionVariableState::default();
    let now = Utc::now();
    let presentation_id = presenter_core::PresentationId::new();
    let slide_id = presenter_core::SlideId::new();
    let layout = StageDisplayLayout {
        code: "timer".to_string(),
        name: "Timer".to_string(),
        description: "Countdown".to_string(),
    };
    let snapshot = StageDisplaySnapshot::new(
        layout.clone(),
        now,
        Some(presentation_id),
        Some("001 Alpha Song".to_string()),
        Some("Alpha Library".to_string()),
        Some("Alpha Song".to_string()),
        Some("001".to_string()),
        None,
        Some(slide_id),
        Some(presenter_core::stage_display::StageDisplaySlide {
            main: "Alpha".to_string(),
            translation: "".to_string(),
            stage: "".to_string(),
            group: None,
            group_color: None,
        }),
        None,
        None,
        presenter_core::timer::TimersOverview::demo(now),
        None,
        Some(1),
        Some(3),
        None,
        None,
        None,
        Vec::new(), // upcoming_groups
    );

    assert!(state.apply_stage_snapshot(snapshot));
    let map: HashMap<_, _> = state
        .to_variables()
        .into_iter()
        .map(|var| (var.name, var.value))
        .collect();
    assert_eq!(map.get("song_name"), Some(&"Alpha Song".to_string()));
    assert_eq!(map.get("band_name"), Some(&"Alpha Library".to_string()));

    let next_snapshot = StageDisplaySnapshot::new(
        layout,
        now + chrono::Duration::seconds(1),
        Some(presenter_core::PresentationId::new()),
        Some("002 Beta Hymn".to_string()),
        Some("Beta Library".to_string()),
        Some("Beta Hymn".to_string()),
        Some("002".to_string()),
        None,
        Some(presenter_core::SlideId::new()),
        Some(presenter_core::stage_display::StageDisplaySlide {
            main: "Beta".to_string(),
            translation: "".to_string(),
            stage: "".to_string(),
            group: None,
            group_color: None,
        }),
        None,
        None,
        presenter_core::timer::TimersOverview::demo(now),
        None,
        Some(1),
        Some(2),
        None,
        None,
        None,
        Vec::new(), // upcoming_groups
    );

    assert!(state.apply_stage_snapshot(next_snapshot));
    let updated: HashMap<_, _> = state
        .to_variables()
        .into_iter()
        .map(|var| (var.name, var.value))
        .collect();
    assert_eq!(updated.get("song_name"), Some(&"Beta Hymn".to_string()));
    assert_eq!(updated.get("band_name"), Some(&"Beta Library".to_string()));
}

#[tokio::test]
async fn stage_set_command_updates_state_and_emits_event() {
    let state = AppState::in_memory().await.unwrap();
    crate::state::seed_sample_library(&state).await.unwrap();
    let libraries = state.libraries().await.unwrap();
    let presentation = &libraries[0].presentations[0];
    let current = &presentation.slides[0];
    let presentation_id = presentation.id.to_string();
    let current_id = current.id.to_string();
    let next = presentation.slides.get(1).map(|slide| slide.id.to_string());

    let payload = json!({
        "presentationId": presentation_id,
        "currentSlideId": current_id,
        "nextSlideId": next.clone(),
    });

    let mut variables = CompanionVariableState::default();
    let mut rx = state.live_hub().subscribe();

    let response = handle_command(&state, &mut variables, "stage.set", payload)
        .await
        .unwrap();

    match response.reply {
        Some(OutgoingMessage::Ack { ref command }) => assert_eq!(command, "stage.set"),
        other => panic!("unexpected response: {other:?}"),
    }
    assert!(response.refresh_variables);
    let stage = variables.stage.as_ref().expect("stage variables present");
    assert_eq!(stage.current_slide_id.as_deref(), Some(current_id.as_str()));

    let mut saw_stage = false;
    for _ in 0..5 {
        let event = timeout(Duration::from_millis(250), rx.recv())
            .await
            .expect("event")
            .unwrap();
        if matches!(event, LiveEvent::Stage { .. }) {
            saw_stage = true;
            break;
        }
    }
    assert!(saw_stage, "expected stage live event");
}

#[tokio::test]
async fn timer_command_updates_overview_and_broadcasts() {
    let state = AppState::in_memory().await.unwrap();
    let target = (Utc::now() + chrono::Duration::minutes(30)).to_rfc3339();
    let payload = json!({ "target": target });
    let mut variables = CompanionVariableState::default();
    let mut rx = state.live_hub().subscribe();

    let response = handle_command(
        &state,
        &mut variables,
        "timer.set_countdown_target",
        payload,
    )
    .await
    .unwrap();

    match response.reply {
        Some(OutgoingMessage::Ack { ref command }) => {
            assert_eq!(command, "timer.set_countdown_target")
        }
        other => panic!("unexpected response: {other:?}"),
    }
    assert!(response.refresh_variables);
    let timers = variables.timers.as_ref().expect("timers populated");
    assert_eq!(timers.countdown_to_start.target.to_rfc3339(), target);

    let mut saw_timers = false;
    for _ in 0..5 {
        let event = timeout(Duration::from_millis(250), rx.recv())
            .await
            .expect("event")
            .unwrap();
        if matches!(event, LiveEvent::Timers { .. }) {
            saw_timers = true;
            break;
        }
    }
    assert!(saw_timers, "expected timers event");
}

#[tokio::test]
async fn bible_trigger_and_clear_flow_updates_variables() {
    let state = AppState::in_memory().await.unwrap();

    let translation = BibleTranslation::new("KJV", "King James Version", "en");
    let reference = BibleReference::new("John", 3, 16, 16).unwrap();
    let passage = BiblePassage::new(
        reference.clone(),
        translation.clone(),
        "For God so loved the world".into(),
    );
    let batch = BibleIngestionBatch::new(translation.clone(), vec![passage]).unwrap();

    state
        .repository()
        .replace_bible_translation_passages(&batch)
        .await
        .unwrap();

    let mut variables = CompanionVariableState::default();
    let mut rx = state.live_hub().subscribe();

    let trigger_payload = json!({
        "translation": "KJV",
        "book": "John",
        "chapter": 3,
        "verseStart": 16,
    });

    let trigger_response = handle_command(&state, &mut variables, "bible.trigger", trigger_payload)
        .await
        .unwrap();
    assert!(matches!(
        trigger_response.reply,
        Some(OutgoingMessage::Ack { ref command }) if command == "bible.trigger"
    ));
    assert!(trigger_response.refresh_variables);
    assert!(variables.bible.is_some());

    let mut saw_bible = false;
    for _ in 0..5 {
        let event = timeout(Duration::from_millis(250), rx.recv())
            .await
            .expect("event")
            .unwrap();
        match event {
            LiveEvent::Bible { .. } => {
                saw_bible = true;
                break;
            }
            _ => continue,
        }
    }
    assert!(saw_bible, "expected bible broadcast");

    let clear_response = handle_command(&state, &mut variables, "bible.clear", Value::Null)
        .await
        .unwrap();
    assert!(matches!(
        clear_response.reply,
        Some(OutgoingMessage::Ack { ref command }) if command == "bible.clear"
    ));
    assert!(clear_response.refresh_variables);
    assert!(variables.bible.is_none());

    let mut saw_clear = false;
    for _ in 0..5 {
        let event = timeout(Duration::from_millis(250), rx.recv())
            .await
            .expect("event")
            .unwrap();
        match event {
            LiveEvent::BibleCleared => {
                saw_clear = true;
                break;
            }
            _ => continue,
        }
    }
    assert!(saw_clear, "expected bible cleared event");
}

#[test]
fn parse_command_accepts_all_documented_commands() {
    let target = (Utc::now() + chrono::Duration::minutes(10)).to_rfc3339();
    let cases: Vec<(&str, Value)> = vec![
        ("timer.start_countdown", json!({})),
        ("timer.pause_countdown", json!({})),
        ("timer.reset_countdown", json!({})),
        ("timer.set_countdown_target", json!({ "target": target })),
        ("timer.start_preach", json!({})),
        ("timer.pause_preach", json!({})),
        ("timer.reset_preach", json!({})),
        ("timer.set_preach_limit", json!({ "seconds": 2700 })),
        ("timer.clear_preach_limit", json!({})),
        ("stage.layout", json!({ "code": "timer" })),
        (
            "stage.set",
            json!({
                "presentationId": "00000000-0000-0000-0000-000000000001",
                "currentSlideId": "00000000-0000-0000-0000-000000000002",
            }),
        ),
        (
            "bible.trigger",
            json!({
                "translation": "KJV",
                "book": "John",
                "chapter": 3,
                "verseStart": 16,
            }),
        ),
        ("bible.clear", json!({})),
        ("broadcast.set_live", json!({ "enabled": true })),
    ];

    for (command, payload) in &cases {
        let result = parse_command(command, payload.clone());
        assert!(
            result.is_ok(),
            "parse_command({command}) should succeed but got: {:?}",
            result.err()
        );
    }

    let unknown = parse_command("nonexistent.command", json!({}));
    assert!(unknown.is_err(), "unknown command should return error");
}

#[test]
fn bible_slide_event_updates_companion_variables() {
    use presenter_core::bible::BibleSlideOutput;

    let mut state = CompanionVariableState::default();
    let now = Utc::now();

    let output = BibleSlideOutput {
        main_text: "For God so loved the world".into(),
        main_reference: "John 3:16 (KJV)".into(),
        secondary_text: String::new(),
        secondary_reference: String::new(),
        triggered_at: now,
    };

    let changed = state.apply_live_event(LiveEvent::BibleSlide { output });
    assert!(changed, "BibleSlide should mark variables as changed");

    let vars: std::collections::HashMap<_, _> = state
        .to_variables()
        .into_iter()
        .map(|v| (v.name, v.value))
        .collect();

    assert_eq!(vars.get("bible_reference").unwrap(), "John 3:16 (KJV)");
    assert_eq!(
        vars.get("bible_text").unwrap(),
        "For God so loved the world"
    );
    assert_eq!(vars.get("bible_translation_code").unwrap(), "KJV");
    assert!(!vars.get("bible_triggered_at").unwrap().is_empty());
}

// ---- Stream-graphics companion commands + variables (#711) ----------------

/// Seed a uniquely-slugged output with a base scene "Chvaly" and an overlay
/// scene "Verse". Returns `(base_id, overlay_id)`. Unique slug per test — the
/// in-memory DB is process-shared (`.claude/rules/stream-graphics.md`).
async fn seed_stream(state: &AppState, slug: &str) -> (i64, i64) {
    use presenter_core::SceneKind;
    state
        .repository()
        .create_stream_output(slug, "Test")
        .await
        .unwrap();
    let base = state
        .repository()
        .create_stream_scene(slug, "Chvaly", SceneKind::Base)
        .await
        .unwrap();
    let overlay = state
        .repository()
        .create_stream_scene(slug, "Verse", SceneKind::Overlay)
        .await
        .unwrap();
    (base.id, overlay.id)
}

async fn expect_ack(state: &AppState, command: &str, payload: Value) {
    let mut variables = CompanionVariableState::default();
    let response = handle_command(state, &mut variables, command, payload)
        .await
        .unwrap();
    match response.reply {
        Some(OutgoingMessage::Ack { command: acked }) => {
            assert_eq!(acked, command, "ack command mismatch")
        }
        other => panic!("expected ack for {command}, got {other:?}"),
    }
}

async fn expect_error(state: &AppState, command: &str, payload: Value) -> String {
    let mut variables = CompanionVariableState::default();
    let response = handle_command(state, &mut variables, command, payload)
        .await
        .unwrap();
    match response.reply {
        Some(OutgoingMessage::Error { message }) => message,
        other => panic!("expected error reply for {command}, got {other:?}"),
    }
}

#[tokio::test]
async fn stream_scene_set_activates_base_and_emits_event() {
    let state = AppState::in_memory().await.unwrap();
    let (base, _overlay) = seed_stream(&state, "t711-set").await;
    let mut rx = state.live_hub().subscribe();

    expect_ack(
        &state,
        "stream_scene_set",
        json!({ "scene": "Chvaly", "output": "t711-set" }),
    )
    .await;

    assert_eq!(
        state
            .stream_show_state("t711-set")
            .await
            .unwrap()
            .active_scene_id,
        Some(base)
    );

    let mut saw = false;
    for _ in 0..5 {
        let event = timeout(Duration::from_millis(250), rx.recv())
            .await
            .expect("event")
            .unwrap();
        if matches!(event, LiveEvent::StreamState { ref output, .. } if output == "t711-set") {
            saw = true;
            break;
        }
    }
    assert!(saw, "expected StreamState live event");
}

#[tokio::test]
async fn stream_scene_set_matches_name_case_insensitively() {
    let state = AppState::in_memory().await.unwrap();
    let (base, _overlay) = seed_stream(&state, "t711-ci").await;

    // Stored name is "Chvaly"; the command sends "chvaly".
    expect_ack(
        &state,
        "stream_scene_set",
        json!({ "scene": "chvaly", "output": "t711-ci" }),
    )
    .await;

    assert_eq!(
        state
            .stream_show_state("t711-ci")
            .await
            .unwrap()
            .active_scene_id,
        Some(base)
    );
}

#[tokio::test]
async fn stream_scene_set_unknown_scene_errors_without_state_change() {
    let state = AppState::in_memory().await.unwrap();
    seed_stream(&state, "t711-unknown-scene").await;

    let message = expect_error(
        &state,
        "stream_scene_set",
        json!({ "scene": "Nope", "output": "t711-unknown-scene" }),
    )
    .await;
    assert!(message.contains("unknown stream scene"), "got: {message}");

    // No base was activated.
    assert_eq!(
        state
            .stream_show_state("t711-unknown-scene")
            .await
            .unwrap()
            .active_scene_id,
        None
    );
}

#[tokio::test]
async fn stream_scene_set_unknown_output_errors() {
    let state = AppState::in_memory().await.unwrap();
    let message = expect_error(
        &state,
        "stream_scene_set",
        json!({ "scene": "Chvaly", "output": "t711-nonexistent-output" }),
    )
    .await;
    assert!(message.contains("unknown stream output"), "got: {message}");
}

#[tokio::test]
async fn stream_scene_set_on_overlay_name_is_refused() {
    let state = AppState::in_memory().await.unwrap();
    seed_stream(&state, "t711-wrongkind").await;

    // "Verse" is an OVERLAY — activating it as the base must be refused (typed
    // RepositoryError::Invalid → error reply), never a panic.
    let message = expect_error(
        &state,
        "stream_scene_set",
        json!({ "scene": "Verse", "output": "t711-wrongkind" }),
    )
    .await;
    assert!(message.contains("not a base scene"), "got: {message}");
    assert_eq!(
        state
            .stream_show_state("t711-wrongkind")
            .await
            .unwrap()
            .active_scene_id,
        None
    );
}

#[tokio::test]
async fn stream_overlay_on_then_off() {
    let state = AppState::in_memory().await.unwrap();
    let (_base, overlay) = seed_stream(&state, "t711-overlay").await;

    expect_ack(
        &state,
        "stream_overlay_on",
        json!({ "scene": "Verse", "output": "t711-overlay" }),
    )
    .await;
    assert_eq!(
        state
            .stream_show_state("t711-overlay")
            .await
            .unwrap()
            .active_overlay_ids,
        vec![overlay]
    );

    expect_ack(
        &state,
        "stream_overlay_off",
        json!({ "scene": "Verse", "output": "t711-overlay" }),
    )
    .await;
    assert!(state
        .stream_show_state("t711-overlay")
        .await
        .unwrap()
        .active_overlay_ids
        .is_empty());
}

#[tokio::test]
async fn stream_overlay_toggle_flips_twice_to_original() {
    let state = AppState::in_memory().await.unwrap();
    let (_base, overlay) = seed_stream(&state, "t711-toggle").await;

    // First toggle turns it on.
    expect_ack(
        &state,
        "stream_overlay_toggle",
        json!({ "scene": "Verse", "output": "t711-toggle" }),
    )
    .await;
    assert_eq!(
        state
            .stream_show_state("t711-toggle")
            .await
            .unwrap()
            .active_overlay_ids,
        vec![overlay]
    );

    // Second toggle turns it back off (original state).
    expect_ack(
        &state,
        "stream_overlay_toggle",
        json!({ "scene": "Verse", "output": "t711-toggle" }),
    )
    .await;
    assert!(state
        .stream_show_state("t711-toggle")
        .await
        .unwrap()
        .active_overlay_ids
        .is_empty());
}

#[tokio::test]
async fn stream_scene_clear_and_clear_reset_state() {
    let state = AppState::in_memory().await.unwrap();
    let (base, overlay) = seed_stream(&state, "t711-clear").await;

    // Activate base + overlay first.
    expect_ack(
        &state,
        "stream_scene_set",
        json!({ "scene": "Chvaly", "output": "t711-clear" }),
    )
    .await;
    expect_ack(
        &state,
        "stream_overlay_on",
        json!({ "scene": "Verse", "output": "t711-clear" }),
    )
    .await;

    // stream_scene_clear drops only the base.
    expect_ack(
        &state,
        "stream_scene_clear",
        json!({ "output": "t711-clear" }),
    )
    .await;
    let after_scene_clear = state.stream_show_state("t711-clear").await.unwrap();
    assert_eq!(after_scene_clear.active_scene_id, None);
    assert_eq!(after_scene_clear.active_overlay_ids, vec![overlay]);

    // Re-set the base, then stream_clear drops base + all overlays.
    expect_ack(
        &state,
        "stream_scene_set",
        json!({ "scene": "Chvaly", "output": "t711-clear" }),
    )
    .await;
    assert_eq!(
        state
            .stream_show_state("t711-clear")
            .await
            .unwrap()
            .active_scene_id,
        Some(base)
    );
    expect_ack(&state, "stream_clear", json!({ "output": "t711-clear" })).await;
    let after_clear = state.stream_show_state("t711-clear").await.unwrap();
    assert_eq!(after_clear.active_scene_id, None);
    assert!(after_clear.active_overlay_ids.is_empty());
}

#[test]
fn stream_command_defaults_output_to_stream_and_trims() {
    use super::stream::StreamCommand;

    // Omitting `output` defaults it to "stream"; scene + output are trimmed.
    match parse_command("stream_scene_set", json!({ "scene": "  Chvaly  " })) {
        Ok(CompanionCommand::Stream(StreamCommand::SceneSet { output, scene })) => {
            assert_eq!(output, "stream");
            assert_eq!(scene, "Chvaly");
        }
        _ => panic!("expected a Stream(SceneSet) with defaulted output"),
    }

    // A blank output also falls back to the default.
    match parse_command("stream_clear", json!({ "output": "   " })) {
        Ok(CompanionCommand::Stream(StreamCommand::Clear { output })) => {
            assert_eq!(output, "stream");
        }
        _ => panic!("expected a Stream(Clear) with defaulted output"),
    }
}

#[tokio::test]
async fn resolve_stream_variables_maps_ids_to_names() {
    use super::stream::resolve_stream_variables;
    let state = AppState::in_memory().await.unwrap();
    let (base, overlay) = seed_stream(&state, "t711-vars").await;

    let vars = resolve_stream_variables(&state, "t711-vars", Some(base), &[overlay]).await;
    assert_eq!(vars.scene, "Chvaly");
    assert_eq!(vars.overlays, "Verse");

    // Cleared activation → placeholders.
    let cleared = resolve_stream_variables(&state, "t711-vars", None, &[]).await;
    assert_eq!(cleared.scene, "-");
    assert_eq!(cleared.overlays, "-");
}

#[tokio::test]
async fn apply_stream_state_event_resolves_names_and_ignores_other_events() {
    use super::stream::apply_stream_state_event;
    let state = AppState::in_memory().await.unwrap();
    let (base, overlay) = seed_stream(&state, "t711-event").await;
    let mut variables = CompanionVariableState::default();

    // A StreamState event is resolved id→name and updates the variables (this is
    // the companion live-loop glue that the mod.rs branch routes to).
    let event = LiveEvent::StreamState {
        output: "t711-event".to_string(),
        active_scene_id: Some(base),
        active_overlay_ids: vec![overlay],
        config_revision: 0,
    };
    assert!(apply_stream_state_event(&state, &mut variables, &event).await);
    let map: std::collections::HashMap<_, _> = variables
        .to_variables()
        .into_iter()
        .map(|var| (var.name, var.value))
        .collect();
    assert_eq!(map.get("stream_scene").unwrap(), "Chvaly");
    assert_eq!(map.get("stream_overlays").unwrap(), "Verse");

    // A non-StreamState event is not this glue's concern → no change reported.
    assert!(!apply_stream_state_event(&state, &mut variables, &LiveEvent::BibleCleared).await);
}

#[test]
fn stream_variables_default_to_placeholders() {
    let map: std::collections::HashMap<_, _> = CompanionVariableState::default()
        .to_variables()
        .into_iter()
        .map(|var| (var.name, var.value))
        .collect();
    assert_eq!(map.get("stream_scene").unwrap(), "-");
    assert_eq!(map.get("stream_overlays").unwrap(), "-");
}

#[test]
fn apply_stream_state_updates_variables_and_dedups() {
    use super::stream::StreamVariables;
    let mut state = CompanionVariableState::default();
    let vars = StreamVariables {
        scene: "Chvaly".to_string(),
        overlays: "Verse, Lower Third".to_string(),
    };
    assert!(state.apply_stream_state(vars.clone()));
    // Idempotent: applying the same values again reports no change.
    assert!(!state.apply_stream_state(vars));

    let map: std::collections::HashMap<_, _> = state
        .to_variables()
        .into_iter()
        .map(|var| (var.name, var.value))
        .collect();
    assert_eq!(map.get("stream_scene").unwrap(), "Chvaly");
    assert_eq!(map.get("stream_overlays").unwrap(), "Verse, Lower Third");
}

#[test]
fn parse_command_accepts_all_stream_commands() {
    let cases: Vec<(&str, Value)> = vec![
        (
            "stream_scene_set",
            json!({ "scene": "Chvaly", "output": "stream" }),
        ),
        ("stream_scene_clear", json!({ "output": "stream" })),
        (
            "stream_overlay_on",
            json!({ "scene": "Verse", "output": "stream" }),
        ),
        (
            "stream_overlay_off",
            json!({ "scene": "Verse", "output": "stream" }),
        ),
        (
            "stream_overlay_toggle",
            json!({ "scene": "Verse", "output": "stream" }),
        ),
        ("stream_clear", json!({ "output": "stream" })),
    ];
    for (command, payload) in &cases {
        let parsed = parse_command(command, payload.clone());
        assert!(
            matches!(parsed, Ok(CompanionCommand::Stream(_))),
            "parse_command({command}) should be a Stream command, got {:?}",
            parsed.err()
        );
    }

    // Missing scene on a scene-required command is a parse error.
    let missing_scene = parse_command("stream_scene_set", json!({ "output": "stream" }));
    assert!(missing_scene.is_err(), "missing scene should error");

    // An unknown stream_* command is still an error (not a silent accept).
    let unknown = parse_command("stream_bogus", json!({}));
    assert!(unknown.is_err(), "unknown stream_* command should error");
}

#[test]
fn bible_slide_event_cleared_by_bible_cleared() {
    use presenter_core::bible::BibleSlideOutput;

    let mut state = CompanionVariableState::default();

    let output = BibleSlideOutput {
        main_text: "In the beginning".into(),
        main_reference: "Genesis 1:1 (SEB)".into(),
        secondary_text: String::new(),
        secondary_reference: String::new(),
        triggered_at: Utc::now(),
    };

    state.apply_live_event(LiveEvent::BibleSlide { output });
    assert!(state.apply_live_event(LiveEvent::BibleCleared));

    let vars: std::collections::HashMap<_, _> = state
        .to_variables()
        .into_iter()
        .map(|v| (v.name, v.value))
        .collect();

    assert_eq!(vars.get("bible_text").unwrap(), "");
    assert_eq!(vars.get("bible_reference").unwrap(), "");
    assert_eq!(vars.get("bible_translation_code").unwrap(), "");
}
