use super::Repository;
use chrono::{Duration, Utc};
use presenter_core::{
    bible::BibleIngestionBatch, playlist::PlaylistEntryKind, AndroidStageDisplayDraft,
    BiblePassage, BibleReference, BibleTranslation, CountdownTimer, Library, LibraryId,
    OscSettingsDraft, PlaylistEntry, PlaylistEntryId, PreachTimer, Presentation, PresentationId,
    ResolumeHostDraft, SearchResultKind, Slide, SlideContent, SlideGroup, SlideId, SlideText,
    StageState, TimerState, TimersState, VelocityMode, VideoSourceDraft, DEFAULT_ADB_PORT,
    DEFAULT_LAUNCH_PACKAGE,
};

fn sample_library() -> Library {
    let presentation = Presentation::new(
        "Welcome",
        vec![
            Slide::new(
                0,
                SlideContent::new(
                    SlideText::new("Nádej v Pánovi").unwrap(),
                    SlideText::new("Hope in the Lord").unwrap(),
                    SlideText::new("Stage").unwrap(),
                    Some(SlideGroup::new("Intro")),
                ),
            )
            .with_id(SlideId::new()),
            Slide::new(
                1,
                SlideContent::new(
                    SlideText::new("Second").unwrap(),
                    SlideText::new("Druga").unwrap(),
                    SlideText::new("Stage 2").unwrap(),
                    None,
                ),
            )
            .with_id(SlideId::new()),
        ],
    )
    .unwrap()
    .with_id(PresentationId::new());
    Library::new("Default", vec![presentation])
        .unwrap()
        .with_id(LibraryId::new())
}

fn library_named(name: &str, song_names: &[&str]) -> Library {
    let presentations = song_names
        .iter()
        .map(|song| {
            Presentation::new(
                *song,
                vec![Slide::new(
                    0,
                    SlideContent::new(
                        SlideText::new("main").unwrap(),
                        SlideText::new("translation").unwrap(),
                        SlideText::new("stage").unwrap(),
                        None,
                    ),
                )
                .with_id(SlideId::new())],
            )
            .unwrap()
            .with_id(PresentationId::new())
        })
        .collect();
    Library::new(name, presentations)
        .unwrap()
        .with_id(LibraryId::new())
}

#[tokio::test]
async fn upsert_library_reimports_same_name_with_fresh_id() {
    // Regression for #463: the importer assigns a fresh random LibraryId on every
    // run, but the DB enforces UNIQUE(libraries.name). Re-importing a library that
    // already exists under the same NAME (the normal dev/prod case) must REPLACE it,
    // not fail with `UNIQUE constraint failed: libraries.name`.
    let repo = Repository::connect_in_memory().await.unwrap();

    let first = library_named("NEW LEVEL", &["Song A"]);
    repo.upsert_library(&first).await.unwrap();

    // Same name, different (fresh) id, plus a newly added song — exactly what
    // `import_propresenter --keep` does after two new .pro files are dropped in.
    let second = library_named("NEW LEVEL", &["Song A", "Song B (new)"]);
    repo.upsert_library(&second).await.unwrap();

    let libraries = repo.fetch_libraries().await.unwrap();
    let new_level: Vec<_> = libraries
        .iter()
        .filter(|library| library.name == "NEW LEVEL")
        .collect();
    assert_eq!(
        new_level.len(),
        1,
        "exactly one NEW LEVEL library after re-import (no duplicate, no leftover)"
    );
    assert_eq!(
        new_level[0].presentations.len(),
        2,
        "re-import reflects the newly added song"
    );

    // `upsert_library` deletes only the stale LIBRARY row and relies on
    // ON DELETE CASCADE (foreign_keys is on) to remove its presentations +
    // slides. `fetch_libraries` joins by library_id and would NOT surface
    // orphans, so a regression that broke the cascade or skipped the delete
    // could pass the checks above. Assert the RAW row counts to catch that:
    // exactly the new library's 2 presentations and 2 slides must remain — an
    // orphaned set from the old import would make these 3.
    use crate::entities::{presentation as presentation_entity, slide as slide_entity};
    use sea_orm::EntityTrait;
    let db = repo.connection_for_tests();
    let presentation_rows = presentation_entity::Entity::find().all(db).await.unwrap();
    assert_eq!(
        presentation_rows.len(),
        2,
        "no orphaned presentations after re-import (stale rows must be deleted)"
    );
    let slide_rows = slide_entity::Entity::find().all(db).await.unwrap();
    assert_eq!(
        slide_rows.len(),
        2,
        "no orphaned slides after re-import (stale rows must be deleted)"
    );
}

#[tokio::test]
async fn round_trip_library() {
    let repo = Repository::connect_in_memory().await.unwrap();
    let library = sample_library();
    repo.upsert_library(&library).await.unwrap();
    let libraries = repo.fetch_libraries().await.unwrap();
    assert_eq!(libraries.len(), 1);
    assert_eq!(libraries[0].name, "Default");
    assert_eq!(libraries[0].presentations.len(), 1);
    assert_eq!(libraries[0].presentations[0].slides.len(), 2);
}

#[tokio::test]
async fn fetch_first_presentation_detail_returns_sample() {
    let repo = Repository::connect_in_memory().await.unwrap();
    let library = sample_library();
    repo.upsert_library(&library).await.unwrap();

    let detail = repo.fetch_first_presentation_detail().await.unwrap();
    assert!(detail.is_some());
    let (_, name, presentation) = detail.unwrap();
    assert_eq!(name, "Default");
    assert_eq!(presentation.slides.len(), 2);
}

#[tokio::test]
async fn update_slide_content_persists_changes() {
    let repo = Repository::connect_in_memory().await.unwrap();
    let library = sample_library();
    let presentation = library.presentations[0].clone();
    let slide = presentation.slides[0].clone();
    repo.upsert_library(&library).await.unwrap();

    let new_content = SlideContent::new(
        SlideText::new("Updated main").unwrap(),
        SlideText::new("Updated translation").unwrap(),
        SlideText::new("Updated stage").unwrap(),
        Some(SlideGroup::new("Updated Group")),
    );

    repo.update_slide_content(presentation.id, slide.id, &new_content)
        .await
        .unwrap();

    let detail = repo
        .fetch_presentation_detail(presentation.id)
        .await
        .unwrap()
        .expect("presentation detail");
    let updated = detail
        .2
        .slides
        .iter()
        .find(|candidate| candidate.id == slide.id)
        .expect("slide present");

    assert_eq!(updated.content.main.value(), "Updated main");
    assert_eq!(updated.content.translation.value(), "Updated translation");
    assert_eq!(updated.content.stage.value(), "Updated stage");
    assert_eq!(
        updated.content.group.as_ref().map(|group| group.name()),
        Some("Updated Group")
    );
}

