use serde::{Deserialize, Serialize};

use super::{delete, get_json, post_json, post_no_content, ApiError};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoSourceDto {
    pub id: String,
    pub label: String,
    pub ndi_name: String,
    pub is_active: bool,
}

/// Live state of one mapped NDI source (#546) — is it actually working?
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct VideoSourceStatusDto {
    pub id: String,
    pub ndi_name: String,
    pub is_active: bool,
    /// `unknown | not-found | ready | connecting | not-broadcasting | live`.
    pub state: String,
    /// The pipeline's error text, when it has one.
    pub detail: Option<String>,
}

/// The server's answer to "what is mapped, what is on the network, and what works".
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct VideoSourceStatusResponse {
    pub ndi_available: bool,
    /// The NDI names that ARE on the network right now.
    pub discovered: Vec<String>,
    pub sources: Vec<VideoSourceStatusDto>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateVideoSourceRequest {
    pub label: String,
    pub ndi_name: String,
}

pub async fn list_video_sources() -> Result<Vec<VideoSourceDto>, ApiError> {
    get_json("/integrations/video-sources").await
}

/// One call for all three facts the settings card needs (#546): whether this server can
/// see the NDI network at all, what is on it, and the state of every mapped source.
pub async fn get_video_source_status() -> Result<VideoSourceStatusResponse, ApiError> {
    get_json("/integrations/video-sources/status").await
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
