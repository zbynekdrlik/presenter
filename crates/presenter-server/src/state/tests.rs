use super::*;
use crate::live::LiveEvent;
use chrono::Utc;
use presenter_core::{
    bible::BibleIngestionBatch, BiblePassage, BibleReference, BibleTranslation, Library, LibraryId,
    Presentation, Slide, SlideContent, SlideId, SlideText, TimerCommand, TimerState,
};
use std::collections::HashMap;

#[tokio::test]
async fn seeded_state_contains_library() {
    let state = AppState::in_memory().await.unwrap();
    let libraries = state.libraries().await.unwrap();
    assert_eq!(libraries.len(), 1);
    assert_eq!(libraries[0].name, "Sample Library");
}

#[tokio::test]
async fn stage_updates_emit_live_event() {
    let state = AppState::in_memory().await.unwrap();
    let hub = state.live_hub();
    let mut rx = hub.subscribe();

    let libraries = state.libraries().await.unwrap();
    let presentation = &libraries[0].presentations[0];
    let current = presentation.slides[0].id;
    let next = presentation.slides.get(1).map(|slide| slide.id);

    state
        .update_stage_state(presentation.id, current, next)
        .await
        .unwrap();

    let mut seen_stage = false;
    for _ in 0..5 {
        match rx.recv().await.unwrap() {
            LiveEvent::Stage { snapshot } => {
                seen_stage = true;
                assert_eq!(snapshot.presentation_name.unwrap(), presentation.name);
                break;
            }
            _ => continue,
        }
    }

    assert!(seen_stage, "expected stage event after update");
}

#[tokio::test]
async fn clear_stage_emits_blank_snapshot() {
    let state = AppState::in_memory().await.unwrap();
    let hub = state.live_hub();
    let mut rx = hub.subscribe();

    state.clear_stage().await.unwrap();

    let stored = state
        .repository()
        .get_stage_state()
        .await
        .unwrap()
        .expect("stage state persisted");
    assert!(stored.presentation_id.is_none());
    assert!(stored.current_slide_id.is_none());

    let mut saw_stage = false;
    for _ in 0..5 {
        match rx.recv().await.unwrap() {
            LiveEvent::Stage { snapshot } => {
                assert!(snapshot.presentation_id.is_none());
                assert!(snapshot.current.is_none());
                saw_stage = true;
                break;
            }
            _ => continue,
        }
    }

    assert!(saw_stage, "expected stage event after clearing");

    let snapshot = state
        .stage_display_snapshot("worship-snv")
        .await
        .unwrap()
        .expect("snapshot available");
    assert!(snapshot.presentation_id.is_none());
    assert!(snapshot.current.is_none());
    assert!(snapshot.next.is_none());
}

#[tokio::test]
async fn update_slide_content_updates_repository() {
    let state = AppState::in_memory().await.unwrap();
    let libraries = state.libraries().await.unwrap();
    let presentation = libraries[0].presentations[0].clone();
    let slide = presentation.slides[0].clone();

    let updated = state
        .update_slide_content(
            presentation.id,
            slide.id,
            "Tablet main".to_string(),
            "Tablet translation".to_string(),
            "Tablet stage".to_string(),
            Some("Tablet Group".to_string()),
        )
        .await
        .unwrap();

    assert_eq!(updated.id, slide.id);
    assert_eq!(updated.order, slide.order);
    assert_eq!(updated.content.main.value(), "Tablet main");
    assert_eq!(updated.content.translation.value(), "Tablet translation");
    assert_eq!(updated.content.stage.value(), "Tablet stage");
    assert_eq!(
        updated.content.group.as_ref().map(|group| group.name()),
        Some("Tablet Group")
    );

    let detail = state
        .presentation_detail(presentation.id)
        .await
        .unwrap()
        .expect("presentation detail");
    let stored = detail
        .2
        .slides
        .iter()
        .find(|candidate| candidate.id == slide.id)
        .expect("slide present");

    assert_eq!(stored.content.main.value(), "Tablet main");
    assert_eq!(stored.content.translation.value(), "Tablet translation");
    assert_eq!(stored.content.stage.value(), "Tablet stage");
}

