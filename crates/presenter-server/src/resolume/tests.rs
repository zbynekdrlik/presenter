use super::clip_map::ClipMapping;
use super::driver::HostDriver;
use super::handlers::translation_short_code;
use super::types::TextTransform;
use super::{
    BibleUpdate, ResolumeConnectionSnapshot, ResolumeConnectionState, TimerFrame, DEFAULT_TIMEOUT,
};
use chrono::Utc;
use presenter_core::{
    BibleBroadcast, BiblePassage, BibleReference, BibleSlideOutput, BibleTranslation, ResolumeHost,
    ResolumeHostId,
};
use reqwest::Client;
use std::sync::Arc;
use tokio::sync::RwLock;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::StageUpdate;

fn sample_host(host: &str) -> ResolumeHost {
    let now = Utc::now();
    ResolumeHost::new(
        ResolumeHostId::new(),
        "Test Resolume".to_string(),
        host.to_string(),
        8090,
        true,
        now,
        now,
    )
}

fn clip(id: i64, name: &str, param_id: Option<i64>) -> serde_json::Value {
    let sourceparams = match param_id {
        Some(value) => serde_json::json!({
            "text": {
                "valuetype": "ParamText",
                "id": value,
            }
        }),
        None => serde_json::json!({}),
    };
    serde_json::json!({
        "id": id,
        "name": { "value": name },
        "video": { "sourceparams": sourceparams },
    })
}

fn count_requests(requests: &[wiremock::Request], method_name: &str, path_name: &str) -> usize {
    requests
        .iter()
        .filter(|req| req.method.as_str() == method_name && req.url.path() == path_name)
        .count()
}

#[test]
fn clip_mapping_parses_tags_inside_names() {
    let composition = serde_json::json!({
        "layers": [
            {
                "clips": [
                    clip(100, "Song Title #main-a-u-re", Some(1)),
                    clip(200, "ALT #translate-b-u", Some(2)),
                    clip(300, "Countdown #timer", Some(3)),
                    clip(400, "#song-name-u", Some(4)),
                    clip(500, "#band-name", Some(5)),
                    clip(600, "#bible-reference-a", Some(6)),
                    clip(700, "#bible-translate-reference-a", Some(7)),
                ],
            }
        ]
    });

    let mapping = ClipMapping::from_composition(&composition).expect("mapping");
    assert_eq!(mapping.main_a.len(), 1);
    assert_eq!(mapping.translation_b.len(), 1);
    assert_eq!(mapping.song_name.len(), 1);
    assert_eq!(mapping.band_name.len(), 1);
    assert_eq!(
        mapping.main_a[0].transforms,
        vec![TextTransform::Uppercase, TextTransform::RemoveLineBreaks]
    );
    assert_eq!(
        mapping.translation_b[0].transforms,
        vec![TextTransform::Uppercase]
    );
    assert_eq!(
        mapping.song_name[0].transforms,
        vec![TextTransform::Uppercase]
    );
    assert_eq!(mapping.bible_reference_a.len(), 1);
    assert_eq!(mapping.bible_translate_reference_a.len(), 1);
}

#[tokio::test]
async fn resolve_endpoint_with_ip_literal_skips_host_header() {
    let host = sample_host("127.0.0.1");
    let driver = HostDriver::new(Client::new(), host);
    let endpoint = driver.resolve_endpoint().await.unwrap();
    assert_eq!(endpoint.base_url, "http://127.0.0.1:8090/api/v1");
    assert!(endpoint.host_header.is_none());
}

#[tokio::test]
async fn resolve_endpoint_with_hostname_sets_header() {
    let host = sample_host("localhost");
    let driver = HostDriver::new(Client::new(), host);
    let endpoint = driver.resolve_endpoint().await.unwrap();
    assert!(
        endpoint.base_url.contains("127.0.0.1:8090") || endpoint.base_url.contains("[::1]:8090")
    );
    assert_eq!(endpoint.host_header.as_deref(), Some("localhost"));
}

