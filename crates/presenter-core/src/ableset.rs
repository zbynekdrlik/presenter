use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

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
    /// Per-number AbleSet<->Presenter title disagreements not currently
    /// acknowledged by the operator (#601). Recomputed on every library
    /// cache rebuild — see `presenter-server`'s `state::ableset_mismatch`.
    /// Empty means the two sides' numbering is aligned; this NEVER blocks
    /// resolution/projection, it is a pre-service checklist only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mismatches: Vec<AbleSetTitleMismatch>,
    /// Total number of AbleSet<->Presenter mismatches BEFORE truncation to
    /// the bounded `mismatches` list above (#655 F15) — the inline list caps
    /// at 25 entries for a 5s-polled status endpoint; this field tells the
    /// operator how many more exist beyond what is shown. `#[serde(default)]`
    /// for the UI round-trip (same precedent as `recent_attempts`, #600).
    #[serde(default)]
    pub mismatch_count: usize,
}

/// One AbleSet<->Presenter numbering disagreement for `GET
/// /integrations/ableset/status` (#601). `ableset_title`/`presenter_title`
/// is empty when the number is missing on that side entirely (a structural
/// gap, always reported, never acknowledgeable).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AbleSetTitleMismatch {
    pub number: String,
    pub ableset_title: String,
    pub presenter_title: String,
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

/// Returns `name` with its leading `length`-digit numeric prefix (and any
/// whitespace immediately after it) removed, for #601's title comparison —
/// the prefix itself is the identity key, not part of the title being
/// compared. Falls back to the trimmed name unchanged when
/// `extract_song_prefix` would not recognise a valid prefix (digits are
/// ASCII, so byte-slicing at `length` is always a valid char boundary).
#[must_use]
pub fn strip_song_prefix(name: &str, length: u8) -> &str {
    let trimmed = name.trim_start();
    if extract_song_prefix(trimmed, length).is_none() {
        return trimmed;
    }
    trimmed[length as usize..].trim_start()
}

/// Folds a small set of non-decomposable Latin letters (#655 F13) — ł, ø, đ,
/// ħ, ŧ (and their uppercase forms) are single base glyphs with a stroke,
/// not a base letter plus a combining mark, so NFD decomposition followed by
/// combining-mark removal never touches them. Any other character passes
/// through unchanged. Applied AFTER mark-stripping in
/// `normalize_title_for_mismatch`, same as a diacritic would be.
fn fold_non_decomposable_letter(ch: char) -> char {
    match ch {
        'ł' | 'Ł' => 'l',
        'ø' | 'Ø' => 'o',
        'đ' | 'Đ' => 'd',
        'ħ' | 'Ħ' => 'h',
        'ŧ' | 'Ŧ' => 't',
        other => other,
    }
}

/// Collapses whitespace runs for `normalize_title_for_mismatch` (#655
/// F11/F12, re-settled design): a run bordered by a digit on BOTH sides is
/// formatting noise (a thousands-grouping space, e.g. "10 000" -> "10000")
/// and is removed entirely; every other whitespace run — including one
/// bordered by a digit on only one side — collapses to a single space, never
/// erased, so a genuine word boundary is preserved. `title` is expected to
/// already have punctuation filtered out (see caller), so "digit" here means
/// any ASCII digit immediately adjacent in the filtered string.
fn collapse_whitespace_digit_aware(title: &str) -> String {
    let chars: Vec<char> = title.chars().collect();
    let mut result = String::with_capacity(title.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            let mut j = i;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            let prev_is_digit = result
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_digit());
            let next_is_digit = chars.get(j).is_some_and(|c| c.is_ascii_digit());
            if !(prev_is_digit && next_is_digit) {
                result.push(' ');
            }
            i = j;
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }
    result
}