#[tokio::test]
async fn purge_presentation_content_clears_tables() {
    let repo = Repository::connect_in_memory().await.unwrap();
    let library = sample_library();
    repo.upsert_library(&library).await.unwrap();

    let presentation = &library.presentations[0];
    let current = presentation.slides[0].id;
    let state = StageState::new(Some(presentation.id), Some(current), None, None);
    repo.upsert_stage_state(&state).await.unwrap();

    repo.purge_presentation_content().await.unwrap();

    let libraries = repo.fetch_libraries().await.unwrap();
    assert!(libraries.is_empty());
    let stage = repo.get_stage_state().await.unwrap();
    assert!(stage.is_none());
}

#[tokio::test]
async fn bible_translation_round_trip() {
    let repo = Repository::connect_in_memory().await.unwrap();
    let translation = BibleTranslation::new("en-kjv", "King James Version", "en")
        .with_source("https://example.org/kjv");
    let reference = BibleReference::new("John", 3, 16, 16).unwrap();
    let passage = BiblePassage::new(
        reference.clone(),
        translation.clone(),
        "For God so loved the world".to_string(),
    );
    let batch = BibleIngestionBatch::new(translation.clone(), vec![passage.clone()]).unwrap();
    repo.replace_bible_translation_passages(&batch)
        .await
        .unwrap();

    let translations = repo.list_bible_translations().await.unwrap();
    assert_eq!(translations.len(), 1);
    assert_eq!(translations[0], translation);

    let fetched = repo
        .find_bible_passage(&translation.code, &reference)
        .await
        .unwrap()
        .expect("passage to exist");
    assert_eq!(fetched.translation, translation);
    // Persistence layer enriches references with canonical book codes
    assert_eq!(fetched.reference.book, reference.book);
    assert_eq!(fetched.reference.chapter, reference.chapter);
    assert_eq!(fetched.reference.verse_start, reference.verse_start);
    assert_eq!(fetched.reference.verse_end, reference.verse_end);
    // book_code and book_number may be enriched from canonical lookup
    assert_eq!(fetched.text, passage.text);
}

#[tokio::test]
async fn bible_translation_large_batch_is_persisted() {
    let repo = Repository::connect_in_memory().await.unwrap();
    let translation = BibleTranslation::new("en-load", "Load Test", "en");
    let mut passages = Vec::new();
    for verse in 1..=1_200u16 {
        let reference = BibleReference::new("Psalm", 119, verse, verse).unwrap();
        let passage = BiblePassage::new(reference, translation.clone(), format!("Verse {verse}"));
        passages.push(passage);
    }

    let batch = BibleIngestionBatch::new(translation.clone(), passages).unwrap();
    repo.replace_bible_translation_passages(&batch)
        .await
        .unwrap();

    let tail_reference = BibleReference::new("Psalm", 119, 1_200, 1_200).unwrap();
    let tail = repo
        .find_bible_passage(&translation.code, &tail_reference)
        .await
        .unwrap();
    assert!(tail.is_some());

    let head_reference = BibleReference::new("Psalm", 119, 1, 1).unwrap();
    let head = repo
        .find_bible_passage(&translation.code, &head_reference)
        .await
        .unwrap();
    assert!(head.is_some());
}

#[tokio::test]
async fn osc_settings_default_is_seeded() {
    let repo = Repository::connect_in_memory().await.unwrap();
    let settings = repo.get_osc_settings().await.unwrap();
    assert!(!settings.enabled);
    assert_eq!(settings.listen_port, 39051);
    assert_eq!(settings.address_pattern, "/note");
    assert_eq!(settings.velocity_mode, VelocityMode::OneBased);
}

#[tokio::test]
async fn osc_settings_upsert_updates_values() {
    let repo = Repository::connect_in_memory().await.unwrap();
    let draft = OscSettingsDraft {
        enabled: true,
        listen_port: 10023,
        address_pattern: "/presenter/trigger".to_string(),
        velocity_mode: VelocityMode::OneBased,
    };
    let updated = repo
        .upsert_osc_settings(
            &draft,
            crate::audit::SettingsAuditSource::HttpSetter,
            "test",
        )
        .await
        .unwrap();
    assert!(updated.enabled);
    assert_eq!(updated.listen_port, 10023);
    assert_eq!(updated.address_pattern, "/presenter/trigger");
    assert_eq!(updated.velocity_mode, VelocityMode::OneBased);

    // upsert idempotency
    let second = repo.get_osc_settings().await.unwrap();
    assert_eq!(second.address_pattern, updated.address_pattern);
    assert_eq!(second.listen_port, updated.listen_port);
}

#[tokio::test]
async fn resolume_host_crud_round_trip() {
    let repo = Repository::connect_in_memory().await.unwrap();
    let draft = ResolumeHostDraft::new("Arena", "resolume.lan", 8090);
    let created = repo
        .create_resolume_host(
            &draft,
            crate::audit::SettingsAuditSource::HttpSetter,
            "test",
        )
        .await
        .unwrap();
    assert_eq!(created.label, "Arena");
    assert_eq!(created.host, "resolume.lan");
    assert_eq!(created.port, 8090);
    assert!(created.is_enabled);

    let hosts = repo.list_resolume_hosts().await.unwrap();
    assert_eq!(hosts.len(), 1);

    let updated_draft =
        ResolumeHostDraft::new("Arena North", "resolume.lan", 8090).with_enabled(false);
    let updated = repo
        .update_resolume_host(
            created.id,
            &updated_draft,
            crate::audit::SettingsAuditSource::HttpSetter,
            "test",
        )
        .await
        .unwrap();
    assert_eq!(updated.label, "Arena North");
    assert!(!updated.is_enabled);

    repo.delete_resolume_host(
        created.id,
        crate::audit::SettingsAuditSource::HttpSetter,
        "test",
    )
    .await
    .unwrap();
    let after_delete = repo.list_resolume_hosts().await.unwrap();
    assert!(after_delete.is_empty());
}

