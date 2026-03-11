use super::{get_json, ApiError};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeatureFlags {
    pub companion_enabled: bool,
    pub companion_port: u16,
}

/// Fetch feature flags.
pub async fn get_features() -> Result<FeatureFlags, ApiError> {
    get_json("/features").await
}
