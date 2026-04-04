use axum::{extract::State, Json};
use presenter_core::FeatureFlags;
use serde::Deserialize;
use tracing::instrument;

use super::AppError;
use crate::state::AppState;

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
) -> Result<Json<FeatureFlags>, AppError> {
    Ok(Json(FeatureFlags {
        companion_enabled: state.companion_enabled(),
        companion_port: state.companion_port(),
    }))
}

#[instrument(skip_all)]
pub(super) async fn update_feature_settings(
    State(state): State<AppState>,
    Json(payload): Json<FeatureSettingsRequest>,
) -> Result<Json<FeatureFlags>, AppError> {
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
    Ok(Json(FeatureFlags {
        companion_enabled: state.companion_enabled(),
        companion_port: state.companion_port(),
    }))
}
