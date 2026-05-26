use axum::{extract::State, Json};
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

/// `GET /ndi/snapshot/:source_id` — diagnostic route exposing the live
/// pipeline state for a single NDI source.
///
/// Returns JSON (camelCase) with `encoderCount`, `consumerCount`, and a
/// per-session `sessions` array. Used by the Playwright fanout E2E test
/// to assert `encoderCount=1` + `consumerCount=2` when two browser tabs
/// are connected to the same NDI source, and as an operator/incident-
/// debugging tool for checking pipeline health without tailing logs.
///
/// 404 — source is not currently active (no pipeline exists for this id).
/// 503 — NDI SDK not available on this host.
#[instrument(skip_all, fields(source_id = %source_id))]
pub(crate) async fn ndi_snapshot(
    axum::extract::Path(source_id): axum::extract::Path<String>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, AppError> {
    let manager = state
        .ndi_manager()
        .ok_or_else(|| AppError::service_unavailable("NDI SDK not available"))?;
    let snap = manager
        .pipeline_snapshot(&source_id)
        .await
        .ok_or_else(|| AppError::not_found("NDI source not active"))?;
    Ok(Json(
        serde_json::to_value(snap).expect("PipelineSnapshot serializes"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    /// Build a fresh in-memory AppState that may or may not have a real NDI
    /// manager attached depending on whether libndi is loadable on the host.
    async fn fresh_state() -> AppState {
        AppState::in_memory().await.expect("in-memory AppState")
    }

    #[tokio::test]
    async fn ndi_snapshot_returns_not_found_or_unavailable_for_unknown_source() {
        let state = fresh_state().await;
        let result = ndi_snapshot(
            axum::extract::Path("00000000-0000-0000-0000-000000000000".to_string()),
            State(state),
        )
        .await;
        assert!(result.is_err(), "expected Err for unknown source");
        let resp = result.unwrap_err().into_response();
        assert!(
            matches!(
                resp.status(),
                StatusCode::NOT_FOUND | StatusCode::SERVICE_UNAVAILABLE,
            ),
            "expected 404 or 503, got {}",
            resp.status(),
        );
    }
}
