use super::stage::{format_countdown_text, sanitize_song_title};
use super::*;
use crate::live::LiveEvent;
use presenter_core::{
    bible::BibleIngestionBatch, BiblePassage, BibleReference, BibleTranslation, Library, LibraryId,
    SlideContent, SlideText, TimerCommand, TimerState,
};

/// Regression for #333 item 6: the startup auto-restore branch must skip when
/// no hardware H264 encoder is registered. Otherwise the host can immediately
/// re-enter the wedge state that took prod down on 2026-05-24 (encoder probe
/// succeeded after registry rescan → supervisor activated source → pipeline
/// melted the N100). The predicate is the single decision point and is
/// dependency-injected for testability.
#[test]
fn should_auto_restore_ndi_requires_manager_and_encoder() {
    assert!(super::should_auto_restore_ndi(true, true));
    assert!(
        !super::should_auto_restore_ndi(true, false),
        "must skip restore when NDI manager exists but no encoder is registered \
         (#333 item 6 — prevent immediate re-melt after registry rescan)"
    );
    assert!(!super::should_auto_restore_ndi(false, true));
    assert!(!super::should_auto_restore_ndi(false, false));
}

/// Structural regression (deep-review 🔵 #7): the predicate is correct in
/// isolation, but item 6's safety depends on EVERY auto-restore site
/// consulting it. There are two sites in `mod.rs`: the one-shot startup
/// restore and the 30s background reconnect loop. A refactor that drops
/// one of the call sites would silently re-introduce the 2026-05-24 wedge
/// failure mode without breaking any other test. This test pins the call
/// sites lexically — if you legitimately refactor (e.g. extract to a
/// shared helper), update the expected count and the comment.
#[test]
fn auto_restore_sites_invoke_encoder_gate_predicate() {
    let src = include_str!("mod.rs");
    let occurrences = src
        .matches("should_auto_restore_ndi(manager_loaded, encoder_available)")
        .count();
    assert_eq!(
        occurrences, 2,
        "expected exactly 2 call sites to should_auto_restore_ndi(manager_loaded, encoder_available) \
         in state/mod.rs (one-shot startup restore + 30s reconnect loop); found {occurrences}. \
         If a call site was intentionally removed or refactored, update this test \
         AND the supporting comments to match. Otherwise #333 item 6's protection \
         has a hole — restore the gate before merging."
    );
}

/// Regression for #339: cold-reboot verification of PR #334 on prod
/// (2026-05-24 17:07 CEST) reproduced the boot-time race. Item 1's forced
/// registry rescan executed (log line emitted) but the resulting registry
/// still showed the `va` plugin with zero features — `vah264enc` remained
/// unregistered for ~50s after boot. Item 6's encoder gate correctly
/// skipped the auto-restore (host stayed administrable), but the user
/// still had to manually restart presenter to get streaming back.
///
/// Fix: add `ExecStartPre` that polls `gst-inspect-1.0 vah264enc` until
/// the encoder is probeable (capped at 30s by `timeout`). systemd will
/// not exec the presenter binary until the encoder is visible, closing
/// the boot race without code-side polling.
///
/// This test pins the ExecStartPre line in BOTH unit files (prod +
/// dev). Without both files, a partial revert would re-open the race
/// on one environment.
#[test]
fn presenter_service_blocks_start_until_h264_encoder_probeable() {
    let prod = include_str!("../../../../scripts/deploy/presenter.service");
    let dev = include_str!("../../../../scripts/deploy/presenter-dev.service");
    for (name, src) in [("presenter.service", prod), ("presenter-dev.service", dev)] {
        assert!(
            src.contains("ExecStartPre=-"),
            "{name}: missing ExecStartPre with leading `-` (best-effort prefix). \
             Without `-`, a 30s timeout (no GPU at all) makes systemd fail the \
             unit — blocking non-NDI features (lyrics, Bible, timers) from \
             starting. The gate must be non-fatal so item 6's encoder gate can \
             take over (#339)"
        );
        assert!(
            src.contains("vah264enc"),
            "{name}: ExecStartPre must probe `vah264enc` (Intel/AMD VA-API — \
             prod N100). The `va` plugin can register without features, so a \
             generic plugin check is insufficient (#339)"
        );
        assert!(
            src.contains("nvh264enc"),
            "{name}: ExecStartPre must ALSO probe `nvh264enc` (Nvidia NVENC — \
             dev2 RTX). Without this branch the dev deploy unit-loops on a \
             machine with Nvidia GPU because vah264enc never registers there \
             (caught 2026-05-24 in CI run 26367361768 — #339 hotfix)"
        );
        assert!(
            src.contains("gst-inspect-1.0"),
            "{name}: ExecStartPre must use `gst-inspect-1.0` — that's the \
             canonical GStreamer feature query and matches what \
             `presenter_ndi::hw_h264_encoder()` will see at startup (#339)"
        );
    }
}

