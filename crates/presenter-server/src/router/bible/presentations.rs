//! Bible presentation CRUD: create/rename/delete presentations, append and
//! reorder slides, and single-slide update/delete. Split out of
//! `router/bible.rs` (#590) — same pattern as `router/integrations/`. Reuses
//! `BibleSlideDto`/`bible_slide_to_dto` from the sibling `resolve` module.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use presenter_core::slide::SlideMetadata;
use presenter_core::{
    BiblePresentation, BiblePresentationId, BiblePresentationSlide, BibleSlideId, SlideText,
};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use super::super::AppError;
use super::resolve::{bible_slide_to_dto, BibleSlideDto};
use crate::state::AppState;

// Bible Presentations
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BiblePresentationSummaryDto {
    id: String,
    name: String,
    slide_count: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BiblePresentationDetailDto {
    id: String,
    name: String,
    slides: Vec<BibleSlideDto>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BiblePresentationCreateRequest {
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RenameBiblePresentationRequest {
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppendBibleSlidesRequest {
    slides: Vec<BibleSlideInput>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BibleSlideInput {
    bible_main: String,
    bible_translation: String,
    #[serde(default)]
    bible_main_reference: String,
    #[serde(default)]
    bible_translation_reference: String,
    #[serde(default)]
    metadata: Option<SlideMetadata>,
}

fn request_slide_to_domain(input: BibleSlideInput) -> Result<BiblePresentationSlide, AppError> {
    let main = SlideText::new(&input.bible_main)
        .map_err(|e| AppError::bad_request_message(e.to_string()))?;
    let secondary = SlideText::new(&input.bible_translation)
        .map_err(|e| AppError::bad_request_message(e.to_string()))?;

    // Unwrap the structured bible metadata from the legacy wire envelope
    // (`{ metadata: { bible: { ... } } }`) — the frontend still sends it in
    // that shape for backwards compatibility.
    let metadata = input.metadata.and_then(|m| m.bible);

    Ok(BiblePresentationSlide {
        id: BibleSlideId::new(),
        // The repository assigns the final order on insert; zero here is a
        // placeholder.
        order: 0,
        main,
        main_reference: input.bible_main_reference,
        secondary,
        secondary_reference: input.bible_translation_reference,
        metadata,
    })
}

fn bible_presentation_to_detail(presentation: &BiblePresentation) -> BiblePresentationDetailDto {
    BiblePresentationDetailDto {
        id: presentation.id.to_string(),
        name: presentation.name.clone(),
        slides: presentation.slides.iter().map(bible_slide_to_dto).collect(),
    }
}

#[instrument(skip_all)]
pub(crate) async fn list_bible_presentations(
    State(state): State<AppState>,
) -> Result<Json<Vec<BiblePresentationSummaryDto>>, AppError> {
    let summaries = state.list_bible_presentations().await?;
    let result = summaries
        .into_iter()
        .map(|s| BiblePresentationSummaryDto {
            id: s.id.to_string(),
            name: s.name,
            slide_count: s.slide_count,
        })
        .collect();
    Ok(Json(result))
}

#[instrument(skip_all)]
pub(crate) async fn create_bible_presentation_handler(
    State(state): State<AppState>,
    Json(payload): Json<BiblePresentationCreateRequest>,
) -> Result<Json<BiblePresentationDetailDto>, AppError> {
    let name = payload.name.trim();
    if name.is_empty() {
        return Err(AppError::bad_request_message("name cannot be empty"));
    }
    let presentation = state.create_bible_presentation(name).await?;
    Ok(Json(bible_presentation_to_detail(&presentation)))
}

#[instrument(skip_all)]
pub(crate) async fn get_bible_presentation(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> Result<Json<BiblePresentationDetailDto>, AppError> {
    let presentation = state
        .bible_presentation_detail(BiblePresentationId::from_uuid(id))
        .await?
        .ok_or_else(|| AppError::not_found("presentation not found"))?;
    Ok(Json(bible_presentation_to_detail(&presentation)))
}

#[instrument(skip_all)]
pub(crate) async fn rename_bible_presentation_handler(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
    Json(payload): Json<RenameBiblePresentationRequest>,
) -> Result<StatusCode, AppError> {
    let name = payload.name.trim();
    if name.is_empty() {
        return Err(AppError::bad_request_message("name cannot be empty"));
    }
    // #633: `RepositoryError::NotFound` maps to 404 by default via the
    // centralized `From<anyhow::Error> for AppError`.
    state
        .rename_bible_presentation(BiblePresentationId::from_uuid(id), name)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[instrument(skip_all)]
pub(crate) async fn delete_bible_presentation_handler(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> Result<StatusCode, AppError> {
    state
        .delete_bible_presentation(BiblePresentationId::from_uuid(id))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Request body for updating a single Bible slide's text and references.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateBibleSlideRequest {
    pub(crate) bible_main: String,
    pub(crate) bible_translation: String,
    pub(crate) bible_main_reference: String,
    pub(crate) bible_translation_reference: String,
}

/// PATCH /bible/presentations/{id}/slides/{slide_id}
///
/// Updates a Bible slide's text and reference labels. The structured bible
/// metadata (book, chapter, verses, translation code) is preserved from the
/// existing slide — the UI only edits text and reference labels.
#[instrument(skip_all)]
pub(crate) async fn update_bible_slide(
    State(state): State<AppState>,
    Path((presentation_id, slide_id)): Path<(String, String)>,
    Json(payload): Json<UpdateBibleSlideRequest>,
) -> Result<Json<BibleSlideDto>, AppError> {
    let pres_uuid = presentation_id
        .parse::<uuid::Uuid>()
        .map_err(|_| AppError::bad_request_message("Invalid presentation ID"))?;
    let slide_uuid = slide_id
        .parse::<uuid::Uuid>()
        .map_err(|_| AppError::bad_request_message("Invalid slide ID"))?;

    let pres_id = BiblePresentationId::from_uuid(pres_uuid);
    let s_id = BibleSlideId::from_uuid(slide_uuid);

    // Preserve the existing structured metadata (book, chapter, verses, etc.)
    // — the UI only edits text and reference labels, not the structured data.
    let presentation = state
        .bible_presentation_detail(pres_id)
        .await?
        .ok_or_else(|| AppError::not_found("Presentation not found"))?;
    let existing_slide = presentation
        .slides
        .iter()
        .find(|s| s.id == s_id)
        .ok_or_else(|| AppError::not_found("Slide not found in presentation"))?;
    let existing_metadata = existing_slide.metadata.clone();

    let updated = state
        .update_bible_slide(
            pres_id,
            s_id,
            payload.bible_main,
            payload.bible_main_reference,
            payload.bible_translation,
            payload.bible_translation_reference,
            existing_metadata,
        )
        .await?;

    Ok(Json(bible_slide_to_dto(&updated)))
}

#[instrument(skip_all)]
pub(crate) async fn append_bible_presentation_handler(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
    Json(payload): Json<AppendBibleSlidesRequest>,
) -> Result<Json<BiblePresentationDetailDto>, AppError> {
    if payload.slides.is_empty() {
        return Err(AppError::bad_request_message("slides cannot be empty"));
    }
    let mut slides = Vec::with_capacity(payload.slides.len());
    for input in payload.slides {
        slides.push(request_slide_to_domain(input)?);
    }
    let presentation = state
        .append_bible_presentation_slides(BiblePresentationId::from_uuid(id), slides)
        .await?;
    Ok(Json(bible_presentation_to_detail(&presentation)))
}

/// DELETE /bible/presentations/{id}/slides/{slide_id}
///
/// Deletes a single slide from a bible presentation and returns the updated
/// presentation detail.
#[instrument(skip_all)]
pub(crate) async fn delete_bible_presentation_slide(
    State(state): State<AppState>,
    Path((presentation_id, slide_id)): Path<(String, String)>,
) -> Result<Json<BiblePresentationDetailDto>, AppError> {
    let pres_uuid = presentation_id
        .parse::<uuid::Uuid>()
        .map_err(|_| AppError::bad_request_message("Invalid presentation ID"))?;
    let slide_uuid = slide_id
        .parse::<uuid::Uuid>()
        .map_err(|_| AppError::bad_request_message("Invalid slide ID"))?;

    let presentation = state
        .delete_bible_slide(
            BiblePresentationId::from_uuid(pres_uuid),
            BibleSlideId::from_uuid(slide_uuid),
        )
        .await?;
    Ok(Json(bible_presentation_to_detail(&presentation)))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReorderBibleSlidesRequest {
    pub(crate) slide_ids: Vec<String>,
}

/// POST /bible/presentations/{id}/slides/reorder
///
/// Reorders slides in a bible presentation according to the supplied ID list
/// and returns the updated presentation detail.
#[instrument(skip_all)]
pub(crate) async fn reorder_bible_presentation_slides(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
    Json(payload): Json<ReorderBibleSlidesRequest>,
) -> Result<Json<BiblePresentationDetailDto>, AppError> {
    let mut slide_ids = Vec::with_capacity(payload.slide_ids.len());
    for raw in payload.slide_ids {
        let uuid = raw
            .parse::<uuid::Uuid>()
            .map_err(|_| AppError::bad_request_message("Invalid slide ID"))?;
        slide_ids.push(BibleSlideId::from_uuid(uuid));
    }

    let presentation = state
        .reorder_bible_slides(BiblePresentationId::from_uuid(id), slide_ids)
        .await?;
    Ok(Json(bible_presentation_to_detail(&presentation)))
}
