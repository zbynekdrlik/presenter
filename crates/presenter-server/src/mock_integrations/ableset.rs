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