/// Regression for #335: cap presenter.service resource usage so a runaway
/// NDI pipeline cannot wedge the entire host. The 2026-05-24 prod incident
/// showed that once `whepserversink` spawned multiple per-consumer
/// encoders, the N100 saturated all 4 cores within 3 minutes — sshd and
/// presenter both stopped responding to TCP handshakes; recovery required
/// power-cycling. CPUQuota leaves at least one core for sshd. MemoryMax
/// bounds growth so the kernel's per-service OOM kicks in before the
/// host-wide OOM killer. TasksMax caps thread/process explosion if a
/// pipeline rebuild leaks (e.g. a leaked tokio task per failed retry).
///
/// Without this test, a future deploy-script refactor could silently
/// drop these directives — re-opening the wedge failure mode that took
/// prod offline. Both unit files (prod + dev) are pinned.
#[test]
fn presenter_service_has_resource_limits() {
    let prod = include_str!("../../../../scripts/deploy/presenter.service");
    let dev = include_str!("../../../../scripts/deploy/presenter-dev.service");
    for (name, src) in [("presenter.service", prod), ("presenter-dev.service", dev)] {
        assert!(
            src.contains("CPUQuota="),
            "{name}: missing CPUQuota directive — runaway pipeline can saturate \
             all cores and lock out sshd (see #335 + 2026-05-24 prod incident)"
        );
        assert!(
            src.contains("MemoryMax="),
            "{name}: missing MemoryMax directive — runaway pipeline can grow \
             until host-wide OOM killer fires unpredictable victims (#335)"
        );
        assert!(
            src.contains("TasksMax="),
            "{name}: missing TasksMax directive — pipeline rebuilds that leak \
             tasks can exhaust the kernel PID space (#335)"
        );
    }
}

#[tokio::test]
async fn empty_state_does_not_auto_seed_library() {
    // Regression guard for issue #228: server startup must NOT auto-import any
    // library. The Import Data workflow is the ONLY (re)populate path.
    let state = AppState::in_memory().await.unwrap();
    let libraries = state.libraries().await.unwrap();
    assert!(
        libraries.is_empty(),
        "expected empty libraries on fresh state, found {}",
        libraries.len()
    );
}

#[tokio::test]
async fn stage_updates_emit_live_event() {
    let state = AppState::in_memory().await.unwrap();
    super::seed_sample_library(&state).await.unwrap();
    let hub = state.live_hub();
    let mut rx = hub.subscribe();

    let libraries = state.libraries().await.unwrap();
    let presentation = &libraries[0].presentations[0];
    let current = presentation.slides[0].id;
    let next = presentation.slides.get(1).map(|slide| slide.id);

    state
        .update_stage_state(presentation.id, current, next, None)
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
        .stage_display_snapshot(DEFAULT_STAGE_LAYOUT_CODE)
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
    super::seed_sample_library(&state).await.unwrap();
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
            None, // metadata
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
        .stage_display_snapshot(DEFAULT_STAGE_LAYOUT_CODE)
        .await
        .unwrap()
        .expect("snapshot");
    assert_eq!(snapshot.presentation_name.unwrap(), "Primer");
    assert_eq!(snapshot.current.unwrap().main, "Prvá veta");
}

