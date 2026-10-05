//! #808 regression (2026-10-05): a composition fetched while Arena is still
//! loading, or before the Resolume operator switched decks, must never leave
//! lyric/Bible pushes skipped.
//!
//! PP incident: after an Arena restart the recovery fetch reached Arena while
//! it was still loading `Songs PP` and got 27 clips (110 KB) with no
//! recognized destination. The driver cached that mapping, every lyric push
//! logged `Resolume has no clips configured for lane` and was skipped, and with
//! the periodic composition fetch gone (#808) nothing re-read it until the
//! operator pressed "Refresh mapping".
//!
//! Owner ruling 2026-10-05: a push is never dropped because of a cached
//! "problem" state, not after a deck switch and not because of a rate limit.
//! `/composition` lists only the SELECTED deck's clips, so a deck switch
//! changes every clip id.
//!
//! `DeckArena` serves a (multi-)deck composition, can pretend to be still
//! loading (a tag-less 27-clip composition for the next N composition GETs),
//! and answers `GET /composition/decks/by-id/{id}` like Resolume. Driver-level
//! tests skip a backoff window by clearing `next_retry_at`, never by sleeping.

use super::driver::HostDriver;
use super::mapping_refresh::Push;
use super::{
    resolume_http_client, ResolumeConnectionSnapshot, ResolumeConnectionState, ResolumeRegistry,
    StageUpdate, TimerFrame,
};
use chrono::Utc;
use presenter_core::{ResolumeHost, ResolumeHostId};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::RwLock;
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

pub(super) type Status = Arc<RwLock<ResolumeConnectionSnapshot>>;

pub(super) const COMPOSITION: &str = "/api/v1/composition";
const PRODUCT: &str = "/api/v1/product";

pub(super) fn clip(id: i64, name: &str, param_id: i64) -> Value {
    json!({
        "id": id,
        "name": { "value": name },
        "video": { "sourceparams": { "text": { "valuetype": "ParamText", "id": param_id } } },
    })
}

pub(super) fn param(id: i64) -> String {
    format!("/api/v1/parameter/by-id/{id}")
}

pub(super) fn connect(id: i64) -> String {
    format!("/api/v1/composition/clips/by-id/{id}/connect")
}

pub(super) fn deck_path(id: i64) -> String {
    format!("/api/v1/composition/decks/by-id/{id}")
}

/// A deck with lyric clips: `#main-a` is clip `clip_base` with text param
/// `param_base`, `#main-b` is clip `clip_base + 1` with param `param_base + 1`.
pub(super) fn lyric_deck(clip_base: i64, param_base: i64) -> Vec<Value> {
    vec![
        clip(clip_base, "#main-a", param_base),
        clip(clip_base + 1, "#main-b", param_base + 1),
    ]
}

/// What Arena served on PP while it was still loading the composition:
/// 27 clips, none of them tagged.
fn loading_composition() -> Value {
    let clips: Vec<Value> = (0..27)
        .map(|i| json!({ "id": 9000 + i, "name": { "value": format!("Clip {i}") } }))
        .collect();
    json!({ "layers": [ { "clips": clips } ] })
}

struct ArenaState {
    /// `(deck id, the clips /composition lists while that deck is selected)`.
    decks: Vec<(i64, Vec<Value>)>,
    selected: usize,
    /// Whether `/composition` lists `decks` (every real Arena does; a
    /// deck-less body exercises the lane path on its own).
    lists_decks: bool,
    /// Composition GETs still answered with [`loading_composition`].
    loading_left: usize,
    online: bool,
    /// When set, deck checks answer this status instead of the deck.
    deck_check_status: Option<u16>,
    /// Composition GETs still answered with a 500.
    failing_left: usize,
}

fn deck_json(state: &ArenaState, index: usize) -> Value {
    json!({
        "id": state.decks[index].0,
        "name": { "value": format!("Deck {}", index + 1) },
        "selected": { "value": index == state.selected },
    })
}

