//! #808 regression: the follow-up schedule of a suspect mapping and the
//! per-deck bookkeeping, driven step by step. `run_follow_up` is what the
//! worker's `select!` calls at the deadline; these tests call it directly
//! instead of sleeping, and check the deadline it set. The mock Arena and the
//! helpers live in `provisional_mapping_tests.rs`.

use super::clip_map::{ClipMapping, MAIN_KINDS, TRANSLATION_KINDS};
use super::driver::{FetchReason, HostDriver};
use super::mapping_refresh::Push;
use super::provisional_mapping::{
    push_required_kinds, selected_deck_id, ProvisionalMapping, FOLLOW_UP_DELAYS,
};
use super::provisional_mapping_tests::{
    backoff_elapsed, clip, connect, count, deck_path, driver_for, host_at, lyric_deck, lyric_line,
    param, restart_arena_mid_load, stage, wait_until, DeckArena, COMPOSITION,
};
use super::{BibleUpdate, ResolumeConnectionState, ResolumeRegistry, TimerFrame};
use chrono::Utc;
use presenter_core::BibleSlideOutput;
use serde_json::json;
use std::time::Duration;
use wiremock::MockServer;

/// The next follow-up is step `step`, due about `FOLLOW_UP_DELAYS[step]` from
/// now (measured from the fetch that scheduled it, a few ms ago).
fn assert_follow_up(driver: &HostDriver, step: usize) {
    let follow_up = driver
        .provisional
        .follow_up
        .expect("a follow-up fetch is scheduled");
    assert_eq!(follow_up.step, step);
    assert_eq!(driver.follow_up_due(), Some(follow_up.due));
    let remaining = follow_up
        .due
        .saturating_duration_since(tokio::time::Instant::now());
    let delay = FOLLOW_UP_DELAYS[step];
    assert!(
        remaining <= delay && remaining + Duration::from_secs(1) >= delay,
        "step {step}: due in {remaining:?}, expected about {delay:?}"
    );
}

#[test]
fn the_follow_up_delays_are_2_5_15_30_60_seconds() {
    assert_eq!(
        FOLLOW_UP_DELAYS.map(|delay| delay.as_secs()),
        [2, 5, 15, 30, 60]
    );
}

/// Arena keeps loading: exactly five follow-up fetches, each about
/// 2, 5, 15, 30, 60 s after the previous one, then nothing more.
#[tokio::test]
async fn follow_ups_run_on_the_schedule_and_stop_after_the_last() {
    let server = MockServer::start().await;
    let arena = DeckArena::start(&server, vec![(1, lyric_deck(100, 1))], false).await;
    let (mut driver, status) = driver_for(&server);
    driver.tick(&status).await; // cold fetch
    restart_arena_mid_load(&arena, &mut driver, &status, 10).await;

    assert_follow_up(&driver, 0);
    for step in 1..FOLLOW_UP_DELAYS.len() {
        driver.run_follow_up(&status).await;
        assert_follow_up(&driver, step);
    }
    driver.run_follow_up(&status).await; // the last step
    assert_eq!(driver.provisional.follow_up, None, "the schedule ended");
    assert_eq!(driver.follow_up_due(), None);
    driver.run_follow_up(&status).await; // nothing scheduled: no request

    assert_eq!(
        count(&server, "GET", COMPOSITION).await,
        2 + FOLLOW_UP_DELAYS.len(),
        "cold fetch, recovery fetch, five follow-ups"
    );
    assert_eq!(
        status.read().await.state,
        ResolumeConnectionState::Connected
    );
}

/// The follow-up that finds the loaded composition stops the schedule, and
/// the next line lands without any refetch.
#[tokio::test]
async fn a_follow_up_that_finds_the_full_composition_stops_the_schedule() {
    let server = MockServer::start().await;
    let arena = DeckArena::start(&server, vec![(1, lyric_deck(100, 1))], false).await;
    let (mut driver, status) = driver_for(&server);
    driver.tick(&status).await; // cold fetch
    restart_arena_mid_load(&arena, &mut driver, &status, 2).await;

    driver.run_follow_up(&status).await; // still loading
    assert_follow_up(&driver, 1);
    driver.run_follow_up(&status).await; // loaded
    assert_eq!(driver.provisional.follow_up, None);
    driver.dispatch_push(stage("Line 1"), &status).await;

    assert_eq!(count(&server, "GET", COMPOSITION).await, 4);
    assert_eq!(count(&server, "PUT", &param(1)).await, 1);
    assert_eq!(count(&server, "POST", &connect(100)).await, 1);
}