#[tokio::test]
async fn stage_resolution_propagates_effective_group() {
    let state = AppState::in_memory().await.unwrap();
    super::seed_sample_library(&state).await.unwrap();
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
    assert_eq!(format_countdown_text(3605), "1h 0m");
    assert_eq!(format_countdown_text(125), "02:05");
    assert_eq!(format_countdown_text(59), "59");
    assert_eq!(format_countdown_text(0), "0");
    assert_eq!(format_countdown_text(-12), "");
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
    let reference = BibleReference::new("John", 3, 16, 16).unwrap();
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
        .trigger_bible_passage("test", &reference, Default::default())
        .await
        .unwrap();
    assert_eq!(broadcast.passage.reference, reference);
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

#[test]
fn sanitize_song_names_remove_numeric_prefix() {
    assert_eq!(sanitize_song_title("001 Amazing Grace"), "Amazing Grace");
    assert_eq!(sanitize_song_title("001   Song"), "Song");
    assert_eq!(sanitize_song_title("100"), "100");
    assert_eq!(sanitize_song_title("No Prefix"), "No Prefix");
}

#[tokio::test]
async fn from_config_against_empty_db_leaves_libraries_empty() {
    // Integration-level regression guard for issue #228: the production
    // constructor path (AppState::from_config) must not auto-import any
    // library on startup. This complements empty_state_does_not_auto_seed_library
    // which covers the in-memory test fixture path.
    let tmp = tempfile::NamedTempFile::new().expect("temp file");
    let url = format!("sqlite://{}?mode=rwc", tmp.path().display());

    let config = crate::config::ServerConfig {
        http: crate::config::HttpConfig {
            port: 0, // unused in test — server is never started
        },
        database: crate::config::DatabaseConfig { url },
        companion: crate::config::CompanionConfig::default(),
        osc: crate::config::OscConfig::default(),
        stage: crate::config::StageConfig {
            heartbeat: crate::stage_connections::StageHeartbeatConfig::default_values(),
        },
        android: crate::config::AndroidConfig::default(),
        network: crate::config::NetworkConfig::default(),
    };

    let state = AppState::from_config(config).await.expect("from_config");
    let libraries = state.libraries().await.expect("libraries");
    assert!(
        libraries.is_empty(),
        "production startup must NOT auto-seed libraries (found {} on fresh DB)",
        libraries.len()
    );
}

#[tokio::test]
async fn api_input_does_not_leak_when_layout_is_worship() {
    use std::time::Duration;
    use tokio::time::timeout;

    let state = AppState::in_memory().await.unwrap();
    state
        .set_stage_layout_code("worship-snv")
        .await
        .expect("set worship-snv");

    let mut rx = state.live_hub().subscribe();

    let api_state = ApiStageState {
        current_text: "test main".to_string(),
        current_group: "test group".to_string(),
        current_song: "test song".to_string(),
        ..Default::default()
    };
    state
        .update_api_stage(api_state)
        .await
        .expect("update_api_stage");

    // Drain any non-Stage events for a short window. Assert no
    // LiveEvent::Stage arrives within the timeout — that's the no-leak invariant.
    let saw_stage = async {
        loop {
            match rx.recv().await {
                Ok(LiveEvent::Stage { .. }) => return true,
                Ok(_) => continue,
                Err(_) => return false,
            }
        }
    };
    let result = timeout(Duration::from_millis(150), saw_stage).await;
    assert!(
        result.is_err(),
        "expected NO LiveEvent::Stage when layout is worship-snv"
    );

    // Sanity: the api_stage state IS stored (not silently discarded).
    let stored = state.api_stage.read().await.clone();
    assert_eq!(stored.current_text, "test main");
}

#[tokio::test]
async fn api_input_publishes_when_layout_is_api() {
    use std::time::Duration;
    use tokio::time::timeout;

    let state = AppState::in_memory().await.unwrap();
    state
        .set_stage_layout_code("api")
        .await
        .expect("set api layout");

    // Subscribe AFTER the layout switch so we don't see leftover events
    // from set_stage_layout_code.
    let mut rx = state.live_hub().subscribe();

    let api_state = ApiStageState {
        current_text: "live api content".to_string(),
        ..Default::default()
    };
    state
        .update_api_stage(api_state)
        .await
        .expect("update_api_stage");

    let stage_event = async {
        loop {
            match rx.recv().await {
                Ok(LiveEvent::Stage { snapshot }) => return Some(snapshot),
                Ok(_) => continue,
                Err(_) => return None,
            }
        }
    };
    let snapshot = timeout(Duration::from_millis(500), stage_event)
        .await
        .expect("Stage event arrived within timeout")
        .expect("Stage event payload");

    assert_eq!(
        snapshot.layout.code, "api",
        "snapshot must use the api layout"
    );
}

#[tokio::test]
async fn switching_to_api_publishes_stored_api_state() {
    use std::time::Duration;
    use tokio::time::timeout;

    let state = AppState::in_memory().await.unwrap();
    state
        .set_stage_layout_code("worship-snv")
        .await
        .expect("set worship-snv");

    // Pre-store API content while not in api layout (the gate prevents
    // an event from publishing here).
    state
        .update_api_stage(ApiStageState {
            current_text: "stored content".to_string(),
            ..Default::default()
        })
        .await
        .expect("update_api_stage");

    // Subscribe AFTER the pre-store, BEFORE the switch.
    let mut rx = state.live_hub().subscribe();

    state
        .set_stage_layout_code("api")
        .await
        .expect("switch to api");

    // Expect at least one StageLayout event AND one Stage event with api
    // layout within the timeout.
    let mut saw_layout = false;
    let mut saw_stage_with_api = false;
    let collect = async {
        for _ in 0..10 {
            if let Ok(ev) = rx.recv().await {
                match ev {
                    LiveEvent::StageLayout { code } if code == "api" => saw_layout = true,
                    LiveEvent::Stage { snapshot } if snapshot.layout.code == "api" => {
                        saw_stage_with_api = true;
                    }
                    _ => {}
                }
                if saw_layout && saw_stage_with_api {
                    return;
                }
            }
        }
    };
    let _ = timeout(Duration::from_millis(500), collect).await;

    assert!(
        saw_layout,
        "expected LiveEvent::StageLayout for api after switch"
    );
    assert!(
        saw_stage_with_api,
        "expected LiveEvent::Stage with api layout after switch"
    );
}

#[tokio::test]
async fn publish_stage_context_emits_camera_crew_snapshot_alongside_operator_layout() {
    use std::time::Duration;
    use tokio::time::timeout;

    let state = AppState::in_memory().await.unwrap();
    state
        .set_stage_layout_code("worship-snv")
        .await
        .expect("set layout");
    super::seed_sample_library(&state).await.unwrap();

    let mut rx = state.live_hub().subscribe();

    state.broadcast_stage_snapshots().await.expect("broadcast");

    let mut saw_worship = false;
    let mut saw_camera = false;
    let collect = async {
        for _ in 0..20 {
            match rx.recv().await {
                Ok(LiveEvent::Stage { snapshot }) => match snapshot.layout.code.as_str() {
                    "worship-snv" => saw_worship = true,
                    "camera-crew" => saw_camera = true,
                    _ => {}
                },
                Ok(_) => continue,
                Err(_) => break,
            }
            if saw_worship && saw_camera {
                return;
            }
        }
    };
    let _ = timeout(Duration::from_millis(500), collect).await;

    assert!(saw_worship, "expected worship-snv snapshot");
    assert!(
        saw_camera,
        "expected camera-crew snapshot alongside worship-snv"
    );
}

#[tokio::test]
async fn publish_stage_context_emits_camera_crew_snapshot_even_when_api_active() {
    use std::time::Duration;
    use tokio::time::timeout;

    let state = AppState::in_memory().await.unwrap();
    state
        .set_stage_layout_code("api")
        .await
        .expect("set layout");
    super::seed_sample_library(&state).await.unwrap();

    let mut rx = state.live_hub().subscribe();

    state.broadcast_stage_snapshots().await.expect("broadcast");

    let mut saw_camera = false;
    let collect = async {
        for _ in 0..20 {
            match rx.recv().await {
                Ok(LiveEvent::Stage { snapshot }) if snapshot.layout.code == "camera-crew" => {
                    saw_camera = true;
                    return;
                }
                Ok(_) => continue,
                Err(_) => break,
            }
        }
    };
    let _ = timeout(Duration::from_millis(500), collect).await;

    assert!(
        saw_camera,
        "camera-crew snapshot must publish even when api layout is selected"
    );
}

#[tokio::test]
async fn stage_displays_excludes_camera_crew_from_operator_picker() {
    let state = AppState::in_memory().await.unwrap();
    let layouts = state.stage_displays().await.unwrap();
    let codes: Vec<&str> = layouts.iter().map(|l| l.code.as_str()).collect();
    assert!(
        !codes.contains(&"camera-crew"),
        "camera-crew must not appear in operator picker; got {codes:?}"
    );
    // Sanity: regular layouts ARE present.
    assert!(codes.contains(&"worship-snv"));
    assert!(codes.contains(&"preach"));
}

#[tokio::test]
async fn set_stage_layout_code_rejects_camera_crew() {
    let state = AppState::in_memory().await.unwrap();
    let result = state.set_stage_layout_code("camera-crew").await;
    assert!(
        result.is_err(),
        "camera-crew must not be settable as operator layout"
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("camera-crew") && msg.contains("not"),
        "error message should explain the rejection; got: {msg}"
    );
}
