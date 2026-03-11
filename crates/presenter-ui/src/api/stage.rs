use super::{get_json, post_no_content, ApiError};
use presenter_core::{StageAppearance, StageClientSnapshot, StageDisplaySnapshot};
use serde::Serialize;

/// Fetch the current stage display snapshot.
pub async fn get_snapshot() -> Result<StageDisplaySnapshot, ApiError> {
    get_json("/stage/snapshot").await
}

/// Fetch stage connections.
pub async fn get_connections() -> Result<Vec<StageClientSnapshot>, ApiError> {
    get_json("/stage/connections").await
}

/// Get stage appearance for a layout.
pub async fn get_appearance(layout: &str) -> Result<StageAppearance, ApiError> {
    get_json(&format!("/stage/appearance/{layout}")).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StageStateRequest {
    pub presentation_id: String,
    pub current_slide_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_slide_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playlist_id: Option<String>,
}

/// Update stage state (trigger a slide).
pub async fn update_state(request: &StageStateRequest) -> Result<(), ApiError> {
    post_no_content("/stage/state", request).await
}

/// Clear stage state.
pub async fn clear() -> Result<(), ApiError> {
    super::post_no_content("/stage/clear", &serde_json::json!({})).await
}

/// Get broadcast live status.
pub async fn get_broadcast_live() -> Result<BroadcastLiveResponse, ApiError> {
    get_json("/stage/broadcast-live").await
}

#[derive(serde::Deserialize)]
pub struct BroadcastLiveResponse {
    pub enabled: bool,
}
