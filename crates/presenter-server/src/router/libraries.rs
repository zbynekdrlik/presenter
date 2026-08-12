use super::AppError;
use crate::state::AppState;
use axum::{
    extract::{Multipart, Path, Query, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, Utc};
use presenter_core::{Library, LibraryId, LibrarySummary};
use serde::Serialize;
use tracing::instrument;
use uuid::Uuid;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct FavoriteLibraryIdsResponse {
    pub(super) ids: Vec<String>,
}

/// Trash row for the settings UI (#644, mirrors `TrashedPresentationDto` in
/// `state/sync.rs`).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TrashedLibraryDto {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) deleted_at: DateTime<Utc>,
}

impl From<presenter_persistence::TrashedLibrary> for TrashedLibraryDto {
    fn from(t: presenter_persistence::TrashedLibrary) -> Self {
        Self {
            id: t.id,
            name: t.name,
            deleted_at: t.deleted_at,
        }
    }
}

#[instrument(skip_all)]
pub(super) async fn list_trashed_libraries(
    State(state): State<AppState>,
) -> Result<Json<Vec<TrashedLibraryDto>>, AppError> {
    let rows = state.list_trashed_libraries().await?;
    Ok(Json(rows.into_iter().map(Into::into).collect()))
}

#[instrument(skip_all)]
pub(super) async fn restore_library(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    // #633: `RepositoryError::NotFound`/`Conflict` map to 404/409 by default
    // via the centralized `From<anyhow::Error> for AppError` — no
    // per-call-site helper needed.
    state.restore_library(LibraryId::from_uuid(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[instrument(skip_all)]
pub(super) async fn list_library_favorites(
    State(state): State<AppState>,
) -> Result<Json<FavoriteLibraryIdsResponse>, AppError> {
    let favorites = state.library_favorites().await?;
    let ids = favorites.into_iter().map(|id| id.to_string()).collect();
    Ok(Json(FavoriteLibraryIdsResponse { ids }))
}

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
    // #633: `RepositoryError::NotFound` maps to 404 by default via the
    // centralized `From<anyhow::Error> for AppError` — no per-call-site
    // helper needed.
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
pub(super) struct CreateSlideInput {
    pub(super) main: String,
    #[serde(default)]
    pub(super) translation: Option<String>,
    #[serde(default)]
    pub(super) stage: Option<String>,
    #[serde(default)]
    pub(super) group: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CreateLibraryPresentationRequest {
    #[serde(default)]
    pub(super) name: Option<String>,
    #[serde(default)]
    pub(super) slides: Option<Vec<CreateSlideInput>>,
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

    let slides = match payload.slides {
        Some(ref inputs) if !inputs.is_empty() => {
            let mut built = Vec::with_capacity(inputs.len());
            for (i, input) in inputs.iter().enumerate() {
                let content = presenter_core::SlideContent::new(
                    presenter_core::SlideText::new(&input.main)
                        .map_err(|e| AppError::bad_request(e))?,
                    presenter_core::SlideText::new(input.translation.as_deref().unwrap_or(""))
                        .map_err(|e| AppError::bad_request(e))?,
                    presenter_core::SlideText::new(input.stage.as_deref().unwrap_or(""))
                        .map_err(|e| AppError::bad_request(e))?,
                    input
                        .group
                        .as_deref()
                        .filter(|g| !g.is_empty())
                        .map(presenter_core::SlideGroup::new),
                );
                built.push(presenter_core::Slide::new(i as u32, content));
            }
            Some(built)
        }
        _ => None,
    };

    let (created_library_id, _library_name, presentation, summary) = state
        .create_presentation(library_id, &name, slides.as_deref())
        .await?;
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
pub(super) async fn import_presentation(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    mut multipart: Multipart,
) -> Result<Json<CreateLibraryPresentationResponse>, AppError> {
    let library_id = LibraryId::from_uuid(id);
    let mut file_bytes: Option<Vec<u8>> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::bad_request(e))?
    {
        if field.name() == Some("file") {
            file_bytes = Some(
                field
                    .bytes()
                    .await
                    .map_err(|e| AppError::bad_request(e))?
                    .to_vec(),
            );
            break;
        }
    }

    let bytes = file_bytes.ok_or_else(|| AppError::bad_request_message("missing file field"))?;
    let imported = presenter_importer::load_presentation_from_bytes(&bytes)
        .map_err(|e| AppError::bad_request(e))?;

    let slides: Vec<presenter_core::Slide> = imported.slides;
    let name = imported.name;
    if name.trim().is_empty() {
        return Err(AppError::bad_request_message(
            "imported presentation has no name",
        ));
    }

    let (created_library_id, _library_name, presentation, summary) = state
        .create_presentation(library_id, name.trim(), Some(&slides))
        .await?;

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
