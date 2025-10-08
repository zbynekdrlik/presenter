use axum::{extract::{Path, State}, http::StatusCode, Json};
use crate::state::AppState;
use super::{
  AppError,
  CreateSlideRequest,
  PresentationDetailDto,
  ReorderSlidesRequest,
  RenamePresentationRequest,
  SlideContentUpdateRequest,
};

pub(super) async fn get_presentation_detail(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<PresentationDetailDto>, AppError> {
    super::get_presentation_detail(Path(id), State(state)).await
}

pub(super) async fn update_presentation(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(payload): Json<RenamePresentationRequest>,
) -> Result<StatusCode, AppError> {
    super::update_presentation(State(state), Path(id), Json(payload)).await
}

pub(super) async fn insert_slide(
    State(state): State<AppState>,
    Path(presentation_id): Path<String>,
    Json(payload): Json<CreateSlideRequest>,
) -> Result<Json<Vec<presenter_core::Slide>>, AppError> {
    super::insert_slide_handler(State(state), Path(presentation_id), Json(payload)).await
}

pub(super) async fn duplicate_slide(
    State(state): State<AppState>,
    Path(ids): Path<(String, String)>,
) -> Result<Json<Vec<presenter_core::Slide>>, AppError> {
    super::duplicate_slide_handler(State(state), Path(ids)).await
}

pub(super) async fn delete_slide(
    State(state): State<AppState>,
    Path(ids): Path<(String, String)>,
) -> Result<Json<Vec<presenter_core::Slide>>, AppError> {
    super::delete_slide_handler(State(state), Path(ids)).await
}

pub(super) async fn reorder_slides(
    State(state): State<AppState>,
    Path(presentation_id): Path<String>,
    Json(payload): Json<ReorderSlidesRequest>,
) -> Result<Json<Vec<presenter_core::Slide>>, AppError> {
    super::reorder_slides_handler(State(state), Path(presentation_id), Json(payload)).await
}

pub(super) async fn update_slide_content(
    State(state): State<AppState>,
    Path(ids): Path<(String, String)>,
    Json(payload): Json<SlideContentUpdateRequest>,
) -> Result<Json<presenter_core::Slide>, AppError> {
    super::update_slide_content_handler(State(state), Path(ids), Json(payload)).await
}
