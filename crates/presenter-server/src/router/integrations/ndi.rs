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
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                break;
            }
        }
    }
}

/// HTTP MJPEG stream using multipart/x-mixed-replace.
///
/// The browser renders this natively in an `<img>` tag with zero JavaScript
/// overhead — no Blob creation, no createImageBitmap, no canvas. This is
/// how IP cameras have streamed MJPEG for decades.
///
/// Usage: `<img src="/ndi/mjpeg" />`
pub(crate) async fn mjpeg_http(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let manager = state
        .ndi_manager()
        .ok_or_else(|| AppError::service_unavailable("NDI SDK not available"))?;
    let rx = manager.subscribe_frames();

    let boundary = "mjpegboundary";
    let content_type = format!("multipart/x-mixed-replace; boundary={boundary}");

    let stream = async_stream::stream! {
        let mut rx = rx;
        loop {
            match rx.recv().await {
                Ok(jpeg) => {
                    // Build multipart frame
                    let part_header = format!(
                        "--{boundary}\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
                        jpeg.len()
                    );
                    yield Ok::<Bytes, std::convert::Infallible>(Bytes::from(part_header));
                    yield Ok(jpeg);
                    yield Ok(Bytes::from("\r\n"));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    // Skip ahead to latest frame
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    break;
                }
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
