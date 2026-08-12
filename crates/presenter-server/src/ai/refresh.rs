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
use presenter_core::{is_expiring_soon, EXPIRY_WARNING_WINDOW};
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;
use tracing::warn;

/// How often the background task re-checks token freshness. Matches the
/// interval CLIProxyAPI's own upstream auto-refresh polls at
/// (`core auth auto-refresh started (interval=15m0s)`, confirmed live
/// against upstream — see #660's design comment), so the proactive warning
/// and any upstream refresh attempt stay on a comparable cadence.
const CHECK_INTERVAL: StdDuration = StdDuration::from_secs(15 * 60);

/// #675 review finding 5: the most recent token expiry we've already WARNed
/// about, so `check_and_warn` warns ONCE per distinct expiry entering the
/// window instead of ~8 times over its 2-hour duration (every 15 min,
/// `CHECK_INTERVAL`). Same shape as `ProxyManager`'s `LAST_REPORTED_AUTH`
/// transition-tracking in `proxy.rs` — a process-global is the right scope
/// here for the same reason: one process runs one `ProxyManager` and one
/// background expiry-warning task in practice.
static LAST_WARNED_EXPIRY: Mutex<Option<DateTime<Utc>>> = Mutex::new(None);

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

/// #675 review finding 5: pure transition-tracking logic (same shape as
/// `ProxyManager::report_auth_transition` in `proxy.rs`), extracted so it is
/// unit-testable against a LOCAL `Mutex` instead of racing the
/// process-global `LAST_WARNED_EXPIRY` against other tests running in
/// parallel in the same binary.
///
/// Returns the expiry timestamp to warn about, or `None` when this check
/// should stay quiet:
/// - `expires_at` is `None`, or not inside `window` (fresh token, refreshed
///   to a later expiry, or already expired/unauthenticated) — resets
///   `last_warned` to `None` so the NEXT distinct expiry that enters the
///   window warns again, and returns `None`.
/// - Inside the window, same timestamp as last time — stays quiet, returns
///   `None`. This is the fix: `check_and_warn` used to re-warn on every
///   ~15-minute tick for the whole 2-hour window (~8 times per token).
/// - Inside the window, first time OR a NEW timestamp (e.g. a partial
///   refresh that still lands inside the window) — records it and returns
///   `Some(expires_at)`.
fn expiry_to_warn(
    expires_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    window: chrono::Duration,
    last_warned: &Mutex<Option<DateTime<Utc>>>,
) -> Option<DateTime<Utc>> {
    let candidate = expires_at.filter(|e| is_expiring_soon(*e, now, window));
    match last_warned.lock() {
        Ok(mut last) => {
            if candidate.is_some() && *last == candidate {
                None
            } else {
                *last = candidate;
                candidate
            }
        }
        // A poisoned lock must not panic (this repo bans unwrap/expect/panic
        // in production code) — fail toward WARNING rather than silence,
        // matching `report_auth_transition`'s own `Err(_) => true` fallback.
        Err(_) => candidate,
    }
}

