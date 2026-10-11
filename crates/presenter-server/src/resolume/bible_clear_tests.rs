//! #841: a Bible CLEAR fades the Bible clips' opacity and triggers no clip.
//!
//! Owner, 2026-10-11: "bible clear chcem aby fungoval tak ze netrigeruje ziaden
//! clip ale iba v tom clipe faduje opacitu". The contract pinned here:
//!
//! - a clear sends no `/connect` and writes no text;
//! - it steps every Bible lane clip's `video.opacity` parameter down to 0 over
//!   the clip layer's transition duration;
//! - it fills no lane, so the A/B lanes do not flip;
//! - the next verse restores a faded clip's opacity BEFORE connecting it;
//! - a fade that fails halfway still gets its clips restored;
//! - a clip stays tracked after its restore, so a composition fetched while
//!   it was faded (opacity 0) never makes a later clear skip it.
//!
//! The mock Arena accepts every PUT and `/connect`; assertions read
//! `received_requests()`, which keeps arrival order. Timing is only ever
//! asserted as a lower bound (a fade cannot end before its duration), so
//! machine load cannot false-fail it.

use super::bible_clear::{
    fade_level, plan_clear_fade, TrackedOpacities, TrackedOpacity, MAX_CLEAR_FADE,
};
use super::clip_map::ClipMapping;
use super::driver::HostDriver;
use super::mapping_refresh::is_stale_id_error;
use super::types::ClipOpacity;
use super::{BibleUpdate, ResolumeConnectionSnapshot, CONNECT_TIMEOUT};
use chrono::Utc;
use presenter_core::{BibleSlideOutput, ResolumeHost, ResolumeHostId};
use reqwest::Client;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

/// The layer transition of the SNV-like fixture: long enough for about 20
/// steps, so a loaded machine still sends several.
const FADE: Duration = Duration::from_millis(600);

const BIBLE_A: i64 = 300;
const REF_A: i64 = 350;
const TRANS_A: i64 = 400;
const TRANS_REF_A: i64 = 450;

/// Opacity parameter ids: the clip's text parameter id + 100.
const A_SIDE_OPACITY: [i64; 4] = [130, 135, 140, 145];
const B_SIDE_OPACITY: [i64; 4] = [131, 136, 141, 146];

fn clip(id: i64, name: &str, text_param: i64, opacity: f64) -> Value {
    json!({
        "id": id,
        "name": { "value": name },
        "video": {
            "opacity": { "id": text_param + 100, "value": opacity },
            "sourceparams": { "text": { "valuetype": "ParamText", "id": text_param } },
        },
    })
}

fn layer(clips: Vec<Value>, transition: Option<Duration>) -> Value {
    match transition {
        Some(fade) => json!({
            "transition": { "duration": { "value": fade.as_secs_f64() } },
            "clips": clips,
        }),
        None => json!({ "clips": clips }),
    }
}

/// The SNV shape: one layer per Bible lane, each with its A/B pair.
fn bible_layers(transition: Option<Duration>) -> Vec<Value> {
    vec![
        layer(
            vec![
                clip(REF_A, "#bible-reference-a", 35, 1.0),
                clip(351, "#bible-reference-b", 36, 1.0),
            ],
            transition,
        ),
        layer(
            vec![
                clip(BIBLE_A, "#bible-a", 30, 1.0),
                clip(301, "#bible-b", 31, 1.0),
            ],
            transition,
        ),
        layer(
            vec![
                clip(TRANS_A, "#bible-translate-a", 40, 1.0),
                clip(401, "#bible-translate-b", 41, 1.0),
            ],
            transition,
        ),
        layer(
            vec![
                clip(TRANS_REF_A, "#bible-translate-reference-a", 45, 1.0),
                clip(451, "#bible-translate-reference-b", 46, 1.0),
            ],
            transition,
        ),
    ]
}

/// A mock Arena serving `layers` that accepts every parameter PUT and clip
/// connect.
async fn start_resolume(layers: Vec<Value>) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/composition"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "layers": layers })))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path_regex(r"^/api/v1/parameter/by-id/\d+$"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path_regex(r"^/api/v1/composition/clips/by-id/\d+/connect$"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    server
}

