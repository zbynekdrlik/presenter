use serde::Deserialize;

use super::{get_json, ApiError};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkModeDto {
    pub mode: String,
}

/// Fetch `/api/network-mode`. Returns `"local"` or `"remote"` on success.
pub async fn fetch_network_mode() -> Result<String, ApiError> {
    let dto: NetworkModeDto = get_json("/api/network-mode").await?;
    Ok(dto.mode)
}
