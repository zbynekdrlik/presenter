use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AbleSetSettingsValidationError {
    #[error("ableset host cannot be empty")]
    EmptyHost,
    #[error("ableset osc/http ports must be between 1 and 65535")]
    InvalidPort,
    #[error("library name cannot be empty")]
    EmptyLibrary,
    #[error("song prefix length must be greater than 0")]
    InvalidPrefixLength,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AbleSetSettings {
    pub enabled: bool,
    pub host: String,
    pub osc_port: u16,
    pub http_port: u16,
    pub library_name: String,
    pub song_prefix_length: u8,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl AbleSetSettings {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        enabled: bool,
        host: String,
        osc_port: u16,
        http_port: u16,
        library_name: String,
        song_prefix_length: u8,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    ) -> Self {
        Self {
            enabled,
            host,
            osc_port,
            http_port,
            library_name,
            song_prefix_length,
            created_at,
            updated_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AbleSetSettingsDraft {
    pub enabled: bool,
    pub host: String,
    pub osc_port: u16,
    pub http_port: u16,
    pub library_name: String,
    #[serde(default = "AbleSetSettingsDraft::default_prefix_length")]
    pub song_prefix_length: u8,
}

impl AbleSetSettingsDraft {
    /// Ensures all required `AbleSet` fields are present and within allowed ranges.
    ///
    /// # Errors
    ///
    /// Returns an [`AbleSetSettingsValidationError`] when any field is empty or out of range.
    pub fn validate(&self) -> Result<(), AbleSetSettingsValidationError> {
        if self.host.trim().is_empty() {
            return Err(AbleSetSettingsValidationError::EmptyHost);
        }
        if self.osc_port == 0 || self.http_port == 0 {
            return Err(AbleSetSettingsValidationError::InvalidPort);
        }
        if self.library_name.trim().is_empty() {
            return Err(AbleSetSettingsValidationError::EmptyLibrary);
        }
        if self.song_prefix_length == 0 {
            return Err(AbleSetSettingsValidationError::InvalidPrefixLength);
        }
        Ok(())
    }

    const fn default_prefix_length() -> u8 {
        3
    }
}

impl Default for AbleSetSettingsDraft {
    fn default() -> Self {
        Self {
            enabled: false,
            host: "fohabl.lan".to_string(),
            osc_port: 39051,
            http_port: 80,
            library_name: "NEW LEVEL".to_string(),
            song_prefix_length: 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AbleSetSongSnapshot {
    pub name: String,
    pub prefix: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<DateTime<Utc>>,
}

impl AbleSetSongSnapshot {
    #[must_use]
    pub fn new(
        name: String,
        prefix: String,
        index: Option<u32>,
        last_seen_at: Option<DateTime<Utc>>,
    ) -> Self {
        Self {
            name,
            prefix,
            index,
            last_seen_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AbleSetStatusSnapshot {
    pub enabled: bool,
    pub tracking: bool,
    pub follow_enabled: bool,
    pub host: String,
    pub http_port: u16,
    pub osc_port: u16,
    pub library_name: String,
    pub song_prefix_length: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_song: Option<AbleSetSongSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

pub fn extract_song_prefix(name: &str, length: u8) -> Option<String> {
    if length == 0 {
        return None;
    }
    let trimmed = name.trim_start();
    let digits: String = trimmed.chars().take_while(char::is_ascii_digit).collect();
    if digits.len() >= length as usize {
        return Some(digits[..length as usize].to_string());
    }
    None
}
