//! #808 regression: the host worker must not pull Arena's whole composition
//! on a timer. On SNV that response is 16.3 MB; every fetch held win-resolume's
//! CPU 0 in an NDIS DPC for 4–11 ms, so the LED wall, SongPlayer's VBAN/NDI
//! output and cg OBS all stalled every 10 s.
//!
//! The 10 s tick is a cheap liveness probe (`GET /api/v1/product`). The full
//! `GET /api/v1/composition` happens only when the mapping is needed: no
//! mapping yet, a push answered 404 for a mapped id, the host came back from an
//! outage (the #563b threshold invalidated the mapping), or an operator refresh.
//!
//! Every test runs against a local wiremock "Arena" and counts the requests it
//! received. The driver-level tests skip the backoff window by clearing
//! `next_retry_at` (the same state change the elapsed window produces), so they
//! never sleep. The endpoint test goes through the real router and host worker.

use super::driver::HostDriver;
use super::mapping_refresh::Push;
use super::{ResolumeConnectionSnapshot, ResolumeConnectionState, StageUpdate, CONNECT_TIMEOUT};
use axum::body::Body;
use axum::http::{Method, Request as HttpRequest, StatusCode};
use chrono::Utc;
use presenter_core::{ResolumeHost, ResolumeHostId};
use reqwest::Client;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::RwLock;
use tower::ServiceExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

type Status = Arc<RwLock<ResolumeConnectionSnapshot>>;

const COMPOSITION: &str = "/api/v1/composition";
const PRODUCT: &str = "/api/v1/product";

fn clip(id: i64, name: &str, param_id: i64) -> Value {
    json!({
        "id": id,
        "name": { "value": name },
        "video": { "sourceparams": { "text": { "valuetype": "ParamText", "id": param_id } } },
    })
}

fn composition(clips: Vec<Value>) -> Value {
    json!({ "layers": [ { "clips": clips } ] })
}

fn param(id: i64) -> String {
    format!("/api/v1/parameter/by-id/{id}")
}

fn connect(id: i64) -> String {
    format!("/api/v1/composition/clips/by-id/{id}/connect")
}

/// A mock Arena: `/composition` serves a swappable body, `/product` the real
/// `ProductInfo` shape, and both answer 503 while the Arena is "offline".
/// Any route that is not mounted answers wiremock's default 404 — exactly what
/// Resolume returns for a parameter or clip id that no longer exists.
#[derive(Clone)]
struct MockArena {
    online: Arc<AtomicBool>,
    composition: Arc<Mutex<Value>>,
}

impl MockArena {
    async fn start(server: &MockServer, initial: Value) -> Self {
        let arena = Self {
            online: Arc::new(AtomicBool::new(true)),
            composition: Arc::new(Mutex::new(initial)),
        };
        for (route, product) in [(COMPOSITION, false), (PRODUCT, true)] {
            Mock::given(method("GET"))
                .and(path(route))
                .respond_with(ArenaRoute {
                    arena: arena.clone(),
                    product,
                })
                .mount(server)
                .await;
        }
        arena
    }

    fn set_online(&self, online: bool) {
        self.online.store(online, Ordering::SeqCst);
    }

    /// The operator loads a different composition in Arena.
    fn swap_composition(&self, next: Value) {
        *self.composition.lock().expect("composition lock") = next;
    }
}

struct ArenaRoute {
    arena: MockArena,
    product: bool,
}

impl Respond for ArenaRoute {
    fn respond(&self, _request: &Request) -> ResponseTemplate {
        if !self.arena.online.load(Ordering::SeqCst) {
            return ResponseTemplate::new(503);
        }
        if self.product {
            return ResponseTemplate::new(200).set_body_json(json!({
                "name": "Arena", "major": 7, "minor": 13, "micro": 2, "revision": 0,
            }));
        }
        let body = self
            .arena
            .composition
            .lock()
            .expect("composition lock")
            .clone();
        ResponseTemplate::new(200).set_body_json(body)
    }
}

/// 200 for these text-parameter PUTs and clip connects.
async fn mount_ok(server: &MockServer, params: &[i64], clips: &[i64]) {
    for id in params {
        Mock::given(method("PUT"))
            .and(path(param(*id)))
            .respond_with(ResponseTemplate::new(200))
            .mount(server)
            .await;
    }
    for id in clips {
        Mock::given(method("POST"))
            .and(path(connect(*id)))
            .respond_with(ResponseTemplate::new(200))
            .mount(server)
            .await;
    }
}

