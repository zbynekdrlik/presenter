use super::protocol::*;
use super::variables::CompanionVariableState;
use super::{handle_command, validate_token};
use crate::live::LiveEvent;
use crate::state::AppState;
use chrono::{TimeZone, Utc};
use presenter_core::{
    bible::BibleIngestionBatch, BiblePassage, BibleReference, BibleTranslation, StageDisplayLayout,
    StageDisplaySnapshot, TimerState, TimersOverview,
};
use serde_json::{json, Value};
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
            seconds_remaining: 120,
        },
        preach_timer: presenter_core::timer::PreachTimerSnapshot {
            state: TimerState::Paused,
            seconds_elapsed: 30,
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
        Some(slide_id),
        Some(presenter_core::stage_display::StageDisplaySlide {
            main: "Alpha".to_string(),
            translation: "".to_string(),
            stage: "".to_string(),
            group: None,
        }),
        None,
        None,
        presenter_core::timer::TimersOverview::demo(now),
        None,
        Some(1),
        Some(3),
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
        Some(presenter_core::SlideId::new()),
        Some(presenter_core::stage_display::StageDisplaySlide {
            main: "Beta".to_string(),
            translation: "".to_string(),
            stage: "".to_string(),
            group: None,
        }),
        None,
        None,
        presenter_core::timer::TimersOverview::demo(now),
        None,
        Some(1),
        Some(2),
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
    let reference = BibleReference::new_with_code("John", "JHN", 43, 3, 16, 16).unwrap();
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
