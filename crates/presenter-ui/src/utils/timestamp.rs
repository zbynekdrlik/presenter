//! Shared RFC3339 -> local-time timestamp formatting.
//!
//! Extracted (#622 post-merge review finding 10) because
//! `components::ai_login_banner::format_expiry` and
//! `pages::settings::format_timestamp` had grown the exact same
//! parse-then-format-then-fall-back-to-raw-string body, differing only in
//! the `strftime` pattern each caller wants (banner: minutes only; settings:
//! down to the second). One helper, two callers, two formats.

use chrono::{DateTime, Local};

/// Parse `value` as an RFC3339 timestamp and format it in LOCAL time using
/// the given `chrono::format::strftime` pattern. Falls back to the raw input
/// string, unchanged, when it cannot be parsed — callers must never hide an
/// operator-relevant timestamp just because it came from a source using a
/// slightly different (or malformed) timestamp convention.
pub fn format_local_timestamp(value: &str, fmt: &str) -> String {
    match value.parse::<DateTime<chrono::Utc>>() {
        Ok(dt) => dt.with_timezone(&Local).format(fmt).to_string(),
        Err(_) => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_a_valid_rfc3339_timestamp() {
        let formatted = format_local_timestamp("2026-08-05T10:00:00Z", "%d.%m.%Y %H:%M");
        assert!(formatted.contains("2026"), "got: {formatted}");
    }

    #[test]
    fn different_patterns_produce_different_precision() {
        let minutes = format_local_timestamp("2026-08-05T10:30:45Z", "%d.%m.%Y %H:%M");
        let seconds = format_local_timestamp("2026-08-05T10:30:45Z", "%d.%m.%Y %H:%M:%S");
        assert_ne!(
            minutes, seconds,
            "the two callers must keep their own precision"
        );
    }

    #[test]
    fn unparseable_timestamp_falls_back_to_the_raw_string() {
        assert_eq!(
            format_local_timestamp("not-a-date", "%d.%m.%Y %H:%M"),
            "not-a-date"
        );
    }
}