/// Answer `status` to `times` PUTs of parameter `param_id` at `priority`,
/// ahead of the catch-all PUT route (priority 5).
async fn answer_param(server: &MockServer, param_id: i64, status: u16, times: u64, priority: u8) {
    Mock::given(method("PUT"))
        .and(path(format!("/api/v1/parameter/by-id/{param_id}")))
        .respond_with(ResponseTemplate::new(status))
        .up_to_n_times(times)
        .with_priority(priority)
        .mount(server)
        .await;
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
    BibleUpdate::from_slide_output(None)
}

fn verse_update() -> BibleUpdate {
    BibleUpdate::from_slide_output(Some(BibleSlideOutput {
        main_text: "For God so loved".to_string(),
        main_reference: "John 3:16 (KJV)".to_string(),
        secondary_text: "Neboť Bůh tak miloval".to_string(),
        secondary_reference: "Jan 3:16 (CEP)".to_string(),
        triggered_at: Utc::now(),
    }))
}

fn param_path(param_id: i64) -> String {
    format!("/api/v1/parameter/by-id/{param_id}")
}

fn body_value(req: &Request) -> Option<Value> {
    serde_json::from_slice::<Value>(&req.body)
        .ok()
        .and_then(|body| body.get("value").cloned())
}

/// The values PUT to parameter `param_id`, in arrival order.
fn values_put(requests: &[Request], param_id: i64) -> Vec<Value> {
    let wanted = param_path(param_id);
    requests
        .iter()
        .filter(|req| req.method.as_str() == "PUT" && req.url.path() == wanted)
        .filter_map(body_value)
        .collect()
}

/// The numeric opacities PUT to parameter `param_id`, in arrival order.
fn opacities_put(requests: &[Request], param_id: i64) -> Vec<f64> {
    values_put(requests, param_id)
        .iter()
        .filter_map(Value::as_f64)
        .collect()
}

fn connects(requests: &[Request]) -> Vec<&Request> {
    requests
        .iter()
        .filter(|req| req.method.as_str() == "POST" && req.url.path().ends_with("/connect"))
        .collect()
}

fn position(requests: &[Request], predicate: impl Fn(&Request) -> bool) -> Option<usize> {
    requests.iter().position(predicate)
}

#[tokio::test]
async fn clear_fades_every_bible_clip_to_zero_and_triggers_nothing() {
    let server = start_resolume(bible_layers(Some(FADE))).await;
    let (mut driver, status) = driver_for(&server);

    let start = Instant::now();
    driver
        .handle_bible(clear_update(), &status)
        .await
        .expect("bible clear");
    let took = start.elapsed();

    let requests = server.received_requests().await.expect("requests");
    assert!(connects(&requests).is_empty(), "a clear triggers no clip");
    for text_param in [30, 31, 35, 36, 40, 41, 45, 46] {
        assert!(
            values_put(&requests, text_param).is_empty(),
            "a clear writes no text (param {text_param})"
        );
    }
    for param in A_SIDE_OPACITY.into_iter().chain(B_SIDE_OPACITY) {
        let steps = opacities_put(&requests, param);
        assert_eq!(
            steps.last(),
            Some(&0.0),
            "param {param} ends at 0: {steps:?}"
        );
        assert!(
            steps.windows(2).all(|pair| pair[1] < pair[0]),
            "param {param} only goes down: {steps:?}"
        );
        assert!(
            steps.len() >= 3 && steps[0] > 0.5,
            "param {param} is faded in steps from near full opacity, not cut: {steps:?}"
        );
    }
    assert!(
        took >= FADE,
        "the fade spans the layer transition ({FADE:?}), took {took:?}"
    );
}

#[tokio::test]
async fn a_layer_without_a_transition_goes_to_zero_in_one_step() {
    let server = start_resolume(bible_layers(None)).await;
    let (mut driver, status) = driver_for(&server);

    driver
        .handle_bible(clear_update(), &status)
        .await
        .expect("bible clear");

    let requests = server.received_requests().await.expect("requests");
    for param in A_SIDE_OPACITY.into_iter().chain(B_SIDE_OPACITY) {
        assert_eq!(opacities_put(&requests, param), vec![0.0], "param {param}");
    }
}