#[tokio::test]
async fn stage_snapshot_defaults_to_first_presentation() {
    let state = AppState::in_memory().await.unwrap();
    state
        .repository()
        .purge_presentation_content()
        .await
        .unwrap();

    let presentation = Presentation::new(
        "Primer",
        vec![Slide::new(
            0,
            SlideContent::new(
                SlideText::new("Prvá veta").unwrap(),
                SlideText::new("First sentence").unwrap(),
                SlideText::new("Stage text").unwrap(),
                None,
            ),
        )
        .with_id(SlideId::new())],
    )
    .unwrap();
    let library = Library::new("Fallback", vec![presentation.clone()])
        .unwrap()
        .with_id(LibraryId::new());
    state.repository().upsert_library(&library).await.unwrap();

    let snapshot = state
        .stage_display_snapshot("worship-snv")
        .await
        .unwrap()
        .expect("snapshot");
    assert_eq!(snapshot.presentation_name.unwrap(), "Primer");
    assert_eq!(snapshot.current.unwrap().main, "Prvá veta");
}

#[tokio::test]
async fn stage_resolution_propagates_effective_group() {
    let state = AppState::in_memory().await.unwrap();
    let libraries = state.libraries().await.unwrap();
    let presentation = libraries[0].presentations[0].clone();
    let first_group = presentation
        .slides
        .first()
        .and_then(|slide| slide.content.group.as_ref())
        .map(|group| group.name().to_string())
        .expect("seed presentation should include group");
    let second_slide = presentation
        .slides
        .get(1)
        .map(|slide| slide.id)
        .expect("seed presentation should include multiple slides");

    let resolved = presentation.resolved_slides();
    assert_eq!(resolved.len(), 2);
    let second_resolved_group = resolved
        .get(1)
        .and_then(|slide| slide.effective_group.as_ref())
        .map(|group| group.name().to_string());
    assert_eq!(second_resolved_group, Some(first_group.clone()));

    let resolution = stage_resolution_from_presentation(
        &presentation,
        Some(libraries[0].name.clone()),
        Some(second_slide),
        None,
    );

    let current_group = resolution
        .current
        .as_ref()
        .and_then(|slide| slide.group.as_ref())
        .cloned();
    assert_eq!(current_group, Some(first_group.clone()));

    let next_group = resolution
        .next
        .as_ref()
        .and_then(|slide| slide.group.as_ref())
        .cloned();
    assert_eq!(next_group, None);
}

#[tokio::test]
async fn timer_commands_emit_live_event() {
    let state = AppState::in_memory().await.unwrap();
    let hub = state.live_hub();
    let mut rx = hub.subscribe();

    let target = Utc::now() + chrono::Duration::minutes(15);
    state
        .execute_timer_command(TimerCommand::SetCountdownTarget { target })
        .await
        .unwrap();
    state
        .execute_timer_command(TimerCommand::StartCountdown)
        .await
        .unwrap();

    let mut seen_running = false;
    for _ in 0..8 {
        match rx.recv().await.unwrap() {
            LiveEvent::Timers { overview }
                if overview.countdown_to_start.state == TimerState::Running =>
            {
                seen_running = true;
                break;
            }
            _ => continue,
        }
    }

    assert!(seen_running, "expected running timers event after command");
}

#[test]
fn countdown_format_switches_below_minute() {
    assert_eq!(super::format_countdown_text(3605), "60:05");
    assert_eq!(super::format_countdown_text(125), "02:05");
    assert_eq!(super::format_countdown_text(59), "59");
    assert_eq!(super::format_countdown_text(0), "0");
    assert_eq!(super::format_countdown_text(-12), "0");
}

#[tokio::test]
async fn tick_timers_emits_live_event() {
    let state = AppState::in_memory().await.unwrap();
    let mut rx = state.live_hub().subscribe();

    state.tick_timers().await.unwrap();

    let mut saw_timers = false;
    for _ in 0..3 {
        if let LiveEvent::Timers { .. } = rx.recv().await.unwrap() {
            saw_timers = true;
            break;
        }
    }

    assert!(saw_timers, "expected timers live event from tick");
}

