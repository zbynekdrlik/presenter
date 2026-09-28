//! Stage text mode for the API-driven stage layouts (#799).
//!
//! An API client (songplayer) can push BOTH an original lyric line and its
//! translation (`PUT /api/stage` `currentTranslation`/`nextTranslation`). The
//! operator picks which of them the API layouts (`api`, `api-ambient`) show.
//! The mode is a persisted server setting, carried to displays inside the api
//! stage snapshot (`StageDisplaySnapshot::text_mode`) and announced to operator
//! surfaces with `LiveEvent::StageTextMode`.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Which lyric text(s) the API stage layouts render.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StageTextMode {
    /// Only the original line.
    Original,
    /// Only the translation (falls back to the original when none was sent).
    Translation,
    /// Original plus the translation below it (default). With no translation
    /// sent, this is identical to [`StageTextMode::Original`].
    #[default]
    Both,
}

impl StageTextMode {
    /// Every mode, in operator-picker order.
    pub const ALL: [Self; 3] = [Self::Original, Self::Translation, Self::Both];

    /// Stable wire / persistence code (`original` | `translation` | `both`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Original => "original",
            Self::Translation => "translation",
            Self::Both => "both",
        }
    }

    /// Compact encoding for a lock-free atomic cell on the server.
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Original => 0,
            Self::Translation => 1,
            Self::Both => 2,
        }
    }

    /// Inverse of [`StageTextMode::to_u8`]; unknown values map to the default.
    pub fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Original,
            1 => Self::Translation,
            _ => Self::Both,
        }
    }
}

impl fmt::Display for StageTextMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error for a text-mode code that is not `original` | `translation` | `both`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownStageTextMode(pub String);

impl fmt::Display for UnknownStageTextMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown stage text mode: {}", self.0)
    }
}

impl std::error::Error for UnknownStageTextMode {}

impl FromStr for StageTextMode {
    type Err = UnknownStageTextMode;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|mode| mode.as_str() == value)
            .ok_or_else(|| UnknownStageTextMode(value.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_both() {
        assert_eq!(StageTextMode::default(), StageTextMode::Both);
    }

    #[test]
    fn serde_uses_lowercase_codes() {
        for mode in StageTextMode::ALL {
            let json = serde_json::to_string(&mode).unwrap();
            assert_eq!(json, format!("\"{}\"", mode.as_str()));
            let back: StageTextMode = serde_json::from_str(&json).unwrap();
            assert_eq!(back, mode);
        }
    }

    #[test]
    fn from_str_round_trips_and_rejects_unknown() {
        for mode in StageTextMode::ALL {
            assert_eq!(mode.as_str().parse::<StageTextMode>(), Ok(mode));
        }
        assert_eq!(
            "Both".parse::<StageTextMode>(),
            Err(UnknownStageTextMode("Both".to_string()))
        );
        assert!("".parse::<StageTextMode>().is_err());
    }

    #[test]
    fn u8_encoding_round_trips_and_defaults_unknown() {
        for mode in StageTextMode::ALL {
            assert_eq!(StageTextMode::from_u8(mode.to_u8()), mode);
        }
        assert_eq!(StageTextMode::from_u8(200), StageTextMode::Both);
    }
}
