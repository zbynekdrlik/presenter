//! Router + filesystem integration tests for the stream-asset pipeline (#708).
//! Each test builds an `AppState::in_memory()` pointed at its own `TempDir`, so
//! the on-disk store is isolated even though the shared-cache in-memory DB is
//! not — assertions therefore target the specific ids/bytes a test created,
//! never global counts.

use crate::router::build_router;
use crate::state::AppState;
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use presenter_core::stream::{Frame, ImageFit, SceneKind, StreamAsset, StreamElementProps};
use tower::ServiceExt;

const BOUNDARY: &str = "TESTBOUNDARY708";

/// An `AppState` whose asset store is an isolated temp dir. The returned
/// `TempDir` guard MUST be kept alive for the test's duration (dropping it
/// removes the directory).
async fn test_state() -> (AppState, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut state = AppState::in_memory().await.expect("in_memory state");
    state.set_stream_assets_dir(dir.path().join("stream-assets"));
    (state, dir)
}

/// A minimal but structurally valid PNG (signature + IHDR) with real
/// dimensions — enough for magic-byte detection AND header dimension parsing.
fn png_bytes(w: u32, h: u32) -> Vec<u8> {
    let mut v = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    v.extend_from_slice(&[0, 0, 0, 13]);
    v.extend_from_slice(b"IHDR");
    v.extend_from_slice(&w.to_be_bytes());
    v.extend_from_slice(&h.to_be_bytes());
    v.extend_from_slice(&[8, 6, 0, 0, 0]);
    v
}

/// Build a `multipart/form-data` body with one `file` field.
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

fn upload_request(filename: &str, declared_ct: &str, bytes: &[u8]) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/stream/assets")
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .body(Body::from(multipart_body(filename, declared_ct, bytes)))
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

/// Upload the given bytes via the real route and return the created asset.
async fn upload(state: &AppState, filename: &str, ct: &str, bytes: &[u8]) -> StreamAsset {
    let response = build_router(state.clone())
        .oneshot(upload_request(filename, ct, bytes))
        .await
        .expect("upload");
    assert_eq!(response.status(), StatusCode::OK, "upload should succeed");
    serde_json::from_value(body_json(response).await).expect("StreamAsset")
}

#[tokio::test]
async fn upload_png_stores_row_and_file_with_dimensions() {
    let (state, _dir) = test_state().await;
    let bytes = png_bytes(1920, 1080);

    let asset = upload(&state, "logo.png", "image/png", &bytes).await;

    assert_eq!(asset.mime, "image/png");
    assert_eq!(asset.size_bytes, bytes.len() as i64);
    assert_eq!(asset.width, Some(1920));
    assert_eq!(asset.height, Some(1080));
    assert_eq!(asset.sha256.len(), 64);

    // The file physically exists on disk under <sha>.png.
    let path = state
        .stream_assets_dir()
        .join(format!("{}.png", asset.sha256));
    assert!(path.exists(), "asset file written to disk: {path:?}");

    // The row is retrievable.
    let fetched = state.repository().get_stream_asset(asset.id).await.unwrap();
    assert_eq!(fetched.sha256, asset.sha256);
}

#[tokio::test]
async fn re_upload_same_bytes_dedups_to_same_asset() {
    let (state, _dir) = test_state().await;
    let bytes = png_bytes(640, 480);

    let first = upload(&state, "a.png", "image/png", &bytes).await;
    let second = upload(&state, "b-different-name.png", "image/png", &bytes).await;

    assert_eq!(first.id, second.id, "identical bytes dedup to one asset id");
    assert_eq!(first.sha256, second.sha256);
}