#[tokio::test]
async fn stage_updates_alternate_main_and_translation_lanes() {
    let server = MockServer::start().await;

    let composition = serde_json::json!({
        "layers": [
            {
                "clips": [
                    clip(100, "#main-a", Some(1)),
                    clip(101, "#main-b", Some(2)),
                    clip(200, "#translate-a", Some(10)),
                    clip(201, "#translate-b", Some(20)),
                    clip(300, "#bible-a", Some(30)),
                    clip(301, "#bible-b", Some(31)),
                    clip(350, "#bible-reference-a", Some(35)),
                    clip(351, "#bible-reference-b", Some(36)),
                    clip(400, "#bible-translate-a", Some(40)),
                    clip(401, "#bible-translate-b", Some(41)),
                    clip(450, "#bible-translate-reference-a", Some(45)),
                    clip(451, "#bible-translate-reference-b", Some(46)),
                    clip(500, "#bible-clear", None),
                    clip(600, "#song-name", Some(60)),
                    clip(601, "#band-name", Some(61)),
                    clip(900, "#timer", Some(90)),
                ],
            }
        ]
    });

    Mock::given(method("GET"))
        .and(path("/api/v1/composition"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&composition))
        .mount(&server)
        .await;

    for endpoint in &[1, 2, 10, 20, 30, 31, 35, 36, 40, 41, 45, 46, 90] {
        let route = format!("/api/v1/parameter/by-id/{endpoint}");
        Mock::given(method("PUT"))
            .and(path(route.as_str()))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
    }
    for endpoint in &[60, 61] {
        let route = format!("/api/v1/parameter/by-id/{endpoint}");
        Mock::given(method("PUT"))
            .and(path(route.as_str()))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
    }

    for clip_id in &[
        100, 101, 200, 201, 300, 301, 350, 351, 400, 401, 450, 451, 500, 900,
    ] {
        let route = format!("/api/v1/composition/clips/by-id/{clip_id}/connect");
        Mock::given(method("POST"))
            .and(path(route.as_str()))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
    }

    let addr = server.address();
    let host = addr.ip().to_string();
    let port = addr.port();
    let now = Utc::now();
    let config = ResolumeHost::new(
        ResolumeHostId::new(),
        "Mock".into(),
        host,
        port,
        true,
        now,
        now,
    );

    let client = Client::builder()
        .timeout(DEFAULT_TIMEOUT)
        .build()
        .expect("client build");
    let mut driver = HostDriver::new(client, config);
    let status = Arc::new(RwLock::new(ResolumeConnectionSnapshot::disabled()));

    driver.refresh_status(&status).await;

    let stage_first = StageUpdate {
        current_main: Some("Line 1".to_string()),
        current_translation: Some("Trans 1".to_string()),
        song_name: Some("First Song".to_string()),
        band_name: Some("Library".to_string()),
    };
    driver
        .handle_stage(stage_first, &status)
        .await
        .expect("first stage");

    let stage_second = StageUpdate {
        current_main: Some("Line 2".to_string()),
        current_translation: Some("Trans 2".to_string()),
        song_name: Some("Second Song".to_string()),
        band_name: Some("Library".to_string()),
    };
    driver
        .handle_stage(stage_second, &status)
        .await
        .expect("second stage");

    let requests = server.received_requests().await.expect("received requests");

    assert_eq!(count_requests(&requests, "GET", "/api/v1/composition"), 1);
    assert_eq!(
        count_requests(&requests, "PUT", "/api/v1/parameter/by-id/1"),
        1
    );
    assert_eq!(
        count_requests(&requests, "PUT", "/api/v1/parameter/by-id/2"),
        1
    );
    assert_eq!(
        count_requests(&requests, "PUT", "/api/v1/parameter/by-id/10"),
        1
    );
    assert_eq!(
        count_requests(&requests, "PUT", "/api/v1/parameter/by-id/20"),
        1
    );
    let song60 = count_requests(&requests, "PUT", "/api/v1/parameter/by-id/60");
    let band61 = count_requests(&requests, "PUT", "/api/v1/parameter/by-id/61");
    assert_eq!(song60, 2);
    assert_eq!(band61, 1);

    assert_eq!(
        count_requests(
            &requests,
            "POST",
            "/api/v1/composition/clips/by-id/100/connect",
        ),
        1
    );
    assert_eq!(
        count_requests(
            &requests,
            "POST",
            "/api/v1/composition/clips/by-id/200/connect",
        ),
        1
    );
    assert_eq!(
        count_requests(
            &requests,
            "POST",
            "/api/v1/composition/clips/by-id/101/connect",
        ),
        1
    );
    assert_eq!(
        count_requests(
            &requests,
            "POST",
            "/api/v1/composition/clips/by-id/201/connect",
        ),
        1
    );
}

