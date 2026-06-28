//! #483 regression: the per-slide push path must serve the cached clip-mapping
//! and never re-fetch the whole `/composition` inline on staleness.
//!
//! Kept in its own file (self-contained helpers) so the test is independent of
//! the larger `tests.rs` fixtures.

use super::driver::{count_clips, duration_ms, FetchReason, HostDriver};
use super::{
    flush_perceived, PerceivedAgg, ResolumeConnectionSnapshot, ResolumeRegistry, StageUpdate,
    CONNECT_TIMEOUT,
};
use chrono::Utc;
use presenter_core::{ResolumeHost, ResolumeHostId};
use presenter_persistence::Repository;
use reqwest::Client;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A clip JSON node with a text param (mirrors the helper in `tests.rs`).
fn clip(id: i64, name: &str, param_id: i64) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "name": { "value": name },
        "video": { "sourceparams": { "text": { "valuetype": "ParamText", "id": param_id } } },
    })
}

fn count_requests(requests: &[wiremock::Request], method_name: &str, path_name: &str) -> usize {
    requests
        .iter()
        .filter(|req| req.method.as_str() == method_name && req.url.path() == path_name)
        .count()
}

fn stage_main(text: &str) -> StageUpdate {
    StageUpdate {
        current_main: Some(text.to_string()),
        current_translation: None,
        song_name: None,
        band_name: None,
        enqueued_at: None,
        correlation_id: None,
    }
}

/// Regression for #483 (Resolume lyrics latency).
///
/// The per-slide push path (`handle_stage`) must NOT re-fetch the whole
/// `/composition` just because the cached clip-mapping is older than the old
/// 1 s `MAPPING_CACHE_TTL`. During singing, slides change every few seconds —
/// i.e. >1 s apart — so the old stale-refetch branch re-fetched + re-parsed the
/// entire composition before almost every line (300–620 ms each → ~1 s
/// perceived LED-wall latency). The fix serves the push path from cache; the
/// existing 10 s background timer and on-error invalidation are the only
/// refresh triggers. A push whose mapping is younger than the refresh interval
/// (10 s) must therefore cause ZERO extra `GET /composition`.
///
/// The cache age is set deterministically by back-dating `last_mapping_refresh`
/// to 3 s ago (real `tokio::time::Instant`, no wall-clock sleep): 3 s is past
/// the OLD 1 s TTL — so this test is RED on the buggy code (the second push
/// re-fetches → GET count == 2) — but below the 10 s background interval — so
/// the fixed code serves from cache → GET count == 1.
#[tokio::test]
async fn stage_push_does_not_refetch_composition_when_cache_is_fresh() {
    let server = MockServer::start().await;

    let composition = serde_json::json!({
        "layers": [
            {
                "clips": [
                    clip(100, "#main-a", 1),
                    clip(101, "#main-b", 2),
                ],
            }
        ]
    });

    Mock::given(method("GET"))
        .and(path("/api/v1/composition"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&composition))
        .mount(&server)
        .await;
    for endpoint in &[1, 2] {
        let route = format!("/api/v1/parameter/by-id/{endpoint}");
        Mock::given(method("PUT"))
            .and(path(route.as_str()))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
    }
    for clip_id in &[100, 101] {
        let route = format!("/api/v1/composition/clips/by-id/{clip_id}/connect");
        Mock::given(method("POST"))
            .and(path(route.as_str()))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
    }

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
        .expect("client build");
    let mut driver = HostDriver::new(client, config);
    let status = Arc::new(RwLock::new(ResolumeConnectionSnapshot::disabled()));
    driver.refresh_status(&status).await;

    // First push: cold cache → exactly one composition fetch.
    driver
        .handle_stage(stage_main("Line 1"), &status)
        .await
        .expect("first stage");

    // Age the cached mapping to 3 s — past the OLD 1 s TTL, below the 10 s
    // background interval (the exact gap between two sung lines).
    driver.last_mapping_refresh = Some(tokio::time::Instant::now() - Duration::from_secs(3));

    // Second push: mapping is cached and younger than the refresh interval, so
    // it MUST be served from cache with NO additional /composition fetch.
    driver
        .handle_stage(stage_main("Line 2"), &status)
        .await
        .expect("second stage");

    let requests = server.received_requests().await.expect("received requests");
    assert_eq!(
        count_requests(&requests, "GET", "/api/v1/composition"),
        1,
        "push path must serve the cached mapping; the 1 s TTL inline refetch (#483) is removed"
    );
}