#[tokio::test]
async fn android_stage_display_crud_round_trip() {
    let repo = Repository::connect_in_memory().await.unwrap();

    let initial = repo.list_android_stage_displays().await.unwrap();
    let initial_count = initial.len();

    let draft = AndroidStageDisplayDraft::new("Stage Left", "test-stage.invalid");
    let created = repo
        .create_android_stage_display(
            &draft,
            crate::audit::SettingsAuditSource::HttpSetter,
            "test",
        )
        .await
        .unwrap();
    assert_eq!(created.label, "Stage Left");
    assert_eq!(created.host, "test-stage.invalid");
    assert_eq!(created.port, DEFAULT_ADB_PORT);
    assert_eq!(created.launch_component, DEFAULT_LAUNCH_PACKAGE.to_string());
    assert!(created.is_enabled);

    let displays = repo.list_android_stage_displays().await.unwrap();
    assert_eq!(displays.len(), initial_count + 1);
    assert!(displays.iter().any(|d| d.id == created.id));

    let updated_draft = AndroidStageDisplayDraft::new("Stage Right", "test-stage2.invalid")
        .with_port(5566)
        .with_launch_component("com.example/.Main")
        .with_enabled(false);
    let updated = repo
        .update_android_stage_display(
            created.id,
            &updated_draft,
            crate::audit::SettingsAuditSource::HttpSetter,
            "test",
        )
        .await
        .unwrap();
    assert_eq!(updated.label, "Stage Right");
    assert_eq!(updated.host, "test-stage2.invalid");
    assert_eq!(updated.port, 5566);
    assert_eq!(updated.launch_component, "com.example/.Main");
    assert!(!updated.is_enabled);

    repo.delete_android_stage_display(
        created.id,
        crate::audit::SettingsAuditSource::HttpSetter,
        "test",
    )
    .await
    .unwrap();
    let after_delete = repo.list_android_stage_displays().await.unwrap();
    assert_eq!(after_delete.len(), initial_count);
    assert!(after_delete.iter().all(|d| d.id != created.id));
}

#[tokio::test]
async fn timers_state_round_trip() {
    let repo = Repository::connect_in_memory().await.unwrap();
    let now = Utc::now();
    let target = now + Duration::minutes(10);
    let countdown = CountdownTimer {
        target,
        state: TimerState::Running,
    };
    let mut preach = PreachTimer::new();
    preach.start(now);
    let pause_instant = now + Duration::minutes(1);
    preach.pause(pause_instant);
    let expected_preach = preach.clone();
    let state = TimersState::new(countdown, preach);

    repo.upsert_timers_state(&state).await.unwrap();

    let stored = repo.get_timers_state().await.unwrap().unwrap();
    assert_eq!(stored.countdown.state, TimerState::Running);
    assert_eq!(stored.countdown.target, target);
    assert_eq!(stored.preach.state, expected_preach.state);
    assert!(stored.preach.elapsed(pause_instant).num_seconds() >= 60);
}

#[tokio::test]
async fn playlist_round_trip_with_separator() {
    let repo = Repository::connect_in_memory().await.unwrap();
    let library = sample_library();
    let presentation = library.presentations[0].clone();
    repo.upsert_library(&library).await.unwrap();

    let playlist = repo
        .create_playlist("Good Fest Friday", true)
        .await
        .unwrap();

    let entries = vec![
        PlaylistEntry {
            id: PlaylistEntryId::new(),
            kind: PlaylistEntryKind::Separator {
                name: "Doors Open".to_string(),
            },
        },
        PlaylistEntry {
            id: PlaylistEntryId::new(),
            kind: PlaylistEntryKind::Presentation {
                presentation_id: presentation.id,
                midi_binding: None,
                presentation_name: None,
            },
        },
    ];

    repo.replace_playlist_entries(playlist.id, &entries)
        .await
        .unwrap();

    let playlists = repo.list_playlists().await.unwrap();
    assert_eq!(playlists.len(), 1);

    let fetched = &playlists[0];
    assert_eq!(fetched.name, "Good Fest Friday");
    assert!(fetched.show_in_dashboard);
    assert_eq!(fetched.entries.len(), 2);
    assert!(matches!(
        fetched.entries[0].kind,
        PlaylistEntryKind::Separator { .. }
    ));
    assert!(matches!(
        fetched.entries[1].kind,
        PlaylistEntryKind::Presentation {
            presentation_id,
            ..
        } if presentation_id == presentation.id
    ));
}

#[tokio::test]
async fn create_library_persists_new_row() {
    let repo = Repository::connect_in_memory().await.unwrap();
    let created = repo.create_library("Autotest Library").await.unwrap();
    assert_eq!(created.name, "Autotest Library");
    assert!(created.presentations.is_empty());

    let libraries = repo.fetch_libraries().await.unwrap();
    assert_eq!(libraries.len(), 1);
    assert_eq!(libraries[0].id, created.id);
    assert_eq!(libraries[0].name, "Autotest Library");
}

#[tokio::test]
async fn rename_library_updates_name() {
    let repo = Repository::connect_in_memory().await.unwrap();
    let created = repo.create_library("Original").await.unwrap();

    repo.rename_library(created.id, "Renamed").await.unwrap();

    let libraries = repo.fetch_libraries().await.unwrap();
    assert_eq!(libraries.len(), 1);
    assert_eq!(libraries[0].name, "Renamed");
}

#[tokio::test]
async fn delete_library_removes_presentations_and_slides() {
    let repo = Repository::connect_in_memory().await.unwrap();
    let library = sample_library();
    let presentation = library.presentations[0].clone();
    repo.upsert_library(&library).await.unwrap();

    repo.delete_library(library.id).await.unwrap();

    let libraries = repo.fetch_libraries().await.unwrap();
    assert!(libraries.is_empty());

    let detail = repo
        .fetch_presentation_detail(presentation.id)
        .await
        .unwrap();
    assert!(detail.is_none());
}

#[tokio::test]
async fn replace_presentation_slides_overwrites_rows() {
    let repo = Repository::connect_in_memory().await.unwrap();
    let library = sample_library();
    let presentation = library.presentations[0].clone();
    repo.upsert_library(&library).await.unwrap();

    let mut slides = presentation.slides.clone();
    let new_slide = Slide::new(
        slides.len() as u32,
        SlideContent::new(
            SlideText::new("New main").unwrap(),
            SlideText::new("New translation").unwrap(),
            SlideText::new("New stage").unwrap(),
            None,
        ),
    );
    slides.push(new_slide);
    for (index, slide) in slides.iter_mut().enumerate() {
        slide.order = index as u32;
    }

    repo.replace_presentation_slides(presentation.id, &slides)
        .await
        .unwrap();

    let detail = repo
        .fetch_presentation_detail(presentation.id)
        .await
        .unwrap()
        .expect("detail");
    assert_eq!(detail.2.slides.len(), slides.len());
    assert_eq!(
        detail.2.slides.last().unwrap().content.main.value(),
        "New main",
    );
}