/// One check cycle, extracted from the loop body so it stays short and
/// so a future test could drive it directly without needing a real
/// `tokio::time::interval`.
async fn check_and_warn(proxy: &ProxyManager) {
    let status = proxy.status().await;
    let expires_at = if status.claude_authenticated {
        status
            .token_expires_at
            .as_deref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.with_timezone(&Utc))
    } else {
        None
    };

    if let Some(ts) = expiry_to_warn(
        expires_at,
        Utc::now(),
        EXPIRY_WARNING_WINDOW,
        &LAST_WARNED_EXPIRY,
    ) {
        warn!(
            expires_at = %ts.to_rfc3339(),
            "Claude OAuth token expires soon — re-login before the next event to avoid an outage"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn local_state() -> Mutex<Option<DateTime<Utc>>> {
        Mutex::new(None)
    }

    #[test]
    fn warns_once_on_first_entry_into_the_window() {
        let now = Utc::now();
        let expires_at = now + Duration::minutes(30);
        let last_warned = local_state();
        assert_eq!(
            expiry_to_warn(Some(expires_at), now, Duration::hours(2), &last_warned),
            Some(expires_at)
        );
    }

    #[test]
    fn stays_quiet_on_repeat_checks_for_the_same_expiry() {
        // The regression this fixes: `check_and_warn` ticks every 15 minutes
        // (`CHECK_INTERVAL`) — over the 2-hour window that is ~8 checks for
        // the SAME token, and only the first one should actually warn.
        let now = Utc::now();
        let expires_at = now + Duration::minutes(30);
        let window = Duration::hours(2);
        let last_warned = local_state();
        assert_eq!(
            expiry_to_warn(Some(expires_at), now, window, &last_warned),
            Some(expires_at),
            "first check must warn"
        );
        let later = now + Duration::minutes(15);
        assert_eq!(
            expiry_to_warn(Some(expires_at), later, window, &last_warned),
            None,
            "second check for the SAME expiry must stay quiet"
        );
        let even_later = now + Duration::minutes(30);
        assert_eq!(
            expiry_to_warn(Some(expires_at), even_later, window, &last_warned),
            None,
            "third check for the SAME expiry must still stay quiet"
        );
    }

    #[test]
    fn warns_again_after_a_refresh_moves_the_expiry_back_outside_the_window_and_later_re_enters() {
        // Refreshed to a later expiry (now outside the window) -> quiet AND
        // resets the tracked state. Time passes, the NEW expiry itself
        // enters the window -> warns again, because it is a genuinely new
        // event the operator hasn't been told about yet.
        let now = Utc::now();
        let window = Duration::hours(2);
        let last_warned = local_state();
        let first_expiry = now + Duration::minutes(30);
        assert_eq!(
            expiry_to_warn(Some(first_expiry), now, window, &last_warned),
            Some(first_expiry)
        );

        // Token refreshed: new expiry is far outside the window.
        let refreshed_expiry = now + Duration::hours(8);
        let after_refresh = now + Duration::minutes(31);
        assert_eq!(
            expiry_to_warn(Some(refreshed_expiry), after_refresh, window, &last_warned),
            None,
            "a refreshed token outside the window must not warn"
        );

        // Much later, the REFRESHED token itself enters the window.
        let re_entry_time = refreshed_expiry - Duration::minutes(30);
        assert_eq!(
            expiry_to_warn(Some(refreshed_expiry), re_entry_time, window, &last_warned),
            Some(refreshed_expiry),
            "the refreshed token's OWN entry into the window must warn again, \
             not stay silent forever because of the earlier token's timestamp"
        );
    }

    #[test]
    fn no_expiry_known_resets_state_and_never_warns() {
        let now = Utc::now();
        let window = Duration::hours(2);
        let last_warned = local_state();
        assert_eq!(expiry_to_warn(None, now, window, &last_warned), None);
        // Confirm the reset actually happened: a later check for a
        // timestamp that WOULD equal a stale `last_warned` value still
        // warns (there's nothing stale to compare against since `None`
        // resets to `None`, not left over from a previous test — this is
        // mostly documentation of intent given each test uses its own
        // local Mutex).
        let expires_at = now + Duration::minutes(10);
        assert_eq!(
            expiry_to_warn(Some(expires_at), now, window, &last_warned),
            Some(expires_at)
        );
    }

    #[tokio::test]
    async fn check_and_warn_is_a_no_op_when_not_authenticated() {
        let tmp = tempfile::tempdir().unwrap();
        let proxy = ProxyManager::new(tmp.path().to_path_buf());
        // No token files on disk at all -> not authenticated. Must not
        // panic and must simply return without warning (nothing to assert
        // on the log here — covered by the pure `expiry_to_warn` tests
        // above; this only proves the integration path doesn't blow up on
        // the "nothing to warn about" case).
        check_and_warn(&proxy).await;
    }
}