fn driver_for(server: &MockServer, enabled: bool) -> (HostDriver, Status) {
    let addr = server.address();
    let now = Utc::now();
    let config = ResolumeHost::new(
        ResolumeHostId::new(),
        "Mock Arena".into(),
        addr.ip().to_string(),
        addr.port(),
        enabled,
        now,
        now,
    );
    let client = Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .build()
        .expect("client");
    (
        HostDriver::new(client, config),
        Arc::new(RwLock::new(ResolumeConnectionSnapshot::disabled())),
    )
}

async fn count(server: &MockServer, verb: &str, route: &str) -> usize {
    server
        .received_requests()
        .await
        .expect("request recording is on")
        .iter()
        .filter(|req| req.method.as_str() == verb && req.url.path() == route)
        .count()
}

fn stage(main: &str, song: Option<&str>) -> Push {
    Push::Stage(StageUpdate {
        current_main: Some(main.to_string()),
        current_translation: None,
        song_name: song.map(str::to_string),
        band_name: None,
        enqueued_at: None,
        correlation_id: None,
    })
}

/// Let the next tick or push run as if the #484 backoff window had elapsed.
fn backoff_elapsed(driver: &mut HostDriver) {
    driver.next_retry_at = None;
}

/// POST `uri` (with an optional JSON body) through the real router. Returns
/// the status and the JSON reply (`Null` when the body is not JSON).
async fn post(app: &axum::Router, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let builder = HttpRequest::builder().method(Method::POST).uri(uri);
    let request = match body {
        Some(json) => builder
            .header("content-type", "application/json")
            .body(Body::from(json.to_string())),
        None => builder.body(Body::empty()),
    }
    .expect("request");
    let response = app.clone().oneshot(request).await.expect("router response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// Wait (bounded) until the mock has seen at least `n` composition GETs.
async fn wait_for_composition_gets(server: &MockServer, n: usize) {
    for _ in 0..100 {
        if count(server, "GET", COMPOSITION).await >= n {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the host worker never fetched the composition");
}

/// RED before #808: every tick re-fetched the whole composition (6 GETs here),
/// and nothing ever asked for `/product`.
#[tokio::test]
async fn ticks_probe_product_and_never_refetch_a_cached_composition() {
    let server = MockServer::start().await;
    MockArena::start(
        &server,
        composition(vec![clip(100, "#main-a", 1), clip(101, "#main-b", 2)]),
    )
    .await;
    mount_ok(&server, &[1, 2], &[100, 101]).await;
    let (mut driver, status) = driver_for(&server, true);

    driver.tick(&status).await; // cold start: no mapping yet → one fetch
    driver.dispatch_push(stage("Line 1", None), &status).await;
    driver.dispatch_push(stage("Line 2", None), &status).await;
    for _ in 0..5 {
        driver.tick(&status).await;
    }

    assert_eq!(
        count(&server, "GET", COMPOSITION).await,
        1,
        "only the cold start may fetch the composition while pushes keep working"
    );
    assert_eq!(
        count(&server, "GET", PRODUCT).await,
        5,
        "each tick with a cached mapping is a /product liveness probe"
    );
    assert_eq!(count(&server, "PUT", &param(1)).await, 1);
    assert_eq!(count(&server, "PUT", &param(2)).await, 1);
    assert_eq!(
        status.read().await.state,
        ResolumeConnectionState::Connected
    );
}

/// RED before #808: the 404 for the stale id was a plain host error — no
/// refetch, no retry, the line never reached the wall, the host went to Error.
///
/// The operator loads a different composition between two lines. Line 2 goes
/// to lane B, whose old param id (2) no longer exists. The driver must refetch
/// exactly once, retry the push on the new ids, and re-send the unchanged song
/// name too (its param id changed, so the dedup of the old id no longer holds).
#[tokio::test]
async fn a_push_answered_404_refetches_once_and_retries_on_the_new_ids() {
    let server = MockServer::start().await;
    let before = composition(vec![
        clip(100, "#main-a", 1),
        clip(101, "#main-b", 2),
        clip(105, "#song-name", 5),
    ]);
    let after = composition(vec![
        clip(110, "#main-a", 11),
        clip(111, "#main-b", 12),
        clip(115, "#song-name", 15),
    ]);
    let arena = MockArena::start(&server, before).await;
    // Param 2 is never mounted: once the composition is swapped it is a stale
    // id and answers 404.
    mount_ok(&server, &[1, 5, 12, 15], &[100, 111]).await;
    let (mut driver, status) = driver_for(&server, true);

    driver
        .dispatch_push(stage("Line 1", Some("Song A")), &status)
        .await;
    arena.swap_composition(after);
    driver
        .dispatch_push(stage("Line 2", Some("Song A")), &status)
        .await;

    assert_eq!(
        count(&server, "GET", COMPOSITION).await,
        2,
        "the cold fetch plus exactly one refetch for the stale id"
    );
    assert_eq!(
        count(&server, "PUT", &param(2)).await,
        1,
        "stale id tried once"
    );
    assert_eq!(
        count(&server, "PUT", &param(12)).await,
        1,
        "the retry writes the new lane-B id"
    );
    assert_eq!(
        count(&server, "POST", &connect(111)).await,
        1,
        "the retry triggers the new lane-B clip"
    );
    assert_eq!(
        count(&server, "PUT", &param(15)).await,
        1,
        "the retry re-sends the song name to the new composition's clip"
    );
    let snap = status.read().await.clone();
    assert_eq!(snap.state, ResolumeConnectionState::Connected);
    assert_eq!(
        snap.consecutive_failures, 0,
        "a stale id the retry fixed is not a host failure"
    );
}

/// RED before #808: every tick fetched the composition — during the outage
/// (3 failing GETs) and after it (4 more), 8 in total.
///
/// Three consecutive failures cross the #563b threshold, which invalidates the
/// mapping. The first tick after the host is back refetches it once (Arena may
/// have restarted or reloaded); after that the ticks only probe again.
#[tokio::test]
async fn a_host_back_from_an_outage_refetches_once_then_only_probes() {
    let server = MockServer::start().await;
    let arena = MockArena::start(&server, composition(vec![clip(100, "#main-a", 1)])).await;
    let (mut driver, status) = driver_for(&server, true);
    driver.tick(&status).await; // cold fetch

    arena.set_online(false);
    for _ in 0..3 {
        backoff_elapsed(&mut driver);
        driver.tick(&status).await;
    }
    assert_eq!(status.read().await.state, ResolumeConnectionState::Error);
    assert!(
        driver.mapping.is_none(),
        "three consecutive failures invalidate the mapping (#563b threshold)"
    );

    arena.set_online(true);
    backoff_elapsed(&mut driver);
    driver.tick(&status).await; // recovery → one refetch
    for _ in 0..3 {
        driver.tick(&status).await;
    }

    assert_eq!(
        count(&server, "GET", COMPOSITION).await,
        2,
        "cold start + one refetch after the recovery — never during the outage"
    );
    assert_eq!(
        count(&server, "GET", PRODUCT).await,
        6,
        "3 failing probes during the outage + 3 after the recovery"
    );
    let snap = status.read().await.clone();
    assert_eq!(snap.state, ResolumeConnectionState::Connected);
    assert_eq!(snap.consecutive_failures, 0);
}

/// RED before #808: the blip tick and the recovery tick each fetched the
/// composition (3 GETs). A one-tick blip stays below the #563b threshold, so
/// the cached mapping is still good and must not be refetched.
#[tokio::test]
async fn a_single_probe_blip_keeps_the_mapping_and_does_not_refetch() {
    let server = MockServer::start().await;
    let arena = MockArena::start(&server, composition(vec![clip(100, "#main-a", 1)])).await;
    let (mut driver, status) = driver_for(&server, true);
    driver.tick(&status).await; // cold fetch

    arena.set_online(false);
    backoff_elapsed(&mut driver);
    driver.tick(&status).await;
    assert_eq!(status.read().await.state, ResolumeConnectionState::Error);
    assert!(driver.mapping.is_some(), "one failure keeps the mapping");

    arena.set_online(true);
    backoff_elapsed(&mut driver);
    driver.tick(&status).await;

    assert_eq!(count(&server, "GET", COMPOSITION).await, 1);
    assert_eq!(count(&server, "GET", PRODUCT).await, 2);
    let snap = status.read().await.clone();
    assert_eq!(snap.state, ResolumeConnectionState::Connected);
    assert_eq!(snap.consecutive_failures, 0);
}

/// RED before #808: the second tick fetched the composition again and stayed
/// Connected. A failing liveness probe must take the same path a failing
/// composition fetch took: Error, a failure count, the #484 backoff window,
/// and no request at all while that window is open.
#[tokio::test]
async fn a_failing_product_probe_puts_the_host_in_error_and_backoff() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(COMPOSITION))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(composition(vec![clip(100, "#main-a", 1)])),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(PRODUCT))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let (mut driver, status) = driver_for(&server, true);

    driver.tick(&status).await; // cold fetch succeeds
    assert_eq!(
        status.read().await.state,
        ResolumeConnectionState::Connected
    );

    driver.tick(&status).await; // the probe fails
    let snap = status.read().await.clone();
    assert_eq!(snap.state, ResolumeConnectionState::Error);
    assert_eq!(snap.consecutive_failures, 1);
    let last_error = snap.last_error.unwrap_or_default();
    assert!(
        last_error.contains("/product"),
        "the error names the failed probe: {last_error}"
    );
    assert!(
        driver.in_backoff(),
        "a failed probe opens the backoff window"
    );
    assert!(driver.mapping.is_some(), "one failure keeps the mapping");

    driver.tick(&status).await; // inside the backoff window: nothing is sent
    assert_eq!(count(&server, "GET", PRODUCT).await, 1);
    assert_eq!(count(&server, "GET", COMPOSITION).await, 1);
}

/// Guards the stale path itself (before #808 there was no stale refetch, so
/// this fails there only on the refetch count): when even a FRESHLY fetched
/// mapping still gets a 404, further 404s must not refetch the composition —
/// otherwise every push of a broken id becomes a 16 MB fetch. They count as
/// ordinary failures instead (#563b threshold, #484 backoff).
#[tokio::test]
async fn an_id_that_404s_on_a_fresh_mapping_pauses_stale_refetches() {
    let server = MockServer::start().await;
    MockArena::start(&server, composition(vec![clip(100, "#main-a", 1)])).await;
    // Param 1 is never mounted: it answers 404 even on a fresh mapping.
    mount_ok(&server, &[], &[100]).await;
    let (mut driver, status) = driver_for(&server, true);

    driver.dispatch_push(stage("Line 1", None), &status).await;
    backoff_elapsed(&mut driver);
    driver.dispatch_push(stage("Line 2", None), &status).await;

    assert_eq!(
        count(&server, "GET", COMPOSITION).await,
        2,
        "the cold fetch + one stale refetch; the second push must not refetch"
    );
    assert_eq!(count(&server, "PUT", &param(1)).await, 3);
    let snap = status.read().await.clone();
    assert_eq!(snap.state, ResolumeConnectionState::Error);
    assert_eq!(snap.consecutive_failures, 2);
}

/// RED before #808: the tick on a disabled host returned `Ok` from
/// `refresh_mapping` and called `mark_connected`, so the host showed Connected.
/// A disabled host's tick must send nothing and leave the status alone.
#[tokio::test]
async fn a_disabled_host_tick_sends_nothing_and_stays_disabled() {
    let server = MockServer::start().await;
    MockArena::start(&server, composition(vec![clip(100, "#main-a", 1)])).await;
    let (mut driver, status) = driver_for(&server, false);
    driver.refresh_status(&status).await;

    driver.tick(&status).await;
    driver.tick(&status).await;

    let received = server.received_requests().await.expect("recording on");
    assert!(received.is_empty(), "a disabled host must not be contacted");
    assert_eq!(status.read().await.state, ResolumeConnectionState::Disabled);
}

/// RED before #808: the endpoint did not exist (404). The settings page's
/// "Refresh mapping" button posts here — the way to pick up a composition edit
/// now that no timer re-reads the composition. Every call must fetch it exactly
/// once and report the clips the new mapping misses; an unknown host is a 404.
#[tokio::test]
async fn the_refresh_mapping_endpoint_refetches_the_composition_on_each_call() {
    let server = MockServer::start().await;
    MockArena::start(&server, composition(vec![clip(100, "#main-a", 1)])).await;
    let state = crate::state::AppState::in_memory().await.expect("state");
    let app = crate::router::build_router(state);
    let addr = server.address();
    let (created_status, created) = post(
        &app,
        "/integrations/resolume/hosts",
        Some(json!({
            "label": "Mock Arena",
            "host": addr.ip().to_string(),
            "port": addr.port(),
            "isEnabled": true,
        })),
    )
    .await;
    assert_eq!(created_status, StatusCode::OK, "{created}");
    let id = created["id"].as_str().expect("host id").to_string();
    wait_for_composition_gets(&server, 1).await; // the worker's cold start
    let refresh_uri = format!("/integrations/resolume/hosts/{id}/refresh-mapping");

    for expected_gets in [2, 3] {
        let (status, reply) = post(&app, &refresh_uri, None).await;
        assert_eq!(status, StatusCode::OK, "{reply}");
        assert_eq!(reply["success"], json!(true), "{reply}");
        let missing = reply["missingClips"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert!(
            missing.contains(&json!("#main-b")),
            "the reply lists the clips the mapping misses: {reply}"
        );
        assert_eq!(
            count(&server, "GET", COMPOSITION).await,
            expected_gets,
            "each refresh is exactly one composition fetch"
        );
    }

    let unknown = format!(
        "/integrations/resolume/hosts/{}/refresh-mapping",
        uuid::Uuid::new_v4()
    );
    let (status, _) = post(&app, &unknown, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