/// Build a ResolumeHost config pointing at a mock server.
fn mock_host(server: &MockServer) -> ResolumeHost {
    let addr = server.address();
    let now = Utc::now();
    ResolumeHost::new(
        ResolumeHostId::new(),
        "Mock".into(),
        addr.ip().to_string(),
        addr.port(),
        true,
        now,
        now,
    )
}

/// Mount the composition GET plus the given parameter PUTs and clip-connect POSTs.
async fn mount_resolume(
    server: &MockServer,
    composition: &serde_json::Value,
    param_ids: &[i64],
    clip_ids: &[i64],
) {
    Mock::given(method("GET"))
        .and(path("/api/v1/composition"))
        .respond_with(ResponseTemplate::new(200).set_body_json(composition))
        .mount(server)
        .await;
    for pid in param_ids {
        Mock::given(method("PUT"))
            .and(path(format!("/api/v1/parameter/by-id/{pid}").as_str()))
            .respond_with(ResponseTemplate::new(200))
            .mount(server)
            .await;
    }
    for cid in clip_ids {
        Mock::given(method("POST"))
            .and(path(
                format!("/api/v1/composition/clips/by-id/{cid}/connect").as_str(),
            ))
            .respond_with(ResponseTemplate::new(200))
            .mount(server)
            .await;
    }
}

#[test]
fn duration_ms_converts_seconds_to_milliseconds() {
    // #483 telemetry conversion — pinned so the `* 1000.0` can't drift.
    assert_eq!(duration_ms(Duration::from_secs(1)), 1000.0);
    assert_eq!(duration_ms(Duration::from_millis(5)), 5.0);
    assert_eq!(duration_ms(Duration::from_micros(500)), 0.5);
    assert_eq!(duration_ms(Duration::ZERO), 0.0);
}

#[test]
fn count_clips_sums_clips_across_layers() {
    // #483: clip_count logged on every composition fetch.
    let composition = serde_json::json!({
        "layers": [
            { "clips": [ clip(1, "#main-a", 1), clip(2, "#main-b", 2) ] },
            { "clips": [ clip(3, "#timer", 3) ] },
        ]
    });
    assert_eq!(count_clips(&composition), 3);
    assert_eq!(count_clips(&serde_json::json!({ "layers": [] })), 0);
    assert_eq!(count_clips(&serde_json::json!({})), 0);
}

#[test]
fn fetch_reason_as_str_maps_each_variant() {
    // #483: the fetch reason is logged on every composition fetch.
    assert_eq!(FetchReason::Missing.as_str(), "missing");
    assert_eq!(FetchReason::ErrorInvalidated.as_str(), "error-invalidated");
    assert_eq!(FetchReason::BackgroundTimer.as_str(), "background-timer");
}

#[test]
fn flush_perceived_emits_and_removes_only_aged_entries() {
    // #483: the cross-host perceived-latency aggregator flushes a correlation id
    // only once all hosts have had time to report (older than PERCEIVED_FLUSH_AFTER
    // = 2 s), keeping fresher ones until their window elapses; `force` drains all.
    let mut pending: HashMap<String, PerceivedAgg> = HashMap::new();
    pending.insert(
        "aged".to_string(),
        PerceivedAgg {
            first_seen: Instant::now() - Duration::from_secs(3),
            max_perceived_ms: 100.0,
            hosts: 2,
            slowest_host: "h1".to_string(),
        },
    );
    pending.insert(
        "fresh".to_string(),
        PerceivedAgg {
            first_seen: Instant::now(),
            max_perceived_ms: 5.0,
            hosts: 1,
            slowest_host: "h2".to_string(),
        },
    );

    // Non-forced sweep flushes only the aged entry.
    flush_perceived(&mut pending, false);
    assert!(!pending.contains_key("aged"), "aged entry must be flushed");
    assert!(pending.contains_key("fresh"), "fresh entry must be kept");

    // Forced flush drains the rest.
    flush_perceived(&mut pending, true);
    assert!(pending.is_empty(), "force must drain all remaining entries");
}

