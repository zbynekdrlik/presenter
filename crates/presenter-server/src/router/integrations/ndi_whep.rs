//! WHEP HTTP shim — bridges browser SDP exchanges into the per-source
//! `whepserversink` element's signaller via `emit_by_name`.

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::HeaderMap,
    response::Response,
};
use presenter_ndi::manager::{WhepOp, WhepReply};
use tracing::instrument;

use super::super::AppError;
use crate::state::AppState;

fn into_response(reply: WhepReply) -> Response {
    let mut builder = Response::builder().status(reply.status);
    if let Some(headers) = reply.headers {
        for (name, value) in headers.iter() {
            if let Ok(s) = value.get::<String>() {
                builder = builder.header(name.to_string(), s);
            }
        }
    }
    builder
        .body(axum::body::Body::from(reply.body.unwrap_or_default()))
        .expect("valid response")
}

#[instrument(skip_all, fields(source_id = %source_id))]
pub(crate) async fn post_whep_endpoint(
    Path(source_id): Path<String>,
    State(state): State<AppState>,
    body: Bytes,
) -> Result<Response, AppError> {
    let manager = state
        .ndi_manager()
        .ok_or_else(|| AppError::service_unavailable("NDI SDK not available"))?;
    if !manager.is_active(&source_id).await {
        return Err(AppError::not_found("NDI source not active"));
    }
    let reply = manager
        .whep_signaller_call(
            &source_id,
            WhepOp::Post {
                id: None,
                body: body.to_vec(),
            },
        )
        .await
        .map_err(|e| AppError::service_unavailable(format!("WHEP POST: {e}")))?;
    Ok(into_response(reply))
}

#[instrument(skip_all, fields(source_id = %source_id, session_id = %session_id))]
pub(crate) async fn post_whep_session(
    Path((source_id, session_id)): Path<(String, String)>,
    State(state): State<AppState>,
    body: Bytes,
) -> Result<Response, AppError> {
    let manager = state
        .ndi_manager()
        .ok_or_else(|| AppError::service_unavailable("NDI SDK not available"))?;
    let reply = manager
        .whep_signaller_call(
            &source_id,
            WhepOp::Post {
                id: Some(session_id),
                body: body.to_vec(),
            },
        )
        .await
        .map_err(|e| AppError::service_unavailable(format!("WHEP POST session: {e}")))?;
    Ok(into_response(reply))
}

#[instrument(skip_all, fields(source_id = %source_id, session_id = %session_id))]
pub(crate) async fn patch_whep_session(
    Path((source_id, session_id)): Path<(String, String)>,
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, AppError> {
    let manager = state
        .ndi_manager()
        .ok_or_else(|| AppError::service_unavailable("NDI SDK not available"))?;
    let hs: Vec<(String, String)> = headers
        .iter()
        .filter_map(|(k, v)| {
            v.to_str()
                .ok()
                .map(|s| (k.as_str().to_string(), s.to_string()))
        })
        .collect();
    let reply = manager
        .whep_signaller_call(
            &source_id,
            WhepOp::Patch {
                id: session_id,
                body: body.to_vec(),
                headers: hs,
            },
        )
        .await
        .map_err(|e| AppError::service_unavailable(format!("WHEP PATCH: {e}")))?;
    Ok(into_response(reply))
}

#[instrument(skip_all, fields(source_id = %source_id, session_id = %session_id))]
pub(crate) async fn delete_whep_session(
    Path((source_id, session_id)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Result<Response, AppError> {
    let manager = state
        .ndi_manager()
        .ok_or_else(|| AppError::service_unavailable("NDI SDK not available"))?;
    let reply = manager
        .whep_signaller_call(&source_id, WhepOp::Delete { id: session_id })
        .await
        .map_err(|e| AppError::service_unavailable(format!("WHEP DELETE: {e}")))?;
    Ok(into_response(reply))
}
