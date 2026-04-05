use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    response::IntoResponse,
    Json,
};
use serde::Serialize;
use tracing::instrument;

use super::super::AppError;
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
    let payload = sources
        .into_iter()
        .map(|s| NdiSourceDto { name: s.name })
        .collect();
    Ok(Json(payload))
}

#[instrument(skip_all)]
pub(crate) async fn ndi_status(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "available": state.ndi_manager().is_some() }))
}

/// WebSocket endpoint that streams MJPEG frames from the active NDI source.
///
/// Each binary message is a complete JPEG image.
pub(crate) async fn mjpeg_ws(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let manager = state
        .ndi_manager()
        .ok_or_else(|| AppError::service_unavailable("NDI SDK not available"))?;
    let rx = manager.subscribe_frames();
    Ok(ws.on_upgrade(move |socket| handle_mjpeg_ws(socket, rx)))
}

async fn handle_mjpeg_ws(
    mut socket: WebSocket,
    mut rx: tokio::sync::broadcast::Receiver<bytes::Bytes>,
) {
    loop {
        match rx.recv().await {
            Ok(jpeg) => {
                if socket
                    .send(Message::Binary(jpeg.to_vec().into()))
                    .await
                    .is_err()
                {
                    break; // client disconnected
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                tracing::debug!("MJPEG WS client lagged {n} frames");
                // skip ahead, don't disconnect
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                break; // stream stopped
            }
        }
    }
}