fn composition_body(state: &ArenaState) -> Value {
    let clips = &state.decks[state.selected].1;
    let mut body = json!({ "layers": [ { "clips": clips } ] });
    if state.lists_decks {
        let decks: Vec<Value> = (0..state.decks.len())
            .map(|index| deck_json(state, index))
            .collect();
        body["decks"] = Value::Array(decks);
    }
    body
}

/// A mock Arena with decks. Text writes and clip connects answer 200 for any
/// id, as Resolume does for clips of a deck that is not selected.
#[derive(Clone)]
pub(super) struct DeckArena {
    state: Arc<Mutex<ArenaState>>,
}

impl DeckArena {
    pub(super) async fn start(
        server: &MockServer,
        decks: Vec<(i64, Vec<Value>)>,
        lists_decks: bool,
    ) -> Self {
        let arena = Self {
            state: Arc::new(Mutex::new(ArenaState {
                decks,
                selected: 0,
                lists_decks,
                loading_left: 0,
                online: true,
                deck_check_status: None,
                failing_left: 0,
            })),
        };
        let routes = [
            (path(COMPOSITION), Route::Composition),
            (path(PRODUCT), Route::Product),
        ];
        for (route_path, route) in routes {
            Mock::given(method("GET"))
                .and(route_path)
                .respond_with(ArenaRoute {
                    arena: arena.clone(),
                    route,
                })
                .mount(server)
                .await;
        }
        Mock::given(method("GET"))
            .and(path_regex(r"^/api/v1/composition/decks/by-id/\d+$"))
            .respond_with(ArenaRoute {
                arena: arena.clone(),
                route: Route::Deck,
            })
            .mount(server)
            .await;
        Mock::given(method("PUT"))
            .and(path_regex(r"^/api/v1/parameter/by-id/\d+$"))
            .respond_with(ResponseTemplate::new(200))
            .mount(server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r"^/api/v1/composition/clips/by-id/\d+/connect$"))
            .respond_with(ResponseTemplate::new(200))
            .mount(server)
            .await;
        arena
    }

    fn with<R>(&self, change: impl FnOnce(&mut ArenaState) -> R) -> R {
        let mut guard = self.state.lock().expect("arena lock");
        let state: &mut ArenaState = &mut guard;
        change(state)
    }

    /// The Resolume operator selects another deck.
    pub(super) fn select(&self, index: usize) {
        self.with(|state| state.selected = index);
    }

    pub(super) fn set_online(&self, online: bool) {
        self.with(|state| state.online = online);
    }

    /// Arena is loading: the next `fetches` composition GETs get the
    /// tag-less loading composition.
    pub(super) fn set_loading(&self, fetches: usize) {
        self.with(|state| state.loading_left = fetches);
    }

    pub(super) fn fail_deck_checks(&self, status: u16) {
        self.with(|state| state.deck_check_status = Some(status));
    }

    /// Deck checks answer the deck again.
    pub(super) fn heal_deck_checks(&self) {
        self.with(|state| state.deck_check_status = None);
    }

    /// The next `fetches` composition GETs answer 500.
    pub(super) fn fail_compositions(&self, fetches: usize) {
        self.with(|state| state.failing_left = fetches);
    }

    /// Whether `/composition` lists `decks` from now on.
    pub(super) fn set_lists_decks(&self, lists_decks: bool) {
        self.with(|state| state.lists_decks = lists_decks);
    }

    /// Another composition was loaded: every deck id is new.
    pub(super) fn replace_decks(&self, decks: Vec<(i64, Vec<Value>)>) {
        self.with(|state| {
            state.decks = decks;
            state.selected = 0;
        });
    }
}

#[derive(Clone, Copy)]
enum Route {
    Composition,
    Product,
    Deck,
}

struct ArenaRoute {
    arena: DeckArena,
    route: Route,
}

