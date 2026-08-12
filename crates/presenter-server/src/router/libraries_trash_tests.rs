//! #644 route-shape tests for the library trash + restore HTTP surface —
//! mirrors `sync_tests.rs`'s `trash_route_is_not_shadowed_and_restore_round_trips`.
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;

use crate::state::AppState;

async fn get_json(app: axum::Router, uri: &str) -> (StatusCode, Value) {
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("oneshot");
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 256 * 1024)
        .await
        .expect("body");
    let json = serde_json::from_slice(&body).unwrap_or(Value::Null);
    (status, json)
}

async fn post_empty(app: axum::Router, uri: &str) -> StatusCode {
    app.oneshot(
        Request::builder()
            .method(Method::POST)
            .uri(uri)
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .expect("oneshot")
    .status()
}

#[tokio::test]
async fn trash_route_is_not_shadowed_and_restore_round_trips() {
    let state = AppState::in_memory().await.unwrap();
    let app = crate::router::build_router(state.clone());

    // Static route answers (not swallowed by /libraries/{id}).
    let (status, json) = get_json(app.clone(), "/libraries/trash").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "trash route must answer — a 404/400 means {{id}} swallowed it"
    );
    assert_eq!(json, Value::Array(vec![]), "empty trash on a fresh DB");

    // Create + delete a library, see it in the trash, restore it, trash empties.
    let library = state.create_library("Songs").await.unwrap();
    state.delete_library(library.id).await.unwrap();

    let (status, json) = get_json(app.clone(), "/libraries/trash").await;
    assert_eq!(status, StatusCode::OK);
    let rows = json.as_array().expect("array");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["name"], "Songs");

    let status = post_empty(app.clone(), &format!("/libraries/{}/restore", library.id)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (_, json) = get_json(app.clone(), "/libraries/trash").await;
    assert_eq!(
        json,
        Value::Array(vec![]),
        "restored library left the trash"
    );

    // And it is reachable again in the ordinary listing.
    let (status, json) = get_json(app, "/libraries/summary").await;
    assert_eq!(status, StatusCode::OK);
    let libs = json.as_array().expect("array");
    assert!(libs.iter().any(|l| l["name"] == "Songs"));
}

#[tokio::test]
async fn restore_library_maps_missing_to_404_and_not_trashed_to_409() {
    let state = AppState::in_memory().await.unwrap();
    let app = crate::router::build_router(state.clone());

    // Missing entirely → 404, via the centralized RepositoryError::NotFound
    // mapping (#633) — no per-handler helper needed.
    let missing_id = uuid::Uuid::new_v4();
    let status = post_empty(app.clone(), &format!("/libraries/{missing_id}/restore")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Live (never trashed) → 409, via RepositoryError::Conflict.
    let library = state.create_library("Live One").await.unwrap();
    let status = post_empty(app, &format!("/libraries/{}/restore", library.id)).await;
    assert_eq!(status, StatusCode::CONFLICT);
}
