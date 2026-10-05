//! Mock Resolume Arena HTTP listener for dev. Listens on
//! `127.0.0.1:8091`, accepts the endpoints presenter-server's outbound
//! resolume driver calls, returns minimal-valid responses, and records
//! every request to the shared `RequestLog`.
//!
//! #808: the composition has tagged clips and one selected deck, like a real
//! Arena. An empty composition would look like Arena still loading, and the
//! driver would re-read it on its follow-up schedule after every fetch.

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Context;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
    routing::{get, post, put},
    Router,
};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tracing::{info, warn};

use super::request_log::{log_handler, RequestLog};

const MOCK_RESOLUME_ADDR: &str = "127.0.0.1:8091";
const MOCK_NAME: &str = "resolume";
/// The mock composition's only deck, always selected.
const MOCK_DECK_ID: i64 = 1;

fn router(log: Arc<RequestLog>) -> Router {
    Router::new()
        .route("/api/v1/composition", get(get_composition))
        .route("/api/v1/composition/decks/by-id/{id}", get(get_deck))
        .route("/api/v1/parameter/by-id/{id}", put(put_parameter))
        .route(
            "/api/v1/composition/clips/by-id/{id}/connect",
            post(post_clip_connect),
        )
        .route("/api/v1/product", get(get_product))
        .route("/__mock/log", get(log_handler))
        .with_state(log)
}

/// Spawn the mock Resolume listener in a background task.
pub async fn spawn(log: Arc<RequestLog>) -> anyhow::Result<()> {
    let app = router(log);

    let addr: SocketAddr = MOCK_RESOLUME_ADDR
        .parse()
        .context("invalid MOCK_RESOLUME_ADDR")?;
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("mock-resolume failed to bind {addr}"))?;
    info!(%addr, "mock-resolume listener started");

    tokio::spawn(async move {
        if let Err(err) = axum::serve(listener, app).await {
            warn!(?err, "mock-resolume listener exited");
        }
    });
    Ok(())
}

/// One clip of the mock composition: `name` carries the presenter tag, and
/// `param_id` is its text parameter (a clear clip has none).
fn mock_clip(id: i64, name: &str, param_id: Option<i64>) -> Value {
    let sourceparams = match param_id {
        Some(param_id) => json!({ "text": { "valuetype": "ParamText", "id": param_id } }),
        None => json!({}),
    };
    json!({ "id": id, "name": { "value": name }, "video": { "sourceparams": sourceparams } })
}

fn mock_deck() -> Value {
    json!({ "id": MOCK_DECK_ID, "name": { "value": "Mock Deck" }, "selected": { "value": true } })
}

/// `GET /api/v1/composition` — a small valid composition: one selected deck
/// and one clip per lyric/Bible lane plus the song and band names. No
/// `#timer` clip, so a running countdown does not write the log every second.
async fn get_composition(State(log): State<Arc<RequestLog>>) -> Json<Value> {
    log.record(MOCK_NAME, "GET", "/api/v1/composition", None);
    let clips = [
        mock_clip(101, "#main-a", Some(1001)),
        mock_clip(102, "#main-b", Some(1002)),
        mock_clip(103, "#translate-a", Some(1003)),
        mock_clip(104, "#translate-b", Some(1004)),
        mock_clip(105, "#bible-a", Some(1005)),
        mock_clip(106, "#bible-b", Some(1006)),
        mock_clip(107, "#bible-reference-a", Some(1007)),
        mock_clip(108, "#bible-reference-b", Some(1008)),
        mock_clip(109, "#bible-clear", None),
        mock_clip(110, "#song-name", Some(1010)),
        mock_clip(111, "#band-name", Some(1011)),
    ];
    Json(json!({
        "name": "Mock Composition",
        "decks": [mock_deck()],
        "layers": [ { "clips": clips } ],
        "columns": [],
    }))
}

