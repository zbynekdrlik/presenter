//! #483 regression: the per-slide push path must serve the cached clip-mapping
//! and never re-fetch the whole `/composition` inline on staleness.
//!
//! Kept in its own file (self-contained helpers) so the test is independent of
//! the larger `tests.rs` fixtures.

use super::driver::HostDriver;
use super::{ResolumeConnectionSnapshot, StageUpdate, CONNECT_TIMEOUT};
use chrono::Utc;
use presenter_core::{ResolumeHost, ResolumeHostId};
use reqwest::Client;
use std::sync::Arc;
use std::time::Duration;
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
