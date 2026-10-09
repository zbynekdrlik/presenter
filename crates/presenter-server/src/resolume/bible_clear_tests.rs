//! #807 regression: on a Bible CLEAR, the `#bible-clear` clip must win its
//! Resolume layer, every time.
//!
//! SNV's composition puts `#bible-clear` in the SAME layer as
//! `#bible-reference-a/b` (layer 29 "L# SK miesto"). The clear path used to POST
//! every `/connect` — the blanked lane clips AND `#bible-clear` — concurrently in
//! one batch, so the reference-lane connect and the clear connect raced inside
//! that one layer and whichever Resolume processed last stayed live: the clear
//! clip showed only ~50% of the time. The fixed contract:
//!
//! - a lane clip whose layer also holds a `#bible-clear` clip is NOT triggered
//!   (its text is still blanked; the clear clip replaces that layer anyway);
//! - the remaining lane clips are triggered first, and `#bible-clear` only after
//!   every one of those connects has COMPLETED (a strictly sequential phase 2).
//!
//! The mock Resolume records the arrival instant of every `/connect` and delays
//! each response by [`RESPONSE_DELAY`]. A connect sent only after the lane
//! connects completed therefore arrives at least `RESPONSE_DELAY` after the last
//! lane connect; a connect dispatched in the same concurrent batch arrives
//! within a few milliseconds of it. Load can only widen the sequential gap, so
//! the assertion cannot false-fail a correct implementation.
//!
//! Self-contained helpers (the `latency_tests.rs` pattern) so the oversized
//! `tests.rs` does not grow.

use super::bible_clear::{clip_ids, plan_bible_clear_triggers, BibleClearTriggers};
use super::clip_map::ClipMapping;
use super::driver::HostDriver;
use super::types::ClipTarget;
use super::{BibleUpdate, ResolumeConnectionSnapshot, CONNECT_TIMEOUT};
use chrono::Utc;
use presenter_core::{BibleSlideOutput, ResolumeHost, ResolumeHostId};
use reqwest::Client;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

/// Delay applied to every `/connect` response by the mock Resolume.
const RESPONSE_DELAY: Duration = Duration::from_millis(200);

const CLEAR: i64 = 500;
const BIBLE_A: i64 = 300;
const BIBLE_B: i64 = 301;
const REF_A: i64 = 350;
const REF_B: i64 = 351;
const TRANS_A: i64 = 400;
const TRANS_B: i64 = 401;
const TRANS_REF_A: i64 = 450;
const TRANS_REF_B: i64 = 451;

type ConnectLog = Arc<Mutex<Vec<(i64, Instant)>>>;

/// Records `(clip_id, arrival)` for every `/connect` POST, then answers 200
/// after [`RESPONSE_DELAY`].
#[derive(Clone)]
struct ConnectRecorder {
    log: ConnectLog,
}

impl wiremock::Respond for ConnectRecorder {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let clip_id = req
            .url
            .path()
            .trim_start_matches("/api/v1/composition/clips/by-id/")
            .trim_end_matches("/connect")
            .parse::<i64>()
            .expect("numeric clip id in /connect path");
        self.log
            .lock()
            .expect("connect log lock")
            .push((clip_id, Instant::now()));
        ResponseTemplate::new(200).set_delay(RESPONSE_DELAY)
    }
}

fn clip(id: i64, name: &str, param_id: Option<i64>) -> serde_json::Value {
    let sourceparams = match param_id {
        Some(value) => serde_json::json!({ "text": { "valuetype": "ParamText", "id": value } }),
        None => serde_json::json!({}),
    };
    serde_json::json!({
        "id": id,
        "name": { "value": name },
        "video": { "sourceparams": sourceparams },
    })
}

fn layer(clips: Vec<serde_json::Value>) -> serde_json::Value {
    serde_json::json!({ "clips": clips })
}

fn reference_clips() -> Vec<serde_json::Value> {
    vec![
        clip(REF_A, "#bible-reference-a", Some(35)),
        clip(REF_B, "#bible-reference-b", Some(36)),
    ]
}

