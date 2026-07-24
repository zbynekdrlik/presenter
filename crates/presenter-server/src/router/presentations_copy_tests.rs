//! Router-level tests for `POST /presentations/{id}/copy` (#570): a valid
//! copy returns 201 + the copy's detail; a vanished target library or a
//! vanished presentation returns 404, not an opaque 500.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use crate::router::build_router;
use crate::state::AppState;

async fn post_json(
    app: &axum::Router,
    uri: &str,
    body: serde_json::Value,
) -> axum::http::Response<Body> {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn body_json(response: axum::http::Response<Body>) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// Source library + one-slide presentation + a target library; returns
/// (presentation_id, target_library_id).
async fn seed(state: &AppState) -> (String, String) {
    let source_lib = state.create_library("Source Lib").await.unwrap();
    let target_lib = state.create_library("Target Lib").await.unwrap();
    let slides = [presenter_core::Slide::new(
        0,
        presenter_core::SlideContent::new(
            presenter_core::SlideText::new("verse").unwrap(),
            presenter_core::SlideText::new("").unwrap(),
            presenter_core::SlideText::new("").unwrap(),
            None,
        ),
    )];
    let (_, _, presentation, _) = state
        .create_presentation(source_lib.id, "Song", Some(&slides))
        .await
        .unwrap();
    (presentation.id.to_string(), target_lib.id.to_string())
}

#[tokio::test]
async fn copy_returns_201_with_the_new_presentation_in_the_target_library() {
    let state = AppState::in_memory().await.unwrap();
    let (pres_id, target_lib_id) = seed(&state).await;
    let app = build_router(state);

    let response = post_json(
        &app,
        &format!("/presentations/{pres_id}/copy"),
        serde_json::json!({ "targetLibraryId": target_lib_id }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);

    let json = body_json(response).await;
    assert_eq!(json["libraryId"], target_lib_id);
    assert_eq!(json["libraryName"], "Target Lib");
    assert_eq!(json["presentation"]["name"], "Song");
    assert_ne!(
        json["presentation"]["id"], pres_id,
        "the copy must have its own id"
    );
}

#[tokio::test]
async fn copy_into_a_vanished_library_is_a_404() {
    let state = AppState::in_memory().await.unwrap();
    let (pres_id, _) = seed(&state).await;
    let app = build_router(state);

    let response = post_json(
        &app,
        &format!("/presentations/{pres_id}/copy"),
        serde_json::json!({ "targetLibraryId": uuid::Uuid::new_v4().to_string() }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn copying_a_vanished_presentation_is_a_404() {
    let state = AppState::in_memory().await.unwrap();
    let (_, target_lib_id) = seed(&state).await;
    let app = build_router(state);

    let response = post_json(
        &app,
        &format!("/presentations/{}/copy", uuid::Uuid::new_v4()),
        serde_json::json!({ "targetLibraryId": target_lib_id }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
