use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum OscSettingsValidationError {
    #[error("osc address cannot be empty")]
    EmptyAddress,
    #[error("osc port must be between 1 and 65535")]
    InvalidPort,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VelocityMode {
    ZeroBased,
    OneBased,
}

impl Default for VelocityMode {
    fn default() -> Self {
        VelocityMode::ZeroBased
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OscSettings {
    pub enabled: bool,
    pub listen_port: u16,
    pub address_pattern: String,
    pub velocity_mode: VelocityMode,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl OscSettings {
    pub fn new(
        enabled: bool,
        listen_port: u16,
        address_pattern: String,
        velocity_mode: VelocityMode,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    ) -> Self {
        Self {
            enabled,
            listen_port,
            address_pattern,
            velocity_mode,
            created_at,
            updated_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OscSettingsDraft {
    pub enabled: bool,
    pub listen_port: u16,
    pub address_pattern: String,
    #[serde(default)]
    pub velocity_mode: VelocityMode,
}

impl OscSettingsDraft {
    pub fn validate(&self) -> Result<(), OscSettingsValidationError> {
        if self.listen_port == 0 {
            return Err(OscSettingsValidationError::InvalidPort);
        }
        if self.address_pattern.trim().is_empty() {
            return Err(OscSettingsValidationError::EmptyAddress);
        }
        Ok(())
    }
}

impl Default for OscSettingsDraft {
    fn default() -> Self {
        Self {
            enabled: true,
            listen_port: 9000,
            address_pattern: "/note".to_string(),
            velocity_mode: VelocityMode::ZeroBased,
        }
    }
}
