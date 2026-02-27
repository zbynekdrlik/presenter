use axum::{
    extract::{Path, State},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use super::{parse_uuid, AppError};
use crate::{stage_ui, state::AppState};
use axum::http::StatusCode;
use presenter_core::{
    PlaylistId, PresentationId, SlideId, StageDesign, StageDisplayLayout, StageDisplaySnapshot,
};

/// Visual appearance settings for a stage display layout.
///
/// Each layout stores its own copy; unknown fields are ignored on
/// deserialization so old clients survive schema additions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StageAppearance {
    pub body_padding_v: f32,
    pub body_padding_h: f32,
    pub current_max_font: f32,
    pub next_max_font: f32,
    pub next_ratio: f32,
    pub group_font_size: f32,
    pub lyrics_gap: f32,
    pub next_padding_bottom: f32,
    pub base_chars: u32,
    pub min_font: f32,
    // worship-pp specific
    pub playlist_font_size: f32,
    pub playlist_header_size: f32,
    pub playlist_padding: f32,
    pub slides_playlist_ratio: String,
}

impl Default for StageAppearance {
    fn default() -> Self {
        Self {
            body_padding_v: 1.0,
            body_padding_h: 2.0,
            current_max_font: 120.0,
            next_max_font: 80.0,
            next_ratio: 0.8,
            group_font_size: 1.6,
            lyrics_gap: 0.5,
            next_padding_bottom: 2.0,
            base_chars: 25,
            min_font: 12.0,
            playlist_font_size: 1.3,
            playlist_header_size: 1.1,
            playlist_padding: 1.0,
            slides_playlist_ratio: "7fr 3fr".to_string(),
        }
    }
}

impl StageAppearance {
    /// Return sensible defaults per layout code.
    pub fn default_for(layout: &str) -> Self {
        match layout {
            "worship-pp" => Self {
                current_max_font: 100.0,
                next_max_font: 64.0,
                ..Self::default()
            },
            _ => Self::default(),
        }
    }
}

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
    #[serde(default)]
    pub(super) playlist_id: Option<String>,
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
    let playlist_id = match payload.playlist_id {
        Some(value) => Some(PlaylistId::from_uuid(parse_uuid("playlistId", &value)?)),
        None => None,
    };
    state
        .update_stage_state(
            presentation_id,
            current_slide_id,
            next_slide_id,
            playlist_id,
        )
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

#[instrument(skip_all)]
pub(super) async fn get_stage_appearance(
    State(state): State<AppState>,
    Path(layout): Path<String>,
) -> Result<Json<StageAppearance>, AppError> {
    let appearance = state.get_stage_appearance(&layout).await?;
    Ok(Json(appearance))
}

#[instrument(skip_all)]
pub(super) async fn update_stage_appearance(
    State(state): State<AppState>,
    Path(layout): Path<String>,
    Json(appearance): Json<StageAppearance>,
) -> Result<StatusCode, AppError> {
    state.set_stage_appearance(&layout, appearance).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BroadcastLiveResponse {
    pub(super) enabled: bool,
}

#[instrument(skip_all)]
pub(super) async fn get_broadcast_live(
    State(state): State<AppState>,
) -> Json<BroadcastLiveResponse> {
    Json(BroadcastLiveResponse {
        enabled: state.broadcast_live(),
    })
}

// Stage Design endpoints

#[instrument(skip_all)]
pub(super) async fn get_stage_design(
    State(state): State<AppState>,
    Path(layout): Path<String>,
) -> Result<Json<StageDesign>, AppError> {
    let design = state.get_stage_design(&layout).await?;
    Ok(Json(design))
}

#[instrument(skip_all)]
pub(super) async fn update_stage_design(
    State(state): State<AppState>,
    Path(layout): Path<String>,
    Json(design): Json<StageDesign>,
) -> Result<StatusCode, AppError> {
    if design.layout_code != layout {
        return Err(AppError::bad_request_message(
            "layout_code in body must match URL parameter",
        ));
    }
    state.set_stage_design(&layout, design).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[instrument(skip_all)]
pub(super) async fn reset_stage_design(
    State(state): State<AppState>,
    Path(layout): Path<String>,
) -> Result<Json<StageDesign>, AppError> {
    let design = state.reset_stage_design(&layout).await?;
    Ok(Json(design))
}
