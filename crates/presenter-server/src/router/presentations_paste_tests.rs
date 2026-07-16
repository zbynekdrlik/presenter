//! Router-level tests for `POST /presentations/{id}/slides/paste` (#554):
//! a valid paste returns 200 + the new slide list; a stale-id paste returns 422.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use presenter_core::Slide;
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

/// Create a library + a 2-slide presentation via the state layer; return its id
/// and the slide ids.
async fn seed(state: &AppState) -> (String, Vec<String>) {
    let library = state.create_library("Paste Lib").await.unwrap();
    let slides = [
        presenter_core::Slide::new(
            0,
            presenter_core::SlideContent::new(
                presenter_core::SlideText::new("A").unwrap(),
                presenter_core::SlideText::new("").unwrap(),
                presenter_core::SlideText::new("").unwrap(),
                None,
            ),
        ),
        presenter_core::Slide::new(
            1,
            presenter_core::SlideContent::new(
                presenter_core::SlideText::new("B").unwrap(),
                presenter_core::SlideText::new("").unwrap(),
                presenter_core::SlideText::new("").unwrap(),
                None,
            ),
        ),
    ];
    let (_, _, presentation, _) = state
        .create_presentation(library.id, "P", Some(&slides))
        .await
        .unwrap();
    (
        presentation.id.to_string(),
        presentation
            .slides
            .iter()
            .map(|s| s.id.to_string())
            .collect(),
    )
}

#[tokio::test]
async fn paste_valid_ids_returns_200_and_the_new_list() {
    let state = AppState::in_memory().await.unwrap();
    let (pres_id, slide_ids) = seed(&state).await;
    let app = build_router(state);

    // #558 V8: the wire request carries `anchorSlideId` (the slide the gap
    // PRECEDES), not a raw index — anchor on B (slide_ids[1]) to insert
    // right before it.
    let resp = post_json(
        &app,
        &format!("/presentations/{pres_id}/slides/paste"),
        serde_json::json!({ "slideIds": [slide_ids[0]], "anchorSlideId": slide_ids[1] }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let slides: Vec<Slide> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(slides.len(), 3, "one slide pasted → 3 total");
    let ids: Vec<String> = slides.iter().map(|s| s.id.to_string()).collect();
    assert_eq!(ids[0], slide_ids[0], "A stays first, untouched");
    assert_eq!(ids[2], slide_ids[1], "B stays last, untouched");
    assert!(
        ids[1] != slide_ids[0] && ids[1] != slide_ids[1],
        "the middle slot is the CLONE (a fresh id), landing right before the anchor B"
    );
}

#[tokio::test]
async fn paste_with_no_anchor_appends_at_the_end() {
    let state = AppState::in_memory().await.unwrap();
    let (pres_id, slide_ids) = seed(&state).await;
    let app = build_router(state);

    // #558 V8: omitting `anchorSlideId` means "insert at the end".
    let resp = post_json(
        &app,
        &format!("/presentations/{pres_id}/slides/paste"),
        serde_json::json!({ "slideIds": [slide_ids[0]] }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let slides: Vec<Slide> = serde_json::from_slice(&bytes).unwrap();
    let ids: Vec<String> = slides.iter().map(|s| s.id.to_string()).collect();
    assert_eq!(ids.len(), 3, "one slide pasted → 3 total");
    assert_eq!(
        &ids[..2],
        &[slide_ids[0].clone(), slide_ids[1].clone()],
        "the originals are untouched and stay first"
    );
}

#[tokio::test]
async fn paste_unknown_id_returns_422() {
    let state = AppState::in_memory().await.unwrap();
    let (pres_id, slide_ids) = seed(&state).await;
    let app = build_router(state);

    let bogus = uuid::Uuid::new_v4().to_string();
    let resp = post_json(
        &app,
        &format!("/presentations/{pres_id}/slides/paste"),
        serde_json::json!({ "slideIds": [bogus], "anchorSlideId": slide_ids[0] }),
    )
    .await;
    assert_eq!(
        resp.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "a stale clipboard id must be 422, not 500"
    );
}

#[tokio::test]
async fn paste_with_a_vanished_anchor_returns_409() {
    // #558 V8: the anchor slide itself was removed by a concurrent
    // structural edit before this paste landed — fail loudly (409) so the
    // client refreshes instead of silently guessing a position.
    let state = AppState::in_memory().await.unwrap();
    let (pres_id, slide_ids) = seed(&state).await;
    let pres_uuid =
        presenter_core::PresentationId::from_uuid(uuid::Uuid::parse_str(&pres_id).unwrap());
    let anchor_uuid =
        presenter_core::SlideId::from_uuid(uuid::Uuid::parse_str(&slide_ids[1]).unwrap());
    state.delete_slide(pres_uuid, anchor_uuid).await.unwrap();

    let app = build_router(state);
    let resp = post_json(
        &app,
        &format!("/presentations/{pres_id}/slides/paste"),
        serde_json::json!({ "slideIds": [slide_ids[0]], "anchorSlideId": slide_ids[1] }),
    )
    .await;
    assert_eq!(
        resp.status(),
        StatusCode::CONFLICT,
        "a paste anchored on a slide that no longer exists must be 409, not 500"
    );
}