/// Normalise a song title for the AbleSet<->Presenter mismatch comparison
/// (#601, re-settled by #655 F11/F12): diacritic-, punctuation-, and
/// case-insensitive, plus the non-decomposable-letter fold (F13). Whitespace
/// is digit-aware: a run BETWEEN DIGITS is formatting noise and is removed
/// (e.g. the prod SNV `10000 armad` vs `10 000 armád` case); every other
/// whitespace run collapses to a single space but is never erased, so a
/// genuine word-boundary difference still compares as a mismatch. An earlier
/// design stripped ALL whitespace unconditionally under CI pressure — that
/// went too far and erased word boundaries; this replaces it.
#[must_use]
pub fn normalize_title_for_mismatch(title: &str) -> String {
    let no_diacritics: String = title.nfd().filter(|ch| !is_combining_mark(*ch)).collect();
    let folded: String = no_diacritics
        .chars()
        .map(fold_non_decomposable_letter)
        .collect();
    let filtered: String = folded
        .chars()
        .filter(|ch| ch.is_alphanumeric() || ch.is_whitespace())
        .collect();
    collapse_whitespace_digit_aware(&filtered)
        .trim()
        .to_lowercase()
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

    // #601 — strip_song_prefix / normalize_title_for_mismatch. RED (this
    // commit) is the compile-time proof: neither function exists yet; the
    // GREEN commit adds them, restoring the crate to a buildable state.

    #[test]
    fn strip_song_prefix_removes_digits_and_following_space() {
        assert_eq!(
            strip_song_prefix("017 Viem, ze Ty Pan", 3),
            "Viem, ze Ty Pan"
        );
    }

    #[test]
    fn strip_song_prefix_falls_back_to_trimmed_name_without_a_valid_prefix() {
        assert_eq!(strip_song_prefix("Song Title", 3), "Song Title");
        assert_eq!(strip_song_prefix("  Song Title", 3), "Song Title");
    }

    #[test]
    fn normalize_title_for_mismatch_ignores_diacritics_case_and_punctuation() {
        assert_eq!(
            normalize_title_for_mismatch("Viem, že Ty Pán?!"),
            normalize_title_for_mismatch("viem ze ty pan")
        );
    }

    #[test]
    fn normalize_title_for_mismatch_removes_whitespace_only_between_digits() {
        // Re-settled design (#655 F11/F12), replacing BOTH prior designs: the
        // ORIGINAL design kept all inner whitespace significant (too narrow —
        // missed the #601 SNV "10000 armad" vs "10 000 armád" case); the
        // `fix(ci)` commit on top of it went too far under CI pressure and
        // stripped ALL whitespace, erasing genuine word boundaries. The
        // re-settled design: a whitespace run BETWEEN DIGITS ("10 000"
        // grouping) is formatting noise and is removed, same class as a
        // diacritic-only difference; every OTHER whitespace run collapses to
        // a single space but is never removed.
        assert_eq!(
            normalize_title_for_mismatch("102 10 000 armád"),
            normalize_title_for_mismatch("102 10000 armad"),
            "a whitespace run between two digits must be treated as formatting noise"
        );
    }

    #[test]
    fn normalize_title_for_mismatch_still_differs_when_non_digit_spacing_differs() {
        // Counterpart to the test above: a whitespace run that is NOT
        // between two digits is a genuine word boundary, not formatting
        // noise — two titles differing only by whether that boundary has a
        // space must still compare as different. This is exactly the
        // over-broad-stripping regression the `fix(ci)` design introduced
        // (and this re-settled design fixes): stripping ALL whitespace would
        // make "Arriba Song" and "ArribaSong" compare equal.
        assert_ne!(
            normalize_title_for_mismatch("Arriba Song"),
            normalize_title_for_mismatch("ArribaSong"),
            "a non-digit whitespace boundary must not be silently erased"
        );
    }

    #[test]
    fn normalize_title_for_mismatch_folds_non_decomposable_letters() {
        // #655 F13: ł, ø, đ, ħ, ŧ (and their uppercase forms) are single base
        // glyphs with a stroke, not a base letter + combining mark — NFD
        // mark-stripping alone does not touch them. A tiny explicit fold
        // table closes the gap so these still compare equal to their plain
        // Latin counterpart, the same way a diacritic (mark-based) letter
        // already does.
        assert_eq!(
            normalize_title_for_mismatch("Łódź"),
            normalize_title_for_mismatch("Lodz"),
            "ł/Ł must fold to l/L"
        );
        assert_eq!(
            normalize_title_for_mismatch("Øresund"),
            normalize_title_for_mismatch("Oresund"),
            "ø/Ø must fold to o/O"
        );
        assert_eq!(
            normalize_title_for_mismatch("Đorđe"),
            normalize_title_for_mismatch("Dorde"),
            "đ/Đ must fold to d/D"
        );
        assert_eq!(
            normalize_title_for_mismatch("Ħamrun"),
            normalize_title_for_mismatch("Hamrun"),
            "ħ/Ħ must fold to h/H"
        );
        assert_eq!(
            normalize_title_for_mismatch("Ŧullio"),
            normalize_title_for_mismatch("Tullio"),
            "ŧ/Ŧ must fold to t/T"
        );
    }

    #[test]
    fn normalize_title_for_mismatch_detects_genuinely_different_titles() {
        assert_ne!(
            normalize_title_for_mismatch("Tvoja blízkosť je nebo"),
            normalize_title_for_mismatch("Viem, ze Ty Pan")
        );
    }
}
