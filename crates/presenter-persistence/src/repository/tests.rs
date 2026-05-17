use super::Repository;
use chrono::{Duration, Utc};
use presenter_core::{
    bible::BibleIngestionBatch, playlist::PlaylistEntryKind, AndroidStageDisplayDraft,
    BiblePassage, BibleReference, BibleTranslation, CountdownTimer, Library, LibraryId,
    OscSettingsDraft, PlaylistEntry, PlaylistEntryId, PreachTimer, Presentation, PresentationId,
    ResolumeHostDraft, SearchResultKind, Slide, SlideContent, SlideGroup, SlideId, SlideText,
    StageState, TimerState, TimersState, VelocityMode, DEFAULT_ADB_PORT, DEFAULT_LAUNCH_COMPONENT,
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
    let created = repo.create_resolume_host(&draft).await.unwrap();
    assert_eq!(created.label, "Arena");
    assert_eq!(created.host, "resolume.lan");
    assert_eq!(created.port, 8090);
    assert!(created.is_enabled);

    let hosts = repo.list_resolume_hosts().await.unwrap();
    assert_eq!(hosts.len(), 1);

    let updated_draft =
        ResolumeHostDraft::new("Arena North", "resolume.lan", 8090).with_enabled(false);
    let updated = repo
        .update_resolume_host(created.id, &updated_draft)
        .await
        .unwrap();
    assert_eq!(updated.label, "Arena North");
    assert!(!updated.is_enabled);

    repo.delete_resolume_host(created.id).await.unwrap();
    let after_delete = repo.list_resolume_hosts().await.unwrap();
    assert!(after_delete.is_empty());
}

#[tokio::test]
async fn android_stage_display_crud_round_trip() {
    let repo = Repository::connect_in_memory().await.unwrap();

    let initial = repo.list_android_stage_displays().await.unwrap();
    let initial_count = initial.len();

    let draft = AndroidStageDisplayDraft::new("Stage Left", "test-stage.invalid");
    let created = repo.create_android_stage_display(&draft).await.unwrap();
    assert_eq!(created.label, "Stage Left");
    assert_eq!(created.host, "test-stage.invalid");
    assert_eq!(created.port, DEFAULT_ADB_PORT);
    assert_eq!(
        created.launch_component,
        DEFAULT_LAUNCH_COMPONENT.to_string()
    );
    assert!(created.is_enabled);

    let displays = repo.list_android_stage_displays().await.unwrap();
    assert_eq!(displays.len(), initial_count + 1);
    assert!(displays.iter().any(|d| d.id == created.id));

    let updated_draft = AndroidStageDisplayDraft::new("Stage Right", "test-stage2.invalid")
        .with_port(5566)
        .with_launch_component("com.example/.Main")
        .with_enabled(false);
    let updated = repo
        .update_android_stage_display(created.id, &updated_draft)
        .await
        .unwrap();
    assert_eq!(updated.label, "Stage Right");
    assert_eq!(updated.host, "test-stage2.invalid");
    assert_eq!(updated.port, 5566);
    assert_eq!(updated.launch_component, "com.example/.Main");
    assert!(!updated.is_enabled);

    repo.delete_android_stage_display(created.id).await.unwrap();
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
        assert_eq!(
            d.launch_component,
            "com.fullykiosk.videokiosk/de.ozerov.fully.MainActivity",
        );
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
    repo.create_android_stage_display(&draft).await.unwrap();
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
    assert_eq!(rows[0].source, crate::audit::SettingsAuditSource::HttpSetter);
    assert_eq!(rows[0].after_json["enabled"], true);
}
