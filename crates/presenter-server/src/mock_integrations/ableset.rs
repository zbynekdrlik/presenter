//! Mock AbleSet HTTP listener for dev. Listens on `127.0.0.1:39042`,
//! serves a static-but-valid `/api/setlist` response, and records every
//! request to the shared `RequestLog`.

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Context;
use axum::{extract::State, response::Json, routing::get, Router};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tracing::{info, warn};

use super::request_log::{log_handler, RequestLog};

const MOCK_ABLESET_ADDR: &str = "127.0.0.1:39042";
const MOCK_NAME: &str = "ableset";

pub async fn spawn(log: Arc<RequestLog>) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/api/setlist", get(get_setlist))
        .route("/__mock/log", get(log_handler))
        .with_state(log);

    let addr: SocketAddr = MOCK_ABLESET_ADDR
        .parse()
        .context("invalid MOCK_ABLESET_ADDR")?;
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("mock-ableset failed to bind {addr}"))?;
    info!(%addr, "mock-ableset listener started");

    tokio::spawn(async move {
        if let Err(err) = axum::serve(listener, app).await {
            warn!(?err, "mock-ableset listener exited");
        }
    });
    Ok(())
}

async fn get_setlist(State(log): State<Arc<RequestLog>>) -> Json<Value> {
    log.record(MOCK_NAME, "GET", "/api/setlist", None);
    Json(json!({
        "activeSongId": null,
        "songs": [],
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    use axum::{
        body::Body,
        http::{Method, Request, StatusCode},
    };
    use tower::ServiceExt;

    #[tokio::test]
    async fn serves_setlist_shape() {
        let log = Arc::new(RequestLog::new());
        let app = Router::new()
            .route("/api/setlist", get(get_setlist))
            .with_state(log.clone());

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/setlist")
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
        assert!(value.get("activeSongId").is_some(), "missing activeSongId");
        assert!(value["songs"].is_array());

        let entries = log.snapshot();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].method, "GET");
        assert_eq!(entries[0].path, "/api/setlist");
    }
}