#[tokio::test]
async fn search_presenter_returns_hits() {
    let repo = Repository::connect_in_memory().await.unwrap();
    let mut library = sample_library();
    library.name = "Chvály Nedeľa".to_string();
    repo.upsert_library(&library).await.unwrap();

    let mut other = sample_library();
    other.name = "Youth Set".to_string();
    repo.upsert_library(&other).await.unwrap();

    let accent_slide = Slide::new(
        0,
        SlideContent::new(
            SlideText::new("Ježiš, ja Ťa chválim").unwrap(),
            SlideText::new("Jesus I praise you").unwrap(),
            SlideText::new("Stage cue").unwrap(),
            None,
        ),
    );
    let accent_presentation = Presentation::new("Ježiš, ja", vec![accent_slide]).unwrap();
    let accent_library = Library::new("TYMY Worship", vec![accent_presentation]).unwrap();
    repo.upsert_library(&accent_library).await.unwrap();

    let results = repo.search_presenter("Chvaly", 10).await.unwrap();
    assert!(!results.is_empty());
    assert!(results
        .iter()
        .any(|result| matches!(result.kind, SearchResultKind::Library)
            && result.library_name == "Chvály Nedeľa"));

    let presentation_results = repo.search_presenter("Welcome", 10).await.unwrap();
    assert!(presentation_results
        .iter()
        .any(|result| matches!(result.kind, SearchResultKind::Presentation)));

    // Slide-text matches now surface the parent Presentation, not a Slide result.
    let slide_results = repo.search_presenter("Nadej", 10).await.unwrap();
    assert!(slide_results
        .iter()
        .any(|result| matches!(result.kind, SearchResultKind::Presentation)));

    let compound_results = repo.search_presenter("jezis, ja tymy", 10).await.unwrap();
    assert!(compound_results.iter().any(|result| {
        matches!(result.kind, SearchResultKind::Presentation)
            && result.library_name == "TYMY Worship"
    }));
}

#[tokio::test]
async fn stage_state_round_trip() {
    let repo = Repository::connect_in_memory().await.unwrap();
    let library = sample_library();
    let presentation = library.presentations[0].clone();
    repo.upsert_library(&library).await.unwrap();

    let current = presentation.slides[0].id;
    let next = presentation.slides.get(1).map(|slide| slide.id);
    let state = StageState::new(Some(presentation.id), Some(current), next, None);

    repo.upsert_stage_state(&state).await.unwrap();

    let stored = repo.get_stage_state().await.unwrap().unwrap();
    assert_eq!(stored.presentation_id, Some(presentation.id));
    assert_eq!(stored.current_slide_id, Some(current));
    assert_eq!(stored.next_slide_id, next);
}

// ── create_presentation tests ───────────────────────────────────────

#[tokio::test]
async fn create_presentation_persists_with_correct_fields() {
    let repo = Repository::connect_in_memory().await.unwrap();
    let library = sample_library();
    repo.upsert_library(&library).await.unwrap();

    let (lib_id, lib_name, presentation) = repo
        .create_presentation(library.id, "New Song", None)
        .await
        .unwrap();

    assert_eq!(lib_id, library.id);
    assert_eq!(lib_name, "Default");
    assert_eq!(presentation.name, "New Song");
    // Default slide should be created when no slides provided
    assert_eq!(presentation.slides.len(), 1);
    assert_eq!(presentation.slides[0].content.main.value(), "");
}

#[tokio::test]
async fn create_presentation_with_slides() {
    let repo = Repository::connect_in_memory().await.unwrap();
    let library = sample_library();
    repo.upsert_library(&library).await.unwrap();

    let slides = vec![
        Slide::new(
            0,
            SlideContent::new(
                SlideText::new("Verse 1").unwrap(),
                SlideText::new("Trans 1").unwrap(),
                SlideText::new("Stage 1").unwrap(),
                Some(SlideGroup::new("Verse")),
            ),
        ),
        Slide::new(
            1,
            SlideContent::new(
                SlideText::new("Chorus").unwrap(),
                SlideText::new("Refrén").unwrap(),
                SlideText::new("Stage C").unwrap(),
                Some(SlideGroup::new("Chorus")),
            ),
        ),
    ];

    let (_, _, presentation) = repo
        .create_presentation(library.id, "Multi Slide", Some(&slides))
        .await
        .unwrap();

    assert_eq!(presentation.name, "Multi Slide");
    assert_eq!(presentation.slides.len(), 2);
    assert_eq!(presentation.slides[0].content.main.value(), "Verse 1");
    assert_eq!(presentation.slides[1].content.main.value(), "Chorus");
}

// ── rename_presentation tests ───────────────────────────────────────

#[tokio::test]
async fn rename_presentation_updates_name() {
    let repo = Repository::connect_in_memory().await.unwrap();
    let library = sample_library();
    repo.upsert_library(&library).await.unwrap();

    let (_, _, created) = repo
        .create_presentation(library.id, "Original Name", None)
        .await
        .unwrap();

    repo.rename_presentation(created.id, "Updated Name")
        .await
        .unwrap();

    let detail = repo
        .fetch_presentation_detail(created.id)
        .await
        .unwrap()
        .expect("detail");
    assert_eq!(detail.2.name, "Updated Name");
}

#[tokio::test]
async fn rename_presentation_rejects_empty_name() {
    let repo = Repository::connect_in_memory().await.unwrap();
    let library = sample_library();
    repo.upsert_library(&library).await.unwrap();

    let (_, _, created) = repo
        .create_presentation(library.id, "Song", None)
        .await
        .unwrap();

    let result = repo.rename_presentation(created.id, "   ").await;
    assert!(result.is_err());
}

// ── delete_presentation tests ───────────────────────────────────────

#[tokio::test]
async fn delete_presentation_cascades_to_slides() {
    let repo = Repository::connect_in_memory().await.unwrap();
    let library = sample_library();
    let presentation_id = library.presentations[0].id;
    repo.upsert_library(&library).await.unwrap();

    // Verify it exists first
    let before = repo
        .fetch_presentation_detail(presentation_id)
        .await
        .unwrap();
    assert!(before.is_some());

    repo.delete_presentation(presentation_id).await.unwrap();

    let after = repo
        .fetch_presentation_detail(presentation_id)
        .await
        .unwrap();
    assert!(after.is_none());
}