#[tokio::test]
async fn trigger_bible_passage_publishes_event_and_state() {
    let state = AppState::in_memory().await.unwrap();
    let translation = BibleTranslation::new("test", "Test", "en");
    let reference = BibleReference::new_with_code("John", "JHN", 43, 3, 16, 16).unwrap();
    let passage = BiblePassage::new(
        reference.clone(),
        translation.clone(),
        "For God so loved".to_string(),
    );
    let batch = BibleIngestionBatch::new(translation, vec![passage]).unwrap();
    state
        .repository()
        .replace_bible_translation_passages(&batch)
        .await
        .unwrap();

    let mut rx = state.live_hub().subscribe();
    let broadcast = state
        .trigger_bible_passage("test", &reference)
        .await
        .unwrap();
    assert_eq!(broadcast.passage.reference.book, reference.book);
    assert_eq!(broadcast.passage.reference.chapter, reference.chapter);
    assert_eq!(
        broadcast.passage.reference.verse_start,
        reference.verse_start
    );
    assert_eq!(broadcast.passage.reference.verse_end, reference.verse_end);
    assert!(state.active_bible_broadcast().await.is_some());

    match rx.recv().await.unwrap() {
        LiveEvent::Bible { broadcast: evt } => {
            assert_eq!(evt.passage.translation.code, "test");
        }
        other => panic!("unexpected live event: {other:?}"),
    }

    state.clear_bible_broadcast().await;
    match rx.recv().await.unwrap() {
        LiveEvent::BibleCleared => {}
        other => panic!("expected bible cleared event, got {other:?}"),
    }

    assert!(state.active_bible_broadcast().await.is_none());
}

#[tokio::test]
async fn list_bible_translations_bootstraps_dashboard_once() {
    let state = AppState::in_memory().await.unwrap();
    let reference = BibleReference::new("John", 1, 1, 1).unwrap();
    let translation_one = BibleTranslation::new("en-one", "One", "en");
    let translation_two =
        BibleTranslation::new("sk-two", "Two", "sk").with_show_in_dashboard(false);

    let passage_one = BiblePassage::new(
        reference.clone(),
        translation_one.clone(),
        "Verse one".to_string(),
    );
    let passage_two = BiblePassage::new(
        reference.clone(),
        translation_two.clone(),
        "Verse two".to_string(),
    );

    let batch_one = BibleIngestionBatch::new(translation_one.clone(), vec![passage_one]).unwrap();
    let batch_two = BibleIngestionBatch::new(translation_two.clone(), vec![passage_two]).unwrap();

    state
        .repository()
        .replace_bible_translation_passages(&batch_one)
        .await
        .unwrap();
    state
        .repository()
        .replace_bible_translation_passages(&batch_two)
        .await
        .unwrap();

    let bootstrapped = state.list_bible_translations().await.unwrap();
    assert_eq!(bootstrapped.len(), 2);
    assert!(
        bootstrapped
            .iter()
            .all(|translation| translation.show_in_dashboard),
        "expected initial bootstrap to pin all Bibles"
    );

    // Simulate operator unpinning a translation after bootstrap.
    state
        .repository()
        .update_bible_translation("sk-two", None, None, Some(false))
        .await
        .unwrap();

    let after_unpin = state.list_bible_translations().await.unwrap();
    let sk_two = after_unpin
        .iter()
        .find(|translation| translation.code == "sk-two")
        .expect("sk-two translation present");
    assert!(
        !sk_two.show_in_dashboard,
        "expected subsequent calls to respect operator dashboard choices"
    );
}

