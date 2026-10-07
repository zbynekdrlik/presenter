//! #819: the text of the compact Resolume / Android list rows — pure, host-tested.
//!
//! The rows read these through a `Memo` of the polled status, so a status change
//! updates the badge / latency / warning text in place without rebuilding the row.

use super::{capitalize, format_timestamp};
use crate::api::settings::{AndroidStatusDto, ResolumeStatusDto};

/// The Resolume badge state (`connected` / `error` / `connecting` / `disabled`).
///
/// A row whose status has not arrived yet (or came back empty) shows `connecting`
/// when enabled and `disabled` when not — never a blank badge.
pub(super) fn resolume_state(status: Option<&ResolumeStatusDto>, is_enabled: bool) -> String {
    match status.map(|s| s.state.as_str()).filter(|s| !s.is_empty()) {
        Some(state) => state.to_lowercase(),
        None if is_enabled => "connecting".to_string(),
        None => "disabled".to_string(),
    }
}

/// The warning line under a Resolume row.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum HostWarning {
    /// The connection keeps failing and Presenter keeps retrying.
    Retrying(String),
    /// An error reported while the state is not `error` (the text as-is).
    Error(String),
}

/// The Resolume row's warning, when the status carries an error.
pub(super) fn resolume_warning(
    status: Option<&ResolumeStatusDto>,
    is_enabled: bool,
) -> Option<HostWarning> {
    let status = status?;
    let err = status.last_error.as_deref()?;
    if resolume_state(Some(status), is_enabled) != "error" {
        return Some(HostWarning::Error(format!("⚠ {err}")));
    }
    let failures = status.consecutive_failures;
    let plural = if failures == 1 { "" } else { "s" };
    let since = status
        .error_since
        .as_deref()
        .map(|s| format!(" since {}", format_timestamp(s)))
        .unwrap_or_default();
    Some(HostWarning::Retrying(format!(
        "⚠ Retrying… ({failures} failure{plural}{since})"
    )))
}

/// The last response time, or a dash before the first answer.
pub(super) fn latency_text(ms: Option<f64>) -> String {
    ms.map(|ms| format!("{ms:.1} ms"))
        .unwrap_or_else(|| "—".to_string())
}

/// The Android badge: (`settings__status--<modifier>`, label). An empty / missing
/// state falls back to Connecting / Disabled, like the Resolume row.
pub(super) fn android_state(
    status: Option<&AndroidStatusDto>,
    is_enabled: bool,
) -> (String, String) {
    let fallback = if is_enabled { "Connecting" } else { "Disabled" };
    let raw = status
        .map(|s| s.state.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(fallback);
    (raw.to_lowercase().replace(' ', "-"), capitalize(raw))
}

/// `Last attempt … · Last success …` — a dash for an event that never happened.
pub(super) fn android_attempts(status: Option<&AndroidStatusDto>) -> String {
    let attempt = timestamp_or_dash(status.and_then(|s| s.last_attempt.as_deref()));
    let success = timestamp_or_dash(status.and_then(|s| s.last_success.as_deref()));
    format!("Last attempt {attempt} · Last success {success}")
}

/// The muted bookkeeping line under a row: `Updated … · Created …`.
pub(super) fn updated_created(updated_at: &str, created_at: &str) -> String {
    format!(
        "Updated {} · Created {}",
        format_timestamp(updated_at),
        format_timestamp(created_at)
    )
}

fn timestamp_or_dash(value: Option<&str>) -> String {
    value
        .map(format_timestamp)
        .unwrap_or_else(|| "—".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Unparseable timestamps format as the raw string, so these stay deterministic in
    // any local timezone.

    fn resolume(state: &str, error: Option<&str>, failures: u32) -> ResolumeStatusDto {
        ResolumeStatusDto {
            state: state.to_string(),
            last_latency_ms: None,
            last_error: error.map(str::to_string),
            consecutive_failures: failures,
            error_since: None,
        }
    }

    fn android(state: &str) -> AndroidStatusDto {
        AndroidStatusDto {
            state: state.to_string(),
            last_attempt: None,
            last_success: None,
            last_error: None,
        }
    }

    #[test]
    fn resolume_state_is_lowercased_and_falls_back_by_enabled_flag() {
        assert_eq!(
            resolume_state(Some(&resolume("Connected", None, 0)), true),
            "connected"
        );
        assert_eq!(
            resolume_state(Some(&resolume("", None, 0)), true),
            "connecting"
        );
        assert_eq!(resolume_state(None, true), "connecting");
        assert_eq!(resolume_state(None, false), "disabled");
    }

    #[test]
    fn a_failing_connection_says_how_often_it_retried() {
        assert_eq!(
            resolume_warning(Some(&resolume("error", Some("refused"), 3)), true),
            Some(HostWarning::Retrying("⚠ Retrying… (3 failures)".into()))
        );
        assert_eq!(
            resolume_warning(Some(&resolume("error", Some("refused"), 1)), true),
            Some(HostWarning::Retrying("⚠ Retrying… (1 failure)".into()))
        );
        let mut since = resolume("error", Some("refused"), 2);
        since.error_since = Some("raw-since".into());
        assert_eq!(
            resolume_warning(Some(&since), true),
            Some(HostWarning::Retrying(
                "⚠ Retrying… (2 failures since raw-since)".into()
            ))
        );
    }

    #[test]
    fn an_error_outside_the_error_state_is_shown_as_is_and_no_error_shows_nothing() {
        assert_eq!(
            resolume_warning(Some(&resolume("connecting", Some("slow"), 0)), true),
            Some(HostWarning::Error("⚠ slow".into()))
        );
        assert_eq!(
            resolume_warning(Some(&resolume("error", None, 4)), true),
            None
        );
        assert_eq!(resolume_warning(None, true), None);
    }

    #[test]
    fn latency_has_one_decimal_or_a_dash() {
        assert_eq!(latency_text(Some(12.345)), "12.3 ms");
        assert_eq!(latency_text(None), "—");
    }

    #[test]
    fn android_badge_normalises_the_server_state() {
        assert_eq!(
            android_state(Some(&android("running")), true),
            ("running".to_string(), "Running".to_string())
        );
        assert_eq!(
            android_state(Some(&android("Not Ready")), true),
            ("not-ready".to_string(), "Not Ready".to_string())
        );
        assert_eq!(
            android_state(Some(&android("")), false),
            ("disabled".to_string(), "Disabled".to_string())
        );
        assert_eq!(
            android_state(None, true),
            ("connecting".to_string(), "Connecting".to_string())
        );
    }

    #[test]
    fn android_attempts_dash_out_missing_events() {
        assert_eq!(android_attempts(None), "Last attempt — · Last success —");
        let mut status = android("running");
        status.last_attempt = Some("raw-attempt".into());
        assert_eq!(
            android_attempts(Some(&status)),
            "Last attempt raw-attempt · Last success —"
        );
    }

    #[test]
    fn updated_and_created_share_one_line() {
        assert_eq!(updated_created("u", "c"), "Updated u · Created c");
    }
}