/// A follow-up due while the host is in its #484 backoff window sends nothing
/// and waits for the window to end; it keeps its step.
#[tokio::test]
async fn a_follow_up_due_inside_a_backoff_window_waits_for_it() {
    let server = MockServer::start().await;
    let arena = DeckArena::start(&server, vec![(1, lyric_deck(100, 1))], false).await;
    let (mut driver, status) = driver_for(&server);
    driver.tick(&status).await; // cold fetch
    restart_arena_mid_load(&arena, &mut driver, &status, 1).await;
    driver.record_error(anyhow::anyhow!("blip"), &status).await;
    assert!(driver.in_backoff());

    driver.run_follow_up(&status).await;

    let follow_up = driver.provisional.follow_up.expect("still scheduled");
    assert_eq!(follow_up.step, 0, "the step is not used up");
    assert_eq!(Some(follow_up.due), driver.next_retry_at);
    assert_eq!(count(&server, "GET", COMPOSITION).await, 2);
}

/// A follow-up that fails still uses up its step (so a host that keeps
/// failing ends the schedule) and counts as a host failure.
#[tokio::test]
async fn a_failed_follow_up_uses_up_its_step() {
    let server = MockServer::start().await;
    let arena = DeckArena::start(&server, vec![(1, lyric_deck(100, 1))], false).await;
    let (mut driver, status) = driver_for(&server);
    driver.tick(&status).await; // cold fetch
    restart_arena_mid_load(&arena, &mut driver, &status, 1).await;
    arena.set_online(false);

    driver.run_follow_up(&status).await;

    assert_follow_up(&driver, 1);
    let snap = status.read().await.clone();
    assert_eq!(snap.state, ResolumeConnectionState::Error);
    assert_eq!(snap.consecutive_failures, 1);
}

/// A partly loaded composition (it has clips, but lacks a kind the deck had)
/// is suspect too.
#[tokio::test]
async fn a_fetch_lacking_a_kind_the_deck_had_is_suspect() {
    let server = MockServer::start().await;
    let mut with_translation = lyric_deck(100, 1);
    with_translation.push(clip(110, "#translate-a", 11));
    with_translation.push(clip(111, "#translate-b", 12));
    let arena = DeckArena::start(&server, vec![(1, with_translation)], true).await;
    let (mut driver, status) = driver_for(&server);
    driver.tick(&status).await; // cold fetch: lyrics + translation
    assert_eq!(driver.provisional.follow_up, None);

    arena.replace_decks(vec![(1, lyric_deck(100, 1))]); // same deck, no translation yet
    driver.invalidate_mapping(FetchReason::ErrorInvalidated);
    driver.tick(&status).await; // recovery refetch

    assert_follow_up(&driver, 0);
    assert_eq!(count(&server, "GET", COMPOSITION).await, 2);
}

/// Refresh mapping is how an intentional clip edit reaches Presenter: its
/// result becomes the deck's reference, so the removed kind is never chased
/// again (no follow-ups, no lane refetch for translated lines).
#[tokio::test]
async fn a_manual_refresh_with_clips_becomes_the_decks_reference() {
    let server = MockServer::start().await;
    let mut with_translation = lyric_deck(100, 1);
    with_translation.push(clip(110, "#translate-a", 11));
    with_translation.push(clip(111, "#translate-b", 12));
    let arena = DeckArena::start(&server, vec![(1, with_translation)], true).await;
    let (mut driver, status) = driver_for(&server);
    driver.tick(&status).await; // cold fetch: lyrics + translation

    arena.replace_decks(vec![(1, lyric_deck(100, 1))]); // the operator removed them
    assert!(driver.manual_refresh(&status).await.success);
    assert_eq!(driver.provisional.follow_up, None);
    assert_eq!(
        driver.provisional.last_good.get(&Some(1)),
        Some(&MAIN_KINDS.to_vec())
    );
    driver
        .dispatch_push(Push::Stage(lyric_line("Line 1", Some("Preklad"))), &status)
        .await;

    assert_eq!(count(&server, "GET", COMPOSITION).await, 2);
    assert_eq!(count(&server, "PUT", &param(1)).await, 1);
}

