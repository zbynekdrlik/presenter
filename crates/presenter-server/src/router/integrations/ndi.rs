use axum::{extract::State, http::StatusCode, Json};
use serde::Serialize;
use tracing::instrument;

use super::super::AppError;
use crate::state::AppState;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct NdiSourceDto {
    name: String,
}

#[instrument(skip_all)]
pub(crate) async fn discover_ndi_sources(
    State(state): State<AppState>,
) -> Result<Json<Vec<NdiSourceDto>>, AppError> {
    let manager = state
        .ndi_manager()
        .ok_or_else(|| AppError::service_unavailable("NDI SDK not available"))?;
    let sources = manager.discover_sources(3000)?;
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

#[instrument(skip_all)]
pub(crate) async fn whep_endpoint(
    State(state): State<AppState>,
    body: String,
) -> Result<(StatusCode, [(&'static str, &'static str); 1], String), AppError> {
    let manager = state
        .ndi_manager()
        .ok_or_else(|| AppError::service_unavailable("NDI SDK not available"))?;
    let sdp_answer = manager.create_whep_session(body).await?;
    Ok((
        StatusCode::CREATED,
        [("content-type", "application/sdp")],
        sdp_answer,
    ))
}
