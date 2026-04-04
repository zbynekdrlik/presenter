use super::{get_json, post_json, put_json, ApiError};
use presenter_core::{AbleSetSettings, AbleSetSettingsDraft, AbleSetStatusSnapshot};
use serde::{Deserialize, Serialize};

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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeatureFlags {
    pub companion_enabled: bool,
    pub companion_port: u16,
}

pub async fn get_features() -> Result<FeatureFlags, ApiError> {
    get_json("/settings/features").await
}
