use axum::{
    extract::{Path, State},
    http::HeaderMap,
    Json,
};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use super::super::AppError;
use super::extract_actor;
use crate::state::AppState;
use presenter_core::{VideoSource, VideoSourceDraft, VideoSourceId};
use presenter_persistence::SettingsAuditSource;
use uuid::Uuid;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VideoSourceDto {
    id: VideoSourceId,
    label: String,
    ndi_name: String,
    is_active: bool,
    created_at: String,
    updated_at: String,
}

impl VideoSourceDto {
    fn from_source(source: VideoSource) -> Self {
        Self {
            id: source.id,
            label: source.label,
            ndi_name: source.ndi_name,
            is_active: source.is_active,
            created_at: source.created_at.to_rfc3339(),
            updated_at: source.updated_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VideoSourceRequest {
    label: String,
    ndi_name: String,
}

#[instrument(skip_all)]
pub(crate) async fn list_video_sources(
    State(state): State<AppState>,
) -> Result<Json<Vec<VideoSourceDto>>, AppError> {
    let sources = state.list_video_sources().await?;
    let payload = sources
        .into_iter()
        .map(VideoSourceDto::from_source)
        .collect::<Vec<_>>();
    Ok(Json(payload))
}

#[instrument(skip_all)]
pub(crate) async fn create_video_source(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<VideoSourceRequest>,
) -> Result<Json<VideoSourceDto>, AppError> {
    let draft = VideoSourceDraft::new(payload.label, payload.ndi_name);
    let actor = extract_actor(&headers);
    let source = state
        .create_video_source(draft, SettingsAuditSource::HttpSetter, &actor)
        .await?;
    Ok(Json(VideoSourceDto::from_source(source)))
}

#[instrument(skip_all)]
pub(crate) async fn update_video_source(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(payload): Json<VideoSourceRequest>,
) -> Result<Json<VideoSourceDto>, AppError> {
    let draft = VideoSourceDraft::new(payload.label, payload.ndi_name);
    let actor = extract_actor(&headers);
    let source = state
        .update_video_source(
            VideoSourceId::from_uuid(id),
            draft,
            SettingsAuditSource::HttpSetter,
            &actor,
        )
        .await?;
    Ok(Json(VideoSourceDto::from_source(source)))
}

#[instrument(skip_all)]
pub(crate) async fn delete_video_source(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<axum::http::StatusCode, AppError> {
    let actor = extract_actor(&headers);
    state
        .delete_video_source(
            VideoSourceId::from_uuid(id),
            SettingsAuditSource::HttpSetter,
            &actor,
        )
        .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[instrument(skip_all)]
pub(crate) async fn activate_video_source(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<VideoSourceDto>, AppError> {
    let actor = extract_actor(&headers);
    let source = state
        .activate_video_source(
            VideoSourceId::from_uuid(id),
            SettingsAuditSource::HttpSetter,
            &actor,
        )
        .await?;
    Ok(Json(VideoSourceDto::from_source(source)))
}

#[instrument(skip_all)]
pub(crate) async fn deactivate_video_sources(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<axum::http::StatusCode, AppError> {
    let actor = extract_actor(&headers);
    state
        .deactivate_video_sources(SettingsAuditSource::HttpSetter, &actor)
        .await?;
    Ok(axum::http::StatusCode::OK)
}
