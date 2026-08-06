use axum::{extract::State, http::HeaderMap, Json};
use serde::Deserialize;
use tracing::instrument;

use super::super::AppError;
use super::extract_actor;
use crate::state::AppState;
use presenter_core::{AbleSetSettings, AbleSetSettingsDraft};
use presenter_persistence::SettingsAuditSource;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AbleSetFollowPayload {
    pub(super) enabled: bool,
}

/// #601: acknowledges a CURRENT title mismatch. The client sends the exact
/// titles it saw on `GET /integrations/ableset/status` — the ack is bound to
/// that pair (per the settled design), not just to the number, so a later
/// title change on either side re-raises the warning.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AcknowledgeAbleSetMismatchPayload {
    pub(super) number: String,
    pub(super) ableset_title: String,
    pub(super) presenter_title: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UnacknowledgeAbleSetMismatchPayload {
    pub(super) number: String,
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
    headers: HeaderMap,
    Json(payload): Json<AbleSetSettingsDraft>,
) -> Result<Json<AbleSetSettings>, AppError> {
    let actor = extract_actor(&headers);
    let settings = state
        .update_ableset_settings(payload, SettingsAuditSource::HttpSetter, &actor)
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

/// #601: "visible/revocable" per the settled design. Idempotent — there is
/// no NotFound case (acknowledging a number that isn't currently mismatched
/// just sits unused until it applies), so a genuine failure here is a real
/// internal fault and falls through to the router's default 500.
#[instrument(skip_all)]
pub(crate) async fn acknowledge_ableset_mismatch(
    State(state): State<AppState>,
    Json(payload): Json<AcknowledgeAbleSetMismatchPayload>,
) -> Result<Json<crate::ableset::AbleSetStatusSnapshot>, AppError> {
    let snapshot = state
        .acknowledge_ableset_mismatch(
            &payload.number,
            &payload.ableset_title,
            &payload.presenter_title,
        )
        .await?;
    Ok(Json(snapshot))
}

#[instrument(skip_all)]
pub(crate) async fn unacknowledge_ableset_mismatch(
    State(state): State<AppState>,
    Json(payload): Json<UnacknowledgeAbleSetMismatchPayload>,
) -> Result<Json<crate::ableset::AbleSetStatusSnapshot>, AppError> {
    let snapshot = state
        .unacknowledge_ableset_mismatch(&payload.number)
        .await?;
    Ok(Json(snapshot))
}