/// After the lane refetch is spent, a complete fetch (the composition loaded)
/// re-arms it for the next incident.
#[tokio::test]
async fn a_complete_fetch_rearms_the_lane_refetch() {
    let server = MockServer::start().await;
    let arena = DeckArena::start(&server, vec![(1, lyric_deck(100, 1))], false).await;
    let (mut driver, status) = driver_for(&server);
    driver.tick(&status).await; // cold fetch
    restart_arena_mid_load(&arena, &mut driver, &status, 2).await;
    driver.dispatch_push(stage("Line 1"), &status).await; // lane refetch, still loading
    assert!(driver.provisional.lane_refetch_spent);

    driver.run_follow_up(&status).await; // loaded

    assert!(!driver.provisional.lane_refetch_spent);
    assert_eq!(driver.provisional.follow_up, None);
}

/// A switch to a deck that legitimately has no lyric clips (a video deck on
/// SNV) is not suspect: no follow-up fetches, one fetch for the switch.
#[tokio::test]
async fn a_switch_to_a_deck_without_lyric_clips_schedules_no_follow_ups() {
    let server = MockServer::start().await;
    let bible_only = vec![clip(300, "#bible-a", 30), clip(301, "#bible-b", 31)];
    let arena = DeckArena::start(
        &server,
        vec![(1, lyric_deck(100, 1)), (2, bible_only)],
        true,
    )
    .await;
    let (mut driver, status) = driver_for(&server);
    driver.tick(&status).await; // cold fetch, deck 1

    arena.select(1);
    driver.tick(&status).await; // the deck check follows the switch

    assert_eq!(driver.provisional.selected_deck, Some(2));
    assert_eq!(driver.provisional.follow_up, None);
    assert_eq!(count(&server, "GET", COMPOSITION).await, 2);
}

fn verse() -> Push {
    Push::Bible(BibleUpdate::from_slide_output(Some(BibleSlideOutput {
        main_text: "For God so loved".to_string(),
        main_reference: "John 3:16 (KJV)".to_string(),
        secondary_text: String::new(),
        secondary_reference: String::new(),
        triggered_at: Utc::now(),
    })))
}

/// A Bible verse in the loading window refetches like a lyric line does.
#[tokio::test]
async fn a_bible_push_in_the_loading_window_refetches_and_lands() {
    let server = MockServer::start().await;
    let bible = vec![
        clip(300, "#bible-a", 30),
        clip(301, "#bible-b", 31),
        clip(310, "#bible-reference-a", 32),
        clip(311, "#bible-reference-b", 33),
    ];
    let arena = DeckArena::start(&server, vec![(1, bible)], false).await;
    let (mut driver, status) = driver_for(&server);
    driver.tick(&status).await; // cold fetch
    restart_arena_mid_load(&arena, &mut driver, &status, 1).await;

    driver.dispatch_push(verse(), &status).await;

    assert_eq!(count(&server, "GET", COMPOSITION).await, 3);
    assert_eq!(count(&server, "PUT", &param(30)).await, 1, "verse text");
    assert_eq!(count(&server, "PUT", &param(32)).await, 1, "reference");
    assert_eq!(count(&server, "POST", &connect(300)).await, 1);
}

#[test]
fn a_push_needs_the_lanes_it_writes() {
    assert_eq!(push_required_kinds(&stage("Line")), MAIN_KINDS.to_vec());
    let translated = Push::Stage(lyric_line("Line", Some("Preklad")));
    let mut both = MAIN_KINDS.to_vec();
    both.extend(TRANSLATION_KINDS);
    assert_eq!(push_required_kinds(&translated), both);
    let verse_kinds = push_required_kinds(&verse());
    assert!(verse_kinds.contains(&"#bible-a") && verse_kinds.contains(&"#bible-reference-b"));
    assert!(!verse_kinds.contains(&"#bible-clear"));
    let clear = Push::Bible(BibleUpdate::from_slide_output(None));
    assert!(push_required_kinds(&clear).contains(&"#bible-clear"));
    let timer = Push::Timer(TimerFrame::new("00:01".into()));
    assert!(push_required_kinds(&timer).is_empty());
}

