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
    /// #564: the port a prior AUTO-DISCOVERY probe found Resolume Arena/Avenue
    /// actually listening on, when it differs from `port` (the user's
    /// configured intent, never auto-changed). `None` means "dial `port`".
    /// Persisted so a restart resumes on the last known-good port instead of
    /// re-learning it. Defaults to `None` via [`ResolumeHost::new`] — set it
    /// with [`ResolumeHost::with_active_port`].
    pub active_port: Option<u16>,
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
            active_port: None,
        }
    }

    /// #564: attach the runtime-discovered active port (from the DB column).
    pub fn with_active_port(mut self, active_port: Option<u16>) -> Self {
        self.active_port = active_port;
        self
    }

    /// #564: the port the driver should actually dial — the discovered active
    /// port when set, otherwise the user's configured port.
    pub fn dial_port(&self) -> u16 {
        self.active_port.unwrap_or(self.port)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_draft() -> ResolumeHostDraft {
        ResolumeHostDraft::new("Main", "192.168.1.100", 7000)
    }

    #[test]
    fn validate_accepts_valid_draft() {
        assert!(valid_draft().validate().is_ok());
    }

    #[test]
    fn validate_rejects_empty_label() {
        let mut draft = valid_draft();
        draft.label = "  ".to_string();
        assert_eq!(
            draft.validate().unwrap_err(),
            ResolumeHostValidationError::EmptyLabel
        );
    }

    #[test]
    fn validate_rejects_empty_host() {
        let mut draft = valid_draft();
        draft.host = "".to_string();
        assert_eq!(
            draft.validate().unwrap_err(),
            ResolumeHostValidationError::EmptyHost
        );
    }

    #[test]
    fn validate_rejects_zero_port() {
        let mut draft = valid_draft();
        draft.port = 0;
        assert_eq!(
            draft.validate().unwrap_err(),
            ResolumeHostValidationError::InvalidPort
        );
    }

    fn sample_host() -> ResolumeHost {
        let now = Utc::now();
        ResolumeHost::new(
            ResolumeHostId::new(),
            "Main".to_string(),
            "192.168.1.100".to_string(),
            8090,
            true,
            now,
            now,
        )
    }

    /// #564: with no drift discovered, dial the configured port.
    #[test]
    fn dial_port_defaults_to_configured_port() {
        assert_eq!(sample_host().dial_port(), 8090);
    }

    /// #564: once a drift is discovered and persisted, dial the ACTIVE port.
    #[test]
    fn dial_port_prefers_the_discovered_active_port() {
        let host = sample_host().with_active_port(Some(8091));
        assert_eq!(host.dial_port(), 8091);
    }

    /// #564: healing back to the configured port clears active_port.
    #[test]
    fn dial_port_falls_back_once_active_port_is_cleared() {
        let host = sample_host()
            .with_active_port(Some(8091))
            .with_active_port(None);
        assert_eq!(host.dial_port(), 8090);
    }
}
