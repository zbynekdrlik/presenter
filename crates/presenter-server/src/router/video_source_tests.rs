//! #789: an NDI video source is identified by its NDI name alone. The REST API
//! accepts `{ndiName}` with no `label`, tolerates (and ignores) a legacy body that
//! still carries one, and never returns a `label`. Full-router tests through
//! `build_router(AppState::in_memory())` + `tower::oneshot`, same harness as
//! `stream_tests.rs`.

use crate::router::build_router;
use crate::state::AppState;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use serde_json::{json, Value};
use tower::ServiceExt;

const BASE: &str = "/integrations/video-sources";

/// Send one request; return `(status, json_body)` (Null body when empty).
async fn req(app: &Router, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let builder = Request::builder().method(method).uri(uri);
    let request = match body {
        Some(b) => builder
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .body(Body::from(b.to_string()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(request).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

#[tokio::test]
async fn create_accepts_ndi_name_alone_and_returns_no_label() {
    let app = build_router(AppState::in_memory().await.unwrap());

    let (status, created) = req(
        &app,
        Method::POST,
        BASE,
        Some(json!({ "ndiName": "RESOLUME-PP (cg-obs)" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {created}");
    assert_eq!(created["ndiName"], "RESOLUME-PP (cg-obs)");
    assert_eq!(created["isActive"], false);
    assert!(
        created.get("label").is_none(),
        "no label on the wire: {created}"
    );

    let (status, list) = req(&app, Method::GET, BASE, None).await;
    assert_eq!(status, StatusCode::OK);
    let row = list
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == created["id"])
        .expect("created source is listed");
    assert_eq!(row["ndiName"], "RESOLUME-PP (cg-obs)");
    assert!(row.get("label").is_none(), "no label in the list: {row}");
}

#[tokio::test]
async fn create_trims_the_ndi_name() {
    let app = build_router(AppState::in_memory().await.unwrap());
    let (status, created) = req(
        &app,
        Method::POST,
        BASE,
        Some(json!({ "ndiName": "  STREAM-SNV (stream)  " })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {created}");
    assert_eq!(created["ndiName"], "STREAM-SNV (stream)");
}

#[tokio::test]
async fn legacy_body_with_label_is_accepted_and_the_label_ignored() {
    let app = build_router(AppState::in_memory().await.unwrap());

    let (status, created) = req(
        &app,
        Method::POST,
        BASE,
        Some(json!({ "label": "Main Camera", "ndiName": "CAM1 (usb)" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {created}");
    assert_eq!(created["ndiName"], "CAM1 (usb)");
    assert!(created.get("label").is_none(), "label ignored: {created}");

    // An EMPTY legacy label used to be refused ("label cannot be empty") — it is
    // simply ignored now.
    let id = created["id"].as_str().unwrap().to_string();
    let (status, updated) = req(
        &app,
        Method::PUT,
        &format!("{BASE}/{id}"),
        Some(json!({ "label": "", "ndiName": "CAM2 (usb)" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {updated}");
    assert_eq!(updated["ndiName"], "CAM2 (usb)");
    assert!(updated.get("label").is_none(), "label ignored: {updated}");
}

#[tokio::test]
async fn update_accepts_ndi_name_alone() {
    let app = build_router(AppState::in_memory().await.unwrap());
    let (_, created) = req(
        &app,
        Method::POST,
        BASE,
        Some(json!({ "ndiName": "CAM1 (usb)" })),
    )
    .await;
    let id = created["id"].as_str().unwrap().to_string();

    let (status, updated) = req(
        &app,
        Method::PUT,
        &format!("{BASE}/{id}"),
        Some(json!({ "ndiName": "CAM3 (hdmi)" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {updated}");
    assert_eq!(updated["ndiName"], "CAM3 (hdmi)");
}

#[tokio::test]
async fn create_with_blank_ndi_name_is_rejected() {
    let app = build_router(AppState::in_memory().await.unwrap());
    let (status, body) = req(&app, Method::POST, BASE, Some(json!({ "ndiName": "   " }))).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a blank NDI name is the client's mistake (422), never a 500: {body}"
    );
    let (_, list) = req(&app, Method::GET, BASE, None).await;
    assert!(
        list.as_array().unwrap().is_empty(),
        "nothing was created: {list}"
    );
}