#[tokio::test]
async fn delete_presentation_not_found_returns_error() {
    let repo = Repository::connect_in_memory().await.unwrap();
    let result = repo.delete_presentation(PresentationId::new()).await;
    assert!(result.is_err());
}

// ── app_setting tests ───────────────────────────────────────────────

#[tokio::test]
async fn app_setting_round_trip_set_get_delete() {
    let repo = Repository::connect_in_memory().await.unwrap();

    // Initially absent
    let val = repo.get_app_setting("theme").await.unwrap();
    assert!(val.is_none());

    // Set
    repo.set_app_setting("theme", "dark").await.unwrap();
    let val = repo.get_app_setting("theme").await.unwrap();
    assert_eq!(val.as_deref(), Some("dark"));

    // Overwrite
    repo.set_app_setting("theme", "light").await.unwrap();
    let val = repo.get_app_setting("theme").await.unwrap();
    assert_eq!(val.as_deref(), Some("light"));

    // Delete
    repo.delete_app_setting("theme").await.unwrap();
    let val = repo.get_app_setting("theme").await.unwrap();
    assert!(val.is_none());
}

// ── update_slide_content_with_metadata tests ────────────────────────

#[tokio::test]
async fn update_slide_content_with_metadata_persists_worship_fields() {
    let repo = Repository::connect_in_memory().await.unwrap();
    let library = sample_library();
    let presentation = library.presentations[0].clone();
    let slide = presentation.slides[0].clone();
    repo.upsert_library(&library).await.unwrap();

    let new_content = SlideContent::new(
        SlideText::new("Updated main").unwrap(),
        SlideText::new("Updated translation").unwrap(),
        SlideText::new("Updated stage").unwrap(),
        Some(SlideGroup::new("Verse 1")),
    );

    repo.update_slide_content_with_metadata(presentation.id, slide.id, &new_content, None)
        .await
        .unwrap();

    let detail = repo
        .fetch_presentation_detail(presentation.id)
        .await
        .unwrap()
        .expect("detail");
    let updated = detail
        .2
        .slides
        .iter()
        .find(|s| s.id == slide.id)
        .expect("slide");

    assert_eq!(updated.content.main.value(), "Updated main");
    assert_eq!(updated.content.translation.value(), "Updated translation");
    assert_eq!(updated.content.stage.value(), "Updated stage");
    assert_eq!(
        updated.content.group.as_ref().map(|g| g.name()),
        Some("Verse 1")
    );
}

#[tokio::test]
async fn seed_migration_populates_four_android_stage_displays_on_empty_table() {
    let repo = Repository::connect_in_memory().await.unwrap();
    let displays = repo.list_android_stage_displays().await.unwrap();
    assert_eq!(displays.len(), 4, "seed should have inserted 4 displays");
    let mut hosts: Vec<_> = displays.iter().map(|d| d.host.clone()).collect();
    hosts.sort();
    assert_eq!(
        hosts,
        vec![
            "sd1l.lan".to_string(),
            "sd2l.lan".to_string(),
            "sd3l.lan".to_string(),
            "sd4l.lan".to_string(),
        ],
        "seed hosts should be sd1l..sd4l",
    );
    for d in &displays {
        assert_eq!(d.port, 5555);
        // The seed (m20260414_000002) inserts the dead Fully Kiosk component;
        // the full migration chain run by `connect_in_memory` then applies
        // m20260616_000001 (→ com.tcl.browser, #404) and finally m20260624_000001
        // (→ our own Presenter Stage app, #472), which is the universal launcher
        // target. So after migrations the launcher target is our app package.
        assert_eq!(d.launch_component, "sk.newlevel.presenterstage");
        assert!(d.is_enabled, "seeded displays should be enabled");
    }
}

#[tokio::test]
async fn seed_migration_is_idempotent_when_rerun() {
    use presenter_migration::{MigrationTrait, MigratorTrait};
    use sea_orm_migration::SchemaManager;

    let repo = Repository::connect_in_memory().await.unwrap();
    // Initial connect ran migrations → 4 seed rows present.
    assert_eq!(repo.list_android_stage_displays().await.unwrap().len(), 4);

    // Operator adds a fifth display manually.
    let draft = AndroidStageDisplayDraft::new("Operator Custom", "custom.invalid");
    repo.create_android_stage_display(
        &draft,
        crate::audit::SettingsAuditSource::HttpSetter,
        "test",
    )
    .await
    .unwrap();
    assert_eq!(repo.list_android_stage_displays().await.unwrap().len(), 5);

    // Manually invoke the seed migration's `up()` a second time.
    let connection = repo.connection_for_tests();
    let schema = SchemaManager::new(connection);
    let migration: Box<dyn MigrationTrait> = presenter_migration::Migrator::migrations()
        .into_iter()
        .find(|m| m.name() == "m20260414_000002_seed_android_stage_displays")
        .expect("seed migration present in registry");
    migration.up(&schema).await.expect("rerun seed");

    // Row count stays at 5 — guard prevented re-insertion.
    let displays = repo.list_android_stage_displays().await.unwrap();
    assert_eq!(
        displays.len(),
        5,
        "seed rerun must not insert duplicates once any row exists",
    );
    assert!(
        displays.iter().any(|d| d.host == "custom.invalid"),
        "operator's custom display must survive the rerun",
    );
}

#[tokio::test]
async fn get_ableset_settings_does_not_rewrite_legacy_values() {
    use crate::audit::SettingsAuditSource;
    use presenter_core::AbleSetSettingsDraft;
    let repo = Repository::connect_in_memory().await.unwrap();

    // Seed a row with legacy values.
    repo.upsert_ableset_settings(
        &AbleSetSettingsDraft {
            enabled: true,
            host: "fohabl.lan".into(),
            osc_port: 5950,
            http_port: 5950,
            library_name: "NEWLEVEL".into(),
            song_prefix_length: 3,
        },
        SettingsAuditSource::HttpSetter,
        "test",
    )
    .await
    .unwrap();

    // Capture audit count.
    let audit_before = repo
        .list_settings_audit(Some("ableset_settings"), None, None, 100)
        .await
        .unwrap()
        .len();

    // Read should NOT mutate.
    let settings = repo.get_ableset_settings().await.unwrap();
    assert_eq!(settings.http_port, 5950);
    assert_eq!(settings.osc_port, 5950);
    assert_eq!(settings.library_name, "NEWLEVEL");

    let audit_after = repo
        .list_settings_audit(Some("ableset_settings"), None, None, 100)
        .await
        .unwrap()
        .len();
    assert_eq!(
        audit_before, audit_after,
        "get_ableset_settings must not write"
    );
}