#[tokio::test]
async fn a_second_clear_sends_nothing() {
    let server = start_resolume(bible_layers(None)).await;
    let (mut driver, status) = driver_for(&server);

    driver
        .handle_bible(clear_update(), &status)
        .await
        .expect("first clear");
    let after_first = server.received_requests().await.expect("requests").len();
    driver
        .handle_bible(clear_update(), &status)
        .await
        .expect("second clear");

    let requests = server.received_requests().await.expect("requests");
    assert!(
        requests[after_first..]
            .iter()
            .all(|req| req.method.as_str() == "GET"),
        "every Bible clip is already invisible, so the second clear sends no PUT"
    );
}

/// The clear fills no lane, so the verse lands on lane A again: its opacity
/// comes back before its connect, and lane B stays invisible.
#[tokio::test]
async fn the_next_verse_restores_the_opacity_before_connecting_and_lanes_do_not_flip() {
    let server = start_resolume(bible_layers(None)).await;
    let (mut driver, status) = driver_for(&server);

    driver
        .handle_bible(clear_update(), &status)
        .await
        .expect("bible clear");
    let after_clear = server.received_requests().await.expect("requests").len();
    driver
        .handle_bible(verse_update(), &status)
        .await
        .expect("verse");

    let requests = server.received_requests().await.expect("requests");
    let verse = &requests[after_clear..];
    assert_eq!(
        values_put(verse, 30),
        vec![json!("For God so loved")],
        "the verse goes to #bible-a: the clear did not flip the lane"
    );
    assert_eq!(values_put(verse, 40), vec![json!("Neboť Bůh tak miloval")]);
    for param in A_SIDE_OPACITY {
        assert_eq!(
            opacities_put(verse, param),
            vec![1.0],
            "param {param} restored"
        );
    }
    for param in B_SIDE_OPACITY {
        assert!(
            opacities_put(verse, param).is_empty(),
            "param {param} stays faded"
        );
    }
    let first_connect = position(verse, |req| req.url.path().ends_with("/connect"))
        .expect("the verse connects its clips");
    let last_restore = A_SIDE_OPACITY
        .iter()
        .filter_map(|param| position(verse, |req| req.url.path() == param_path(*param)))
        .max()
        .expect("restores sent");
    assert!(
        last_restore < first_connect,
        "every opacity is restored before the first connect"
    );
    let mut connected: Vec<i64> = connects(verse)
        .iter()
        .filter_map(|req| {
            req.url
                .path()
                .trim_start_matches("/api/v1/composition/clips/by-id/")
                .trim_end_matches("/connect")
                .parse()
                .ok()
        })
        .collect();
    connected.sort_unstable();
    assert_eq!(connected, vec![BIBLE_A, REF_A, TRANS_A, TRANS_REF_A]);
}

/// A restored clip is not restored again by the verse after next.
#[tokio::test]
async fn a_restored_clip_is_not_restored_twice() {
    let server = start_resolume(bible_layers(None)).await;
    let (mut driver, status) = driver_for(&server);

    driver
        .handle_bible(clear_update(), &status)
        .await
        .expect("bible clear");
    driver
        .handle_bible(verse_update(), &status)
        .await
        .expect("verse 1 (lane A)");
    driver
        .handle_bible(verse_update(), &status)
        .await
        .expect("verse 2 (lane B)");
    let after_two = server.received_requests().await.expect("requests").len();
    driver
        .handle_bible(verse_update(), &status)
        .await
        .expect("verse 3 (lane A)");

    let requests = server.received_requests().await.expect("requests");
    for param in A_SIDE_OPACITY {
        assert!(
            opacities_put(&requests[after_two..], param).is_empty(),
            "param {param} was restored by verse 1 already"
        );
    }
}

