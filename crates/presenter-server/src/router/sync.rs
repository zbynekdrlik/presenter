//! #555 song-sync + trash HTTP surface: the two peer-facing read endpoints, the
//! operator status endpoint, and the trash list/restore endpoints.
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use tracing::instrument;

use super::AppError;
use crate::state::sync::{
    SyncManifestEntryDto, SyncPresentationDto, SyncStatus, TrashedPresentationDto,
};
use crate::state::AppState;
use presenter_core::PresentationId;

#[instrument(skip_all)]
pub(super) async fn get_sync_manifest(
    State(state): State<AppState>,
) -> Result<Json<Vec<SyncManifestEntryDto>>, AppError> {
    let rows = state.repository().list_sync_manifest().await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| SyncManifestEntryDto {
                sync_id: r.sync_id,
                library_name: r.library_name,
                name: r.name,
                updated_at: r.updated_at,
                deleted_at: r.deleted_at,
            })
            .collect(),
    ))
}

#[instrument(skip_all)]
pub(super) async fn get_sync_presentation(
    Path(sync_id): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<SyncPresentationDto>, AppError> {
    match state.repository().fetch_sync_presentation(&sync_id).await? {
        Some(p) => Ok(Json(SyncPresentationDto {
            sync_id: p.sync_id,
            library_name: p.library_name,
            name: p.name,
            updated_at: p.updated_at,
            deleted_at: p.deleted_at,
            slides: p.slides,
        })),
        None => Err(AppError::not_found(format!(
            "sync presentation {sync_id} not found"
        ))),
    }
}

#[instrument(skip_all)]
pub(super) async fn get_sync_status(
    State(state): State<AppState>,
) -> Result<Json<SyncStatus>, AppError> {
    Ok(Json(state.sync_status_snapshot().await))
}

#[instrument(skip_all)]
pub(super) async fn list_trash(
    State(state): State<AppState>,
) -> Result<Json<Vec<TrashedPresentationDto>>, AppError> {
    let rows = state.repository().list_trashed_presentations().await?;
    Ok(Json(rows.into_iter().map(Into::into).collect()))
}

#[instrument(skip_all)]
pub(super) async fn restore_presentation(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let uuid = super::parse_uuid("presentationId", &id)?;
    state
        .restore_presentation(PresentationId::from_uuid(uuid))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
