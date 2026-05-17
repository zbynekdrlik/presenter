use axum::{extract::State, Json};
use serde::Deserialize;
use tracing::instrument;

use super::super::AppError;
use crate::state::AppState;
use presenter_core::{AbleSetSettings, AbleSetSettingsDraft};
use presenter_persistence::SettingsAuditSource;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AbleSetFollowPayload {
    pub(super) enabled: bool,
}

#[instrument(skip_all)]
pub(crate) async fn get_ableset_settings(
    State(state): State<AppState>,
) -> Result<Json<AbleSetSettings>, AppError> {
    let settings = state.ableset_settings().await?;
    Ok(Json(settings))
}

#[instrument(skip_all)]
pub(crate) async fn update_ableset_settings(
    State(state): State<AppState>,
    Json(payload): Json<AbleSetSettingsDraft>,
) -> Result<Json<AbleSetSettings>, AppError> {
    // HTTP wiring (Task 11) will replace these placeholders with the real actor + source.
    let settings = state
        .update_ableset_settings(payload, SettingsAuditSource::HttpSetter, "http")
        .await
        .map_err(|err| AppError::bad_request_message(err.to_string()))?;
    Ok(Json(settings))
}

#[instrument(skip_all)]
pub(crate) async fn get_ableset_status(
    State(state): State<AppState>,
) -> Result<Json<crate::ableset::AbleSetStatusSnapshot>, AppError> {
    Ok(Json(state.ableset_status_snapshot().await))
}

#[instrument(skip_all)]
pub(crate) async fn set_ableset_follow(
    State(state): State<AppState>,
    Json(payload): Json<AbleSetFollowPayload>,
) -> Result<Json<crate::ableset::AbleSetStatusSnapshot>, AppError> {
    let snapshot = state.set_ableset_follow(payload.enabled).await;
    Ok(Json(snapshot))
}