#[tokio::test]
async fn a_fade_that_fails_halfway_is_still_restored() {
    let server = start_resolume(bible_layers(Some(FADE))).await;
    // Two fade steps of #bible-reference-a land, the third fails.
    answer_param(&server, 135, 200, 2, 1).await;
    answer_param(&server, 135, 500, 1, 2).await;
    let (mut driver, status) = driver_for(&server);

    let err = driver
        .handle_bible(clear_update(), &status)
        .await
        .expect_err("the failed opacity PUT fails the clear");
    assert!(!is_stale_id_error(&err), "a 500 is a host failure: {err:#}");
    let requests = server.received_requests().await.expect("requests");
    assert_eq!(
        opacities_put(&requests, 135).len(),
        3,
        "the fade stopped at its third step"
    );
    let after_clear = requests.len();

    driver.next_retry_at = None;
    driver
        .handle_bible(verse_update(), &status)
        .await
        .expect("verse");

    let requests = server.received_requests().await.expect("requests");
    for param in A_SIDE_OPACITY {
        assert_eq!(
            opacities_put(&requests[after_clear..], param),
            vec![1.0],
            "param {param} is put back although the fade stopped halfway"
        );
    }
}

#[tokio::test]
async fn a_missing_opacity_parameter_is_a_stale_mapping() {
    let server = start_resolume(bible_layers(None)).await;
    answer_param(&server, 130, 404, 10, 1).await;
    let (mut driver, _status) = driver_for(&server);
    driver.ensure_mapping().await.expect("mapping");
    let mapping = driver.mapping.clone().expect("mapping cached");

    let err = driver
        .handle_bible_clear(&mapping)
        .await
        .expect_err("404 on an opacity parameter");
    assert!(is_stale_id_error(&err), "{err:#}");
}

/// Review finding (#841): a composition fetched while a clip sat at opacity 0
/// cached 0 as its value. Once a verse restored the clip, a clear read that
/// cached 0, skipped the clip, and the verse stayed on the wall.
#[tokio::test]
async fn a_clip_restored_after_a_fetch_at_zero_is_faded_by_the_next_clear() {
    let mut layers = bible_layers(None);
    // The A side as Arena reports it after a clear: invisible.
    layers[1] = layer(
        vec![
            clip(BIBLE_A, "#bible-a", 30, 0.0),
            clip(301, "#bible-b", 31, 1.0),
        ],
        None,
    );
    let server = start_resolume(layers).await;
    let (mut driver, status) = driver_for(&server);

    driver
        .handle_bible(verse_update(), &status)
        .await
        .expect("verse");
    let after_verse = server.received_requests().await.expect("requests");
    assert_eq!(
        opacities_put(&after_verse, 130),
        vec![1.0],
        "restored to full"
    );
    driver
        .handle_bible(clear_update(), &status)
        .await
        .expect("bible clear");

    let requests = server.received_requests().await.expect("requests");
    assert_eq!(
        opacities_put(&requests[after_verse.len()..], 130),
        vec![0.0],
        "the clear fades the restored #bible-a, whatever the cached mapping says"
    );
}

#[test]
fn the_composition_records_each_clips_opacity_and_layer_fade() {
    let mut layers = bible_layers(Some(FADE));
    layers.push(layer(
        vec![clip(900, "#timer", 90, 0.6)],
        Some(Duration::from_secs(10)),
    ));
    layers.push(layer(
        vec![json!({ "id": 901, "name": { "value": "#song-name" } })],
        None,
    ));
    for (id, name, secs) in [(902, "#band-name", 0.0), (903, "#main-a", -1.0)] {
        layers.push(json!({
            "transition": { "duration": { "value": secs } },
            "clips": [clip(id, name, id - 800, 1.0)],
        }));
    }
    let mapping = ClipMapping::from_composition(&json!({ "layers": layers })).expect("mapping");

    let bible_a = &mapping.bible_a[0];
    assert_eq!(
        bible_a.opacity,
        Some(ClipOpacity {
            param_id: 130,
            base: 1.0
        })
    );
    assert_eq!(bible_a.fade, FADE);
    let timer = &mapping.timer[0];
    assert_eq!(timer.opacity.map(|opacity| opacity.base), Some(0.6));
    assert_eq!(timer.fade, MAX_CLEAR_FADE, "a 10 s transition is capped");
    let song = &mapping.song_name[0];
    assert_eq!(song.opacity, None);
    assert_eq!(song.fade, Duration::ZERO);
    assert_eq!(
        mapping.band_name[0].fade,
        Duration::ZERO,
        "a 0 s transition"
    );
    assert_eq!(
        mapping.main_a[0].fade,
        Duration::ZERO,
        "a negative transition"
    );
}

