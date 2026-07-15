use serde::Deserialize;

use super::{get_json, post_no_content, ApiError};

/// A soft-deleted song, as served by `GET /presentations/trash` (#555).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashedSongDto {
    pub id: String,
    pub name: String,
    pub library_name: String,
    pub deleted_at: String,
}

pub async fn list_trash() -> Result<Vec<TrashedSongDto>, ApiError> {
    get_json("/presentations/trash").await
}

pub async fn restore_song(id: &str) -> Result<(), ApiError> {
    post_no_content(&format!("/presentations/{id}/restore"), &serde_json::json!({})).await
}