fn bible_layer() -> serde_json::Value {
    layer(vec![
        clip(BIBLE_A, "#bible-a", Some(30)),
        clip(BIBLE_B, "#bible-b", Some(31)),
    ])
}

fn translation_layer() -> serde_json::Value {
    layer(vec![
        clip(TRANS_A, "#bible-translate-a", Some(40)),
        clip(TRANS_B, "#bible-translate-b", Some(41)),
    ])
}

fn translate_reference_layer() -> serde_json::Value {
    layer(vec![
        clip(TRANS_REF_A, "#bible-translate-reference-a", Some(45)),
        clip(TRANS_REF_B, "#bible-translate-reference-b", Some(46)),
    ])
}

fn clear_clip() -> serde_json::Value {
    clip(CLEAR, "#bible-clear", None)
}

/// Start a mock Resolume serving `layers`, accepting every text-param PUT and
/// recording every `/connect` POST.
async fn start_resolume(layers: Vec<serde_json::Value>) -> (MockServer, ConnectLog) {
    let server = MockServer::start().await;
    let composition = serde_json::json!({ "layers": layers });
    Mock::given(method("GET"))
        .and(path("/api/v1/composition"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&composition))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path_regex(r"^/api/v1/parameter/by-id/\d+$"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let log: ConnectLog = Arc::new(Mutex::new(Vec::new()));
    Mock::given(method("POST"))
        .and(path_regex(r"^/api/v1/composition/clips/by-id/\d+/connect$"))
        .respond_with(ConnectRecorder {
            log: Arc::clone(&log),
        })
        .mount(&server)
        .await;
    (server, log)
}

fn driver_for(server: &MockServer) -> (HostDriver, Arc<RwLock<ResolumeConnectionSnapshot>>) {
    let addr = server.address();
    let now = Utc::now();
    let config = ResolumeHost::new(
        ResolumeHostId::new(),
        "Mock".into(),
        addr.ip().to_string(),
        addr.port(),
        true,
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

/// A Bible update with no slide output and no passage = the CLEAR path.
fn clear_update() -> BibleUpdate {
    BibleUpdate {
        passage: None,
        secondary_text: None,
        secondary_translation_code: None,
        secondary_book: None,
        slide_output: None,
    }
}

/// Run one Bible clear against a fresh driver (lanes start on A) and return the
/// mock server plus the recorded `/connect` log in arrival order.
async fn clear_against(layers: Vec<serde_json::Value>) -> (MockServer, Vec<(i64, Instant)>) {
    let (server, log) = start_resolume(layers).await;
    let (mut driver, status) = driver_for(&server);
    driver
        .handle_bible(clear_update(), &status)
        .await
        .expect("bible clear");
    let connects = log.lock().expect("connect log lock").clone();
    (server, connects)
}

fn connected_ids(connects: &[(i64, Instant)]) -> Vec<i64> {
    let mut ids: Vec<i64> = connects.iter().map(|(id, _)| *id).collect();
    ids.sort_unstable();
    ids
}

/// `#bible-clear` was connected exactly once, and only after every connect in
/// `lane_ids` had COMPLETED (arrived ≥ one response delay after the last one).
fn assert_clear_after_lanes(connects: &[(i64, Instant)], lane_ids: &[i64]) {
    let clear_at: Vec<Instant> = connects
        .iter()
        .filter(|(id, _)| *id == CLEAR)
        .map(|(_, at)| *at)
        .collect();
    assert_eq!(clear_at.len(), 1, "#bible-clear connected exactly once");
    let last_lane_at = connects
        .iter()
        .filter(|(id, _)| lane_ids.contains(id))
        .map(|(_, at)| *at)
        .max()
        .expect("lane connects recorded");
    let gap = clear_at[0].saturating_duration_since(last_lane_at);
    assert!(
        gap >= RESPONSE_DELAY,
        "#bible-clear must be sent only after every lane connect completed: it arrived \
         {gap:?} after the last lane connect, a sequential phase 2 needs >= {RESPONSE_DELAY:?}"
    );
}

/// The SNV topology: `#bible-clear` shares its layer with `#bible-reference-a/b`.
/// The reference-lane clip must NOT be connected (it would race the clear clip
/// inside that layer), its text must still be blanked, and `#bible-clear` must
/// fire after the other lane connects completed.
#[tokio::test]
async fn clear_skips_same_layer_lane_clip_and_fires_clear_after_lane_connects() {
    let mut miesto = reference_clips();
    miesto.push(clear_clip());
    let (server, connects) = clear_against(vec![
        layer(miesto),
        bible_layer(),
        translation_layer(),
        translate_reference_layer(),
    ])
    .await;

    assert_eq!(
        connected_ids(&connects),
        vec![BIBLE_A, TRANS_A, TRANS_REF_A, CLEAR],
        "the reference-lane clip sharing #bible-clear's layer must not be connected"
    );
    assert_clear_after_lanes(&connects, &[BIBLE_A, TRANS_A, TRANS_REF_A]);

    let requests = server.received_requests().await.expect("requests");
    let reference_blanked = requests.iter().any(|req| {
        req.method.as_str() == "PUT"
            && req.url.path() == "/api/v1/parameter/by-id/35"
            && serde_json::from_slice::<serde_json::Value>(&req.body)
                .ok()
                .and_then(|body| body.get("value").cloned())
                == Some(serde_json::Value::String(String::new()))
    });
    assert!(
        reference_blanked,
        "the skipped reference clip's text must still be blanked"
    );
}

/// `#bible-clear` in a layer of its own: every blanked lane clip is connected,
/// and `#bible-clear` still fires strictly after all of them completed.
#[tokio::test]
async fn clear_in_own_layer_connects_all_lanes_then_clear_last() {
    let (_server, connects) = clear_against(vec![
        layer(reference_clips()),
        bible_layer(),
        translation_layer(),
        translate_reference_layer(),
        layer(vec![clear_clip()]),
    ])
    .await;

    assert_eq!(
        connected_ids(&connects),
        vec![BIBLE_A, REF_A, TRANS_A, TRANS_REF_A, CLEAR],
        "every blanked lane clip is connected when no lane shares #bible-clear's layer"
    );
    assert_clear_after_lanes(&connects, &[BIBLE_A, REF_A, TRANS_A, TRANS_REF_A]);
}

/// A failed lane-clip trigger must not swallow the clear clip. Phase 1 never
/// holds a clip in a clear layer, so `#bible-clear` still fires, and the push
/// still reports the lane failure (so the host's error/backoff handling runs).
#[tokio::test]
async fn clear_clip_still_fires_when_a_lane_clip_trigger_fails() {
    let (server, log) = start_resolume(vec![
        layer(reference_clips()),
        bible_layer(),
        translation_layer(),
        translate_reference_layer(),
        layer(vec![clear_clip()]),
    ])
    .await;
    Mock::given(method("POST"))
        .and(path(
            format!("/api/v1/composition/clips/by-id/{BIBLE_A}/connect").as_str(),
        ))
        .respond_with(ResponseTemplate::new(500))
        .with_priority(1)
        .mount(&server)
        .await;
    let (mut driver, status) = driver_for(&server);

    let result = driver.handle_bible(clear_update(), &status).await;

    assert!(result.is_err(), "the failed lane trigger is still reported");
    let connects = log.lock().expect("connect log lock").clone();
    assert!(
        connects.iter().any(|(id, _)| *id == CLEAR),
        "#bible-clear must still be connected after a phase-1 trigger failure"
    );
}

// ── Unit pins for the layer bookkeeping + the pure planner ─────────────

fn target(clip_id: i64, layer_index: usize) -> ClipTarget {
    ClipTarget {
        clip_id,
        text_param_id: None,
        transforms: Vec::new(),
        layer_index,
    }
}

/// The composition parse records each clip's layer position, and a clear clip
/// never carries a text param (it is only ever triggered).
#[test]
fn composition_parse_records_each_clips_layer_index() {
    let composition = serde_json::json!({ "layers": [
        layer(reference_clips()),
        bible_layer(),
        layer(vec![clip(CLEAR, "#bible-clear", Some(99))]),
    ]});
    let mapping = ClipMapping::from_composition(&composition).expect("mapping");

    assert_eq!(mapping.bible_reference_a[0].layer_index, 0);
    assert_eq!(mapping.bible_reference_b[0].layer_index, 0);
    assert_eq!(mapping.bible_a[0].layer_index, 1);
    assert_eq!(mapping.bible_b[0].layer_index, 1);
    assert_eq!(mapping.bible_clear[0].layer_index, 2);
    assert_eq!(mapping.bible_clear[0].clip_id, CLEAR);
    assert_eq!(mapping.bible_clear[0].text_param_id, None);
}

#[test]
fn plan_skips_only_the_lane_clips_in_a_clear_layer() {
    let plan = plan_bible_clear_triggers(
        vec![
            target(BIBLE_A, 1),
            target(REF_A, 0),
            target(TRANS_A, 2),
            target(TRANS_REF_A, 3),
        ],
        &[target(CLEAR, 0), target(501, 3)],
    );

    assert_eq!(
        plan,
        BibleClearTriggers {
            lanes: vec![target(BIBLE_A, 1), target(TRANS_A, 2)],
            skipped: vec![target(REF_A, 0), target(TRANS_REF_A, 3)],
            clear: vec![target(CLEAR, 0), target(501, 3)],
        }
    );
}

#[test]
fn plan_without_a_clear_clip_triggers_every_lane_clip() {
    let blanked = vec![target(BIBLE_A, 0), target(REF_A, 0)];
    let plan = plan_bible_clear_triggers(blanked.clone(), &[]);

    assert_eq!(
        plan,
        BibleClearTriggers {
            lanes: blanked,
            skipped: Vec::new(),
            clear: Vec::new(),
        }
    );
}

#[test]
fn clip_ids_lists_the_ids_in_order() {
    assert_eq!(
        clip_ids(&[target(REF_A, 0), target(BIBLE_A, 1)]),
        vec![REF_A, BIBLE_A]
    );
    assert!(clip_ids(&[]).is_empty());
}

fn verse_update() -> BibleUpdate {
    BibleUpdate {
        passage: None,
        secondary_text: None,
        secondary_translation_code: None,
        secondary_book: None,
        slide_output: Some(BibleSlideOutput {
            main_text: "For God so loved".to_string(),
            main_reference: "John 3:16 (KJV)".to_string(),
            secondary_text: "Neboť Bůh tak miloval".to_string(),
            secondary_reference: "Jan 3:16 (CEP)".to_string(),
            triggered_at: Utc::now(),
        }),
    }
}

/// Whether a `PUT /parameter/by-id/{param_id}` carried `{"value": value}`.
fn put_sent(requests: &[Request], param_id: i64, value: &str) -> bool {
    let wanted = format!("/api/v1/parameter/by-id/{param_id}");
    requests.iter().any(|req| {
        req.method.as_str() == "PUT"
            && req.url.path() == wanted
            && serde_json::from_slice::<serde_json::Value>(&req.body)
                .ok()
                .and_then(|body| {
                    body.get("value")
                        .and_then(|v| v.as_str())
                        .map(str::to_owned)
                })
                .as_deref()
                == Some(value)
    })
}

/// The clear keeps the A/B alternation unchanged: it blanks lane A (even the
/// skipped same-layer reference clip) and flips, so the next verse lands on
/// lane B of both the bible and the bible-translation slots.
#[tokio::test]
async fn clear_flips_the_lanes_so_the_next_verse_lands_on_lane_b() {
    let mut miesto = reference_clips();
    miesto.push(clear_clip());
    let (server, _log) = start_resolume(vec![
        layer(miesto),
        bible_layer(),
        translation_layer(),
        translate_reference_layer(),
    ])
    .await;
    let (mut driver, status) = driver_for(&server);

    driver
        .handle_bible(clear_update(), &status)
        .await
        .expect("bible clear");
    driver
        .handle_bible(verse_update(), &status)
        .await
        .expect("verse");

    let requests = server.received_requests().await.expect("requests");
    assert!(
        put_sent(&requests, 31, "For God so loved"),
        "after a clear the verse goes to #bible-b"
    );
    assert!(
        put_sent(&requests, 41, "Neboť Bůh tak miloval"),
        "after a clear the secondary text goes to #bible-translate-b"
    );
}