/// `GET /api/v1/composition/decks/by-id/:id` — #808: the driver's deck check
/// before every push. The one mock deck is always selected; any other id is a
/// 404, as on Arena for a deck that no longer exists.
async fn get_deck(
    State(log): State<Arc<RequestLog>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    log.record(
        MOCK_NAME,
        "GET",
        &format!("/api/v1/composition/decks/by-id/{id}"),
        None,
    );
    if id == MOCK_DECK_ID.to_string() {
        Ok(Json(mock_deck()))
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

/// `GET /api/v1/product` — #564: identifies this mock as "Arena", the same
/// shape the real REST API returns (confirmed against the bitfocus
/// `companion-module-resolume-arena` client's `ArenaProductResponse`), so a
/// port-drift probe pointed at this mock validates exactly like the real
/// thing.
async fn get_product(State(log): State<Arc<RequestLog>>) -> Json<Value> {
    log.record(MOCK_NAME, "GET", "/api/v1/product", None);
    Json(json!({
        "name": "Arena",
        "major": 7,
        "minor": 13,
        "micro": 2,
        "revision": 0,
    }))
}

/// `PUT /api/v1/parameter/by-id/:id` — accepts `{"value": "..."}`, returns 200.
async fn put_parameter(
    State(log): State<Arc<RequestLog>>,
    Path(id): Path<String>,
    body: String,
) -> StatusCode {
    // Truncate at a UTF-8 char boundary near 256 bytes. Worship lyrics
    // contain multi-byte diacritics (Slovak/Czech: á, é, š, č, ď, etc.);
    // a naive byte-index slice would panic mid-character.
    let preview = if body.len() > 256 {
        let end = (0..=256)
            .rev()
            .find(|&i| body.is_char_boundary(i))
            .unwrap_or(0);
        Some(format!("{}...", &body[..end]))
    } else {
        Some(body)
    };
    log.record(
        MOCK_NAME,
        "PUT",
        &format!("/api/v1/parameter/by-id/{id}"),
        preview,
    );
    StatusCode::OK
}

/// `POST /api/v1/composition/clips/by-id/:id/connect` — clip trigger, returns 200.
async fn post_clip_connect(
    State(log): State<Arc<RequestLog>>,
    Path(id): Path<String>,
) -> StatusCode {
    log.record(
        MOCK_NAME,
        "POST",
        &format!("/api/v1/composition/clips/by-id/{id}/connect"),
        None,
    );
    StatusCode::OK
}

#[cfg(test)]
mod tests {
    use super::*;

    use axum::{
        body::Body,
        http::{Method, Request, StatusCode},
    };
    use tower::ServiceExt;

    async fn get_json(app: Router, uri: &str) -> (StatusCode, Value) {
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
        let body_bytes = axum::body::to_bytes(response.into_body(), 65_536)
            .await
            .expect("body bytes");
        (
            status,
            serde_json::from_slice(&body_bytes).unwrap_or(Value::Null),
        )
    }

    /// #808: the composition looks like a loaded Arena, a selected deck and
    /// tagged clips, not like one still loading.
    #[tokio::test]
    async fn composition_lists_a_selected_deck_and_tagged_clips() {
        let log = Arc::new(RequestLog::new());
        let (status, value) = get_json(router(log), "/api/v1/composition").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["decks"][0]["id"], MOCK_DECK_ID);
        assert_eq!(value["decks"][0]["selected"]["value"], true);
        let names: Vec<&str> = value["layers"][0]["clips"]
            .as_array()
            .expect("clips")
            .iter()
            .filter_map(|clip| clip["name"]["value"].as_str())
            .collect();
        assert!(names.contains(&"#main-a") && names.contains(&"#bible-clear"));
        assert!(!names.contains(&"#timer"));
    }

    /// #808: the deck check answers the selected deck, and 404 for others.
    #[tokio::test]
    async fn deck_check_answers_the_selected_deck_and_404_for_others() {
        let log = Arc::new(RequestLog::new());
        let app = router(log.clone());

        let (status, deck) = get_json(app.clone(), "/api/v1/composition/decks/by-id/1").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(deck["selected"]["value"], true);
        let (status, _) = get_json(app, "/api/v1/composition/decks/by-id/7").await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let entries = log.snapshot();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].path, "/api/v1/composition/decks/by-id/1");
    }

    #[tokio::test]
    async fn accepts_product_get_and_identifies_as_arena() {
        let log = Arc::new(RequestLog::new());
        let app = router(log.clone());

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/v1/product")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("oneshot");

        assert_eq!(response.status(), StatusCode::OK);
        let body_bytes = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .expect("body bytes");
        let value: serde_json::Value = serde_json::from_slice(&body_bytes).expect("json");
        assert_eq!(value["name"], "Arena");

        let entries = log.snapshot();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "/api/v1/product");
    }

    #[tokio::test]
    async fn accepts_composition_get() {
        let log = Arc::new(RequestLog::new());
        let app = router(log.clone());

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/v1/composition")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("oneshot");

        assert_eq!(response.status(), StatusCode::OK);
        let body_bytes = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .expect("body bytes");
        let value: serde_json::Value = serde_json::from_slice(&body_bytes).expect("json");
        assert_eq!(value["name"], "Mock Composition");
        assert!(value["layers"].is_array());
        assert!(value["columns"].is_array());

        let entries = log.snapshot();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].method, "GET");
    }

    #[tokio::test]
    async fn accepts_clip_trigger_and_logs_path() {
        let log = Arc::new(RequestLog::new());
        let app = router(log.clone());

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/composition/clips/by-id/abc-123/connect")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("oneshot");

        assert_eq!(response.status(), StatusCode::OK);
        let entries = log.snapshot();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].method, "POST");
        assert_eq!(
            entries[0].path,
            "/api/v1/composition/clips/by-id/abc-123/connect"
        );
    }
}