#[tokio::test]
async fn clip_name_transforms_apply_to_payload() {
    let server = MockServer::start().await;

    let composition = serde_json::json!({
        "layers": [
            {
                "clips": [
                    clip(100, "#main-a-u-re", Some(1)),
                    clip(101, "#main-b", Some(2)),
                    clip(200, "#translate-a", Some(10)),
                    clip(201, "#translate-b", Some(20)),
                    clip(300, "#bible-a", Some(30)),
                    clip(301, "#bible-b", Some(31)),
                    clip(350, "#bible-reference-a", Some(35)),
                    clip(351, "#bible-reference-b", Some(36)),
                    clip(400, "#bible-translate-a", Some(40)),
                    clip(401, "#bible-translate-b", Some(41)),
                    clip(450, "#bible-translate-reference-a", Some(45)),
                    clip(451, "#bible-translate-reference-b", Some(46)),
                    clip(500, "#bible-clear", None),
                    clip(910, "#song-name", Some(95)),
                    clip(911, "#band-name", Some(96)),
                    clip(900, "#timer", Some(90)),
                ],
            }
        ]
    });

    Mock::given(method("GET"))
        .and(path("/api/v1/composition"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&composition))
        .mount(&server)
        .await;

    Mock::given(method("PUT"))
        .and(path("/api/v1/parameter/by-id/1"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    for endpoint in [95, 96] {
        let route = format!("/api/v1/parameter/by-id/{endpoint}");
        Mock::given(method("PUT"))
            .and(path(route.as_str()))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
    }

    Mock::given(method("PUT"))
        .and(path("/api/v1/parameter/by-id/90"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/api/v1/composition/clips/by-id/100/connect"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

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
        .timeout(DEFAULT_TIMEOUT)
        .build()
        .expect("client");

    let mut driver = HostDriver::new(client, config);
    let status = Arc::new(RwLock::new(ResolumeConnectionSnapshot::disabled()));

    driver.ensure_mapping().await.unwrap();

    let stage = StageUpdate {
        current_main: Some(
            "Line 1
Line 2"
                .to_string(),
        ),
        current_translation: None,
        song_name: Some("Song".to_string()),
        band_name: Some("Band".to_string()),
    };

    driver
        .handle_stage(stage, &status)
        .await
        .expect("stage update");

    let requests = server.received_requests().await.expect("requests");
    let payload_request = requests
        .iter()
        .find(|req| req.method.as_str() == "PUT" && req.url.path() == "/api/v1/parameter/by-id/1")
        .expect("transform request");
    let body: serde_json::Value = serde_json::from_slice(&payload_request.body).expect("json body");
    assert_eq!(
        body.get("value"),
        Some(&serde_json::Value::String("LINE 1 LINE 2".into()))
    );
}

#[tokio::test]
async fn timer_updates_send_text_without_trigger() {
    let server = MockServer::start().await;

    let composition = serde_json::json!({
        "layers": [
            {
                "clips": [
                    clip(100, "#main-a", Some(1)),
                    clip(101, "#main-b", Some(2)),
                    clip(200, "#translate-a", Some(10)),
                    clip(201, "#translate-b", Some(20)),
                    clip(300, "#bible-a", Some(30)),
                    clip(301, "#bible-b", Some(31)),
                    clip(350, "#bible-reference-a", Some(35)),
                    clip(351, "#bible-reference-b", Some(36)),
                    clip(400, "#bible-translate-a", Some(40)),
                    clip(401, "#bible-translate-b", Some(41)),
                    clip(450, "#bible-translate-reference-a", Some(45)),
                    clip(451, "#bible-translate-reference-b", Some(46)),
                    clip(500, "#bible-clear", None),
                    clip(900, "#timer", Some(90)),
                ],
            }
        ]
    });

    Mock::given(method("GET"))
        .and(path("/api/v1/composition"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&composition))
        .mount(&server)
        .await;

    Mock::given(method("PUT"))
        .and(path("/api/v1/parameter/by-id/90"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

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
        .timeout(DEFAULT_TIMEOUT)
        .build()
        .expect("client");

    let mut driver = HostDriver::new(client, config);
    let status = Arc::new(RwLock::new(ResolumeConnectionSnapshot::disabled()));

    driver.refresh_status(&status).await;

    driver
        .handle_timer(TimerFrame::new("05:00".to_string()), &status)
        .await
        .expect("initial timer update");

    let mut requests = server.received_requests().await.expect("requests");
    assert_eq!(
        count_requests(&requests, "PUT", "/api/v1/parameter/by-id/90",),
        1
    );

    driver
        .handle_timer(TimerFrame::new("05:00".to_string()), &status)
        .await
        .expect("deduplicated timer update");

    requests = server.received_requests().await.expect("requests");
    assert_eq!(
        count_requests(&requests, "PUT", "/api/v1/parameter/by-id/90",),
        1
    );

    driver
        .handle_timer(TimerFrame::new("59".to_string()), &status)
        .await
        .expect("second timer update");

    requests = server.received_requests().await.expect("requests");
    assert_eq!(
        count_requests(&requests, "PUT", "/api/v1/parameter/by-id/90",),
        2
    );

    assert!(requests.iter().any(|req| {
        req.method.as_str() == "PUT"
            && req.url.path() == "/api/v1/parameter/by-id/90"
            && std::str::from_utf8(&req.body)
                .unwrap_or_default()
                .contains("59")
    }));
}

#[tokio::test]
async fn refreshes_mapping_after_cache_ttl_for_new_deck() {
    let server = MockServer::start().await;

    let composition_a = serde_json::json!({
        "layers": [
            {
                "clips": [
                    clip(100, "#main-a", Some(1)),
                    clip(101, "#main-b", Some(2)),
                    clip(200, "#translate-a", Some(10)),
                    clip(201, "#translate-b", Some(20)),
                    clip(300, "#bible-a", Some(30)),
                    clip(301, "#bible-b", Some(31)),
                    clip(350, "#bible-reference-a", Some(35)),
                    clip(351, "#bible-reference-b", Some(36)),
                    clip(400, "#bible-translate-a", Some(40)),
                    clip(401, "#bible-translate-b", Some(41)),
                    clip(450, "#bible-translate-reference-a", Some(45)),
                    clip(451, "#bible-translate-reference-b", Some(46)),
                    clip(500, "#bible-clear", None),
                    clip(900, "#timer", Some(90)),
                ],
            }
        ]
    });

    let composition_b = serde_json::json!({
        "layers": [
            {
                "clips": [
                    clip(300, "#main-a", Some(101)),
                    clip(301, "#main-b", Some(102)),
                    clip(400, "#translate-a", Some(110)),
                    clip(401, "#translate-b", Some(120)),
                    clip(500, "#bible-a", Some(130)),
                    clip(501, "#bible-b", Some(131)),
                    clip(550, "#bible-reference-a", Some(135)),
                    clip(551, "#bible-reference-b", Some(136)),
                    clip(600, "#bible-translate-a", Some(140)),
                    clip(601, "#bible-translate-b", Some(141)),
                    clip(650, "#bible-translate-reference-a", Some(145)),
                    clip(651, "#bible-translate-reference-b", Some(146)),
                    clip(700, "#bible-clear", None),
                    clip(960, "#song-name", Some(195)),
                    clip(961, "#band-name", Some(196)),
                    clip(950, "#timer", Some(190)),
                ],
            }
        ]
    });

    Mock::given(method("GET"))
        .and(path("/api/v1/composition"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&composition_a))
        .mount(&server)
        .await;

    for endpoint in [1, 2, 10, 20, 30, 31, 35, 36, 40, 41, 45, 46, 90, 95, 96] {
        let route = format!("/api/v1/parameter/by-id/{endpoint}");
        Mock::given(method("PUT"))
            .and(path(route.as_str()))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
    }

    for clip_id in [
        100, 101, 200, 201, 300, 301, 350, 351, 400, 401, 450, 451, 500, 900, 910, 911,
    ] {
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
        .timeout(DEFAULT_TIMEOUT)
        .build()
        .expect("client");

    let mut driver = HostDriver::new(client, config);
    let status = Arc::new(RwLock::new(ResolumeConnectionSnapshot::disabled()));

    driver.ensure_mapping().await.unwrap();

    let first = StageUpdate {
        current_main: Some("First".to_string()),
        current_translation: None,
        song_name: Some("First Song".to_string()),
        band_name: Some("Band A".to_string()),
    };
    driver
        .handle_stage(first, &status)
        .await
        .expect("first stage");

    server.reset().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/composition"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&composition_b))
        .mount(&server)
        .await;

    for endpoint in [
        101, 102, 110, 120, 130, 131, 135, 136, 140, 141, 145, 146, 190, 195, 196,
    ] {
        let route = format!("/api/v1/parameter/by-id/{endpoint}");
        Mock::given(method("PUT"))
            .and(path(route.as_str()))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
    }

    for clip_id in [
        300, 301, 400, 401, 500, 550, 551, 600, 601, 650, 651, 700, 950, 960, 961,
    ] {
        let route = format!("/api/v1/composition/clips/by-id/{clip_id}/connect");
        Mock::given(method("POST"))
            .and(path(route.as_str()))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
    }

    driver.refresh_mapping().await.unwrap();

    let second = StageUpdate {
        current_main: Some("Second".to_string()),
        current_translation: None,
        song_name: Some("Second Song".to_string()),
        band_name: Some("Band B".to_string()),
    };
    driver
        .handle_stage(second, &status)
        .await
        .expect("second stage");

    let requests = server.received_requests().await.expect("requests");
    let new_param_hits = count_requests(&requests, "PUT", "/api/v1/parameter/by-id/101")
        + count_requests(&requests, "PUT", "/api/v1/parameter/by-id/102");
    assert_eq!(new_param_hits, 1);
    assert_eq!(
        count_requests(&requests, "PUT", "/api/v1/parameter/by-id/2"),
        0
    );
}

// ── translation_short_code tests ────────────────────────────────────

#[test]
fn translation_short_code_extracts_after_last_dash() {
    assert_eq!(translation_short_code("en-kjv"), "KJV");
    assert_eq!(translation_short_code("cs-cep"), "CEP");
}

#[test]
fn translation_short_code_handles_no_dash() {
    assert_eq!(translation_short_code("kjv"), "KJV");
}

#[test]
fn translation_short_code_handles_multiple_dashes() {
    assert_eq!(translation_short_code("sk-rob-2007"), "2007");
}

#[test]
fn translation_short_code_handles_empty_string() {
    assert_eq!(translation_short_code(""), "");
}

// ── Helper: create a mock driver with a running wiremock server ─────

async fn setup_bible_driver() -> (
    MockServer,
    HostDriver,
    Arc<RwLock<ResolumeConnectionSnapshot>>,
) {
    let server = MockServer::start().await;

    let composition = serde_json::json!({
        "layers": [
            {
                "clips": [
                    clip(100, "#main-a", Some(1)),
                    clip(101, "#main-b", Some(2)),
                    clip(200, "#translate-a", Some(10)),
                    clip(201, "#translate-b", Some(20)),
                    clip(300, "#bible-a", Some(30)),
                    clip(301, "#bible-b", Some(31)),
                    clip(350, "#bible-reference-a", Some(35)),
                    clip(351, "#bible-reference-b", Some(36)),
                    clip(400, "#bible-translate-a", Some(40)),
                    clip(401, "#bible-translate-b", Some(41)),
                    clip(450, "#bible-translate-reference-a", Some(45)),
                    clip(451, "#bible-translate-reference-b", Some(46)),
                    clip(500, "#bible-clear", None),
                    clip(600, "#song-name", Some(60)),
                    clip(601, "#band-name", Some(61)),
                    clip(900, "#timer", Some(90)),
                ],
            }
        ]
    });

    Mock::given(method("GET"))
        .and(path("/api/v1/composition"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&composition))
        .mount(&server)
        .await;

    // Mount PUT endpoints for all text params
    for param_id in [1, 2, 10, 20, 30, 31, 35, 36, 40, 41, 45, 46, 60, 61, 90] {
        let route = format!("/api/v1/parameter/by-id/{param_id}");
        Mock::given(method("PUT"))
            .and(path(route.as_str()))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
    }

    // Mount POST (trigger) endpoints for all clips
    for clip_id in [
        100, 101, 200, 201, 300, 301, 350, 351, 400, 401, 450, 451, 500, 900,
    ] {
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
        .timeout(DEFAULT_TIMEOUT)
        .build()
        .expect("client");
    let driver = HostDriver::new(client, config);
    let status = Arc::new(RwLock::new(ResolumeConnectionSnapshot::disabled()));

    (server, driver, status)
}

// ── handle_bible: routing tests ─────────────────────────────────────

#[tokio::test]
async fn handle_bible_routes_to_slide_output_when_present() {
    let (server, mut driver, status) = setup_bible_driver().await;

    let output = BibleSlideOutput {
        main_text: "For God so loved".to_string(),
        main_reference: "John 3:16 (KJV)".to_string(),
        secondary_text: "Neboť Bůh tak miloval".to_string(),
        secondary_reference: "Jan 3:16 (CEP)".to_string(),
        triggered_at: Utc::now(),
    };
    let update = BibleUpdate {
        passage: None,
        secondary_text: None,
        secondary_translation_code: None,
        slide_output: Some(output),
    };

    driver.handle_bible(update, &status).await.expect("bible");

    let requests = server.received_requests().await.expect("requests");
    // Should have sent to bible-a (param 30) with slide output main text
    assert!(requests.iter().any(|req| {
        req.method.as_str() == "PUT"
            && req.url.path() == "/api/v1/parameter/by-id/30"
            && std::str::from_utf8(&req.body)
                .unwrap_or_default()
                .contains("For God so loved")
    }));
    // Should have sent reference to bible-reference-a (param 35)
    assert!(requests.iter().any(|req| {
        req.method.as_str() == "PUT"
            && req.url.path() == "/api/v1/parameter/by-id/35"
            && std::str::from_utf8(&req.body)
                .unwrap_or_default()
                .contains("John 3:16 (KJV)")
    }));
    // Should have sent secondary text to bible-translate-a (param 40)
    assert!(requests.iter().any(|req| {
        req.method.as_str() == "PUT"
            && req.url.path() == "/api/v1/parameter/by-id/40"
            && std::str::from_utf8(&req.body)
                .unwrap_or_default()
                .contains("Neboť Bůh tak miloval")
    }));
    // Should have sent secondary reference to bible-translate-reference-a (param 45)
    assert!(requests.iter().any(|req| {
        req.method.as_str() == "PUT"
            && req.url.path() == "/api/v1/parameter/by-id/45"
            && std::str::from_utf8(&req.body)
                .unwrap_or_default()
                .contains("Jan 3:16 (CEP)")
    }));
}

#[tokio::test]
async fn handle_bible_routes_to_legacy_when_no_slide_output() {
    let (server, mut driver, status) = setup_bible_driver().await;

    let reference = BibleReference::new("John", 3, 16, 16).expect("ref");
    let translation = BibleTranslation::new("en-kjv", "King James Version", "en");
    let passage = BiblePassage::new(
        reference,
        translation,
        "For God so loved the world".to_string(),
    );
    let broadcast = BibleBroadcast::new(passage, Utc::now());
    let update = BibleUpdate {
        passage: Some(broadcast),
        secondary_text: Some("Neboť Bůh tak miloval svět".to_string()),
        secondary_translation_code: Some("cs-cep".to_string()),
        slide_output: None,
    };

    driver.handle_bible(update, &status).await.expect("bible");

    let requests = server.received_requests().await.expect("requests");
    // Legacy path should send verse text to bible-a (param 30)
    assert!(requests.iter().any(|req| {
        req.method.as_str() == "PUT"
            && req.url.path() == "/api/v1/parameter/by-id/30"
            && std::str::from_utf8(&req.body)
                .unwrap_or_default()
                .contains("For God so loved the world")
    }));
    // Legacy path should send reference with short code to bible-reference-a (param 35)
    assert!(requests.iter().any(|req| {
        req.method.as_str() == "PUT"
            && req.url.path() == "/api/v1/parameter/by-id/35"
            && std::str::from_utf8(&req.body)
                .unwrap_or_default()
                .contains("John 3:16 (KJV)")
    }));
    // Secondary text to bible-translate-a (param 40)
    assert!(requests.iter().any(|req| {
        req.method.as_str() == "PUT"
            && req.url.path() == "/api/v1/parameter/by-id/40"
            && std::str::from_utf8(&req.body)
                .unwrap_or_default()
                .contains("Neboť Bůh tak miloval svět")
    }));
    // Secondary reference with CEP code
    assert!(requests.iter().any(|req| {
        req.method.as_str() == "PUT"
            && req.url.path() == "/api/v1/parameter/by-id/45"
            && std::str::from_utf8(&req.body)
                .unwrap_or_default()
                .contains("(CEP)")
    }));
}

#[tokio::test]
async fn handle_bible_routes_to_clear_when_no_passage() {
    let (server, mut driver, status) = setup_bible_driver().await;

    let update = BibleUpdate {
        passage: None,
        secondary_text: None,
        secondary_translation_code: None,
        slide_output: None,
    };

    driver
        .handle_bible(update, &status)
        .await
        .expect("bible clear");

    let requests = server.received_requests().await.expect("requests");
    // Clear should send empty text to bible-a (param 30)
    let bible_a_puts: Vec<_> = requests
        .iter()
        .filter(|req| {
            req.method.as_str() == "PUT" && req.url.path() == "/api/v1/parameter/by-id/30"
        })
        .collect();
    assert!(!bible_a_puts.is_empty());
    let body: serde_json::Value = serde_json::from_slice(&bible_a_puts[0].body).expect("json");
    assert_eq!(
        body.get("value"),
        Some(&serde_json::Value::String(String::new()))
    );

    // Clear should also trigger the bible-clear clip (clip 500)
    assert!(requests.iter().any(|req| {
        req.method.as_str() == "POST"
            && req.url.path() == "/api/v1/composition/clips/by-id/500/connect"
    }));
}

// ── record_error tests ──────────────────────────────────────────────

#[tokio::test]
async fn record_error_transitions_to_error_state_and_clears_cache() {
    let (_server, mut driver, status) = setup_bible_driver().await;

    // First establish a mapping
    driver.ensure_mapping().await.expect("mapping");
    assert!(driver.mapping.is_some());

    // Record an error
    driver
        .record_error(anyhow::anyhow!("connection refused"), &status)
        .await;

    let snap = status.read().await;
    assert_eq!(snap.state, ResolumeConnectionState::Error);
    assert_eq!(snap.last_error.as_deref(), Some("connection refused"));
    drop(snap);

    // Verify caches are cleared
    assert!(driver.mapping.is_none());
    assert!(driver.endpoint.is_none());
    assert!(driver.last_mapping_refresh.is_none());
    assert!(driver.last_timer_payload.is_none());
}

// ── update_config tests ─────────────────────────────────────────────

#[tokio::test]
async fn update_config_resets_all_cached_state() {
    let (_server, mut driver, _status) = setup_bible_driver().await;

    // Establish cached state
    driver.ensure_mapping().await.expect("mapping");
    driver.last_timer_payload = Some("05:00".to_string());
    driver.last_song_name_payload = Some("Song".to_string());
    driver.last_band_name_payload = Some("Band".to_string());
    assert!(driver.mapping.is_some());
    assert!(driver.endpoint.is_some());

    // Update config
    let new_config = sample_host("192.168.1.100");
    driver.update_config(new_config);

    assert!(driver.mapping.is_none());
    assert!(driver.endpoint.is_none());
    assert!(driver.last_mapping_refresh.is_none());
    assert!(driver.last_timer_payload.is_none());
    assert!(driver.last_song_name_payload.is_none());
    assert!(driver.last_band_name_payload.is_none());
}

// ── update_metadata_targets deduplication test ──────────────────────

#[tokio::test]
async fn update_metadata_targets_deduplicates_same_payload() {
    let (server, mut driver, status) = setup_bible_driver().await;

    let stage_first = StageUpdate {
        current_main: Some("Lyrics".to_string()),
        current_translation: None,
        song_name: Some("Amazing Grace".to_string()),
        band_name: None,
    };
    driver
        .handle_stage(stage_first, &status)
        .await
        .expect("first stage");

    // Send same song name again
    let stage_second = StageUpdate {
        current_main: Some("More lyrics".to_string()),
        current_translation: None,
        song_name: Some("Amazing Grace".to_string()),
        band_name: None,
    };
    driver
        .handle_stage(stage_second, &status)
        .await
        .expect("second stage");

    let requests = server.received_requests().await.expect("requests");
    // Song name (param 60) should only be sent once due to dedup
    let song_puts = count_requests(&requests, "PUT", "/api/v1/parameter/by-id/60");
    assert_eq!(song_puts, 1);
}

#[tokio::test]
async fn update_metadata_targets_sends_new_payload() {
    let (server, mut driver, status) = setup_bible_driver().await;

    let stage_first = StageUpdate {
        current_main: Some("Lyrics".to_string()),
        current_translation: None,
        song_name: Some("Amazing Grace".to_string()),
        band_name: None,
    };
    driver
        .handle_stage(stage_first, &status)
        .await
        .expect("first");

    // Send different song name
    let stage_second = StageUpdate {
        current_main: Some("More lyrics".to_string()),
        current_translation: None,
        song_name: Some("How Great Thou Art".to_string()),
        band_name: None,
    };
    driver
        .handle_stage(stage_second, &status)
        .await
        .expect("second");

    let requests = server.received_requests().await.expect("requests");
    let song_puts = count_requests(&requests, "PUT", "/api/v1/parameter/by-id/60");
    assert_eq!(song_puts, 2);
}

// ── note_latency test ───────────────────────────────────────────────

#[tokio::test]
async fn note_latency_records_in_status() {
    let (_server, driver, status) = setup_bible_driver().await;

    driver
        .note_latency(&status, std::time::Duration::from_millis(42))
        .await;

    let snap = status.read().await;
    assert!(snap.last_latency_ms.is_some());
    let latency = snap.last_latency_ms.expect("latency");
    assert!((latency - 42.0).abs() < 1.0);
}
