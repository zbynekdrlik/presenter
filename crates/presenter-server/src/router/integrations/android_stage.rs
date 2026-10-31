use axum::{
    extract::{Path, State},
    http::HeaderMap,
    Json,
};
use serde::{Deserialize, Serialize};
use tracing::instrument;
use uuid::Uuid;

use super::super::AppError;
use super::extract_actor;
use crate::android_stage::AndroidStageDisplayStatusSnapshot;
use crate::state::AppState;
use presenter_core::{
    AndroidStageDisplay, AndroidStageDisplayDraft, AndroidStageDisplayId, DEFAULT_ADB_PORT,
    DEFAULT_LAUNCH_PACKAGE,
};
use presenter_persistence::SettingsAuditSource;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AndroidStageDisplayDto {
    id: AndroidStageDisplayId,
    label: String,
    host: String,
    port: u16,
    launch_component: String,
    is_enabled: bool,
    created_at: String,
    updated_at: String,
    status: AndroidStageDisplayStatusSnapshot,
}

impl AndroidStageDisplayDto {
    fn from_display(
        display: AndroidStageDisplay,
        status: AndroidStageDisplayStatusSnapshot,
    ) -> Self {
        Self {
            id: display.id,
            label: display.label,
            host: display.host,
            port: display.port,
            launch_component: display.launch_component,
            is_enabled: display.is_enabled,
            created_at: display.created_at.to_rfc3339(),
            updated_at: display.updated_at.to_rfc3339(),
            status,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AndroidStageDisplayRequest {
    label: String,
    host: String,
    #[serde(default = "default_android_stage_port")]
    port: u16,
    /// Absent (the Settings form): a new display gets our stage app, an update
    /// keeps what is stored (#879).
    #[serde(default)]
    launch_component: Option<String>,
    #[serde(default = "super::default_true")]
    is_enabled: bool,
}

const fn default_android_stage_port() -> u16 {
    DEFAULT_ADB_PORT
}

fn normalize_launch_component(component: Option<&str>) -> String {
    match component.map(str::trim) {
        Some(trimmed) if !trimmed.is_empty() => trimmed.to_string(),
        _ => DEFAULT_LAUNCH_PACKAGE.to_string(),
    }
}

#[instrument(skip_all)]
pub(crate) async fn list_android_stage_displays(
    State(state): State<AppState>,
) -> Result<Json<Vec<AndroidStageDisplayDto>>, AppError> {
    let displays = state.list_android_stage_displays().await?;
    let statuses = state.android_stage_status_snapshot().await;
    let payload = displays
        .into_iter()
        .map(|display| {
            let status = statuses
                .get(&display.id)
                .cloned()
                .unwrap_or_else(AndroidStageDisplayStatusSnapshot::disabled);
            AndroidStageDisplayDto::from_display(display, status)
        })
        .collect();
    Ok(Json(payload))
}

#[instrument(skip_all)]
pub(crate) async fn create_android_stage_display(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<AndroidStageDisplayRequest>,
) -> Result<Json<AndroidStageDisplayDto>, AppError> {
    let draft = AndroidStageDisplayDraft::new(payload.label, payload.host)
        .with_port(payload.port)
        .with_launch_component(normalize_launch_component(
            payload.launch_component.as_deref(),
        ))
        .with_enabled(payload.is_enabled);
    let actor = extract_actor(&headers);
    let display = state
        .create_android_stage_display(draft, SettingsAuditSource::HttpSetter, &actor)
        .await?;
    let status = state.android_stage_status_for(display.id).await;
    Ok(Json(AndroidStageDisplayDto::from_display(display, status)))
}

#[instrument(skip_all)]
pub(crate) async fn update_android_stage_display(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(payload): Json<AndroidStageDisplayRequest>,
) -> Result<Json<AndroidStageDisplayDto>, AppError> {
    let id = AndroidStageDisplayId::from_uuid(id);
    let stored = state
        .list_android_stage_displays()
        .await?
        .into_iter()
        .find(|display| display.id == id)
        .map(|display| display.launch_component);
    let launch_component = payload.launch_component.or(stored);
    let draft = AndroidStageDisplayDraft::new(payload.label, payload.host)
        .with_port(payload.port)
        .with_launch_component(normalize_launch_component(launch_component.as_deref()))
        .with_enabled(payload.is_enabled);
    let actor = extract_actor(&headers);
    // #633: `RepositoryError::NotFound` maps to 404 by default via the
    // centralized `From<anyhow::Error> for AppError`.
    let display = state
        .update_android_stage_display(id, draft, SettingsAuditSource::HttpSetter, &actor)
        .await?;
    let status = state.android_stage_status_for(display.id).await;
    Ok(Json(AndroidStageDisplayDto::from_display(display, status)))
}

#[instrument(skip_all)]
pub(crate) async fn delete_android_stage_display(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<axum::http::StatusCode, AppError> {
    let actor = extract_actor(&headers);
    state
        .delete_android_stage_display(
            AndroidStageDisplayId::from_uuid(id),
            SettingsAuditSource::HttpSetter,
            &actor,
        )
        .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[instrument(skip_all)]
pub(crate) async fn launch_now_android_stage_display(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<axum::http::StatusCode, AppError> {
    state
        .launch_now_android_stage_display(AndroidStageDisplayId::from_uuid(id))
        .await
        .map_err(|err| AppError::bad_request_message(err.to_string()))?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}
