use crate::id::ResolumeHostId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ResolumeHostValidationError {
    #[error("label cannot be empty")]
    EmptyLabel,
    #[error("host cannot be empty")]
    EmptyHost,
    #[error("port must be between 1 and 65535")]
    InvalidPort,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolumeHost {
    pub id: ResolumeHostId,
    pub label: String,
    pub host: String,
    pub port: u16,
    pub is_enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ResolumeHost {
    pub fn new(
        id: ResolumeHostId,
        label: String,
        host: String,
        port: u16,
        is_enabled: bool,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id,
            label,
            host,
            port,
            is_enabled,
            created_at,
            updated_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolumeHostDraft {
    pub label: String,
    pub host: String,
    pub port: u16,
    pub is_enabled: bool,
}

impl ResolumeHostDraft {
    pub fn new(label: impl Into<String>, host: impl Into<String>, port: u16) -> Self {
        Self {
            label: label.into(),
            host: host.into(),
            port,
            is_enabled: true,
        }
    }

    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.is_enabled = enabled;
        self
    }

    pub fn validate(&self) -> Result<(), ResolumeHostValidationError> {
        if self.label.trim().is_empty() {
            return Err(ResolumeHostValidationError::EmptyLabel);
        }
        if self.host.trim().is_empty() {
            return Err(ResolumeHostValidationError::EmptyHost);
        }
        if self.port == 0 {
            return Err(ResolumeHostValidationError::InvalidPort);
        }
        Ok(())
    }
}
