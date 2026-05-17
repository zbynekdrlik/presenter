use axum::{
    extract::{Path, State},
    http::HeaderMap,
    Json,
};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use super::extract_actor;
use super::super::AppError;
use crate::resolume::ResolumeConnectionSnapshot;
use crate::state::AppState;
use presenter_core::{ResolumeHost, ResolumeHostDraft, ResolumeHostId};
use presenter_persistence::SettingsAuditSource;
use uuid::Uuid;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResolumeHostDto {
    id: ResolumeHostId,
    label: String,
    host: String,
    port: u16,
    is_enabled: bool,
    created_at: String,
    updated_at: String,
    status: ResolumeConnectionSnapshot,
}

impl ResolumeHostDto {
    fn from_host(host: ResolumeHost, status: ResolumeConnectionSnapshot) -> Self {
        Self {
            id: host.id,
            label: host.label,
            host: host.host,
            port: host.port,
            is_enabled: host.is_enabled,
            created_at: host.created_at.to_rfc3339(),
            updated_at: host.updated_at.to_rfc3339(),
            status,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResolumeHostRequest {
    label: String,
    host: String,
    #[serde(default = "default_resolume_port")]
    port: u16,
    #[serde(default = "super::default_true")]
    is_enabled: bool,
}

const fn default_resolume_port() -> u16 {
    8090
}

#[instrument(skip_all)]
pub(crate) async fn list_resolume_hosts(
    State(state): State<AppState>,
) -> Result<Json<Vec<ResolumeHostDto>>, AppError> {
    let hosts = state.list_resolume_hosts().await?;
    let statuses = state.resolume_status_snapshot().await;
    let payload = hosts
        .into_iter()
        .map(|host| {
            let status = statuses
                .get(&host.id)
                .cloned()
                .unwrap_or_else(ResolumeConnectionSnapshot::disabled);
            ResolumeHostDto::from_host(host, status)
        })
        .collect::<Vec<_>>();
    Ok(Json(payload))
}

#[instrument(skip_all)]
pub(crate) async fn create_resolume_host(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ResolumeHostRequest>,
) -> Result<Json<ResolumeHostDto>, AppError> {
    let draft = ResolumeHostDraft::new(payload.label, payload.host, payload.port)
        .with_enabled(payload.is_enabled);
    let actor = extract_actor(&headers, None);
    let host = state
        .create_resolume_host(draft, SettingsAuditSource::HttpSetter, &actor)
        .await?;
    let status = state.resolume_status_for(host.id).await;
    Ok(Json(ResolumeHostDto::from_host(host, status)))
}

#[instrument(skip_all)]
pub(crate) async fn update_resolume_host(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(payload): Json<ResolumeHostRequest>,
) -> Result<Json<ResolumeHostDto>, AppError> {
    let draft = ResolumeHostDraft::new(payload.label, payload.host, payload.port)
        .with_enabled(payload.is_enabled);
    let actor = extract_actor(&headers, None);
    let host = state
        .update_resolume_host(
            ResolumeHostId::from_uuid(id),
            draft,
            SettingsAuditSource::HttpSetter,
            &actor,
        )
        .await?;
    let status = state.resolume_status_for(host.id).await;
    Ok(Json(ResolumeHostDto::from_host(host, status)))
}

#[instrument(skip_all)]
pub(crate) async fn delete_resolume_host(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<axum::http::StatusCode, AppError> {
    let actor = extract_actor(&headers, None);
    state
        .delete_resolume_host(
            ResolumeHostId::from_uuid(id),
            SettingsAuditSource::HttpSetter,
            &actor,
        )
        .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TestConnectionResponse {
    success: bool,
    latency_ms: Option<f64>,
    error: Option<String>,
}

#[instrument(skip_all)]
pub(crate) async fn test_resolume_host(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<TestConnectionResponse>, AppError> {
    let result = state
        .test_resolume_host_connection(ResolumeHostId::from_uuid(id))
        .await?;
    Ok(Json(TestConnectionResponse {
        success: result.success,
        latency_ms: result.latency_ms,
        error: result.error,
    }))
}
