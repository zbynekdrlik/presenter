use serde::Deserialize;

use super::{get_json, ApiError};

#[derive(Debug, Deserialize)]
pub struct NdiStatusResponse {
    pub available: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoSourceDto {
    pub id: String,
    pub label: String,
    pub ndi_name: String,
    pub is_active: bool,
}

pub async fn get_ndi_status() -> Result<NdiStatusResponse, ApiError> {
    get_json("/ndi/status").await
}

pub async fn list_video_sources() -> Result<Vec<VideoSourceDto>, ApiError> {
    get_json("/integrations/video-sources").await
}