#[tokio::test]
async fn second_startup_writes_no_audit_rows() {
    use crate::audit::SettingsAuditSource;

    // Connect, trigger settings reads (force singleton creation).
    let repo = Repository::connect_in_memory().await.unwrap();
    let _ = repo.get_osc_settings().await.unwrap();
    let _ = repo.get_ableset_settings().await.unwrap();

    let first_count = repo
        .list_settings_audit(None, None, None, 10_000)
        .await
        .unwrap()
        .len();
    assert!(
        first_count >= 2,
        "expected at least 2 startup default rows, got {first_count}"
    );

    // Second "startup" — same DB, same reads.
    let _ = repo.get_osc_settings().await.unwrap();
    let _ = repo.get_ableset_settings().await.unwrap();

    let second_count = repo
        .list_settings_audit(None, None, None, 10_000)
        .await
        .unwrap()
        .len();
    assert_eq!(
        first_count, second_count,
        "second startup must not write any audit rows"
    );

    // Sanity: every existing row's source is StartupDefault.
    for row in repo
        .list_settings_audit(None, None, None, 10_000)
        .await
        .unwrap()
    {
        assert_eq!(row.source, SettingsAuditSource::StartupDefault);
    }
}

#[tokio::test]
async fn record_and_list_settings_audit_roundtrip() {
    let repo = Repository::connect_in_memory().await.unwrap();
    repo.record_settings_audit(
        "ableset_settings",
        "singleton",
        crate::audit::SettingsAuditSource::HttpSetter,
        "10.0.0.5",
        Some(serde_json::json!({"enabled": false})),
        serde_json::json!({"enabled": true}),
    )
    .await
    .unwrap();

    let rows = repo
        .list_settings_audit(Some("ableset_settings"), None, None, 10)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].actor, "10.0.0.5");
    assert_eq!(
        rows[0].source,
        crate::audit::SettingsAuditSource::HttpSetter
    );
    assert_eq!(rows[0].after_json["enabled"], true);
}

/// Look up the legacy-ableset-defaults migration from the registry.
fn legacy_ableset_defaults_migration() -> Box<dyn presenter_migration::MigrationTrait> {
    use presenter_migration::MigratorTrait;
    presenter_migration::Migrator::migrations()
        .into_iter()
        .find(|m| m.name() == "m20260517_000002_fix_legacy_ableset_defaults")
        .expect("legacy-defaults migration present in registry")
}

/// A fresh DB whose ableset row is ALREADY at the new defaults must get zero
/// audit rows from the legacy-defaults migration.
async fn assert_legacy_migration_skips_already_current_row(
    migration: &dyn presenter_migration::MigrationTrait,
) {
    use sea_orm::{ConnectionTrait, Statement};
    use sea_orm_migration::SchemaManager;

    let repo2 = Repository::connect_in_memory().await.unwrap();
    // Force table creation.
    let _ = repo2.get_ableset_settings().await.unwrap();
    // Wipe the audit log so we can isolate the migration's behaviour on a row
    // that is ALREADY at new defaults.
    let conn = repo2.connection_for_tests();
    conn.execute(Statement::from_string(
        conn.get_database_backend(),
        "DELETE FROM settings_audit".to_string(),
    ))
    .await
    .unwrap();
    // The default row already has http_port=80, osc_port=39051, library="NEW LEVEL".
    let schema2 = SchemaManager::new(conn);
    migration
        .up(&schema2)
        .await
        .expect("migration on already-current row");
    let audit_clean = repo2
        .list_settings_audit(None, None, None, 100)
        .await
        .unwrap();
    assert!(
        audit_clean.is_empty(),
        "migration must write zero audit rows when no legacy values are present, got {audit_clean:?}"
    );
}

#[tokio::test]
async fn legacy_ableset_defaults_migration_rewrites_and_audits() {
    use crate::audit::SettingsAuditSource;
    use presenter_core::AbleSetSettingsDraft;
    use presenter_migration::MigrationTrait;
    use sea_orm_migration::SchemaManager;

    // Fresh DB with all migrations applied — `settings_audit` is empty,
    // `ableset_settings` does not yet exist (created lazily).
    let repo = Repository::connect_in_memory().await.unwrap();

    // Force lazy creation of `ableset_settings` by writing a seed row via
    // the audited setter. That generates exactly one StartupDefault-style
    // audit row (HttpSetter here, since we provide a source explicitly).
    repo.upsert_ableset_settings(
        &AbleSetSettingsDraft {
            enabled: true,
            host: "fohabl.lan".into(),
            osc_port: 5950,
            http_port: 5950,
            library_name: "NEWLEVEL".into(),
            song_prefix_length: 3,
        },
        SettingsAuditSource::HttpSetter,
        "test",
    )
    .await
    .unwrap();

    // Sanity: the seed row really has legacy values, and there is exactly
    // one audit row for it (from the upsert above).
    let before = repo.get_ableset_settings().await.unwrap();
    assert_eq!(before.http_port, 5950);
    assert_eq!(before.osc_port, 5950);
    assert_eq!(before.library_name, "NEWLEVEL");
    let audit_pre = repo
        .list_settings_audit(Some("ableset_settings"), None, None, 100)
        .await
        .unwrap();
    assert_eq!(
        audit_pre.len(),
        1,
        "seed upsert should produce exactly one audit row"
    );

    // Run the legacy-defaults migration directly against the live connection.
    let connection = repo.connection_for_tests();
    let schema = SchemaManager::new(connection);
    let migration: Box<dyn MigrationTrait> = legacy_ableset_defaults_migration();
    migration
        .up(&schema)
        .await
        .expect("rerun legacy-defaults migration");

    // Row was rewritten to new defaults.
    let after = repo.get_ableset_settings().await.unwrap();
    assert_eq!(after.http_port, 80);
    assert_eq!(after.osc_port, 39051);
    assert_eq!(after.library_name, "NEW LEVEL");

    // One additional audit row exists, source = schema_migration, with
    // before/after JSON reflecting the rewrite.
    let audit_post = repo
        .list_settings_audit(Some("ableset_settings"), None, None, 100)
        .await
        .unwrap();
    assert_eq!(
        audit_post.len(),
        audit_pre.len() + 1,
        "migration must add exactly one audit row"
    );
    let migration_rows: Vec<_> = audit_post
        .iter()
        .filter(|r| r.source == SettingsAuditSource::SchemaMigration)
        .collect();
    assert_eq!(
        migration_rows.len(),
        1,
        "exactly one audit row must have source=schema_migration"
    );
    let row = migration_rows[0];
    assert_eq!(row.setting_table, "ableset_settings");
    assert_eq!(row.actor, "migration");
    let before_json = row
        .before_json
        .as_ref()
        .expect("schema_migration audit row must carry before_json");
    assert_eq!(before_json["httpPort"], 5950);
    assert_eq!(before_json["oscPort"], 5950);
    assert_eq!(before_json["libraryName"], "NEWLEVEL");
    assert_eq!(row.after_json["httpPort"], 80);
    assert_eq!(row.after_json["oscPort"], 39051);
    assert_eq!(row.after_json["libraryName"], "NEW LEVEL");

    // Idempotency: rerunning the migration writes no further audit rows.
    migration
        .up(&schema)
        .await
        .expect("rerun legacy-defaults migration second time");
    let audit_final = repo
        .list_settings_audit(Some("ableset_settings"), None, None, 100)
        .await
        .unwrap();
    assert_eq!(
        audit_final.len(),
        audit_post.len(),
        "migration must be idempotent — no new audit rows on rerun"
    );

    // Also sanity-check: a fresh DB with NO legacy rows produces no audit
    // rows from this migration.
    assert_legacy_migration_skips_already_current_row(migration.as_ref()).await;
}

