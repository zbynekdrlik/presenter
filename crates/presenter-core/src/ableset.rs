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
    /// Number of resolved entries in the library-name → presentation cache
    /// (#600). `None` when the cache has not been enriched by the router-level
    /// status handler (the bridge's own `status_snapshot` leaves it `None`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_size: Option<usize>,
    /// When the library cache was last rebuilt (#600). Mirrors `last_updated`
    /// from `AbleSetLibraryCache`. `None` when the cache has not been built.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_last_updated: Option<DateTime<Utc>>,
    /// Last error from a library cache rebuild (#600). Separate from
    /// `last_error` (which is the bridge-level error) because a cache rebuild
    /// can fail (e.g. "library not found") even when the bridge is healthy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_last_error: Option<String>,
    /// Ring buffer of recent resolution attempts (#600), newest last. Each
    /// entry records the prefix, timestamp, and whether it resolved. Empty
    /// until the first resolve call after startup.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_attempts: Vec<AbleSetResolutionAttempt>,
}

/// A single AbleSet song-resolution attempt, surfaced read-only via
/// `/integrations/ableset/status` (#600).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AbleSetResolutionAttempt {
    /// ISO-8601 timestamp of the resolution attempt.
    pub timestamp: DateTime<Utc>,
    /// The incoming prefix that was looked up.
    pub input: String,
    /// Whether the prefix resolved to a known presentation (`true`) or was a
    /// cache miss (`false`).
    pub found: bool,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_song_prefix_returns_digits_of_given_length() {
        assert_eq!(
            extract_song_prefix("123 Song Title", 3),
            Some("123".to_string())
        );
    }

    #[test]
    fn extract_song_prefix_truncates_longer_digit_run() {
        assert_eq!(
            extract_song_prefix("12345 Song", 3),
            Some("123".to_string())
        );
    }

    #[test]
    fn extract_song_prefix_returns_none_for_insufficient_digits() {
        assert_eq!(extract_song_prefix("12 Song", 3), None);
    }

    #[test]
    fn extract_song_prefix_returns_none_for_no_digits() {
        assert_eq!(extract_song_prefix("Song Title", 3), None);
    }

    #[test]
    fn extract_song_prefix_returns_none_for_empty_string() {
        assert_eq!(extract_song_prefix("", 3), None);
    }

    #[test]
    fn extract_song_prefix_returns_none_for_zero_length() {
        assert_eq!(extract_song_prefix("123 Song", 0), None);
    }

    #[test]
    fn extract_song_prefix_trims_leading_whitespace() {
        assert_eq!(
            extract_song_prefix("  456 Song", 3),
            Some("456".to_string())
        );
    }
}
