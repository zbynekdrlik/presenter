//! #874: the Prepared tab's order and archive routes, driven through the
//! real router.

use axum::body::{to_bytes, Body};
use axum::http::{Method, Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::router::build_router;
use crate::state::AppState;

async fn call(
    state: &AppState,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(value) => {
            builder = builder.header("content-type", "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    let response = build_router(state.clone())
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json)
}

async fn create(state: &AppState, name: &str) -> String {
    let (status, body) = call(
        state,
        Method::POST,
        "/bible/presentations",
        Some(json!({ "name": name })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    body["id"].as_str().unwrap().to_string()
}

async fn names(state: &AppState, uri: &str) -> Vec<String> {
    let (status, body) = call(state, Method::GET, uri, None).await;
    assert_eq!(status, StatusCode::OK);
    body.as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn reorder_archive_and_restore_through_the_routes() {
    let state = AppState::in_memory().await.unwrap();
    let a = create(&state, "A").await;
    let b = create(&state, "B").await;
    let c = create(&state, "C").await;
    assert_eq!(names(&state, "/bible/presentations").await, ["C", "B", "A"]);

    let (status, _) = call(
        &state,
        Method::PUT,
        "/bible/presentations/order",
        Some(json!({ "ids": [a, b, c] })),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(names(&state, "/bible/presentations").await, ["A", "B", "C"]);

    let (status, _) = call(
        &state,
        Method::POST,
        &format!("/bible/presentations/{b}/archive"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(names(&state, "/bible/presentations").await, ["A", "C"]);
    assert_eq!(
        names(&state, "/bible/presentations?archived=true").await,
        ["B"]
    );

    let (status, _) = call(
        &state,
        Method::POST,
        &format!("/bible/presentations/{b}/restore"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(names(&state, "/bible/presentations").await, ["B", "A", "C"]);
    assert!(names(&state, "/bible/presentations?archived=true")
        .await
        .is_empty());
}

#[tokio::test]
async fn a_stale_order_is_409_and_an_unknown_id_404() {
    let state = AppState::in_memory().await.unwrap();
    let a = create(&state, "A").await;
    create(&state, "B").await;
    let (status, body) = call(
        &state,
        Method::PUT,
        "/bible/presentations/order",
        Some(json!({ "ids": [a] })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    let unknown = uuid::Uuid::new_v4();
    for action in ["archive", "restore"] {
        let (status, _) = call(
            &state,
            Method::POST,
            &format!("/bible/presentations/{unknown}/{action}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{action}");
    }
}

#[tokio::test]
async fn archiving_publishes_the_list_change() {
    let state = AppState::in_memory().await.unwrap();
    let a = create(&state, "A").await;
    let mut events = state.live_hub().subscribe();
    let (status, _) = call(
        &state,
        Method::POST,
        &format!("/bible/presentations/{a}/archive"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let event = events.recv().await.unwrap();
    assert!(
        matches!(&event, crate::live::LiveEvent::BibleSlidesChanged { presentation_id } if *presentation_id == a),
        "{event:?}"
    );
}
