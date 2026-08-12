//! Proactive Claude OAuth expiry warning (#660 item 2).
//!
//! Before this, the ONLY warning about a dying Claude OAuth token fired
//! *after* it had already expired (`ProxyManager::report_auth_transition`)
//! — which is exactly why the 2026-07-26 and 2026-08-02 outages were only
//! discovered once a live event started. This module adds a periodic
//! background check that WARNs (journalctl) *before* the token dies, while
//! there is still time for an operator to re-login.
//!
//! New logic lives here rather than growing `proxy.rs` (already 921 lines
//! against the 800-line warning / 1000-line hard-fail cap).

use crate::ai::proxy::ProxyManager;
use chrono::{DateTime, Utc};
use std::sync::Arc;
use std::time::Duration as StdDuration;
use tracing::warn;

/// How far ahead of a token's expiry the proactive warning starts firing —
/// long enough that an operator re-checking the dashboard before a service
/// has a real chance to notice and re-login before the token actually dies.
const EXPIRY_WARNING_WINDOW: chrono::Duration = chrono::Duration::hours(2);

/// How often the background task re-checks token freshness. Matches the
/// interval CLIProxyAPI's own upstream auto-refresh polls at
/// (`core auth auto-refresh started (interval=15m0s)`, confirmed live
/// against upstream — see #660's design comment), so the proactive warning
/// and any upstream refresh attempt stay on a comparable cadence.
const CHECK_INTERVAL: StdDuration = StdDuration::from_secs(15 * 60);

/// Pure predicate: is `expires_at` inside the warning window — i.e. not yet
/// expired, but due to expire within `window`? Extracted so it is
/// unit-testable without a real clock or a live `ProxyManager`.
///
/// An ALREADY-expired token (`expires_at <= now`) is NOT "expiring soon" —
/// that case is `ProxyManager::report_auth_transition`'s job, which already
/// warns on it. This predicate is only for the window strictly BEFORE
/// expiry.
pub(crate) fn is_expiring_soon(
    expires_at: DateTime<Utc>,
    now: DateTime<Utc>,
    window: chrono::Duration,
) -> bool {
    let remaining = expires_at - now;
    remaining > chrono::Duration::zero() && remaining <= window
}

/// Spawn the background task that proactively WARNs when the Claude OAuth
/// token is within `EXPIRY_WARNING_WINDOW` of expiring. Patterned after
/// `AppState::spawn_ableset_rebroadcast` (`state/mod.rs`) — a simple
/// `tokio::spawn` loop, no extra process-global state needed since this
/// only ticks every 15 minutes (unlike the 5s-polled `/ai/status` endpoint,
/// which DOES need `LAST_REPORTED_AUTH` transition-tracking to avoid
/// spamming the log).
pub(crate) fn spawn_expiry_warning(proxy: Arc<ProxyManager>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(CHECK_INTERVAL);
        // The first tick fires immediately; skip it so the warning doesn't
        // race the proxy's own startup before any token scan is meaningful.
        interval.tick().await;
        loop {
            interval.tick().await;
            check_and_warn(&proxy).await;
        }
    });
}

/// One check cycle, extracted from the loop body so it stays short and
/// so a future test could drive it directly without needing a real
/// `tokio::time::interval`.
async fn check_and_warn(proxy: &ProxyManager) {
    let status = proxy.status().await;
    if !status.claude_authenticated {
        return;
    }
    let Some(expires_at) = status.token_expires_at.as_deref() else {
        return;
    };
    let Ok(parsed) = DateTime::parse_from_rfc3339(expires_at) else {
        return;
    };
    if is_expiring_soon(
        parsed.with_timezone(&Utc),
        Utc::now(),
        EXPIRY_WARNING_WINDOW,
    ) {
        warn!(
            expires_at = %expires_at,
            "Claude OAuth token expires soon — re-login before the next event to avoid an outage"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn not_expiring_soon_when_well_outside_the_window() {
        let now = Utc::now();
        let expires_at = now + Duration::hours(8);
        assert!(!is_expiring_soon(expires_at, now, Duration::hours(2)));
    }

    #[test]
    fn expiring_soon_when_inside_the_window() {
        let now = Utc::now();
        let expires_at = now + Duration::minutes(30);
        assert!(is_expiring_soon(expires_at, now, Duration::hours(2)));
    }

    #[test]
    fn not_expiring_soon_once_already_expired() {
        // An already-expired token is `report_auth_transition`'s job, not
        // this predicate's — must not double-report as "expiring soon".
        let now = Utc::now();
        let expires_at = now - Duration::minutes(5);
        assert!(!is_expiring_soon(expires_at, now, Duration::hours(2)));
    }

    #[test]
    fn boundary_exactly_at_the_window_counts_as_expiring_soon() {
        // Inclusive boundary (`<=`) — exactly at the window edge still warns,
        // giving the operator the full advertised window rather than one
        // tick less than promised.
        let now = Utc::now();
        let expires_at = now + Duration::hours(2);
        assert!(is_expiring_soon(expires_at, now, Duration::hours(2)));
    }

    #[test]
    fn boundary_exactly_at_expiry_is_not_expiring_soon() {
        // `remaining == 0` (expires exactly now) is the expired branch, not
        // the warning branch — kills the off-by-one mutant that would flip
        // the lower bound to `>=`.
        let now = Utc::now();
        assert!(!is_expiring_soon(now, now, Duration::hours(2)));
    }

    #[tokio::test]
    async fn check_and_warn_is_a_no_op_when_not_authenticated() {
        let tmp = tempfile::tempdir().unwrap();
        let proxy = ProxyManager::new(tmp.path().to_path_buf());
        // No token files on disk at all -> not authenticated. Must not
        // panic and must simply return without warning (nothing to assert
        // on the log here — covered by the pure `is_expiring_soon` tests
        // above; this only proves the integration path doesn't blow up on
        // the "nothing to warn about" case).
        check_and_warn(&proxy).await;
    }
}
