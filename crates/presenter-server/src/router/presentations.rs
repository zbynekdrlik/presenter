use super::AppError;
use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use presenter_core::{LibraryId, Presentation, PresentationId, Slide, SlideId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::instrument;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PresentationDetailDto {
    pub(super) library_id: LibraryId,
    pub(super) library_name: String,
    pub(super) presentation: Presentation,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RenamePresentationRequest {
    pub(super) name: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CreateSlideRequest {
    pub(super) position: Option<u32>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReorderSlidesRequest {
    pub(super) slide_ids: Vec<uuid::Uuid>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SlideContentUpdateRequest {
    pub(super) main: String,
    pub(super) translation: String,
    pub(super) stage: String,
    #[serde(default)]
    pub(super) group: Option<String>,
    #[serde(default)]
    pub(super) metadata: Option<presenter_core::slide::SlideMetadata>,
}

#[instrument(skip_all)]
pub(super) async fn get_presentation_detail(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<PresentationDetailDto>, AppError> {
    let uuid = uuid::Uuid::parse_str(&id)
        .map_err(|_| AppError::bad_request_message("presentationId must be a valid UUID"))?;
    let presentation_id = PresentationId::from_uuid(uuid);
    match state.presentation_detail(presentation_id).await? {
        Some((library_id, library_name, presentation)) => Ok(Json(PresentationDetailDto {
            library_id,
            library_name,
            presentation,
        })),
        None => Err(AppError::not_found(format!(
            "presentation {} not found",
            id
        ))),
    }
}

#[instrument(skip_all)]
pub(super) async fn update_presentation(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<RenamePresentationRequest>,
) -> Result<StatusCode, AppError> {
    let name = payload.name.trim();
    if name.is_empty() {
        return Err(AppError::bad_request_message("name cannot be empty"));
    }
    let presentation_uuid = super::parse_uuid("presentationId", &id)?;
    state
        .rename_presentation(PresentationId::from_uuid(presentation_uuid), name)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[instrument(skip_all)]
pub(super) async fn delete_presentation(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let presentation_uuid = super::parse_uuid("presentationId", &id)?;
    state
        .delete_presentation(PresentationId::from_uuid(presentation_uuid))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[instrument(skip_all)]
pub(super) async fn insert_slide(
    State(state): State<AppState>,
    Path(presentation_id): Path<String>,
    Json(payload): Json<CreateSlideRequest>,
) -> Result<Json<Vec<Slide>>, AppError> {
    let presentation_uuid = super::parse_uuid("presentationId", &presentation_id)?;
    let slides = state
        .insert_blank_slide(
            PresentationId::from_uuid(presentation_uuid),
            payload.position,
        )
        .await?;
    Ok(Json(slides))
}

#[instrument(skip_all)]
pub(super) async fn duplicate_slide(
    State(state): State<AppState>,
    Path((presentation_id, slide_id)): Path<(String, String)>,
) -> Result<Json<Vec<Slide>>, AppError> {
    let presentation_uuid = super::parse_uuid("presentationId", &presentation_id)?;
    let slide_uuid = super::parse_uuid("slideId", &slide_id)?;
    let slides = state
        .duplicate_slide(
            PresentationId::from_uuid(presentation_uuid),
            SlideId::from_uuid(slide_uuid),
        )
        .await?;
    Ok(Json(slides))
}

#[instrument(skip_all)]
pub(super) async fn delete_slide(
    State(state): State<AppState>,
    Path((presentation_id, slide_id)): Path<(String, String)>,
) -> Result<Json<Vec<Slide>>, AppError> {
    let presentation_uuid = super::parse_uuid("presentationId", &presentation_id)?;
    let slide_uuid = super::parse_uuid("slideId", &slide_id)?;
    let slides = state
        .delete_slide(
            PresentationId::from_uuid(presentation_uuid),
            SlideId::from_uuid(slide_uuid),
        )
        .await?;
    Ok(Json(slides))
}

#[instrument(skip_all)]
pub(super) async fn reorder_slides(
    State(state): State<AppState>,
    Path(presentation_id): Path<String>,
    Json(payload): Json<ReorderSlidesRequest>,
) -> Result<Json<Vec<Slide>>, AppError> {
    let presentation_uuid = super::parse_uuid("presentationId", &presentation_id)?;
    let order = payload
        .slide_ids
        .into_iter()
        .map(SlideId::from_uuid)
        .collect();
    let slides = state
        .reorder_slides(PresentationId::from_uuid(presentation_uuid), order)
        .await?;
    Ok(Json(slides))
}

#[instrument(skip_all)]
pub(super) async fn update_slide_content(
    State(state): State<AppState>,
    Path((presentation_id, slide_id)): Path<(String, String)>,
    Json(payload): Json<SlideContentUpdateRequest>,
) -> Result<Json<Slide>, AppError> {
    let presentation_uuid = super::parse_uuid("presentationId", &presentation_id)?;
    let slide_uuid = super::parse_uuid("slideId", &slide_id)?;
    let updated = state
        .update_slide_content(
            PresentationId::from_uuid(presentation_uuid),
            SlideId::from_uuid(slide_uuid),
            payload.main,
            payload.translation,
            payload.stage,
            payload.group,
            payload.metadata,
        )
        .await?;
    Ok(Json(updated))
}

#[instrument(skip_all)]
pub(super) async fn get_group_colors(
    State(state): State<AppState>,
) -> Json<HashMap<String, String>> {
    Json(state.get_all_group_colors().await)
}
