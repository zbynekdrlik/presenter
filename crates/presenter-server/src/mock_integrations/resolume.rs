//! Mock Resolume Arena HTTP listener for dev. Listens on
//! `127.0.0.1:8091`, accepts the 3 endpoints presenter-server's outbound
//! resolume driver calls, returns minimal-valid responses, and records
//! every request to the shared `RequestLog`.

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

/// Spawn the mock Resolume listener in a background task.
pub async fn spawn(log: Arc<RequestLog>) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/api/v1/composition", get(get_composition))
        .route("/api/v1/parameter/by-id/{id}", put(put_parameter))
        .route(
            "/api/v1/composition/clips/by-id/{id}/connect",
            post(post_clip_connect),
        )
        .route("/__mock/log", get(log_handler))
        .with_state(log);

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

/// `GET /api/v1/composition` — returns a minimal valid Composition JSON.
async fn get_composition(State(log): State<Arc<RequestLog>>) -> Json<Value> {
    log.record(MOCK_NAME, "GET", "/api/v1/composition", None);
    Json(json!({
        "name": "Mock Composition",
        "layers": [],
        "columns": [],
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

    fn router(log: Arc<RequestLog>) -> Router {
        Router::new()
            .route("/api/v1/composition", get(get_composition))
            .route("/api/v1/parameter/by-id/{id}", put(put_parameter))
            .route(
                "/api/v1/composition/clips/by-id/{id}/connect",
                post(post_clip_connect),
            )
            .route("/__mock/log", get(log_handler))
            .with_state(log)
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