#[tokio::test]
async fn activate_video_source_audits_each_deactivated_sibling() {
    use crate::audit::SettingsAuditSource;
    let repo = Repository::connect_in_memory().await.unwrap();

    // Create three video sources.
    let a = repo
        .create_video_source(
            &VideoSourceDraft::new("Cam A", "CAM_A"),
            SettingsAuditSource::HttpSetter,
            "test",
        )
        .await
        .unwrap();
    let b = repo
        .create_video_source(
            &VideoSourceDraft::new("Cam B", "CAM_B"),
            SettingsAuditSource::HttpSetter,
            "test",
        )
        .await
        .unwrap();
    let c = repo
        .create_video_source(
            &VideoSourceDraft::new("Cam C", "CAM_C"),
            SettingsAuditSource::HttpSetter,
            "test",
        )
        .await
        .unwrap();

    // Activate A and B directly (no siblings active yet for A, then B
    // deactivates A as its sibling — already covered in deeper testing
    // below).
    repo.activate_video_source(a.id, SettingsAuditSource::HttpSetter, "test")
        .await
        .unwrap();
    repo.activate_video_source(b.id, SettingsAuditSource::HttpSetter, "test")
        .await
        .unwrap();
    // After the two calls above, only B is active and A is deactivated.
    // The activation of B should have produced TWO audit rows: one for A's
    // deactivation and one for B's activation.

    // Reset the audit log to isolate the next call.
    let conn = repo.connection_for_tests();
    sea_orm::ConnectionTrait::execute(
        conn,
        sea_orm::Statement::from_string(
            sea_orm::ConnectionTrait::get_database_backend(conn),
            "DELETE FROM settings_audit".to_string(),
        ),
    )
    .await
    .unwrap();
    // Manually activate A too via raw SQL so we have TWO siblings active
    // when we activate C — exercising the per-row audit path against more
    // than one sibling.
    sea_orm::ConnectionTrait::execute(
        conn,
        sea_orm::Statement::from_string(
            sea_orm::ConnectionTrait::get_database_backend(conn),
            format!(
                "UPDATE video_sources SET is_active = 1 WHERE id IN ('{}', '{}')",
                a.id, b.id
            ),
        ),
    )
    .await
    .unwrap();

    // Activate C — this must deactivate both A and B and emit one audit
    // row per deactivation PLUS one for the C activation = 3 audit rows.
    let activated = repo
        .activate_video_source(c.id, SettingsAuditSource::HttpSetter, "ip-1.2.3.4")
        .await
        .unwrap();
    assert!(activated.is_active);
    assert_eq!(activated.id, c.id);

    let rows = repo
        .list_settings_audit(Some("video_source"), None, None, 100)
        .await
        .unwrap();
    assert_eq!(
        rows.len(),
        3,
        "expected one audit row per deactivated sibling + one for the activation, got {rows:?}"
    );

    // Each audit row carries the per-row setting_id (not "deactivate_all"
    // and not a single combined row).
    let setting_ids: std::collections::HashSet<&str> =
        rows.iter().map(|r| r.setting_id.as_str()).collect();
    assert!(setting_ids.contains(a.id.to_string().as_str()));
    assert!(setting_ids.contains(b.id.to_string().as_str()));
    assert!(setting_ids.contains(c.id.to_string().as_str()));

    // Sibling rows show the is_active=true → is_active=false transition.
    for sibling_id in [a.id.to_string(), b.id.to_string()] {
        assert_sibling_deactivation_audit_row(&rows, &sibling_id);
    }

    // The target row's audit shows the activation.
    let c_row = rows
        .iter()
        .find(|r| r.setting_id == c.id.to_string())
        .expect("audit row for activated target");
    assert_eq!(c_row.after_json["isActive"], true);
}