#[test]
fn the_selected_deck_is_read_from_the_composition() {
    let composition = json!({
        "decks": [
            { "id": 4, "selected": { "value": false } },
            { "id": 9, "selected": { "value": true } },
        ],
        "layers": [],
    });
    assert_eq!(selected_deck_id(&composition), Some(9));
    assert_eq!(selected_deck_id(&json!({ "layers": [] })), None);
    let none_selected = json!({ "decks": [ { "id": 4, "selected": { "value": false } } ] });
    assert_eq!(selected_deck_id(&none_selected), None);
}

#[test]
fn destination_kinds_name_the_clips_the_mapping_has() {
    let composition = json!({ "layers": [ { "clips": [
        clip(100, "#main-a", 1),
        clip(600, "#timer", 60),
    ] } ] });
    let mapping = ClipMapping::from_composition(&composition).expect("mapping");
    assert_eq!(mapping.destination_kinds(), vec!["#main-a", "#timer"]);
    assert!(!mapping.missing_tokens().contains(&"#main-a"));
    assert!(mapping.missing_tokens().contains(&"#main-b"));
    assert_eq!(
        mapping.destination_kinds().len() + mapping.missing_tokens().len(),
        16,
        "every kind is either present or missing"
    );
}

/// Presenter (re)starts while Arena is still loading: there is no last good
/// mapping yet, but a cached mapping with no recognized clip at all is
/// suspect, so the first line refetches once and lands.
#[tokio::test]
async fn a_push_after_a_cold_fetch_mid_load_refetches_and_lands() {
    let server = MockServer::start().await;
    let arena = DeckArena::start(&server, vec![(1, lyric_deck(100, 1))], true).await;
    arena.set_loading(1);
    let (mut driver, status) = driver_for(&server);
    driver.tick(&status).await; // cold fetch mid-load
    assert_follow_up(&driver, 0);

    driver.dispatch_push(stage("Line 1"), &status).await;

    assert_eq!(count(&server, "GET", COMPOSITION).await, 2);
    assert_eq!(count(&server, "PUT", &param(1)).await, 1);
    assert_eq!(count(&server, "POST", &connect(100)).await, 1);
    assert_eq!(driver.provisional.follow_up, None);
}

/// The worker's follow-up branch really waits for its deadline: the first
/// follow-up fetch comes about 2 s after the suspect fetch, never at once.
#[tokio::test]
async fn the_worker_waits_for_the_follow_up_deadline() {
    let server = MockServer::start().await;
    let arena = DeckArena::start(&server, vec![(1, lyric_deck(100, 1))], true).await;
    arena.set_loading(1);
    let registry = ResolumeRegistry::new().expect("registry");
    registry.set_hosts(vec![host_at(&server)]).await;

    wait_until(&server, "GET", COMPOSITION, 1, Duration::from_secs(5)).await;
    let suspect_fetch_seen = tokio::time::Instant::now();
    wait_until(&server, "GET", COMPOSITION, 2, Duration::from_secs(10)).await;
    let waited = suspect_fetch_seen.elapsed();

    assert!(
        waited >= Duration::from_millis(1500),
        "the first follow-up came {waited:?} after the suspect fetch"
    );
}

/// A deck check that keeps failing warns at most once per 300 s per host.
#[test]
fn deck_check_warnings_are_rate_limited() {
    let mut provisional = ProvisionalMapping::default();
    let now = tokio::time::Instant::now();
    assert!(provisional.should_warn_deck_check(now));
    assert!(!provisional.should_warn_deck_check(now + Duration::from_secs(299)));
    assert!(provisional.should_warn_deck_check(now + Duration::from_secs(300)));
    assert!(!provisional.should_warn_deck_check(now + Duration::from_secs(301)));
}

fn lyric_with_translation() -> Vec<serde_json::Value> {
    let mut clips = lyric_deck(100, 1);
    clips.push(clip(110, "#translate-a", 11));
    clips.push(clip(111, "#translate-b", 12));
    clips
}

