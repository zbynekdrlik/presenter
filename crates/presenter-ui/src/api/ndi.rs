use serde::Deserialize;

use super::{get_json, ApiError};

#[derive(Debug, Deserialize)]
pub struct NdiStatusResponse {
    pub available: bool,
}

pub async fn get_ndi_status() -> Result<NdiStatusResponse, ApiError> {
    get_json("/ndi/status").await
}
