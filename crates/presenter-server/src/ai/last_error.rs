//! Per-`AppState` record of the most recent REAL AI completion call's health
//! (#764).
//!
//! **Why this exists:** `evaluate_ai_status` (`router/ai.rs`) derives
//! `connected` from a single `list_models` (`GET /models`) probe. A metered
//! backend (OpenRouter, #761) serves `/models` 200 even when the workspace
//! budget is exhausted, the key was revoked, or completions 402/429 — so
//! `/ai/status` and `/healthz.ai` reported `connected:true` while every real
//! `POST /ai/chat` 403'd ("Workspace daily budget … exceeded"). `list_models`
//! is NOT a liveness proof for a metered backend. This type captures the
//! outcome of the one production completion path (`ai::agent::run_agent`) so
//! the shared status verdict can fold in the last REAL failure.
//!
//! Scoped PER `AppState` (an `Arc` field on `AppState`, `pub(crate)` accessor),
//! never a module-level `static` — the test suite builds many `AppState`s in
//! one process and a global would cross-contaminate tests (same rationale as
//! `crate::ai::health_cache::AiHealthCache`).

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long a recorded completion failure keeps `connected:false` when no
/// later completion has succeeded. After this window without any call, the
/// verdict falls back to the live `list_models` probe (so a transient outage
/// the operator stops exercising does not pin a permanent false `false`). A
/// newer success clears it immediately, regardless of the window.
pub(crate) const AI_LAST_FAILURE_WINDOW: Duration = Duration::from_secs(15 * 60);

/// Max chars of the redacted backend-error excerpt kept for the operator
/// status. Bounds what a large provider error page can put into
/// `/healthz`/`/ai/status`.
const EXCERPT_MAX_CHARS: usize = 200;

/// Records the health of the most recent REAL AI completion call. Every
/// completion in `run_agent` records a failure (redacted, truncated excerpt +
/// the time it happened) or clears it on success.
#[derive(Default)]
pub(crate) struct AiCallHealth {
    last_failure: Mutex<Option<(Instant, String)>>,
}

impl AiCallHealth {
    /// Record a failed completion. `raw_error` is redacted (credentials
    /// stripped) and truncated before it is stored, so a key in a provider
    /// error body can never reach `/healthz`.
    pub(crate) fn record_failure(&self, raw_error: &str) {
        let excerpt = build_excerpt(raw_error);
        *self.guard() = Some((Instant::now(), excerpt));
    }

    /// Record a successful completion — clears any stored failure immediately.
    pub(crate) fn record_success(&self) {
        *self.guard() = None;
    }

    /// The redacted excerpt of the last failure IF it happened within `window`
    /// and no success has cleared it since; `None` otherwise (never failed,
    /// cleared by a later success, or older than `window`). `window` is a
    /// parameter so expiry is unit-testable without a real 15-minute wait.
    pub(crate) fn active_failure(&self, window: Duration) -> Option<String> {
        self.guard()
            .as_ref()
            .filter(|(at, _)| at.elapsed() < window)
            .map(|(_, excerpt)| excerpt.clone())
    }

    /// Lock helper that recovers a poisoned mutex instead of panicking (the
    /// stored value is a plain snapshot — a poisoned lock carries no broken
    /// invariant), so this never trips the crate's no-`unwrap`/no-panic rule.
    fn guard(&self) -> std::sync::MutexGuard<'_, Option<(Instant, String)>> {
        self.last_failure
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Redact credentials from a backend error string, then cap it to
/// [`EXCERPT_MAX_CHARS`]. Reuses the shared proxy-relay redaction so a new
/// key format is covered in exactly one place.
fn build_excerpt(raw_error: &str) -> String {
    crate::ai::proxy_output_relay::redact_proxy_output_line(raw_error)
        .chars()
        .take(EXCERPT_MAX_CHARS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUDGET_BODY: &str = "AI API returned 403 Forbidden: {\"error\":{\"message\":\
        \"Workspace daily budget of $1.00 exceeded. Contact your org admin.\",\"code\":403}}";

    #[test]
    fn no_failure_recorded_is_none() {
        let health = AiCallHealth::default();
        assert!(health.active_failure(AI_LAST_FAILURE_WINDOW).is_none());
    }

    #[test]
    fn a_recorded_failure_is_active_within_the_window() {
        let health = AiCallHealth::default();
        health.record_failure(BUDGET_BODY);
        let active = health
            .active_failure(AI_LAST_FAILURE_WINDOW)
            .expect("a fresh failure must be active within the window");
        assert!(
            active.contains("budget"),
            "the excerpt must carry the backend budget message: {active}"
        );
    }

    #[test]
    fn a_success_clears_a_prior_failure() {
        let health = AiCallHealth::default();
        health.record_failure(BUDGET_BODY);
        health.record_success();
        assert!(
            health.active_failure(AI_LAST_FAILURE_WINDOW).is_none(),
            "a successful completion must clear the prior failure immediately"
        );
    }

    #[test]
    fn a_failure_older_than_the_window_is_not_active() {
        // #764 test (c): the window is a parameter, so a zero-length window
        // proves the `elapsed() < window` expiry gate rejects a failure that
        // is (by any positive elapsed time) older than the window — without a
        // real 15-minute wait or a clock injection.
        let health = AiCallHealth::default();
        health.record_failure(BUDGET_BODY);
        assert!(
            health.active_failure(Duration::ZERO).is_none(),
            "a failure must not stay active once it is older than the window"
        );
        // ...but a generous window still sees the SAME recorded failure.
        assert!(
            health.active_failure(Duration::from_secs(3600)).is_some(),
            "the same failure must still be active under a generous window"
        );
    }

    #[test]
    fn a_credential_in_the_error_body_is_redacted_before_storage() {
        // #764 test (d): an OpenRouter key in the provider error body must
        // never survive into the stored excerpt (and thus never into
        // `/healthz`/`/ai/status`).
        let health = AiCallHealth::default();
        health.record_failure(
            "AI API returned 401: {\"error\":\"invalid key sk-or-v1-FAKEnotARealKey_test_XYZ\"}",
        );
        let active = health
            .active_failure(AI_LAST_FAILURE_WINDOW)
            .expect("failure recorded");
        assert!(
            !active.contains("sk-or-v1-"),
            "the OpenRouter key must be redacted out of the stored excerpt: {active}"
        );
    }

    #[test]
    fn the_excerpt_is_capped() {
        let health = AiCallHealth::default();
        health.record_failure(&"x".repeat(1000));
        let active = health
            .active_failure(AI_LAST_FAILURE_WINDOW)
            .expect("failure recorded");
        assert!(
            active.chars().count() <= EXCERPT_MAX_CHARS,
            "excerpt must be capped to {EXCERPT_MAX_CHARS} chars, was {}",
            active.chars().count()
        );
    }
}