/// #375: re-activating the SAME source that is already active (and the only
/// active row) must be a no-op — it must NOT write a settings_audit row, and a
/// genuine switch must still write its audit rows. The 30s NDI auto-reconnect
/// loop re-calls `activate_video_source` every tick while an active source is
/// unreachable (pipeline start fails but the DB row stays `is_active=true`);
/// before this fix each tick wrote a fresh audit row, spamming the forensic log
/// with one no-op row every ~30s forever.
#[tokio::test]
async fn reactivating_already_active_source_writes_no_audit_row() {
    use crate::audit::SettingsAuditSource;
    let repo = Repository::connect_in_memory().await.unwrap();

    let a = repo
        .create_video_source(
            &VideoSourceDraft::new("Cam A", "CAM_A"),
            SettingsAuditSource::HttpSetter,
            "test",
        )
        .await
        .unwrap();
    let b = repo
        .create_video_source(
            &VideoSourceDraft::new("Cam B", "CAM_B"),
            SettingsAuditSource::HttpSetter,
            "test",
        )
        .await
        .unwrap();

    // Baseline: creating A+B already wrote one audit row each. Measure all
    // subsequent assertions as deltas against this baseline.
    let baseline = repo
        .list_settings_audit(Some("video_source"), None, None, 1000)
        .await
        .unwrap()
        .len();

    // Genuine activation of A: writes exactly one audit row (A activated, no
    // siblings active yet).
    repo.activate_video_source(a.id, SettingsAuditSource::StartupDefault, "system")
        .await
        .unwrap();
    let after_first = repo
        .list_settings_audit(Some("video_source"), None, None, 1000)
        .await
        .unwrap();
    assert_eq!(
        after_first.len(),
        baseline + 1,
        "first (genuine) activation must write exactly one audit row, got {after_first:?}"
    );

    // Capture the activated row's updated_at so we can assert it is NOT bumped
    // by the no-op re-activation.
    let updated_at_before = repo
        .get_active_video_source()
        .await
        .unwrap()
        .expect("A is active")
        .updated_at;

    // Re-activate A three times (simulating three 30s auto-reconnect ticks
    // against an unreachable-but-active source). Each is a no-op: A is already
    // active and is the only active row, so nothing changes.
    for _ in 0..3 {
        let returned = repo
            .activate_video_source(a.id, SettingsAuditSource::StartupDefault, "system")
            .await
            .unwrap();
        assert!(
            returned.is_active,
            "re-activation still returns the active row"
        );
        assert_eq!(returned.id, a.id);
    }

    let after_noops = repo
        .list_settings_audit(Some("video_source"), None, None, 1000)
        .await
        .unwrap();
    assert_eq!(
        after_noops.len(),
        baseline + 1,
        "re-activating an already-active source must add NO audit rows, got {after_noops:?}"
    );

    let updated_at_after = repo
        .get_active_video_source()
        .await
        .unwrap()
        .expect("A still active")
        .updated_at;
    assert_eq!(
        updated_at_before, updated_at_after,
        "no-op re-activation must NOT bump updated_at"
    );

    // A genuine switch A -> B must STILL write audit rows (B activated + A
    // deactivated): the idempotency guard must not break the real audit trail.
    repo.activate_video_source(b.id, SettingsAuditSource::HttpSetter, "ip-9.9.9.9")
        .await
        .unwrap();
    let switch_rows = repo
        .list_settings_audit(Some("video_source"), None, None, 1000)
        .await
        .unwrap();
    // Delta = +2 over (baseline + 1): B activation + A deactivation.
    assert_eq!(
        switch_rows.len(),
        baseline + 3,
        "a genuine A->B switch must still write its audit rows, got {switch_rows:?}"
    );
    let b_row = switch_rows
        .iter()
        .find(|r| r.setting_id == b.id.to_string())
        .expect("audit row for newly-activated B");
    assert_eq!(b_row.after_json["isActive"], true);
}

#[tokio::test]
async fn apply_sqlite_pragmas_enables_wal_journal_mode() {
    // Regression for #467: `apply_sqlite_pragmas` must actually run.
    //
    // The test must DISTINGUISH the real body from the "replace body with Ok(())"
    // mutant. That is harder than it looks: sqlx (under SeaORM) already enables
    // `foreign_keys=ON` and `busy_timeout=5000` on EVERY connection by default,
    // so asserting those values proves nothing — they hold even when the body is
    // a no-op. (That is exactly why #466, reverted in 303ec9eb, was rejected: its
    // assertions read defaults sqlx had already set.)
    //
    // `journal_mode = WAL` is the ONE pragma here that sqlx does NOT apply: a
    // fresh FILE-backed connection defaults to `journal_mode = delete`, and only
    // our explicit `PRAGMA journal_mode = WAL` flips it to `wal`. The `Ok(())`
    // mutant leaves it `delete` → this assertion fails → the mutant is caught.
    //
    // It must be a FILE DB (not `:memory:`, where journal_mode is always
    // "memory" and WAL is inapplicable) and pinned to one pooled connection so
    // the before/after reads hit the same connection the pragma is applied to.
    use sea_orm::{ConnectOptions, ConnectionTrait, Database};

    let dir = std::env::temp_dir().join(format!(
        "presenter-pragma-test-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let db_path = dir.join("pragma.db");
    let url = format!("sqlite://{}?mode=rwc", db_path.display());

    let mut opts = ConnectOptions::new(url);
    opts.max_connections(1).min_connections(1);
    let db = Database::connect(opts).await.unwrap();
    let backend = db.get_database_backend();

    let read_journal_mode = |db: &sea_orm::DatabaseConnection| {
        let db = db.clone();
        async move {
            db.query_one(sea_orm::Statement::from_string(
                backend,
                "PRAGMA journal_mode".to_string(),
            ))
            .await
            .unwrap()
            .expect("PRAGMA journal_mode returns a row")
            .try_get_by_index::<String>(0)
            .unwrap()
        }
    };

    // Before: a fresh file-backed connection defaults to the rollback journal,
    // NOT WAL (sqlx does not enable WAL).
    assert_eq!(
        read_journal_mode(&db).await,
        "delete",
        "fresh file connection must default to the delete (rollback) journal",
    );

    // Apply the pragmas — the body under test.
    Repository::apply_sqlite_pragmas(&db).await.unwrap();

    // After: only the real body flips the journal to WAL. The `Ok(())` mutant
    // leaves it `delete`.
    assert_eq!(
        read_journal_mode(&db).await,
        "wal",
        "apply_sqlite_pragmas must switch the journal to WAL",
    );

    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Assert that `rows` contains the audit row for a sibling video source that
/// was deactivated by an activation: actor/source match and the row records the
/// is_active=true → is_active=false transition.
fn assert_sibling_deactivation_audit_row(
    rows: &[crate::audit::SettingsAuditEntry],
    sibling_id: &str,
) {
    use crate::audit::SettingsAuditSource;
    let r = rows
        .iter()
        .find(|r| r.setting_id == sibling_id)
        .expect("audit row for deactivated sibling");
    assert_eq!(r.actor, "ip-1.2.3.4");
    assert_eq!(r.source, SettingsAuditSource::HttpSetter);
    let before = r
        .before_json
        .as_ref()
        .expect("deactivation audit needs before_json");
    assert_eq!(before["isActive"], true, "before should have isActive=true");
    assert_eq!(
        r.after_json["isActive"], false,
        "after should have isActive=false",
    );
}
