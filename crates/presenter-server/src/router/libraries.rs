use axum::{extract::{Path, Query, State}, http::StatusCode, Json};
use crate::state::AppState;
use super::{
  AppError,
  CreateLibraryPresentationRequest,
  CreateLibraryPresentationResponse,
  CreateLibraryRequest,
  LibrarySummaryQuery,
  RenameLibraryRequest,
  UpdateLibraryFavoriteRequest,
};
use uuid::Uuid;

pub(super) async fn list_library_summaries(
    State(state): State<AppState>,
    Query(params): Query<LibrarySummaryQuery>,
) -> Result<Json<Vec<presenter_core::LibrarySummary>>, AppError> {
    super::list_library_summaries(State(state), Query(params)).await
}

pub(super) async fn list_libraries(
    State(state): State<AppState>,
) -> Result<Json<Vec<presenter_core::Library>>, AppError> {
    super::list_libraries(State(state)).await
}

pub(super) async fn create_library(
    State(state): State<AppState>,
    Json(payload): Json<CreateLibraryRequest>,
) -> Result<Json<presenter_core::Library>, AppError> {
    super::create_library(State(state), Json(payload)).await
}

pub(super) async fn rename_library(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<RenameLibraryRequest>,
) -> Result<StatusCode, AppError> {
    super::rename_library(State(state), Path(id), Json(payload)).await
}

pub(super) async fn delete_library(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    super::delete_library(State(state), Path(id)).await
}

pub(super) async fn create_library_presentation(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<CreateLibraryPresentationRequest>,
) -> Result<Json<CreateLibraryPresentationResponse>, AppError> {
    super::create_library_presentation(State(state), Path(id), Json(payload)).await
}

pub(super) async fn set_library_favorite(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateLibraryFavoriteRequest>,
) -> Result<StatusCode, AppError> {
    super::set_library_favorite(State(state), Path(id), Json(payload)).await
}