/// A composition that did not change over the follow-ups (here: translation
/// clips removed on purpose, no Refresh mapping) is what the deck holds now.
/// It becomes the deck's reference, so translated lines stop refetching.
#[tokio::test]
async fn a_composition_unchanged_over_the_follow_ups_settles_as_the_decks_content() {
    let server = MockServer::start().await;
    let arena = DeckArena::start(&server, vec![(1, lyric_with_translation())], true).await;
    let (mut driver, status) = driver_for(&server);
    driver.tick(&status).await; // cold fetch: lyrics + translation
    arena.replace_decks(vec![(1, lyric_deck(100, 1))]); // translation removed
    driver.invalidate_mapping(FetchReason::ErrorInvalidated);
    driver.tick(&status).await; // recovery refetch: suspect
    for _ in 0..FOLLOW_UP_DELAYS.len() {
        driver.run_follow_up(&status).await;
    }

    assert_eq!(driver.provisional.follow_up, None);
    assert_eq!(
        driver.provisional.last_good.get(&Some(1)),
        Some(&MAIN_KINDS.to_vec())
    );
    let fetches = count(&server, "GET", COMPOSITION).await;
    assert_eq!(fetches, 2 + FOLLOW_UP_DELAYS.len());
    driver
        .dispatch_push(Push::Stage(lyric_line("Line 1", Some("Preklad"))), &status)
        .await;
    assert_eq!(
        count(&server, "GET", COMPOSITION).await,
        fetches,
        "no lane refetch"
    );
    assert_eq!(count(&server, "PUT", &param(1)).await, 1);
}

/// A listed deck with no presenter clip at all (a video deck), seen for the
/// first time: once the follow-ups found the same composition it is known to
/// hold none, and lyric lines on it refetch nothing.
#[tokio::test]
async fn a_listed_deck_without_presenter_clips_settles_after_the_follow_ups() {
    let server = MockServer::start().await;
    let video = vec![json!({ "id": 800, "name": { "value": "Intro video" } })];
    DeckArena::start(&server, vec![(5, video)], true).await;
    let (mut driver, status) = driver_for(&server);
    driver.tick(&status).await; // cold fetch: no recognized clip, suspect
    assert_follow_up(&driver, 0);
    for _ in 0..FOLLOW_UP_DELAYS.len() {
        driver.run_follow_up(&status).await;
    }

    assert_eq!(
        driver.provisional.last_good.get(&Some(5)),
        Some(&Vec::new())
    );
    let fetches = count(&server, "GET", COMPOSITION).await;
    assert_eq!(fetches, 1 + FOLLOW_UP_DELAYS.len());
    for n in 0..3 {
        driver
            .dispatch_push(stage(&format!("Line {n}")), &status)
            .await;
    }
    assert_eq!(count(&server, "GET", COMPOSITION).await, fetches);
}

/// A lane refetch that fails (Arena busy) never blocks the line: it goes out
/// on the cached mapping, and the next lane refetch waits `LANE_REFETCH_RETRY`.
#[tokio::test]
async fn a_failed_lane_refetch_still_pushes_on_the_cached_mapping() {
    let server = MockServer::start().await;
    let arena = DeckArena::start(&server, vec![(1, lyric_with_translation())], true).await;
    let (mut driver, status) = driver_for(&server);
    driver.tick(&status).await; // cold fetch: lyrics + translation
    arena.replace_decks(vec![(1, lyric_deck(100, 1))]); // partly loaded: no translation yet
    driver.invalidate_mapping(FetchReason::ErrorInvalidated);
    driver.tick(&status).await; // recovery refetch: suspect
    arena.fail_compositions(1);

    driver
        .dispatch_push(Push::Stage(lyric_line("Line 1", Some("Preklad"))), &status)
        .await;

    assert_eq!(
        count(&server, "GET", COMPOSITION).await,
        3,
        "the lane refetch was tried"
    );
    assert_eq!(
        count(&server, "PUT", &param(1)).await,
        1,
        "the line still went out"
    );
    assert_eq!(
        status.read().await.state,
        ResolumeConnectionState::Connected
    );
}

