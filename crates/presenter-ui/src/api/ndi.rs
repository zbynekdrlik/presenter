use serde::{Deserialize, Serialize};

use super::{delete, get_json, post_json, post_no_content, ApiError};

#[derive(Debug, Deserialize)]
pub struct NdiStatusResponse {
    pub available: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoSourceDto {
    pub id: String,
    pub label: String,
    pub ndi_name: String,
    pub is_active: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NdiSourceDto {
    pub name: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateVideoSourceRequest {
    pub label: String,
    pub ndi_name: String,
}

pub async fn get_ndi_status() -> Result<NdiStatusResponse, ApiError> {
    get_json("/ndi/status").await
}

pub async fn list_video_sources() -> Result<Vec<VideoSourceDto>, ApiError> {
    get_json("/integrations/video-sources").await
}

pub async fn discover_ndi_sources() -> Result<Vec<NdiSourceDto>, ApiError> {
    get_json("/ndi/sources").await
}

pub async fn create_video_source(label: &str, ndi_name: &str) -> Result<VideoSourceDto, ApiError> {
    post_json(
        "/integrations/video-sources",
        &CreateVideoSourceRequest {
            label: label.to_string(),
            ndi_name: ndi_name.to_string(),
        },
    )
    .await
}

pub async fn activate_video_source(id: &str) -> Result<VideoSourceDto, ApiError> {
    post_json(
        &format!("/integrations/video-sources/{id}/activate"),
        &serde_json::json!({}),
    )
    .await
}

pub async fn deactivate_video_sources() -> Result<(), ApiError> {
    post_no_content(
        "/integrations/video-sources/deactivate",
        &serde_json::json!({}),
    )
    .await
}

pub async fn delete_video_source(id: &str) -> Result<(), ApiError> {
    delete(&format!("/integrations/video-sources/{id}")).await
}

/// WHEP endpoint URL for a given source ID.
///
/// Browsers POST an SDP offer to this URL; presenter bridges the exchange
/// into `whepserversink`'s signaller. The MJPEG URL builder it replaces is
/// gone — `<NdiVideo>` is the only client.
pub fn whep_url(source_id: &str) -> String {
    format!("/ndi/whep/{source_id}")
}