#[tokio::test]
async fn upload_with_bad_magic_bytes_is_rejected_422() {
    let (state, _dir) = test_state().await;
    // Declares image/png but the bytes are not any accepted image.
    let response = build_router(state.clone())
        .oneshot(upload_request(
            "fake.png",
            "image/png",
            b"this is not an image",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn upload_over_20_mib_is_rejected_413() {
    let (state, _dir) = test_state().await;
    // 21 MiB of valid-PNG-prefixed bytes: passes the body-limit layer (24 MiB)
    // and reaches the handler, which enforces the 20 MiB business cap.
    let mut bytes = png_bytes(8, 8);
    bytes.resize(21 * 1024 * 1024, 0);
    let response = build_router(state.clone())
        .oneshot(upload_request("huge.png", "image/png", &bytes))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn serve_returns_bytes_with_immutable_cache_header() {
    let (state, _dir) = test_state().await;
    let bytes = png_bytes(32, 32);
    let asset = upload(&state, "s.png", "image/png", &bytes).await;

    let response = build_router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/stream/assets/{}", asset.id))
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
        "image/png"
    );
    assert_eq!(
        body_bytes(response).await,
        bytes,
        "served bytes match upload"
    );
}

#[tokio::test]
async fn serve_missing_asset_is_404() {
    let (state, _dir) = test_state().await;
    let response = build_router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/stream/assets/999999")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn list_contains_the_uploaded_asset() {
    let (state, _dir) = test_state().await;
    let asset = upload(&state, "l.png", "image/png", &png_bytes(10, 20)).await;

    let response = build_router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/stream/api/assets")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let list: Vec<StreamAsset> = serde_json::from_value(body_json(response).await).unwrap();
    assert!(
        list.iter()
            .any(|a| a.id == asset.id && a.sha256 == asset.sha256),
        "list includes the uploaded asset"
    );
}

#[tokio::test]
async fn delete_unreferenced_removes_row_and_file() {
    let (state, _dir) = test_state().await;
    let asset = upload(&state, "d.png", "image/png", &png_bytes(16, 16)).await;
    let path = state
        .stream_assets_dir()
        .join(format!("{}.png", asset.sha256));
    assert!(path.exists());

    let response = build_router(state.clone())
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/stream/assets/{}", asset.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    assert!(!path.exists(), "file removed from disk");
    // Row gone → serve 404.
    let serve = build_router(state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/stream/assets/{}", asset.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(serve.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_referenced_asset_is_409_naming_the_scene() {
    let (state, _dir) = test_state().await;
    let asset = upload(&state, "used.png", "image/png", &png_bytes(24, 24)).await;

    // Seed a base scene with an image element referencing the asset.
    let scene = state
        .repository()
        .create_stream_scene("stream", "Referenced Scene 708", SceneKind::Base)
        .await
        .unwrap();
    state
        .repository()
        .create_stream_element(
            scene.id,
            StreamElementProps::Image {
                asset_id: asset.id,
                fit: ImageFit::Contain,
                frame: Frame {
                    x_pct: 0.0,
                    y_pct: 0.0,
                    w_pct: 50.0,
                    h_pct: 50.0,
                },
                opacity: 1.0,
            },
        )
        .await
        .unwrap();

    let response = build_router(state.clone())
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/stream/assets/{}", asset.id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = String::from_utf8(body_bytes(response).await).unwrap();
    assert!(
        body.contains("Referenced Scene 708"),
        "409 body names the referencing scene: {body}"
    );

    // The file must still be on disk — a refused delete removes nothing.
    let path = state
        .stream_assets_dir()
        .join(format!("{}.png", asset.sha256));
    assert!(path.exists(), "referenced asset file is untouched");
}

#[tokio::test]
async fn delete_missing_asset_is_404() {
    let (state, _dir) = test_state().await;
    let response = build_router(state.clone())
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/stream/assets/424242")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn upload_missing_file_field_is_400() {
    let (state, _dir) = test_state().await;
    // A multipart body whose only field is named "other", not "file".
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
    body.extend_from_slice(b"Content-Disposition: form-data; name=\"other\"\r\n\r\n");
    body.extend_from_slice(b"x");
    body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
    let response = build_router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/stream/assets")
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={BOUNDARY}"),
                )
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