impl Respond for ArenaRoute {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let mut state = self.arena.state.lock().expect("arena lock");
        if !state.online {
            return ResponseTemplate::new(503);
        }
        match self.route {
            Route::Product => ResponseTemplate::new(200).set_body_json(json!({
                "name": "Arena", "major": 7, "minor": 13, "micro": 2, "revision": 0,
            })),
            Route::Composition => {
                if state.failing_left > 0 {
                    state.failing_left -= 1;
                    return ResponseTemplate::new(500);
                }
                if state.loading_left > 0 {
                    state.loading_left -= 1;
                    return ResponseTemplate::new(200).set_body_json(loading_composition());
                }
                ResponseTemplate::new(200).set_body_json(composition_body(&state))
            }
            Route::Deck => {
                if let Some(status) = state.deck_check_status {
                    return ResponseTemplate::new(status);
                }
                let id = request
                    .url
                    .path()
                    .rsplit('/')
                    .next()
                    .and_then(|segment| segment.parse::<i64>().ok());
                match state
                    .decks
                    .iter()
                    .position(|(deck_id, _)| Some(*deck_id) == id)
                {
                    Some(index) => {
                        ResponseTemplate::new(200).set_body_json(deck_json(&state, index))
                    }
                    None => ResponseTemplate::new(404),
                }
            }
        }
    }
}

pub(super) fn host_at(server: &MockServer) -> ResolumeHost {
    let addr = server.address();
    let now = Utc::now();
    ResolumeHost::new(
        ResolumeHostId::new(),
        "Mock Arena".into(),
        addr.ip().to_string(),
        addr.port(),
        true,
        now,
        now,
    )
}

pub(super) fn driver_for(server: &MockServer) -> (HostDriver, Status) {
    let client = resolume_http_client().expect("client");
    (
        HostDriver::new(client, host_at(server)),
        Arc::new(RwLock::new(ResolumeConnectionSnapshot::disabled())),
    )
}

pub(super) async fn count(server: &MockServer, verb: &str, route: &str) -> usize {
    server
        .received_requests()
        .await
        .expect("request recording is on")
        .iter()
        .filter(|req| req.method.as_str() == verb && req.url.path() == route)
        .count()
}

