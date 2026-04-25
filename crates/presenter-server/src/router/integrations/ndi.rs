use axum::http::header;
use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    response::IntoResponse,
    Json,
};
use bytes::Bytes;
use presenter_ndi::{Tier, TierSubscription};
use serde::Serialize;
use std::time::Instant;
use tokio::sync::broadcast::error::RecvError;
use tracing::instrument;

use super::super::AppError;
use crate::adaptive_mjpeg::{AdaptController, AdaptDecision};
use crate::state::AppState;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NdiSourceDto {
    name: String,
}

#[instrument(skip_all)]
pub(crate) async fn discover_ndi_sources(
    State(state): State<AppState>,
) -> Result<Json<Vec<NdiSourceDto>>, AppError> {
    let manager = state
        .ndi_manager()
        .ok_or_else(|| AppError::service_unavailable("NDI SDK not available"))?;
    let sources = manager.discover_sources(0)?;
    Ok(Json(
        sources
            .into_iter()
            .map(|s| NdiSourceDto { name: s.name })
            .collect(),
    ))
}

#[instrument(skip_all)]
pub(crate) async fn ndi_status(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "available": state.ndi_manager().is_some() }))
}

/// WebSocket endpoint that streams JPEG frames; tier adapts per-connection.
pub(crate) async fn mjpeg_ws(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let manager = state
        .ndi_manager()
        .ok_or_else(|| AppError::service_unavailable("NDI SDK not available"))?;
    let sub = manager.subscribe_tier(Tier::L0).await;
    Ok(ws.on_upgrade(move |socket| handle_mjpeg_ws(socket, sub, state)))
}

async fn handle_mjpeg_ws(mut socket: WebSocket, mut sub: TierSubscription, state: AppState) {
    let mut controller = AdaptController::new(Tier::L0);
    loop {
        match sub.rx.recv().await {
            Ok(jpeg) => {
                let decision = controller.on_frame(Instant::now());
                if let AdaptDecision::Promote(next) = decision {
                    if let Some(manager) = state.ndi_manager() {
                        sub = manager.subscribe_tier(next).await;
                    }
                }
                if socket
                    .send(Message::Binary(jpeg.to_vec().into()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            Err(RecvError::Lagged(n)) => {
                tracing::info!(lag = n, tier = ?controller.tier(), "MJPEG WS client lagged");
                let decision = controller.on_lag(Instant::now());
                if let AdaptDecision::Demote(next) = decision {
                    tracing::info!(from = ?controller.tier(), to = ?next, "MJPEG WS demoting tier");
                    if let Some(manager) = state.ndi_manager() {
                        sub = manager.subscribe_tier(next).await;
                    }
                }
            }
            Err(RecvError::Closed) => break,
        }
    }
}

/// HTTP MJPEG stream using multipart/x-mixed-replace.
pub(crate) async fn mjpeg_http(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let manager = state
        .ndi_manager()
        .ok_or_else(|| AppError::service_unavailable("NDI SDK not available"))?;

    let initial_sub = manager.subscribe_tier(Tier::L0).await;
    let state_clone = state.clone();
    let boundary = "mjpegboundary";
    let content_type = format!("multipart/x-mixed-replace; boundary={boundary}");

    let stream = async_stream::stream! {
        let mut sub = initial_sub;
        let mut controller = AdaptController::new(Tier::L0);
        loop {
            match sub.rx.recv().await {
                Ok(jpeg) => {
                    let decision = controller.on_frame(Instant::now());
                    if let AdaptDecision::Promote(next) = decision {
                        if let Some(manager) = state_clone.ndi_manager() {
                            sub = manager.subscribe_tier(next).await;
                        }
                    }
                    let part_header = format!(
                        "--{boundary}\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
                        jpeg.len()
                    );
                    yield Ok::<Bytes, std::convert::Infallible>(Bytes::from(part_header));
                    yield Ok(jpeg);
                    yield Ok(Bytes::from("\r\n"));
                }
                Err(RecvError::Lagged(n)) => {
                    tracing::info!(lag = n, tier = ?controller.tier(), "MJPEG HTTP client lagged");
                    let decision = controller.on_lag(Instant::now());
                    if let AdaptDecision::Demote(next) = decision {
                        tracing::info!(from = ?controller.tier(), to = ?next, "MJPEG HTTP demoting tier");
                        if let Some(manager) = state_clone.ndi_manager() {
                            sub = manager.subscribe_tier(next).await;
                        }
                    }
                }
                Err(RecvError::Closed) => break,
            }
        }
    };

    let body = axum::body::Body::from_stream(stream);
    Ok((
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "no-cache".to_string()),
            (header::CONNECTION, "keep-alive".to_string()),
        ],
        body,
    ))
}
