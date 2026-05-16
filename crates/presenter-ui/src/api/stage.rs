use super::{get_json, post_json, post_no_content, ApiError};
use presenter_core::{StageClientSnapshot, StageDisplayLayout, StageDisplaySnapshot};
use serde::{Deserialize, Serialize};

pub async fn get_snapshot() -> Result<StageDisplaySnapshot, ApiError> {
    get_json("/stage/snapshot").await
}

pub async fn get_snapshot_for(layout: &str) -> Result<StageDisplaySnapshot, ApiError> {
    let path = format!("/stage/snapshot?layout={layout}");
    get_json(&path).await
}

pub async fn get_connections() -> Result<Vec<StageClientSnapshot>, ApiError> {
    get_json("/stage/connections").await
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

pub async fn update_state(request: &StageStateRequest) -> Result<(), ApiError> {
    post_no_content("/stage/state", request).await
}

pub async fn clear() -> Result<(), ApiError> {
    post_no_content("/stage/clear", &serde_json::json!({})).await
}

pub async fn get_broadcast_live() -> Result<BroadcastLiveResponse, ApiError> {
    get_json("/stage/broadcast-live").await
}

#[derive(serde::Deserialize)]
pub struct BroadcastLiveResponse {
    pub enabled: bool,
}

pub async fn get_layouts() -> Result<Vec<StageDisplayLayout>, ApiError> {
    get_json("/stage-displays").await
}

#[derive(Serialize)]
struct StageLayoutUpdateRequest {
    code: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageLayoutResponse {
    pub code: String,
    pub layout: StageDisplayLayout,
}

pub async fn set_layout(code: &str) -> Result<StageLayoutResponse, ApiError> {
    post_json(
        "/stage/layout",
        &StageLayoutUpdateRequest {
            code: code.to_string(),
        },
    )
    .await
}

pub async fn get_layout() -> Result<StageLayoutResponse, ApiError> {
    get_json("/stage/layout").await
}
