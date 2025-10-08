use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::state::AppState;
use super::AppError;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct FeatureSettingsResponse {
    pub(super) companion_enabled: bool,
    pub(super) companion_port: u16,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct FeatureSettingsRequest {
    #[serde(alias = "enabled", alias = "companion_enabled")]
    pub(super) companion_enabled: bool,
    #[serde(default, alias = "companion_port", alias = "port")]
    pub(super) companion_port: Option<u16>,
}

#[instrument(skip_all)]
pub(super) async fn get_feature_settings(
    State(state): State<AppState>,
) -> Result<Json<FeatureSettingsResponse>, AppError> {
    Ok(Json(FeatureSettingsResponse {
        companion_enabled: state.companion_enabled(),
        companion_port: state.companion_port(),
    }))
}

#[instrument(skip_all)]
pub(super) async fn update_feature_settings(
    State(state): State<AppState>,
    Json(payload): Json<FeatureSettingsRequest>,
) -> Result<Json<FeatureSettingsResponse>, AppError> {
    let requested_port = payload
        .companion_port
        .unwrap_or_else(|| state.companion_port());
    if requested_port == 0 {
        return Err(AppError::bad_request_message(
            "companionPort must be between 1 and 65535",
        ));
    }

    state
        .set_companion_settings(payload.companion_enabled, requested_port)
        .await?;
    Ok(Json(FeatureSettingsResponse {
        companion_enabled: state.companion_enabled(),
        companion_port: state.companion_port(),
    }))
}

