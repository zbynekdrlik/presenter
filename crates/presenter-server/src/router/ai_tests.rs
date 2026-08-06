//! Router-level tests for `/ai/status` (#597): the `connected` field MUST NOT
//! report `true` when `proxy.claudeAuthenticated == false`. The TCP-only
//! connectivity check is necessary but not sufficient — actual AI readiness
//! requires a valid Claude OAuth session as well.

use crate::router::ai::{compute_ai_connected, compute_ai_status_error};

#[test]
fn connected_is_false_when_claude_not_authenticated_even_if_connectivity_ok() {
    // #597: CLIProxyAPI process is running and answering /models (so the TCP
    // connectivity ping succeeds), but the OAuth token has expired and
    // `claudeAuthenticated` is false. Every real AI request would fail with
    // `authentication_error`, so `connected` MUST be false — not the
    // misleading `true` it reported on prod SNV during the 2026-07 incident.
    assert!(
        !compute_ai_connected(true, false),
        "connected must be false when claudeAuthenticated is false, even if \
         the proxy port answers the connectivity ping"
    );
}

#[test]
fn connected_is_true_only_when_both_connectivity_and_auth_are_ok() {
    assert!(
        compute_ai_connected(true, true),
        "connected is true only when the proxy is reachable AND Claude is authenticated"
    );
}

#[test]
fn connected_is_false_when_connectivity_fails_even_if_auth_appears_ok() {
    // Edge case: auth reports true (e.g. credential file exists) but the
    // proxy process is down/unreachable. `connected` must still be false.
    assert!(
        !compute_ai_connected(false, true),
        "connected must be false when the proxy port is unreachable, regardless of auth state"
    );
}

#[test]
fn connected_is_false_when_both_signals_fail() {
    assert!(
        !compute_ai_connected(false, false),
        "connected must be false when neither connectivity nor auth is present"
    );
}

// #624: `check_status` discarded the real error from `check_connectivity`
// behind a hardcoded "AI proxy unreachable" string, even when the actual
// failure was an HTTP-level error (401/500) that explains WHY. These pin
// `compute_ai_status_error`'s full branch table — previously the handler
// itself (not just `compute_ai_connected`'s truth table) had zero coverage.

#[test]
fn status_error_is_none_when_connected() {
    assert_eq!(compute_ai_status_error(true, true, None), None);
}

#[test]
fn status_error_reports_unauthenticated_regardless_of_connectivity_error() {
    // claude_authenticated=false takes priority even if a connectivity error
    // string happens to be present — the auth message is the more actionable one.
    assert_eq!(
        compute_ai_status_error(false, false, Some("AI API returned status 500")),
        Some("Claude not authenticated — run /ai/proxy/login to re-authorize".to_string())
    );
}

#[test]
fn status_error_surfaces_the_real_connectivity_failure_message() {
    // The regression this ticket fixes: a 401 from the proxy must be visible
    // to the caller, not silently replaced by a generic "unreachable" string.
    let error = compute_ai_status_error(false, true, Some("AI API returned status 401"));
    assert_eq!(
        error,
        Some("AI proxy unreachable: AI API returned status 401".to_string()),
        "the real connectivity error must be surfaced, not replaced by a generic message"
    );
}

#[test]
fn status_error_falls_back_to_generic_message_when_no_error_string_available() {
    // Defensive branch: connectivity_ok=false with no captured error string
    // (shouldn't normally happen, but must not panic or fabricate text).
    assert_eq!(
        compute_ai_status_error(false, true, None),
        Some("AI proxy unreachable".to_string())
    );
}
