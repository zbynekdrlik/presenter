use super::AppError;
use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use presenter_core::{Library, LibraryId, LibrarySummary};
use tracing::instrument;
use uuid::Uuid;

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CreateLibraryRequest {
    pub(super) name: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RenameLibraryRequest {
    pub(super) name: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UpdateLibraryFavoriteRequest {
    pub(super) favorite: bool,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct LibrarySummaryQuery {
    #[serde(default)]
    pub(super) q: Option<String>,
}

#[instrument(skip_all)]
pub(super) async fn list_library_summaries(
    State(state): State<AppState>,
    Query(params): Query<LibrarySummaryQuery>,
) -> Result<Json<Vec<LibrarySummary>>, AppError> {
    let summaries = state.library_summaries(params.q.as_deref()).await?;
    Ok(Json(summaries))
}

#[instrument(skip_all)]
pub(super) async fn list_libraries(
    State(state): State<AppState>,
) -> Result<Json<Vec<Library>>, AppError> {
    let libraries = state.libraries().await?;
    Ok(Json(libraries))
}

#[instrument(skip_all)]
pub(super) async fn create_library(
    State(state): State<AppState>,
    Json(payload): Json<CreateLibraryRequest>,
) -> Result<Json<Library>, AppError> {
    let name = payload.name.trim();
    if name.is_empty() {
        return Err(AppError::bad_request_message("name cannot be empty"));
    }
    let library = state.create_library(name).await?;
    Ok(Json(library))
}

#[instrument(skip_all)]
pub(super) async fn rename_library(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<RenameLibraryRequest>,
) -> Result<StatusCode, AppError> {
    let name = payload.name.trim();
    if name.is_empty() {
        return Err(AppError::bad_request_message("name cannot be empty"));
    }
    state.rename_library(LibraryId::from_uuid(id), name).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[instrument(skip_all)]
pub(super) async fn delete_library(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    state.delete_library(LibraryId::from_uuid(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CreateLibraryPresentationRequest {
    #[serde(default)]
    pub(super) name: Option<String>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CreateLibraryPresentationResponse {
    pub(super) library_id: Uuid,
    pub(super) presentation: presenter_core::Presentation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) library_summary: Option<LibrarySummary>,
}

#[instrument(skip_all)]
pub(super) async fn create_library_presentation(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<CreateLibraryPresentationRequest>,
) -> Result<Json<CreateLibraryPresentationResponse>, AppError> {
    let name = payload.name.unwrap_or_default().trim().to_string();
    if name.is_empty() {
        return Err(AppError::bad_request_message("name cannot be empty"));
    }
    let library_id = LibraryId::from_uuid(id);
    let (created_library_id, _library_name, presentation, summary) =
        state.create_presentation(library_id, &name).await?;
    if created_library_id != library_id {
        return Err(AppError::bad_request_message(
            "created presentation belongs to a different library",
        ));
    }
    Ok(Json(CreateLibraryPresentationResponse {
        library_id: created_library_id.into_uuid(),
        presentation,
        library_summary: summary,
    }))
}

#[instrument(skip_all)]
pub(super) async fn set_library_favorite(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(payload): Json<UpdateLibraryFavoriteRequest>,
) -> Result<StatusCode, AppError> {
    state
        .set_library_favorite(LibraryId::from_uuid(id), payload.favorite)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
