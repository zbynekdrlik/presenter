//! Router-level tests for `/ai/status` (#597): the `connected` field MUST NOT
//! report `true` when `proxy.claudeAuthenticated == false`. The TCP-only
//! connectivity check is necessary but not sufficient — actual AI readiness
//! requires a valid Claude OAuth session as well.

use crate::router::ai::compute_ai_connected;

#[test]
fn connected_is_false_when_claude_not_authenticated_even_if_connectivity_ok() {
    // #597: CLIProxyAPI process is running and answering /models (so the TCP
    // connectivity ping succeeds), but the OAuth token has expired and
    // `claudeAuthenticated` is false. Every real AI request would fail with
    // `authentication_error`, so `connected` MUST be false — not the
    // misleading `true` it reported on prod SNV during the 2026-07 incident.
    assert_eq!(
        compute_ai_connected(true, false),
        false,
        "connected must be false when claudeAuthenticated is false, even if \
         the proxy port answers the connectivity ping"
    );
}

#[test]
fn connected_is_true_only_when_both_connectivity_and_auth_are_ok() {
    assert_eq!(
        compute_ai_connected(true, true),
        true,
        "connected is true only when the proxy is reachable AND Claude is authenticated"
    );
}

#[test]
fn connected_is_false_when_connectivity_fails_even_if_auth_appears_ok() {
    // Edge case: auth reports true (e.g. credential file exists) but the
    // proxy process is down/unreachable. `connected` must still be false.
    assert_eq!(
        compute_ai_connected(false, true),
        false,
        "connected must be false when the proxy port is unreachable, regardless of auth state"
    );
}

#[test]
fn connected_is_false_when_both_signals_fail() {
    assert_eq!(
        compute_ai_connected(false, false),
        false,
        "connected must be false when neither connectivity nor auth is present"
    );
}
