use axum::{
    extract::State,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use super::{parse_uuid, AppError};
use crate::{stage_ui, state::AppState};
use axum::http::StatusCode;
use presenter_core::{PresentationId, SlideId, StageDisplayLayout, StageDisplaySnapshot};

#[instrument(skip_all)]
pub(super) async fn stage_display_selected_html(
    State(state): State<AppState>,
) -> Result<Response, AppError> {
    match state.selected_stage_display_snapshot().await? {
        Some(snapshot) => {
            Ok(stage_ui::render_stage_display(snapshot, state.heartbeat_config()).into_response())
        }
        None => Ok((StatusCode::SERVICE_UNAVAILABLE, "Stage display unavailable").into_response()),
    }
}

#[instrument(skip_all)]
pub(super) async fn stage_display_selected_snapshot_json(
    State(state): State<AppState>,
) -> Result<Json<StageDisplaySnapshot>, AppError> {
    match state.selected_stage_display_snapshot().await? {
        Some(snapshot) => Ok(Json(snapshot)),
        None => Err(AppError::not_found("Stage display unavailable")),
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StageLayoutResponse {
    pub(super) code: String,
    pub(super) layout: StageDisplayLayout,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StageLayoutUpdateRequest {
    pub(super) code: String,
}

#[instrument(skip_all)]
pub(super) async fn get_stage_layout(
    State(state): State<AppState>,
) -> Result<Json<StageLayoutResponse>, AppError> {
    let code = state.stage_layout_code().await;
    let layouts = state.stage_displays().await?;
    let layout = layouts
        .into_iter()
        .find(|layout| layout.code == code)
        .or_else(|| StageDisplayLayout::built_in().into_iter().next())
        .ok_or_else(|| AppError::internal("no stage layouts available"))?;
    Ok(Json(StageLayoutResponse {
        code: layout.code.clone(),
        layout,
    }))
}

#[instrument(skip_all)]
pub(super) async fn set_stage_layout(
    State(state): State<AppState>,
    Json(payload): Json<StageLayoutUpdateRequest>,
) -> Result<Json<StageLayoutResponse>, AppError> {
    let code = payload.code.trim();
    if code.is_empty() {
        return Err(AppError::bad_request_message("code cannot be empty"));
    }
    let layout = state
        .set_stage_layout_code(code)
        .await
        .map_err(|err| AppError::not_found(err.to_string()))?;
    Ok(Json(StageLayoutResponse {
        code: layout.code.clone(),
        layout,
    }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StageStateRequest {
    pub(super) presentation_id: String,
    pub(super) current_slide_id: String,
    #[serde(default)]
    pub(super) next_slide_id: Option<String>,
}

#[instrument(skip_all)]
pub(super) async fn update_stage_state(
    State(state): State<AppState>,
    Json(payload): Json<StageStateRequest>,
) -> Result<StatusCode, AppError> {
    let presentation_id =
        PresentationId::from_uuid(parse_uuid("presentationId", &payload.presentation_id)?);
    let current_slide_id =
        SlideId::from_uuid(parse_uuid("currentSlideId", &payload.current_slide_id)?);
    let next_slide_id = match payload.next_slide_id {
        Some(value) => Some(SlideId::from_uuid(parse_uuid("nextSlideId", &value)?)),
        None => None,
    };
    state
        .update_stage_state(presentation_id, current_slide_id, next_slide_id)
        .await
        .map_err(AppError::bad_request)?;
    Ok(StatusCode::NO_CONTENT)
}

#[instrument(skip_all)]
pub(super) async fn list_stage_connections(
    State(state): State<AppState>,
) -> Result<Json<Vec<crate::stage_connections::StageClientSnapshot>>, AppError> {
    let snapshot = state.stage_connections_snapshot().await;
    Ok(Json(snapshot))
}

#[instrument(skip_all)]
pub(super) async fn list_stage_displays(
    State(state): State<AppState>,
) -> Result<Json<Vec<StageDisplayLayout>>, AppError> {
    let displays = state.stage_displays().await?;
    Ok(Json(displays))
}

#[instrument(skip_all)]
pub(super) async fn clear_stage_state(
    State(state): State<AppState>,
) -> Result<StatusCode, AppError> {
    state.clear_stage().await?;
    Ok(StatusCode::NO_CONTENT)
}
