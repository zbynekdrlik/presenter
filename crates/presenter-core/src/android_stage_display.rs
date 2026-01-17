use crate::id::AndroidStageDisplayId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const DEFAULT_LAUNCH_COMPONENT: &str = "com.fullykiosk.videokiosk/de.ozerov.fully.MainActivity";
pub const DEFAULT_ADB_PORT: u16 = 5555;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AndroidStageDisplayValidationError {
    #[error("label cannot be empty")]
    EmptyLabel,
    #[error("host cannot be empty")]
    EmptyHost,
    #[error("host contains invalid characters (only alphanumeric, dots, and hyphens allowed)")]
    InvalidHostCharacters,
    #[error("port must be between 1 and 65535")]
    InvalidPort,
    #[error("launch component cannot be empty")]
    EmptyLaunchComponent,
    #[error("launch component must be in format 'package/activity' with valid characters")]
    InvalidLaunchComponent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AndroidStageDisplay {
    pub id: AndroidStageDisplayId,
    pub label: String,
    pub host: String,
    pub port: u16,
    pub launch_component: String,
    pub is_enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl AndroidStageDisplay {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        id: AndroidStageDisplayId,
        label: String,
        host: String,
        port: u16,
        launch_component: String,
        is_enabled: bool,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id,
            label,
            host,
            port,
            launch_component,
            is_enabled,
            created_at,
            updated_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AndroidStageDisplayDraft {
    pub label: String,
    pub host: String,
    pub port: u16,
    pub launch_component: String,
    pub is_enabled: bool,
}

impl AndroidStageDisplayDraft {
    #[must_use]
    pub fn new(label: impl Into<String>, host: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            host: host.into(),
            port: DEFAULT_ADB_PORT,
            launch_component: DEFAULT_LAUNCH_COMPONENT.to_string(),
            is_enabled: true,
        }
    }

    #[must_use]
    pub fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    #[must_use]
    pub fn with_launch_component(mut self, launch_component: impl Into<String>) -> Self {
        self.launch_component = launch_component.into();
        self
    }

    #[must_use]
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.is_enabled = enabled;
        self
    }

    /// Rejects drafts whose label, host, port, or launch component are invalid.
    ///
    /// # Errors
    ///
    /// Returns an [`AndroidStageDisplayValidationError`] when any field is empty, contains
    /// invalid characters, or the port is out of range.
    ///
    /// # Security
    ///
    /// This validation prevents command injection by restricting host and `launch_component`
    /// to safe character sets. The host is used in ADB commands and must not contain
    /// shell metacharacters.
    pub fn validate(&self) -> Result<(), AndroidStageDisplayValidationError> {
        if self.label.trim().is_empty() {
            return Err(AndroidStageDisplayValidationError::EmptyLabel);
        }
        let host = self.host.trim();
        if host.is_empty() {
            return Err(AndroidStageDisplayValidationError::EmptyHost);
        }
        // Host must only contain alphanumeric, dots, and hyphens (no shell metacharacters)
        if !host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        {
            return Err(AndroidStageDisplayValidationError::InvalidHostCharacters);
        }
        if self.port == 0 {
            return Err(AndroidStageDisplayValidationError::InvalidPort);
        }
        let component = self.launch_component.trim();
        if component.is_empty() {
            return Err(AndroidStageDisplayValidationError::EmptyLaunchComponent);
        }
        // Launch component must be in package/activity format with valid characters
        if !component.contains('/')
            || !component
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '/')
        {
            return Err(AndroidStageDisplayValidationError::InvalidLaunchComponent);
        }
        Ok(())
    }
}

impl Default for AndroidStageDisplayDraft {
    fn default() -> Self {
        Self::new("Stage Display", "sd1l.lan")
    }
}
