use super::clip_map::ClipMapping;
use super::driver::HostDriver;
use super::types::TextTransform;
use super::{ResolumeConnectionSnapshot, TimerFrame, DEFAULT_TIMEOUT};
use chrono::Utc;
use presenter_core::{ResolumeHost, ResolumeHostId};
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
