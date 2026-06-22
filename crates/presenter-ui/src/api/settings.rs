//! Client API for the settings page (`/ui/settings`).
//!
//! Wraps every `/integrations/*` and `/settings/features` endpoint the settings
//! UI talks to. The status snapshots (`ResolumeConnectionSnapshot`,
//! `AndroidStageDisplayStatusSnapshot`, `OscStatusSnapshot`) live in
//! `presenter-server` (not `presenter-core`), so we mirror their JSON shape with
//! local DTOs that deserialize the camelCase contract the page consumes — the
//! same approach `api/ndi.rs` uses for `VideoSourceDto`. Timestamps arrive as
//! RFC3339 strings and are kept as `String` for client-side formatting.

use super::{delete, get_json, post_json, put_json, ApiError};
pub use presenter_core::AbleSetStatusSnapshot;
use presenter_core::{AbleSetSettings, AbleSetSettingsDraft, OscSettingsDraft};
use serde::{Deserialize, Serialize};

// ── AbleSet ──────────────────────────────────────────────────────────────

pub async fn get_ableset_status() -> Result<AbleSetStatusSnapshot, ApiError> {
    get_json("/integrations/ableset/status").await
}

pub async fn get_ableset_settings() -> Result<AbleSetSettings, ApiError> {
    get_json("/integrations/ableset/settings").await
}

pub async fn update_ableset_settings(
    draft: &AbleSetSettingsDraft,
) -> Result<AbleSetSettings, ApiError> {
    put_json("/integrations/ableset/settings", draft).await
}

#[derive(Serialize)]
struct AbleSetFollowPayload {
    enabled: bool,
}

pub async fn set_ableset_follow(enabled: bool) -> Result<AbleSetStatusSnapshot, ApiError> {
    post_json(
        "/integrations/ableset/follow",
        &AbleSetFollowPayload { enabled },
    )
    .await
}

// ── Feature flags (Companion) ──────────────────────────────────────────────

pub use presenter_core::FeatureFlags;

pub async fn get_features() -> Result<FeatureFlags, ApiError> {
    get_json("/settings/features").await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeatureFlagsDraft {
    pub companion_enabled: bool,
    pub companion_port: u16,
}

pub async fn update_features(draft: &FeatureFlagsDraft) -> Result<FeatureFlags, ApiError> {
    post_json("/settings/features", draft).await
}

// ── Resolume hosts ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolumeStatusDto {
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub last_latency_ms: Option<f64>,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub consecutive_failures: u32,
    #[serde(default)]
    pub error_since: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolumeHostDto {
    pub id: String,
    pub label: String,
    pub host: String,
    pub port: u16,
    pub is_enabled: bool,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub status: Option<ResolumeStatusDto>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolumeHostDraft {
    pub label: String,
    pub host: String,
    pub port: u16,
    pub is_enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolumeTestResult {
    pub success: bool,
    #[serde(default)]
    pub latency_ms: Option<f64>,
    #[serde(default)]
    pub error: Option<String>,
}

pub async fn list_resolume_hosts() -> Result<Vec<ResolumeHostDto>, ApiError> {
    get_json("/integrations/resolume/hosts").await
}

pub async fn create_resolume_host(draft: &ResolumeHostDraft) -> Result<ResolumeHostDto, ApiError> {
    post_json("/integrations/resolume/hosts", draft).await
}

pub async fn update_resolume_host(
    id: &str,
    draft: &ResolumeHostDraft,
) -> Result<ResolumeHostDto, ApiError> {
    put_json(&format!("/integrations/resolume/hosts/{id}"), draft).await
}

pub async fn delete_resolume_host(id: &str) -> Result<(), ApiError> {
    delete(&format!("/integrations/resolume/hosts/{id}")).await
}

pub async fn test_resolume_host(id: &str) -> Result<ResolumeTestResult, ApiError> {
    post_json(
        &format!("/integrations/resolume/hosts/{id}/test"),
        &serde_json::json!({}),
    )
    .await
}

// ── Android stage displays ─────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AndroidStatusDto {
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub last_attempt: Option<String>,
    #[serde(default)]
    pub last_success: Option<String>,
    #[serde(default)]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AndroidDisplayDto {
    pub id: String,
    pub label: String,
    pub host: String,
    pub port: u16,
    pub launch_component: String,
    pub is_enabled: bool,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub status: Option<AndroidStatusDto>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AndroidDisplayDraft {
    pub label: String,
    pub host: String,
    pub port: u16,
    pub launch_component: String,
    pub is_enabled: bool,
}

pub async fn list_android_displays() -> Result<Vec<AndroidDisplayDto>, ApiError> {
    get_json("/integrations/android-stage/displays").await
}

pub async fn create_android_display(
    draft: &AndroidDisplayDraft,
) -> Result<AndroidDisplayDto, ApiError> {
    post_json("/integrations/android-stage/displays", draft).await
}

pub async fn update_android_display(
    id: &str,
    draft: &AndroidDisplayDraft,
) -> Result<AndroidDisplayDto, ApiError> {
    put_json(&format!("/integrations/android-stage/displays/{id}"), draft).await
}

pub async fn delete_android_display(id: &str) -> Result<(), ApiError> {
    delete(&format!("/integrations/android-stage/displays/{id}")).await
}

pub async fn launch_android_display(id: &str) -> Result<(), ApiError> {
    super::post_no_content(
        &format!("/integrations/android-stage/displays/{id}/launch-now"),
        &serde_json::json!({}),
    )
    .await
}

// ── OSC ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OscStatusDto {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub listening: bool,
    #[serde(default)]
    pub listen_port: u16,
    #[serde(default)]
    pub last_message_at: Option<String>,
    #[serde(default)]
    pub last_note: Option<u8>,
    #[serde(default)]
    pub last_velocity: Option<u8>,
    #[serde(default)]
    pub last_error: Option<String>,
}

pub async fn get_osc_status() -> Result<OscStatusDto, ApiError> {
    get_json("/integrations/osc/status").await
}

pub async fn update_osc_settings(draft: &OscSettingsDraft) -> Result<(), ApiError> {
    // Server returns the persisted OscSettings; we only need success here.
    super::put_no_content("/integrations/osc/settings", draft).await
}