#[test]
fn bible_clear_is_no_longer_a_clip_tag() {
    let mut layers = bible_layers(None);
    layers.push(layer(
        vec![
            clip(500, "#bible-clear", 50, 0.0),
            clip(501, "#bibleclear", 51, 0.0),
        ],
        None,
    ));
    let mapping = ClipMapping::from_composition(&json!({ "layers": layers })).expect("mapping");

    assert!(!mapping.destination_kinds().contains(&"#bible-clear"));
    assert!(!mapping.missing_tokens().contains(&"#bible-clear"));
    let lane_clip_ids: Vec<i64> = [&mapping.bible_a, &mapping.bible_reference_a]
        .into_iter()
        .flatten()
        .map(|target| target.clip_id)
        .collect();
    assert_eq!(lane_clip_ids, vec![BIBLE_A, REF_A]);
}

#[test]
fn fade_level_runs_linearly_from_the_start_value_to_zero() {
    let duration = Duration::from_millis(500);
    assert_eq!(fade_level(0.8, Duration::ZERO, duration), 0.8);
    assert!((fade_level(0.8, Duration::from_millis(250), duration) - 0.4).abs() < 1e-9);
    assert_eq!(fade_level(0.8, duration, duration), 0.0);
    assert_eq!(fade_level(0.8, Duration::from_secs(9), duration), 0.0);
    assert_eq!(fade_level(1.0, Duration::ZERO, Duration::ZERO), 0.0);
}

#[test]
fn the_plan_fades_visible_bible_clips_from_where_they_are() {
    let mut layers = bible_layers(Some(FADE));
    // A clip tagged twice fades once; a lyric clip never fades.
    layers.push(layer(
        vec![
            clip(302, "#bible-a #bible-b", 32, 1.0),
            clip(100, "#main-a", 10, 1.0),
        ],
        None,
    ));
    let mapping = ClipMapping::from_composition(&json!({ "layers": layers })).expect("mapping");
    let mut tracked = TrackedOpacities::default();
    tracked.adopt_invisible(&mapping, None);

    let plan = plan_clear_fade(&mapping, &tracked);
    let mut params: Vec<i64> = plan.iter().map(|target| target.param_id).collect();
    params.sort_unstable();
    assert_eq!(params, vec![130, 131, 132, 135, 136, 140, 141, 145, 146]);
    assert!(plan
        .iter()
        .all(|target| target.from == 1.0 && target.base == 1.0));
    assert!(plan
        .iter()
        .filter(|t| t.param_id != 132)
        .all(|t| t.duration == FADE));
}

#[test]
fn a_clip_mapped_invisible_is_adopted_once_per_fetch_and_restored_to_full() {
    let mut layers = bible_layers(None);
    layers[1] = layer(
        vec![
            clip(BIBLE_A, "#bible-a", 30, 0.0),
            clip(301, "#bible-b", 31, 1.0),
        ],
        None,
    );
    let mapping = ClipMapping::from_composition(&json!({ "layers": layers })).expect("mapping");
    let fetched = Some(tokio::time::Instant::now());
    let mut tracked = TrackedOpacities::default();

    tracked.adopt_invisible(&mapping, fetched);
    assert_eq!(
        tracked.get(130),
        Some(TrackedOpacity {
            base: 1.0,
            level: 0.0
        }),
        "a Bible clip left at 0 (e.g. by a previous process) goes back to full"
    );
    assert_eq!(tracked.get(131), None);
    let plan = plan_clear_fade(&mapping, &tracked);
    assert!(
        plan.iter().all(|target| target.param_id != 130),
        "an invisible clip is not faded again"
    );
}
