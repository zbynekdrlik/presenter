use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use super::super::AppError;
use crate::state::AppState;
use presenter_core::{OscSettings, OscSettingsDraft, VelocityMode};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OscSettingsResponse {
    enabled: bool,
    listen_port: u16,
    address_pattern: String,
    velocity_mode: VelocityMode,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<OscSettings> for OscSettingsResponse {
    fn from(settings: OscSettings) -> Self {
        Self {
            enabled: settings.enabled,
            listen_port: settings.listen_port,
            address_pattern: settings.address_pattern,
            velocity_mode: settings.velocity_mode,
            created_at: settings.created_at,
            updated_at: settings.updated_at,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateOscSettingsRequest {
    enabled: bool,
    listen_port: u16,
    address_pattern: String,
    velocity_mode: VelocityMode,
}

#[instrument(skip_all)]
pub(crate) async fn get_osc_settings(
    State(state): State<AppState>,
) -> Result<Json<OscSettingsResponse>, AppError> {
    let settings = state.osc_settings().await?;
    Ok(Json(OscSettingsResponse::from(settings)))
}

#[instrument(skip_all)]
pub(crate) async fn update_osc_settings(
    State(state): State<AppState>,
    Json(payload): Json<UpdateOscSettingsRequest>,
) -> Result<Json<OscSettingsResponse>, AppError> {
    if payload.address_pattern.trim().is_empty() {
        return Err(AppError::bad_request_message(
            "address pattern cannot be empty",
        ));
    }
    if payload.listen_port == 0 {
        return Err(AppError::bad_request_message(
            "listener port must be between 1 and 65535",
        ));
    }
    let draft = OscSettingsDraft {
        enabled: payload.enabled,
        listen_port: payload.listen_port,
        address_pattern: payload.address_pattern.trim().to_string(),
        velocity_mode: payload.velocity_mode,
    };
    let settings = state
        .update_osc_settings(draft)
        .await
        .map_err(|err| AppError::bad_request_message(err.to_string()))?;
    Ok(Json(OscSettingsResponse::from(settings)))
}

#[instrument(skip_all)]
pub(crate) async fn get_osc_status(
    State(state): State<AppState>,
) -> Result<Json<crate::osc::OscStatusSnapshot>, AppError> {
    Ok(Json(state.osc_status_snapshot().await))
}
