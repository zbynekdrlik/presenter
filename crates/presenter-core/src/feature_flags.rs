use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeatureFlags {
    pub companion_enabled: bool,
    pub companion_port: u16,
}
