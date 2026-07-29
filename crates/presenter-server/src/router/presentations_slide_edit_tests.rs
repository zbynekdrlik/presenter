//! Router-level regression tests for the slide-edit typed-refusal sweep (#611).
//!
//! Before #611, `state/slides/edit_ops.rs` raised bare `anyhow::anyhow!("...
//! not found")` at five sites — `lock_and_read_presentation_for_edit` (shared
//! by every slide-edit op) and four slide-lookups. Those fell through the
//! router's blanket `impl From<anyhow::Error> for AppError` (`router.rs:599`)
//! as an opaque **500**, even though the correct response is 404 (URL
//! resource missing) or 422 (a body-referenced slide missing). These tests
//! drive the REAL registered routes through `build_router` + `tower::oneshot`
//! — the non-vacuous pattern from `presentations_copy_tests.rs` — so a wrong
//! URI cannot make axum itself return 404 and the assertion pass vacuously.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use crate::router::build_router;
use crate::state::AppState;

async fn request(
    app: &axum::Router,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
) -> axum::http::Response<Body> {
    let mut builder = Request::builder().method(method).uri(uri);
    let request = match body {
        Some(json) => {
            builder = builder.header("content-type", "application/json");
            builder.body(Body::from(json.to_string())).unwrap()
        }
        None => builder.body(Body::empty()).unwrap(),
    };
    app.clone().oneshot(request).await.unwrap()
}

/// Seeds a library + presentation with two slides; returns
/// `(presentation_id, [slide_id, slide_id])` as UUID strings.
async fn seed(state: &AppState) -> (String, [String; 2]) {
    let library = state.create_library("Edit Refusal Lib").await.unwrap();
    let mk = |main: &str| {
        presenter_core::Slide::new(
            0,
            presenter_core::SlideContent::new(
                presenter_core::SlideText::new(main).unwrap(),
                presenter_core::SlideText::new("").unwrap(),
                presenter_core::SlideText::new("").unwrap(),
                None,
            ),
        )
    };
    let (_, _, presentation, _) = state
        .create_presentation(library.id, "Edit Refusal Song", Some(&[mk("A"), mk("B")]))
        .await
        .unwrap();
    let ids: [String; 2] = [
        presentation.slides[0].id.to_string(),
        presentation.slides[1].id.to_string(),
    ];
    (presentation.id.to_string(), ids)
}

fn fresh_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

// --- duplicate_slide: POST /presentations/{pid}/slides/{sid}/duplicate ---

#[tokio::test]
async fn duplicate_on_a_vanished_presentation_is_a_404() {
    let state = AppState::in_memory().await.unwrap();
    let app = build_router(state);
    let resp = request(
        &app,
        "POST",
        &format!("/presentations/{}/slides/{}/duplicate", fresh_uuid(), fresh_uuid()),
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn duplicate_of_a_vanished_slide_is_a_404() {
    let state = AppState::in_memory().await.unwrap();
    let (pres_id, _) = seed(&state).await;
    let app = build_router(state);
    let resp = request(
        &app,
        "POST",
        &format!("/presentations/{pres_id}/slides/{}/duplicate", fresh_uuid()),
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// --- delete_slide: DELETE /presentations/{pid}/slides/{sid} ---

#[tokio::test]
async fn delete_on_a_vanished_presentation_is_a_404() {
    let state = AppState::in_memory().await.unwrap();
    let app = build_router(state);
    let resp = request(
        &app,
        "DELETE",
        &format!("/presentations/{}/slides/{}", fresh_uuid(), fresh_uuid()),
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_of_a_vanished_slide_is_a_404() {
    let state = AppState::in_memory().await.unwrap();
    let (pres_id, _) = seed(&state).await;
    let app = build_router(state);
    let resp = request(
        &app,
        "DELETE",
        &format!("/presentations/{pres_id}/slides/{}", fresh_uuid()),
        None,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// --- update_slide_content: PATCH /presentations/{pid}/slides/{sid} ---

#[tokio::test]
async fn update_content_on_a_vanished_presentation_is_a_404() {
    let state = AppState::in_memory().await.unwrap();
    let app = build_router(state);
    let body = serde_json::json!({
        "main": "x", "translation": "", "stage": ""
    });
    let resp = request(
        &app,
        "PATCH",
        &format!("/presentations/{}/slides/{}", fresh_uuid(), fresh_uuid()),
        Some(body),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn update_content_of_a_vanished_slide_is_a_404() {
    let state = AppState::in_memory().await.unwrap();
    let (pres_id, _) = seed(&state).await;
    let app = build_router(state);
    let body = serde_json::json!({
        "main": "x", "translation": "", "stage": ""
    });
    let resp = request(
        &app,
        "PATCH",
        &format!("/presentations/{pres_id}/slides/{}", fresh_uuid()),
        Some(body),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// --- reorder_slides: POST /presentations/{pid}/slides/reorder ---

#[tokio::test]
async fn reorder_on_a_vanished_presentation_is_a_404() {
    let state = AppState::in_memory().await.unwrap();
    let app = build_router(state);
    let body = serde_json::json!({ "slideIds": [fresh_uuid()] });
    let resp = request(
        &app,
        "POST",
        &format!("/presentations/{}/slides/reorder", fresh_uuid()),
        Some(body),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn reorder_naming_an_unknown_slide_in_the_body_is_a_422() {
    // The `slideIds` are BODY-referenced (not in the URL path), so a slide id
    // that does not belong to the presentation is a body-referenced missing
    // target → 422 (TargetNotFound), not 404 — matching the established
    // `paste_slides` UnknownSlides→422 convention in the same module. The
    // bogus id is sent ALONGSIDE a real one with matching length so the
    // request reaches the per-slide lookup (`:452`), not the earlier
    // length-mismatch guard (`:446`).
    let state = AppState::in_memory().await.unwrap();
    let (pres_id, [real_a, _real_b]) = seed(&state).await;
    let app = build_router(state);
    let body = serde_json::json!({ "slideIds": [real_a, fresh_uuid()] });
    let resp = request(
        &app,
        "POST",
        &format!("/presentations/{pres_id}/slides/reorder"),
        Some(body),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

// --- paste_slides: POST /presentations/{pid}/slides/paste ---
// (paste's own UnknownSlides/AnchorVanished semantics are covered by
// `presentations_paste_tests.rs`; this only nails the shared
// vanished-presentation path that used to be a 500.)

#[tokio::test]
async fn paste_on_a_vanished_presentation_is_a_404() {
    let state = AppState::in_memory().await.unwrap();
    let app = build_router(state);
    let body = serde_json::json!({ "slideIds": [fresh_uuid()] });
    let resp = request(
        &app,
        "POST",
        &format!("/presentations/{}/slides/paste", fresh_uuid()),
        Some(body),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