/// After a deck switch the old deck's ids are never used: when the refetch
/// fails, the line fails (host error, backoff) and nothing is written to the
/// deck that left the wall. The next line refetches and lands on the new deck.
#[tokio::test]
async fn a_deck_switch_whose_refetch_fails_writes_nothing_to_the_old_deck() {
    let server = MockServer::start().await;
    let arena = DeckArena::start(
        &server,
        vec![(1, lyric_deck(100, 1)), (2, lyric_deck(200, 21))],
        true,
    )
    .await;
    let (mut driver, status) = driver_for(&server);
    driver.tick(&status).await; // cold fetch, deck 1
    driver.dispatch_push(stage("Line 1"), &status).await; // lane A of deck 1

    arena.select(1);
    arena.fail_compositions(1);
    driver.dispatch_push(stage("Line 2"), &status).await;

    assert_eq!(
        count(&server, "PUT", &param(2)).await,
        0,
        "nothing for deck 1's lane B"
    );
    assert_eq!(count(&server, "PUT", &param(22)).await, 0);
    let snap = status.read().await.clone();
    assert_eq!(snap.state, ResolumeConnectionState::Error);
    assert_eq!(snap.consecutive_failures, 1);

    backoff_elapsed(&mut driver);
    driver.dispatch_push(stage("Line 3"), &status).await;
    assert_eq!(
        count(&server, "PUT", &param(22)).await,
        1,
        "deck 2's lane B"
    );
    assert_eq!(count(&server, "GET", COMPOSITION).await, 3);
}

/// A deck check that answers 404 for the deck a fresh composition still
/// selects (a broken answer) costs one refetch, then its 404s are ignored:
/// no composition fetch on every push and tick.
#[tokio::test]
async fn a_deck_called_gone_that_the_composition_still_selects_costs_one_refetch() {
    let server = MockServer::start().await;
    let arena = DeckArena::start(&server, vec![(1, lyric_deck(100, 1))], true).await;
    let (mut driver, status) = driver_for(&server);
    driver.tick(&status).await; // cold fetch, deck 1
    arena.fail_deck_checks(404);

    for line in ["Line 1", "Line 2", "Line 3"] {
        driver.dispatch_push(stage(line), &status).await;
        driver.tick(&status).await;
    }

    assert_eq!(
        count(&server, "GET", COMPOSITION).await,
        2,
        "the cold fetch + one deck refetch"
    );
    assert_eq!(count(&server, "GET", &deck_path(1)).await, 6);
    assert_eq!(
        count(&server, "PUT", &param(1)).await + count(&server, "PUT", &param(2)).await,
        3,
        "every line landed"
    );
}

/// A body without `decks` (Arena mid-load) is judged against the deck that
/// was selected before, so a partly loaded one is suspect, not "complete".
#[tokio::test]
async fn a_body_without_decks_is_judged_against_the_deck_selected_before() {
    let server = MockServer::start().await;
    let arena = DeckArena::start(&server, vec![(1, lyric_with_translation())], true).await;
    let (mut driver, status) = driver_for(&server);
    driver.tick(&status).await; // cold fetch, deck 1: lyrics + translation
    arena.set_lists_decks(false);
    arena.replace_decks(vec![(1, lyric_deck(100, 1))]); // partly loaded, no deck list
    driver.invalidate_mapping(FetchReason::ErrorInvalidated);
    driver.tick(&status).await;

    assert_follow_up(&driver, 0);
}

/// The lane refetch is part of the push's mapping step: its audit row says
/// `refetched` (and its time counts as `t_ensure_mapping_ms`, not queue wait).
#[tokio::test]
async fn a_lane_refetch_is_audited_as_refetched() {
    let server = MockServer::start().await;
    let arena = DeckArena::start(&server, vec![(1, lyric_deck(100, 1))], false).await;
    let (mut driver, status) = driver_for(&server);
    let (audit_tx, mut audit_rx) = tokio::sync::mpsc::channel(8);
    driver.audit_tx = Some(audit_tx);
    driver.tick(&status).await; // cold fetch
    restart_arena_mid_load(&arena, &mut driver, &status, 1).await;

    driver.dispatch_push(stage("Line 1"), &status).await;

    let row = audit_rx.try_recv().expect("an audit row for the push");
    assert!(row.refetched, "the lane refetch is in the audit row");
    assert_eq!(row.outcome, "ok");
}
