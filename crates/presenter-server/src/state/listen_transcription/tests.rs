//! #852 C3-7: the transcription endpoints through the real router, and the
//! store's context read from a real sermon and tip.

use super::*;
use crate::router::build_router;
use crate::state::interpreter_tips::TipOrigin;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use presenter_core::interpreter_tips::NewTipRequest;
use presenter_core::listen::mics::MicConfig;
use presenter_persistence::{NewSermon, SermonItemRecord, SermonWrite, SettingsAuditEntry};
use serde_json::{json, Value};
use tower::ServiceExt;

async fn send(app: &axum::Router, method: Method, uri: &str, body: Value) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("x-forwarded-for", "203.0.113.9")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn guest_mic() -> Vec<MicConfig> {
    serde_json::from_value(
        json!([{"id": "guest", "name": "Hosť", "streamName": "GUEST_EN",
        "senderIp": null, "channel": "left", "language": "en", "role": "guest"}]),
    )
    .unwrap()
}

async fn audit(state: &AppState, key: &str) -> Vec<SettingsAuditEntry> {
    state
        .repository()
        .list_settings_audit(Some("app_settings"), Some(key), None, 10)
        .await
        .unwrap()
}

#[tokio::test]
async fn put_transcription_validates_and_audits() {
    let state = AppState::in_memory().await.unwrap();
    state.listen.apply(guest_mic());
    let app = build_router(state.clone());
    let uri = "/listen/transcription";
    let (status, body) = send(
        &app,
        Method::PUT,
        uri,
        json!({"enabled": true, "guestMic": "guest", "dailyCapMinutes": 5}),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body.to_string().contains("10 až 600 minút"), "{body}");
    let (status, body) = send(
        &app,
        Method::PUT,
        uri,
        json!({"enabled": true, "guestMic": "nobody"}),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body.to_string().contains("nobody"), "{body}");
    assert!(
        audit(&state, TRANSCRIPTION_KEY).await.is_empty(),
        "a refusal stores nothing"
    );
    let row = json!({"enabled": true, "guestMic": "guest", "dailyCapMinutes": 120});
    let (status, body) = send(&app, Method::PUT, uri, row.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["setting"], row);
    assert_eq!(body["mode"], "auto");
    let stored = state
        .repository()
        .get_app_setting(TRANSCRIPTION_KEY)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(serde_json::from_str::<Value>(&stored).unwrap(), row);
    let rows = audit(&state, TRANSCRIPTION_KEY).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].actor, "203.0.113.9");
    assert_eq!(rows[0].source, SettingsAuditSource::HttpSetter);
    assert_eq!(rows[0].after_json, row);
    // The chosen mic goes missing: the switch can still go off (spec gap 7).
    state.listen.apply(Vec::new());
    let off = json!({"enabled": false, "guestMic": "guest", "dailyCapMinutes": 120});
    let (status, _) = send(&app, Method::PUT, uri, off).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(audit(&state, TRANSCRIPTION_KEY).await.len(), 2);
}

#[tokio::test]
async fn put_session_sets_todays_mode() {
    let state = AppState::in_memory().await.unwrap();
    let app = build_router(state.clone());
    let (status, body) = send(&app, Method::PUT, "/listen/session", json!({"mode": "on"})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["mode"], "on");
    let stored = state
        .repository()
        .get_app_setting(SESSION_KEY)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::from_str::<SessionOverride>(&stored).unwrap(),
        SessionOverride {
            day: state.listen.today(),
            mode: SessionMode::On,
            extra_minutes: 0
        }
    );
    let (status, body) = send(
        &app,
        Method::PUT,
        "/listen/session",
        json!({"mode": "auto"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["mode"], "auto");
    assert_eq!(audit(&state, SESSION_KEY).await.len(), 2);
    let (status, _) = send(
        &app,
        Method::PUT,
        "/listen/session",
        json!({"mode": "later"}),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(audit(&state, SESSION_KEY).await.len(), 2);
}

#[tokio::test]
async fn status_reports_key_state_without_the_key() {
    let state = AppState::in_memory().await.unwrap();
    let app = build_router(state.clone());
    let request = Request::builder()
        .uri("/listen/status")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    let transcription = &body["transcription"];
    assert_eq!(transcription["keyConfigured"], json!(false));
    assert_eq!(transcription["model"], "stt-rt-v6");
    assert_eq!(transcription["setting"]["enabled"], json!(false));
    assert_eq!(transcription["mics"], json!([]));
    let fields: Vec<&String> = transcription.as_object().unwrap().keys().collect();
    assert!(
        !fields.iter().any(|f| *f == "key" || *f == "apiKey"),
        "{fields:?}"
    );
}

#[tokio::test]
async fn the_context_reads_todays_sermon_references_and_active_tip_terms() {
    let state = AppState::in_memory().await.unwrap();
    let today = state.listen.today();
    let created = state
        .repository()
        .create_sermon(&NewSermon {
            title: "Kázeň".to_string(),
            service_date: today,
            default_translation_code: "slk-seb".to_string(),
            secondary_translation_code: None,
            author_email: None,
        })
        .await
        .unwrap();
    let item = SermonItemRecord {
        id: uuid::Uuid::new_v4(),
        translation_code: "slk-seb".to_string(),
        book_code: "HEB".to_string(),
        book_number: 58,
        book_name: "Hebrejom".to_string(),
        chapter: 11,
        verse_start: 1,
        verse_end: 3,
        source: Vec::new(),
        edits: Default::default(),
        slide_count: 0,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    let write = SermonWrite {
        expected_revision: 0,
        title: "Kázeň".to_string(),
        service_date: today,
        secondary_translation_code: None,
        items: vec![item],
        slides: Vec::new(),
    };
    state
        .repository()
        .save_sermon_change(created.id, &write)
        .await
        .unwrap();
    let tip = NewTipRequest {
        sender_name: "Šepkár".to_string(),
        source_term: Some("Shuttlesworth".to_string()),
        target_term: Some("Shuttlesworth".to_string()),
        note: None,
    };
    let origin = TipOrigin {
        client_ip: "203.0.113.9".to_string(),
        user_agent: None,
    };
    let _created = state.create_interpreter_tip(&tip, origin).await.unwrap();
    let inputs = state.context_inputs(today).await.unwrap();
    assert_eq!(inputs.prepared_refs, ["Hebrews 11:1-3"]);
    assert_eq!(inputs.tip_terms, ["Shuttlesworth"]);
}
