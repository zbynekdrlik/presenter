//! Shared Claude OAuth token expiry-warning logic (#660, #675 review
//! finding 4).
//!
//! Two independent surfaces both need to agree on WHEN a Claude OAuth token
//! counts as "expiring soon":
//! - presenter-server's background journalctl WARN
//!   (`ai::refresh::check_and_warn`)
//! - presenter-ui's operator-header AI status chip
//!   (`components::ai_status::AiStatusChip`)
//!
//! Before this module existed, both sides hand-duplicated the 2-hour window
//! constant AND the `is_expiring_soon` predicate, with a comment admitting
//! "keep both in sync by hand". `presenter-core` is a domain crate BOTH
//! `presenter-server` and `presenter-ui` already depend on (see
//! `timer.rs`'s `DateTime<Utc>`/`Duration`-based public API for the existing
//! precedent of sharing chrono-based predicates this way) — living here
//! makes drift between the server and the UI a compile-time impossibility
//! instead of a hand-maintained promise.

use chrono::{DateTime, Duration, Utc};

/// How far ahead of expiry Presenter starts warning about a dying Claude
/// OAuth token — long enough that an operator checking the dashboard, or a
/// server operator watching logs, has a real chance to notice and re-login
/// before the token actually dies.
pub const EXPIRY_WARNING_WINDOW: Duration = Duration::hours(2);

/// Pure predicate: is `expires_at` inside `window` of expiring — i.e. not
/// yet expired, but due to expire within `window`? An ALREADY-expired token
/// is NOT "expiring soon" — that is a separate, already-dead-login state on
/// both call sites (presenter-server's `ProxyManager::report_auth_transition`,
/// presenter-ui's `logged-out` chip state).
pub fn is_expiring_soon(expires_at: DateTime<Utc>, now: DateTime<Utc>, window: Duration) -> bool {
    let remaining = expires_at - now;
    remaining > Duration::zero() && remaining <= window
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let now = Utc::now();
        let expires_at = now - Duration::minutes(5);
        assert!(!is_expiring_soon(expires_at, now, Duration::hours(2)));
    }

    #[test]
    fn boundary_exactly_at_the_window_counts_as_expiring_soon() {
        let now = Utc::now();
        let expires_at = now + Duration::hours(2);
        assert!(is_expiring_soon(expires_at, now, Duration::hours(2)));
    }

    #[test]
    fn boundary_exactly_at_expiry_is_not_expiring_soon() {
        let now = Utc::now();
        assert!(!is_expiring_soon(now, now, Duration::hours(2)));
    }
}