/// Bounded retry-with-assert: wait until the mock saw at least `at_least`
/// `verb route` requests.
pub(super) async fn wait_until(
    server: &MockServer,
    verb: &str,
    route: &str,
    at_least: usize,
    bound: Duration,
) {
    let deadline = tokio::time::Instant::now() + bound;
    loop {
        let seen = count(server, verb, route).await;
        if seen >= at_least {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{verb} {route}: saw {seen} request(s), expected at least {at_least} within {bound:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

pub(super) fn lyric_line(main: &str, translation: Option<&str>) -> StageUpdate {
    StageUpdate {
        current_main: Some(main.to_string()),
        current_translation: translation.map(str::to_string),
        song_name: None,
        band_name: None,
        enqueued_at: None,
        correlation_id: None,
    }
}

pub(super) fn stage(main: &str) -> Push {
    Push::Stage(lyric_line(main, None))
}

/// Let the next tick or push run as if the #484 backoff window had elapsed.
pub(super) fn backoff_elapsed(driver: &mut HostDriver) {
    driver.next_retry_at = None;
}

/// Arena goes down long enough to cross the #563b threshold (3 failed ticks)
/// and comes back still loading: the next `loading_fetches` composition GETs
/// get the tag-less loading composition. The recovery tick refetches and
/// caches it, exactly like PP's `reason="error-invalidated" clip_count=27`.
pub(super) async fn restart_arena_mid_load(
    arena: &DeckArena,
    driver: &mut HostDriver,
    status: &Status,
    loading_fetches: usize,
) {
    arena.set_online(false);
    for _ in 0..3 {
        backoff_elapsed(driver);
        driver.tick(status).await;
    }
    assert!(
        driver.mapping.is_none(),
        "three failed ticks invalidate the mapping (#563b)"
    );
    arena.set_loading(loading_fetches);
    arena.set_online(true);
    backoff_elapsed(driver);
    driver.tick(status).await;
    let mapping = driver
        .mapping
        .as_ref()
        .expect("the recovery fetch cached a mapping");
    assert!(
        mapping.main_a.is_empty() && mapping.main_b.is_empty(),
        "the loading composition has no lyric clips"
    );
}

/// RED before the fix: the worker cached the loading composition (no
/// recognized clip) at its cold start and never fetched again, so the line was
/// skipped. The suspect mapping must be re-read on the follow-up schedule
/// (first step about 2 s later) with no push and no operator action, and the
/// next line must land.
#[tokio::test]
async fn a_worker_started_while_arena_loads_reaches_the_full_mapping_by_itself() {
    let server = MockServer::start().await;
    let arena = DeckArena::start(&server, vec![(1, lyric_deck(100, 1))], true).await;
    arena.set_loading(1);
    let registry = ResolumeRegistry::new().expect("registry");
    registry.set_hosts(vec![host_at(&server)]).await;

    wait_until(&server, "GET", COMPOSITION, 2, Duration::from_secs(10)).await;
    registry.stage_update(lyric_line("Line 1", None)).await;
    wait_until(&server, "PUT", &param(1), 1, Duration::from_secs(5)).await;

    assert_eq!(
        count(&server, "GET", COMPOSITION).await,
        2,
        "the cold fetch hit the loading composition, one follow-up got the full one"
    );
    wait_until(&server, "POST", &connect(100), 1, Duration::from_secs(5)).await;
}

/// RED before the fix: the line was skipped ("no clips configured for lane")
/// and nothing refetched. A push whose lane the deck's last good mapping had
/// refetches the composition once before it is applied, and lands.
#[tokio::test]
async fn a_push_during_the_loading_window_refetches_and_lands_on_the_loaded_clips() {
    let server = MockServer::start().await;
    let arena = DeckArena::start(&server, vec![(1, lyric_deck(100, 1))], false).await;
    let (mut driver, status) = driver_for(&server);
    driver.tick(&status).await; // cold fetch: the full composition
    restart_arena_mid_load(&arena, &mut driver, &status, 1).await;

    driver.dispatch_push(stage("Line 1"), &status).await;

    assert_eq!(count(&server, "PUT", &param(1)).await, 1, "the line landed");
    assert_eq!(count(&server, "POST", &connect(100)).await, 1);
    assert_eq!(
        count(&server, "GET", COMPOSITION).await,
        3,
        "cold fetch, the recovery fetch mid-load, one refetch for the push"
    );
    let snap = status.read().await.clone();
    assert_eq!(snap.state, ResolumeConnectionState::Connected);
    assert_eq!(snap.consecutive_failures, 0);
}

/// RED before the fix (no refetch at all). While the lane stays empty after
/// that refetch, the next pushes refetch nothing until the next follow-up
/// step (or the retry instant): never one refetch per push.
#[tokio::test]
async fn a_lane_that_stays_empty_refetches_once_until_the_next_follow_up() {
    let server = MockServer::start().await;
    let arena = DeckArena::start(&server, vec![(1, lyric_deck(100, 1))], false).await;
    let (mut driver, status) = driver_for(&server);
    driver.tick(&status).await; // cold fetch
    restart_arena_mid_load(&arena, &mut driver, &status, 5).await;

    for line in ["Line 1", "Line 2", "Line 3", "Line 4"] {
        driver.dispatch_push(stage(line), &status).await;
    }

    assert_eq!(
        count(&server, "GET", COMPOSITION).await,
        3,
        "cold fetch, the recovery fetch, exactly one refetch for the pushes"
    );
    assert_eq!(count(&server, "PUT", &param(1)).await, 0);
    assert_eq!(count(&server, "PUT", &param(2)).await, 0);
}

/// Bridge-like host: its composition never had `#main` clips. Lyric pushes
/// must never refetch it, and the ticks only probe.
#[tokio::test]
async fn a_host_without_lyric_clips_never_refetches_on_lyric_pushes() {
    let server = MockServer::start().await;
    let bible_only = vec![clip(300, "#bible-a", 30), clip(301, "#bible-b", 31)];
    DeckArena::start(&server, vec![(1, bible_only)], true).await;
    let (mut driver, status) = driver_for(&server);
    driver.tick(&status).await; // cold fetch

    for n in 0..10 {
        driver
            .dispatch_push(stage(&format!("Line {n}")), &status)
            .await;
        driver.tick(&status).await;
    }

    assert_eq!(count(&server, "GET", COMPOSITION).await, 1);
    assert_eq!(count(&server, "GET", PRODUCT).await, 10);
    let puts = server
        .received_requests()
        .await
        .expect("recording on")
        .iter()
        .filter(|req| req.method.as_str() == "PUT")
        .count();
    assert_eq!(puts, 0, "no lyric clip, nothing written");
    assert_eq!(
        status.read().await.state,
        ResolumeConnectionState::Connected
    );
}

/// SNV-like host: no `#translate` clips. Many ticks and translated lines must
/// never fetch the composition again (#808: no periodic 16 MB fetch).
#[tokio::test]
async fn a_host_without_translation_clips_never_refetches_over_many_ticks() {
    let server = MockServer::start().await;
    DeckArena::start(&server, vec![(1, lyric_deck(100, 1))], true).await;
    let (mut driver, status) = driver_for(&server);
    driver.tick(&status).await; // cold fetch

    for _ in 0..20 {
        driver.tick(&status).await;
    }
    for n in 0..5 {
        let line = format!("Line {n}");
        driver
            .dispatch_push(Push::Stage(lyric_line(&line, Some("Preklad"))), &status)
            .await;
    }

    assert_eq!(count(&server, "GET", COMPOSITION).await, 1);
    assert_eq!(count(&server, "GET", PRODUCT).await, 20);
    assert_eq!(
        count(&server, "PUT", &param(1)).await + count(&server, "PUT", &param(2)).await,
        5,
        "every line landed on the lyric lanes"
    );
}

/// RED before the fix: the second line went to deck 1's lane-B clip, which is
/// no longer on the wall. After the operator switches decks between two
/// pushes, the second push must land on the NEW deck's clip ids.
#[tokio::test]
async fn a_deck_switch_between_two_pushes_lands_the_second_push_on_the_new_deck() {
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
    driver.dispatch_push(stage("Line 2"), &status).await; // lane B, now deck 2

    assert_eq!(count(&server, "PUT", &param(1)).await, 1);
    assert_eq!(
        count(&server, "PUT", &param(22)).await,
        1,
        "deck 2's #main-b text"
    );
    assert_eq!(count(&server, "POST", &connect(201)).await, 1);
    assert_eq!(
        count(&server, "PUT", &param(2)).await,
        0,
        "nothing written to the deck that left the wall"
    );
    assert_eq!(count(&server, "POST", &connect(101)).await, 0);
    assert_eq!(count(&server, "GET", COMPOSITION).await, 2);
    assert_eq!(
        status.read().await.state,
        ResolumeConnectionState::Connected
    );
}

/// RED before the fix: the old deck's ids were used. Another composition was
/// loaded, so the cached deck id is gone (404): that is a deck change too.
#[tokio::test]
async fn a_deck_that_no_longer_exists_refetches_before_the_push() {
    let server = MockServer::start().await;
    let arena = DeckArena::start(&server, vec![(1, lyric_deck(100, 1))], true).await;
    let (mut driver, status) = driver_for(&server);
    driver.tick(&status).await; // cold fetch, deck 1

    arena.replace_decks(vec![(7, lyric_deck(700, 70))]);
    driver.dispatch_push(stage("Line 1"), &status).await;

    assert_eq!(count(&server, "GET", &deck_path(1)).await, 1);
    assert_eq!(count(&server, "PUT", &param(70)).await, 1);
    assert_eq!(count(&server, "POST", &connect(700)).await, 1);
    assert_eq!(count(&server, "PUT", &param(1)).await, 0);
    assert_eq!(count(&server, "GET", COMPOSITION).await, 2);
}

/// A deck check that fails (Arena answers 500) must never drop the push: it
/// goes out on the cached mapping, without a composition fetch.
#[tokio::test]
async fn a_failing_deck_check_still_pushes_on_the_cached_mapping() {
    let server = MockServer::start().await;
    let arena = DeckArena::start(&server, vec![(1, lyric_deck(100, 1))], true).await;
    let (mut driver, status) = driver_for(&server);
    driver.tick(&status).await; // cold fetch
    arena.fail_deck_checks(500);

    driver.dispatch_push(stage("Line 1"), &status).await;
    driver.tick(&status).await;

    assert_eq!(count(&server, "PUT", &param(1)).await, 1);
    assert_eq!(count(&server, "POST", &connect(100)).await, 1);
    assert_eq!(count(&server, "GET", COMPOSITION).await, 1);
    let snap = status.read().await.clone();
    assert_eq!(snap.state, ResolumeConnectionState::Connected);
    assert_eq!(snap.consecutive_failures, 0);
}

/// RED before the fix (every line went to deck 1). Deck 2 has no `#main`
/// clips: switching to it costs exactly one refetch (the deck change), the
/// repeated pushes to it refetch nothing more, and nothing is written to
/// deck 1's clips meanwhile. Switching back is a deck change again, never
/// rate-limited.
#[tokio::test]
async fn repeated_pushes_to_a_deck_without_main_clips_refetch_at_most_once_per_deck() {
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
    driver.dispatch_push(stage("Line 1"), &status).await; // lane A of deck 1

    arena.select(1);
    for n in 2..7 {
        driver
            .dispatch_push(stage(&format!("Line {n}")), &status)
            .await;
    }
    assert_eq!(
        count(&server, "GET", COMPOSITION).await,
        2,
        "the cold fetch + the one refetch for the deck change"
    );
    assert_eq!(count(&server, "PUT", &param(1)).await, 1);
    assert_eq!(
        count(&server, "PUT", &param(2)).await,
        0,
        "nothing written to deck 1 while deck 2 is on the wall"
    );

    arena.select(0);
    driver.dispatch_push(stage("Line 7"), &status).await;
    assert_eq!(count(&server, "GET", COMPOSITION).await, 3);
    assert_eq!(
        count(&server, "PUT", &param(2)).await,
        1,
        "back on deck 1, the line lands on its lane B"
    );
}

/// RED before the fix: the tick only probed `/product`. Without any push, the
/// 10 s tick notices the switch (deck check) and fetches the new deck's
/// mapping.
#[tokio::test]
async fn the_liveness_tick_follows_a_deck_switch_without_any_push() {
    let server = MockServer::start().await;
    let arena = DeckArena::start(
        &server,
        vec![(1, lyric_deck(100, 1)), (2, lyric_deck(200, 21))],
        true,
    )
    .await;
    let (mut driver, status) = driver_for(&server);
    driver.tick(&status).await; // cold fetch, deck 1
    driver.tick(&status).await; // probe + deck check: deck 1 still selected
    assert_eq!(count(&server, "GET", COMPOSITION).await, 1);

    arena.select(1);
    driver.tick(&status).await;

    assert_eq!(count(&server, "GET", COMPOSITION).await, 2);
    let mapping = driver.mapping.as_ref().expect("mapping");
    assert_eq!(mapping.main_a[0].clip_id, 200, "deck 2's #main-a clip");
    assert_eq!(
        status.read().await.state,
        ResolumeConnectionState::Connected
    );
}

/// Timer frames tick every second: they never pay for a deck check.
#[tokio::test]
async fn timer_frames_never_check_the_deck() {
    let server = MockServer::start().await;
    DeckArena::start(&server, vec![(1, vec![clip(600, "#timer", 60)])], true).await;
    let (mut driver, status) = driver_for(&server);
    driver.tick(&status).await; // cold fetch

    for second in 0..5 {
        let frame = TimerFrame::new(format!("00:0{second}"));
        driver.dispatch_push(Push::Timer(frame), &status).await;
    }

    assert_eq!(count(&server, "PUT", &param(60)).await, 5);
    assert_eq!(count(&server, "GET", &deck_path(1)).await, 0);
}
