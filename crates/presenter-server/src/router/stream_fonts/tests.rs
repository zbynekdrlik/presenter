//! Router + filesystem integration tests for the stream-font pipeline (#778),
//! mirroring `router/stream_assets/tests.rs`. Each test builds an
//! `AppState::in_memory()` pointed at its own `TempDir` (the font store is a
//! `fonts/` subdir of that), so the on-disk store is isolated even though the
//! shared-cache in-memory DB is not — assertions target the specific ids/bytes a
//! test created, never global counts.

use crate::router::build_router;
use crate::state::AppState;
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use presenter_core::stream::{SceneKind, StreamElementProps, StreamFont, TextAlign, TextStyle};
use tower::ServiceExt;

const BOUNDARY: &str = "TESTBOUNDARY778";

/// The committed OFL fixture font (family "Gruppo"), shared with the Rust
/// metadata test and the Playwright E2E — one licence-clean font for all layers.
const FIXTURE_TTF: &[u8] =
    include_bytes!("../../../../../tests/e2e/fixtures/fonts/Gruppo-Regular.ttf");

async fn test_state() -> (AppState, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut state = AppState::in_memory().await.expect("in_memory state");
    state.set_stream_assets_dir(dir.path().join("stream-assets"));
    (state, dir)
}

fn multipart_body(filename: &str, declared_ct: &str, bytes: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
    body.extend_from_slice(
        format!("Content-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n")
            .as_bytes(),
    );
    body.extend_from_slice(format!("Content-Type: {declared_ct}\r\n\r\n").as_bytes());
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
    body
}

fn upload_request(filename: &str, ct: &str, bytes: &[u8]) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/stream/fonts")
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .body(Body::from(multipart_body(filename, ct, bytes)))
        .expect("request")
}

async fn body_bytes(response: axum::response::Response) -> Vec<u8> {
    axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body")
        .to_vec()
}

async fn body_json(response: axum::response::Response) -> serde_json::Value {
    serde_json::from_slice(&body_bytes(response).await).expect("json")
}

async fn upload(state: &AppState, filename: &str, bytes: &[u8]) -> StreamFont {
    let response = build_router(state.clone())
        .oneshot(upload_request(filename, "font/ttf", bytes))
        .await
        .expect("upload");
    assert_eq!(response.status(), StatusCode::OK, "upload should succeed");
    serde_json::from_value(body_json(response).await).expect("StreamFont")
}

fn text_style(family: &str) -> TextStyle {
    TextStyle {
        font_family: family.to_string(),
        size_pct: 8.0,
        color: "#ffffff".to_string(),
        weight: 400,
        align: TextAlign::Center,
        line_height: 1.2,
        shadow: None,
    }
}

#[tokio::test]
async fn upload_ttf_stores_row_and_file_with_metadata() {
    let (state, _dir) = test_state().await;
    let font = upload(&state, "Gruppo-Regular.ttf", FIXTURE_TTF).await;

    assert_eq!(font.family, "Gruppo", "family parsed from the name table");
    assert_eq!(font.format, "ttf");
    assert!(!font.italic);
    assert!((1..=1000).contains(&font.weight), "weight {}", font.weight);
    assert_eq!(font.size_bytes, FIXTURE_TTF.len() as i64);
    assert_eq!(font.sha256.len(), 64);

    let path = state
        .stream_assets_dir()
        .join("fonts")
        .join(format!("{}.ttf", font.sha256));
    assert!(path.exists(), "font file written to disk: {path:?}");

    let fetched = state.repository().get_stream_font(font.id).await.unwrap();
    assert_eq!(fetched.sha256, font.sha256);
}

#[tokio::test]
async fn re_upload_same_bytes_dedups() {
    let (state, _dir) = test_state().await;
    let first = upload(&state, "a.ttf", FIXTURE_TTF).await;
    let second = upload(&state, "b-other-name.ttf", FIXTURE_TTF).await;
    assert_eq!(first.id, second.id, "identical bytes dedup to one font id");
    assert_eq!(first.sha256, second.sha256);
}