#[test]
fn compose_bible_slides_respects_character_limit() {
    let translation = BibleTranslation::new("svk", "Slovak", "sk");
    let passages = vec![
        BiblePassage::new(
            BibleReference::new_with_code("John", "JHN", 43, 3, 1, 1).unwrap(),
            translation.clone(),
            "Alpha".to_string(),
        ),
        BiblePassage::new(
            BibleReference::new_with_code("John", "JHN", 43, 3, 2, 2).unwrap(),
            translation.clone(),
            "Beta".to_string(),
        ),
        BiblePassage::new(
            BibleReference::new_with_code("John", "JHN", 43, 3, 3, 3).unwrap(),
            translation.clone(),
            "Gamma".to_string(),
        ),
    ];
    let lookup: HashMap<u16, BiblePassage> = HashMap::new();

    let slides = compose_bible_slides(&translation, None, &passages, &lookup, 20).unwrap();

    assert_eq!(
        slides.len(),
        2,
        "expected verses to batch without splitting"
    );
    let first = &slides[0];
    let second = &slides[1];

    assert_eq!(first.content.main.value(), "1. Alpha\n2. Beta");
    assert_eq!(second.content.main.value(), "3. Gamma");
    assert_eq!(first.content.stage.value(), first.content.main.value());
    assert_eq!(second.content.stage.value(), second.content.main.value());

    let first_span = first
        .metadata
        .as_ref()
        .and_then(|meta| meta.bible.as_ref())
        .and_then(|bible| bible.verse_span())
        .unwrap();
    assert_eq!(first_span, (1, 2));
    let first_meta = first
        .metadata
        .as_ref()
        .and_then(|meta| meta.bible.as_ref())
        .expect("bible metadata present");
    assert_eq!(first_meta.book_code.as_deref(), Some("JHN"));
    assert_eq!(first_meta.book_number, Some(43));

    let second_span = second
        .metadata
        .as_ref()
        .and_then(|meta| meta.bible.as_ref())
        .and_then(|bible| bible.verse_span())
        .unwrap();
    assert_eq!(second_span, (3, 3));
    let second_meta = second
        .metadata
        .as_ref()
        .and_then(|meta| meta.bible.as_ref())
        .expect("bible metadata present");
    assert_eq!(second_meta.book_code.as_deref(), Some("JHN"));
    assert_eq!(second_meta.book_number, Some(43));
}

#[test]
fn compose_bible_slides_includes_secondary_translation_text() {
    let translation = BibleTranslation::new("svk", "Slovak", "sk");
    let secondary = BibleTranslation::new("eng", "English", "en");
    let main_passages = vec![
        BiblePassage::new(
            BibleReference::new_with_code("John", "JHN", 43, 3, 16, 16).unwrap(),
            translation.clone(),
            "For God so loved".to_string(),
        ),
        BiblePassage::new(
            BibleReference::new_with_code("John", "JHN", 43, 3, 17, 17).unwrap(),
            translation.clone(),
            "For God did not send".to_string(),
        ),
    ];

    let mut secondary_lookup: HashMap<u16, BiblePassage> = HashMap::new();
    secondary_lookup.insert(
        16,
        BiblePassage::new(
            BibleReference::new_with_code("John", "JHN", 43, 3, 16, 16).unwrap(),
            secondary.clone(),
            "Secondary Sixteen".to_string(),
        ),
    );
    secondary_lookup.insert(
        17,
        BiblePassage::new(
            BibleReference::new_with_code("John", "JHN", 43, 3, 17, 17).unwrap(),
            secondary.clone(),
            "Secondary Seventeen".to_string(),
        ),
    );

    let slides = compose_bible_slides(
        &translation,
        Some(&secondary),
        &main_passages,
        &secondary_lookup,
        200,
    )
    .unwrap();

    assert_eq!(slides.len(), 1);
    let slide = &slides[0];
    assert!(slide
        .content
        .translation
        .value()
        .contains("16. Secondary Sixteen"));
    assert!(slide
        .content
        .translation
        .value()
        .contains("17. Secondary Seventeen"));
}

#[test]
fn sanitize_song_names_remove_numeric_prefix() {
    assert_eq!(sanitize_song_title("001 Amazing Grace"), "Amazing Grace");
    assert_eq!(sanitize_song_title("001   Song"), "Song");
    assert_eq!(sanitize_song_title("100"), "100");
    assert_eq!(sanitize_song_title("No Prefix"), "No Prefix");
}