#[tokio::test]
async fn stage_push_persists_audit_row_through_registry() {
    // #483: end-to-end audit path — a stage push through the registry spawns a
    // host worker, which records a push-audit row via the DB-backed writer.
    let server = MockServer::start().await;
    let composition = serde_json::json!({
        "layers": [ { "clips": [ clip(100, "#main-a", 1), clip(101, "#main-b", 2) ] } ]
    });
    mount_resolume(&server, &composition, &[1, 2], &[100, 101]).await;

    let repo = Repository::connect_in_memory().await.expect("repo");
    let registry = ResolumeRegistry::new().expect("registry");
    registry.attach_audit_writer(repo.clone());
    registry.set_hosts(vec![mock_host(&server)]).await;

    let correlation_id = uuid::Uuid::new_v4();
    let mut update = stage_main("Line 1");
    update.correlation_id = Some(correlation_id);
    registry.stage_update(update).await;

    // Retry-with-assert: the worker + writer run on background tasks.
    let mut found = None;
    for _ in 0..60 {
        let rows = repo
            .list_resolume_push_audit(None, None, None, 10)
            .await
            .expect("list audit");
        if let Some(row) = rows.into_iter().next() {
            found = Some(row);
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let row = found.expect("a push-audit row must be persisted for the stage push");
    assert_eq!(
        row.correlation_id.as_deref(),
        Some(correlation_id.to_string().as_str())
    );
    assert_eq!(row.host, server.address().ip().to_string());
    assert_eq!(row.outcome, "ok");
    // First push is a cold-cache fetch.
    assert!(
        row.refetched,
        "the first push fetches the composition inline"
    );
}

#[tokio::test]
async fn failed_stage_push_persists_error_audit_row_through_registry() {
    // #489: a push that FAILS (Resolume returns a non-2xx on /composition) must
    // still leave an audit row with outcome starting `error`, so the failures
    // that hurt most — a down Resolume → COMPOSITION_TIMEOUT — are visible in
    // the audit table instead of being silently dropped (the gap #489 fixes).
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/composition"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let repo = Repository::connect_in_memory().await.expect("repo");
    let registry = ResolumeRegistry::new().expect("registry");
    registry.attach_audit_writer(repo.clone());
    registry.set_hosts(vec![mock_host(&server)]).await;

    let correlation_id = uuid::Uuid::new_v4();
    let mut update = stage_main("Line 1");
    update.correlation_id = Some(correlation_id);
    registry.stage_update(update).await;

    // Retry-with-assert: the worker + writer run on background tasks.
    let mut found = None;
    for _ in 0..60 {
        let rows = repo
            .list_resolume_push_audit(None, Some("error"), None, 10)
            .await
            .expect("list audit");
        if let Some(row) = rows.into_iter().next() {
            found = Some(row);
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let row = found.expect("a failed push must persist an outcome=error audit row");
    assert_eq!(
        row.correlation_id.as_deref(),
        Some(correlation_id.to_string().as_str())
    );
    assert_eq!(row.host, server.address().ip().to_string());
    assert!(
        row.outcome.starts_with("error"),
        "failed push outcome must start with `error`, got {:?}",
        row.outcome
    );
}

#[tokio::test]
async fn main_only_push_keeps_translation_lane_in_sync() {
    // #483 (lane-sync edge): when main is filled but translation is NOT, and the
    // mapping HAS translation clips, BOTH lanes flip so a later translation lands
    // on the same A/B side as main. Without the flip, the next translation would
    // be triggered on the stale (un-flipped) lane.
    let server = MockServer::start().await;
    let composition = serde_json::json!({
        "layers": [ { "clips": [
            clip(100, "#main-a", 1),
            clip(101, "#main-b", 2),
            clip(200, "#translate-a", 10),
            clip(201, "#translate-b", 20),
        ] } ]
    });
    mount_resolume(
        &server,
        &composition,
        &[1, 2, 10, 20],
        &[100, 101, 200, 201],
    )
    .await;

    let client = Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .build()
        .expect("client build");
    let mut driver = HostDriver::new(client, mock_host(&server));
    let status = Arc::new(RwLock::new(ResolumeConnectionSnapshot::disabled()));
    driver.refresh_status(&status).await;

    // Push 1: main only (no translation) → flips BOTH lanes A→B.
    driver
        .handle_stage(stage_main("Line 1"), &status)
        .await
        .expect("first stage");

    // Push 2: main + translation → translation must land on lane B (clip 201),
    // proving the lane-sync flip happened on push 1.
    let mut second = stage_main("Line 2");
    second.current_translation = Some("Trans 2".to_string());
    driver
        .handle_stage(second, &status)
        .await
        .expect("second stage");

    let requests = server.received_requests().await.expect("requests");
    assert_eq!(
        count_requests(
            &requests,
            "POST",
            "/api/v1/composition/clips/by-id/201/connect"
        ),
        1,
        "translation must trigger lane B (#translate-b) after the sync flip"
    );
    assert_eq!(
        count_requests(
            &requests,
            "POST",
            "/api/v1/composition/clips/by-id/200/connect"
        ),
        0,
        "translation must NOT trigger lane A (#translate-a); lanes would be desynced"
    );
}