#[tokio::test]
async fn upload_garbage_is_422() {
    let (state, _dir) = test_state().await;
    let response = build_router(state.clone())
        .oneshot(upload_request(
            "fake.ttf",
            "font/ttf",
            b"this is not a font",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn upload_ttc_collection_is_422() {
    let (state, _dir) = test_state().await;
    // A TrueType Collection ("ttcf") — a browser cannot @font-face it.
    let mut bytes = b"ttcf".to_vec();
    bytes.extend_from_slice(&[0u8; 64]);
    let response = build_router(state.clone())
        .oneshot(upload_request("coll.ttc", "font/collection", &bytes))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn upload_woff2_is_422() {
    let (state, _dir) = test_state().await;
    let mut bytes = b"wOF2".to_vec();
    bytes.extend_from_slice(&[0u8; 64]);
    let response = build_router(state.clone())
        .oneshot(upload_request("web.woff2", "font/woff2", &bytes))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn upload_over_5mib_is_413() {
    let (state, _dir) = test_state().await;
    // Valid ttf magic then padded past the 5 MiB business cap (under the raw
    // body-limit layer so the handler's precise 413 fires).
    let mut bytes = vec![0x00, 0x01, 0x00, 0x00];
    bytes.resize(5 * 1024 * 1024 + 16, 0);
    let response = build_router(state.clone())
        .oneshot(upload_request("huge.ttf", "font/ttf", &bytes))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn list_contains_the_uploaded_font() {
    let (state, _dir) = test_state().await;
    let font = upload(&state, "l.ttf", FIXTURE_TTF).await;
    let response = build_router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/stream/api/fonts")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let list: Vec<StreamFont> = serde_json::from_value(body_json(response).await).unwrap();
    assert!(list.iter().any(|f| f.id == font.id && f.family == "Gruppo"));
}

#[tokio::test]
async fn serve_returns_bytes_with_immutable_cache_and_font_mime() {
    let (state, _dir) = test_state().await;
    let font = upload(&state, "s.ttf", FIXTURE_TTF).await;
    let response = build_router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/stream/fonts/{}", font.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL).unwrap(),
        "public, max-age=31536000, immutable"
    );
    assert_eq!(
        response.headers().get(header::CONTENT_TYPE).unwrap(),
        "font/ttf"
    );
    assert_eq!(
        response
            .headers()
            .get(header::X_CONTENT_TYPE_OPTIONS)
            .unwrap(),
        "nosniff"
    );
    assert_eq!(
        body_bytes(response).await,
        FIXTURE_TTF,
        "served bytes match"
    );
}

#[tokio::test]
async fn serve_missing_font_is_404() {
    let (state, _dir) = test_state().await;
    let response = build_router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/stream/fonts/999999")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_unused_font_removes_row_and_file() {
    let (state, _dir) = test_state().await;
    let font = upload(&state, "d.ttf", FIXTURE_TTF).await;
    let path = state
        .stream_assets_dir()
        .join("fonts")
        .join(format!("{}.ttf", font.sha256));
    assert!(path.exists());

    let response = build_router(state.clone())
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/stream/fonts/{}", font.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(!path.exists(), "file removed from disk");
}

#[tokio::test]
async fn delete_last_in_use_font_is_409_naming_scene() {
    let (state, _dir) = test_state().await;
    let font = upload(&state, "used.ttf", FIXTURE_TTF).await;

    // Now that "Gruppo" is an uploaded family, an element may use it.
    let scene = state
        .repository()
        .create_stream_scene("stream", "Uses Gruppo 778", SceneKind::Base)
        .await
        .unwrap();
    state
        .repository()
        .create_stream_element(
            scene.id,
            StreamElementProps::Countdown {
                timer_id: 1,
                style: text_style("Gruppo"),
                frame: presenter_core::stream::Frame {
                    x_pct: 0.0,
                    y_pct: 0.0,
                    w_pct: 50.0,
                    h_pct: 50.0,
                },
                content_transition: presenter_core::stream::ContentTransition::default(),
            },
        )
        .await
        .unwrap();

    let response = build_router(state.clone())
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/stream/fonts/{}", font.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = String::from_utf8(body_bytes(response).await).unwrap();
    assert!(
        body.contains("Uses Gruppo 778"),
        "409 body names the referencing scene: {body}"
    );

    // A refused delete removes nothing.
    let path = state
        .stream_assets_dir()
        .join("fonts")
        .join(format!("{}.ttf", font.sha256));
    assert!(path.exists(), "referenced font file is untouched");
}

#[tokio::test]
async fn fonts_css_has_face_rule_and_etag_revalidation() {
    let (state, _dir) = test_state().await;
    let font = upload(&state, "css.ttf", FIXTURE_TTF).await;

    let response = build_router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/stream/fonts.css")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL).unwrap(),
        "no-cache"
    );
    let etag = response
        .headers()
        .get(header::ETAG)
        .expect("etag present")
        .to_str()
        .unwrap()
        .to_string();
    let css = String::from_utf8(body_bytes(response).await).unwrap();
    assert!(css.contains("@font-face"), "css has a face rule: {css}");
    assert!(css.contains("\"Gruppo\""), "css names the family: {css}");
    assert!(
        css.contains(&format!("/stream/fonts/{}", font.id)),
        "css src points at the serve route: {css}"
    );
    assert!(
        css.contains("format(\"truetype\")"),
        "ttf format hint: {css}"
    );

    // A conditional GET with the same ETag → 304 (revalidation, no transfer).
    let cond = build_router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/stream/fonts.css")
                .header(header::IF_NONE_MATCH, &etag)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cond.status(), StatusCode::NOT_MODIFIED);
}
