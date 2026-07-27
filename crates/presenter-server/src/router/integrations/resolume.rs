use axum::{
    extract::{Path, State},
    http::HeaderMap,
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::instrument;

use super::super::AppError;
use super::extract_actor;
use crate::resolume::{ResolumeConnectionSnapshot, ResolumeErrorKind};
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

/// Maps a repository refusal to its HTTP status via the TYPED
/// `RepositoryError` variant returned by the persistence layer — never a
/// string match on the `Display` text (#586, mirrors `router/libraries.rs`'s
/// `map_repository_not_found`, #584). Any other error falls through to the
/// default 500 mapping.
fn map_repository_not_found(err: anyhow::Error) -> AppError {
    match err.downcast_ref::<presenter_persistence::RepositoryError>() {
        Some(presenter_persistence::RepositoryError::NotFound(msg)) => AppError::not_found(*msg),
        _ => err.into(),
    }
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
    let actor = extract_actor(&headers);
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
    let actor = extract_actor(&headers);
    let host = state
        .update_resolume_host(
            ResolumeHostId::from_uuid(id),
            draft,
            SettingsAuditSource::HttpSetter,
            &actor,
        )
        .await
        .map_err(map_repository_not_found)?;
    let status = state.resolume_status_for(host.id).await;
    Ok(Json(ResolumeHostDto::from_host(host, status)))
}

#[instrument(skip_all)]
pub(crate) async fn delete_resolume_host(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<axum::http::StatusCode, AppError> {
    let actor = extract_actor(&headers);
    state
        .delete_resolume_host(
            ResolumeHostId::from_uuid(id),
            SettingsAuditSource::HttpSetter,
            &actor,
        )
        .await
        .map_err(map_repository_not_found)?;
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

/// #563d/#564: the lightweight, operator-page-facing status poll — every
/// enabled host's connection state, error classification, backoff
/// countdown, configured-vs-active port (and whether it drifted), and any
/// missing composition clips. Mirrors the `/integrations/video-sources/status`
/// pattern (#546): a dedicated poll endpoint distinct from the settings
/// page's CRUD listing, so the operator header can poll on its own cheap
/// cadence without pulling in host-management concerns.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResolumeConnectionStatusDto {
    host_id: ResolumeHostId,
    label: String,
    is_enabled: bool,
    configured_port: u16,
    active_port: Option<u16>,
    port_drifted: bool,
    state: crate::resolume::ResolumeConnectionState,
    last_success: Option<chrono::DateTime<Utc>>,
    last_latency_ms: Option<f64>,
    last_error: Option<String>,
    last_error_kind: Option<ResolumeErrorKind>,
    consecutive_failures: u32,
    next_retry_in_secs: Option<i64>,
    /// #564: how long the host has been continuously erroring — computed
    /// SERVER-SIDE (like `next_retry_in_secs`) so the WASM client never has
    /// to parse/diff timestamps itself, just render a number. Feeds the
    /// chip's green/yellow/red threshold (failing >2 min ⇒ red).
    failing_for_secs: Option<i64>,
    missing_clips: Vec<String>,
}

impl ResolumeConnectionStatusDto {
    fn from_host(
        host: ResolumeHost,
        status: ResolumeConnectionSnapshot,
        now: chrono::DateTime<Utc>,
    ) -> Self {
        let failing_for_secs = status
            .error_since
            .map(|since| (now - since).num_seconds().max(0));
        Self {
            host_id: host.id,
            label: host.label,
            is_enabled: host.is_enabled,
            configured_port: host.port,
            active_port: status.active_port,
            port_drifted: status.active_port.is_some(),
            next_retry_in_secs: status.next_retry_in_secs(now),
            state: status.state,
            last_success: status.last_success,
            last_latency_ms: status.last_latency_ms,
            last_error: status.last_error,
            last_error_kind: status.last_error_kind,
            consecutive_failures: status.consecutive_failures,
            failing_for_secs,
            missing_clips: status.missing_clips,
        }
    }
}

#[instrument(skip_all)]
pub(crate) async fn resolume_connection_status(
    State(state): State<AppState>,
) -> Result<Json<Vec<ResolumeConnectionStatusDto>>, AppError> {
    let hosts = state.list_resolume_hosts().await?;
    let statuses = state.resolume_status_snapshot().await;
    let now = Utc::now();
    let payload = hosts
        .into_iter()
        .map(|host| {
            let status = statuses
                .get(&host.id)
                .cloned()
                .unwrap_or_else(ResolumeConnectionSnapshot::disabled);
            ResolumeConnectionStatusDto::from_host(host, status, now)
        })
        .collect::<Vec<_>>();
    Ok(Json(payload))
}

#[cfg(test)]
mod status_dto_tests {
    use super::*;
    use crate::resolume::ResolumeConnectionState;

    fn sample_host(port: u16, active_port: Option<u16>) -> ResolumeHost {
        let now = Utc::now();
        ResolumeHost::new(
            ResolumeHostId::new(),
            "resolume-pp".to_string(),
            "10.77.8.201".to_string(),
            port,
            true,
            now,
            now,
        )
        .with_active_port(active_port)
    }

    /// #564: no drift discovered → `portDrifted` is false and `activePort` is
    /// `null`, so the operator chip renders the plain "connected" state with
    /// no drift note.
    #[test]
    fn dto_reports_no_drift_when_active_port_is_unset() {
        let host = sample_host(8090, None);
        let status = ResolumeConnectionSnapshot::disabled();
        let dto = ResolumeConnectionStatusDto::from_host(host, status, Utc::now());
        assert_eq!(dto.configured_port, 8090);
        assert_eq!(dto.active_port, None);
        assert!(!dto.port_drifted);
    }

    /// #564: a discovered drift is a DIFFERENT port from the configured one —
    /// `portDrifted` must be true and both ports present, so the tooltip can
    /// render "drifted 8090→8091".
    #[test]
    fn dto_reports_a_drift_when_active_port_differs_from_configured() {
        let host = sample_host(8090, Some(8091));
        let mut status = ResolumeConnectionSnapshot::disabled();
        status.state = ResolumeConnectionState::Connected;
        status.active_port = Some(8091);
        let dto = ResolumeConnectionStatusDto::from_host(host, status, Utc::now());
        assert_eq!(dto.configured_port, 8090);
        assert_eq!(dto.active_port, Some(8091));
        assert!(dto.port_drifted);
    }

    /// #563d: `nextRetryInSecs` is derived FRESH at read time from the
    /// snapshot's absolute `next_retry_at`, so it reflects however long has
    /// actually elapsed since the error was recorded — not a stale value
    /// baked in at write time.
    #[test]
    fn dto_derives_next_retry_in_secs_from_the_snapshot_at_read_time() {
        let host = sample_host(8090, None);
        let mut status = ResolumeConnectionSnapshot::disabled();
        let now = Utc::now();
        status.next_retry_at = Some(now + chrono::Duration::seconds(30));
        let dto = ResolumeConnectionStatusDto::from_host(
            host,
            status,
            now + chrono::Duration::seconds(10),
        );
        assert_eq!(dto.next_retry_in_secs, Some(20));
    }

    /// #564: `failingForSecs` is likewise computed server-side, from
    /// `error_since`, so the chip's red-threshold check never needs the WASM
    /// client to parse or diff a timestamp itself.
    #[test]
    fn dto_derives_failing_for_secs_from_error_since_at_read_time() {
        let host = sample_host(8090, None);
        let mut status = ResolumeConnectionSnapshot::disabled();
        status.state = ResolumeConnectionState::Error;
        let now = Utc::now();
        status.error_since = Some(now - chrono::Duration::seconds(150));
        let dto = ResolumeConnectionStatusDto::from_host(host, status, now);
        assert_eq!(dto.failing_for_secs, Some(150));
    }

    #[test]
    fn dto_reports_no_failing_duration_when_healthy() {
        let host = sample_host(8090, None);
        let status = ResolumeConnectionSnapshot::disabled();
        let dto = ResolumeConnectionStatusDto::from_host(host, status, Utc::now());
        assert_eq!(dto.failing_for_secs, None);
    }
}
