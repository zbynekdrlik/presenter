//! Router-level tests for AI connectivity/status CLASSIFICATION (#597, #661,
//! #679, #683): the `connected` field MUST NOT report `true` when
//! `proxy.claudeAuthenticated == false`, and the "is this the bundled
//! proxy?" classification (`is_bundled_proxy_address`,
//! `should_self_heal_to_canonical`, `get_settings_internal`'s
//! `is_bundled_default`) must recognize both the literal default address AND
//! a historically-substituted `127.0.0.1`/`localhost` form.
//!
//! Split out of `ai_tests.rs` (#684) once the #679/#683 rounds pushed that
//! file past the repo's 1000-line file-size cap — this file holds every
//! status-CLASSIFICATION test (`compute_ai_connected`,
//! `compute_ai_status_error`, `is_bundled_proxy_address`,
//! `should_self_heal_to_canonical`, and the `get_settings_internal`/
//! `/ai/status` bundled-detection tests); `ai_tests.rs` keeps the
//! chat/settings-audit/idle-clear tests. Mechanical move only — no test
//! body was changed, only `use` paths were re-scoped to what this half
//! actually calls.

use crate::router::ai::{
    compute_ai_connected, compute_ai_status_error, get_settings_internal, is_bundled_proxy_address,
    should_self_heal_to_canonical,
};

#[test]
fn connected_is_false_when_claude_not_authenticated_even_if_connectivity_ok() {
    // #597: CLIProxyAPI process is running and answering /models (so the TCP
    // connectivity ping succeeds), but the OAuth token has expired and
    // `claudeAuthenticated` is false. Every real AI request would fail with
    // `authentication_error`, so `connected` MUST be false — not the
    // misleading `true` it reported on prod SNV during the 2026-07 incident.
    assert!(
        !compute_ai_connected(true, false, true, true),
        "connected must be false when claudeAuthenticated is false, even if \
         the proxy port answers the connectivity ping"
    );
}

#[test]
fn connected_is_true_only_when_all_three_signals_are_ok() {
    assert!(
        compute_ai_connected(true, true, true, true),
        "connected is true only when the proxy is reachable, Claude is authenticated, AND the configured model is valid"
    );
}

#[test]
fn connected_is_false_when_connectivity_fails_even_if_auth_appears_ok() {
    // Edge case: auth reports true (e.g. credential file exists) but the
    // proxy process is down/unreachable. `connected` must still be false.
    assert!(
        !compute_ai_connected(false, true, true, true),
        "connected must be false when the proxy port is unreachable, regardless of auth state"
    );
}

#[test]
fn connected_is_false_when_both_signals_fail() {
    assert!(
        !compute_ai_connected(false, false, true, true),
        "connected must be false when neither connectivity nor auth is present"
    );
}

// #661: the incident this fixes — an invalid `model` id sat `connected:
// true` for 4 days because nothing checked it. `model_valid=false` must
// flip `connected` to false even when connectivity AND auth are both fine.

#[test]
fn connected_is_false_when_the_configured_model_is_not_in_the_catalog() {
    assert!(
        !compute_ai_connected(true, true, false, true),
        "connected must be false when connectivity and auth are fine but the \
         configured model is not one the proxy actually serves"
    );
}

// #679: a user pointing `apiUrl` at their own non-bundled OpenAI-compatible
// endpoint (the #662 local-LLM scenario) never needs a Claude login at all —
// `claude_authenticated` must be ignored when `requires_claude_auth` is
// false.

#[test]
fn connected_ignores_claude_auth_when_not_required() {
    assert!(
        compute_ai_connected(true, false, true, false),
        "connected must be true when connectivity and the model are both fine \
         and Claude auth isn't required, even though claude_authenticated is false"
    );
}

// #624: `check_status` discarded the real error from `check_connectivity`
// behind a hardcoded "AI proxy unreachable" string, even when the actual
// failure was an HTTP-level error (401/500) that explains WHY. These pin
// `compute_ai_status_error`'s full branch table — previously the handler
// itself (not just `compute_ai_connected`'s truth table) had zero coverage.

#[test]
fn status_error_is_none_when_connected() {
    assert_eq!(
        compute_ai_status_error(true, true, true, "claude-opus-4-6", None, true),
        None
    );
}

#[test]
fn status_error_reports_unauthenticated_regardless_of_connectivity_error() {
    // claude_authenticated=false takes priority even if a connectivity error
    // string happens to be present — the auth message is the more actionable one.
    assert_eq!(
        compute_ai_status_error(
            false,
            false,
            true,
            "claude-opus-4-6",
            Some("AI API returned status 500"),
            true
        ),
        Some("Claude not authenticated — run /ai/proxy/login to re-authorize".to_string())
    );
}

#[test]
fn status_error_surfaces_the_real_connectivity_failure_message() {
    // The regression this ticket fixes: a 401 from the proxy must be visible
    // to the caller, not silently replaced by a generic "unreachable" string.
    let error = compute_ai_status_error(
        false,
        true,
        true,
        "claude-opus-4-6",
        Some("AI API returned status 401"),
        true,
    );
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
        compute_ai_status_error(false, true, true, "claude-opus-4-6", None, true),
        Some("AI proxy unreachable".to_string())
    );
}

// #661: the new third branch — connectivity and auth are both fine, but the
// configured model isn't in the proxy's catalog. This is the exact
// incident: a model id that would 502 on the first real chat call, sitting
// `connected: true` because nothing ever checked it.

#[test]
fn status_error_names_the_invalid_model_when_connectivity_and_auth_are_fine() {
    let error = compute_ai_status_error(false, true, false, "claude-opus-4-8", None, true);
    assert_eq!(
        error,
        Some(
            "Configured AI model 'claude-opus-4-8' is not available in the proxy's model catalog — check AI settings"
                .to_string()
        )
    );
}

#[test]
fn status_error_prefers_the_auth_message_over_the_model_message_when_both_are_wrong() {
    // If Claude isn't even authenticated, that is the more actionable
    // problem to surface first — a model-validity check would be moot
    // without a working login anyway.
    let error = compute_ai_status_error(false, false, false, "claude-opus-4-8", None, true);
    assert_eq!(
        error,
        Some("Claude not authenticated — run /ai/proxy/login to re-authorize".to_string())
    );
}

// #679: when Claude auth isn't required (a non-bundled `apiUrl`), the
// "Claude not authenticated" message must never appear — even when
// `connected` is false for a genuinely different reason (here: connectivity
// itself failed) and `claude_authenticated` also happens to be false.

#[test]
fn status_error_never_blames_claude_auth_when_not_required() {
    let error = compute_ai_status_error(
        false,
        false,
        true,
        "some-model",
        Some("connection refused"),
        false,
    );
    assert_eq!(
        error,
        Some("AI proxy unreachable: connection refused".to_string()),
        "must never emit the 'Claude not authenticated' message when Claude \
         auth isn't required, got: {error:?}"
    );
}

// #679 review finding 1: `get_settings_internal` used to ALSO substitute the
// live-resolved bundled-proxy URL into `settings.api_url` whenever it was
// the default and the proxy was running — and BOTH `update_settings`
// (persistence) and `get_settings` (display, later echoed back on an
// ordinary settings save) read straight from that mutated value. Once
// persisted, the substituted URL no longer equals `AiSettings::default()`'s
// literal string, so `is_bundled_default`/`requires_claude_auth` silently
// and PERMANENTLY flip to `false` for what is still, functionally, the
// bundled proxy — defeating this whole ticket's "byte-identical default
// behavior" requirement on the very first ordinary settings save.
// `get_settings_internal` is a PURE database read that never MUTATES
// `api_url` — this test pins that invariant directly: however this
// function is called, its returned `api_url` is EXACTLY the raw
// stored/default value, never a substituted one. (#683 later added a
// read-only `state.ai_proxy().configured_port()` call so the bundled/
// non-bundled CLASSIFICATION can also recognize a historically-substituted
// address — see `is_bundled_proxy_address` — but that call only ever reads
// the proxy's configured port; it still never touches or rewrites the
// returned `api_url` itself.)

#[tokio::test]
async fn get_settings_internal_never_substitutes_the_live_proxy_url() {
    let state = crate::state::AppState::in_memory().await.unwrap();
    let (settings, is_bundled_default) = get_settings_internal(&state).await.unwrap();
    assert_eq!(
        settings.api_url,
        crate::ai::AiSettings::default().api_url,
        "with nothing stored yet, api_url must be exactly the literal \
         default — never a live-resolved proxy URL"
    );
    assert!(
        is_bundled_default,
        "the literal default api_url must be classified as bundled"
    );
}

// #683: #679 fixed the SUBSTITUTE-then-persist bug going forward (see the
// comment above), but never migrated rows a PRE-#679 build had already
// poisoned — every DB that ever saved AI settings before that fix stored
// the SUBSTITUTED live-proxy address (`http://127.0.0.1:{port}/v1`), not
// the literal default placeholder. `get_settings_internal`'s literal-only
// equality check misclassifies that address as non-bundled FOREVER,
// because `127.0.0.1:18787` never equals `localhost:8787` as a string —
// even though it is, functionally, still the exact address the bundled
// proxy listens on. Verified live on prod: `apiUrl` stored as
// `http://127.0.0.1:18787/v1`, `/ai/status` reporting
// `requiresClaudeAuth: false` on a box running the bundled proxy.
//
// `18787` is hardcoded here rather than read from a live `ProxyManager`
// getter (added by the fix, not present yet when this test was written) —
// safe because `AppState::in_memory()`'s `ProxyManager` is always
// constructed via `ProxyConfig::default()`, and nothing in this codebase
// ever changes its port (no setter exists), so every future run of this
// test sees the same port.

#[tokio::test]
async fn get_settings_internal_treats_a_historically_substituted_bundled_url_as_bundled() {
    use crate::ai::AI_SETTINGS_KEY;

    let state = crate::state::AppState::in_memory().await.unwrap();
    let poisoned = crate::ai::AiSettings {
        api_url: "http://127.0.0.1:18787/v1".to_string(),
        api_key: None,
        model: "claude-opus-4-6".to_string(),
        system_prompt_extra: None,
    };
    state
        .repository()
        .set_app_setting(AI_SETTINGS_KEY, &serde_json::to_string(&poisoned).unwrap())
        .await
        .unwrap();

    let (settings, is_bundled_default) = get_settings_internal(&state).await.unwrap();
    assert_eq!(settings.api_url, "http://127.0.0.1:18787/v1");
    assert!(
        is_bundled_default,
        "a stored api_url that structurally matches the bundled proxy's own \
         address must be classified as bundled, even though it is not the \
         literal default placeholder string"
    );
}

// #683: the SAME regression, proven end-to-end through the REAL
// `/ai/status` handler this time (not just the pure `get_settings_internal`
// unit test above) — driving the full router the way the `put_ai_settings_...`
// tests below do. `AppState::in_memory()`'s `ProxyManager` is never started
// (no child process spawned in-test), so this ALSO pins the proxy-STOPPED
// semantics decided in the design comment on this issue: a stored address
// that structurally matches the bundled proxy's own CONFIGURED port must
// still report `requiresClaudeAuth: true` — the port is static config, not
// something that exists only while the process happens to be running.

#[tokio::test]
async fn ai_status_reports_requires_claude_auth_for_a_historically_substituted_bundled_url() {
    use crate::ai::AI_SETTINGS_KEY;
    use crate::router::build_router;
    use crate::state::AppState;
    use axum::body::Body;
    use axum::http::{Method, Request};
    use tower::ServiceExt;

    let state = AppState::in_memory().await.unwrap();
    let poisoned = crate::ai::AiSettings {
        api_url: "http://127.0.0.1:18787/v1".to_string(),
        api_key: None,
        model: "claude-opus-4-6".to_string(),
        system_prompt_extra: None,
    };
    state
        .repository()
        .set_app_setting(AI_SETTINGS_KEY, &serde_json::to_string(&poisoned).unwrap())
        .await
        .unwrap();

    let app = build_router(state);
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/ai/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        body.get("requiresClaudeAuth").and_then(|v| v.as_bool()),
        Some(true),
        "a historically-substituted bundled api_url must still require \
         Claude auth, even with the proxy process not running in this test: {body:?}"
    );
}

// #683: `is_bundled_proxy_address` unit coverage — the pure structural
// matcher's own truth table, independent of any DB/AppState plumbing.

#[test]
fn is_bundled_proxy_address_matches_the_substituted_127_0_0_1_form() {
    assert!(is_bundled_proxy_address("http://127.0.0.1:18787/v1", 18787));
}

#[test]
fn is_bundled_proxy_address_matches_the_localhost_form_defensively() {
    // No code path in this repo has ever written this exact form to the DB
    // (only the 127.0.0.1 substituted form, and the literal-default
    // "localhost:8787" checked separately) — matched anyway per the
    // issue's own fix-shape request, since it costs nothing and keeps the
    // matcher symmetric with the two hosts this process could ever produce.
    assert!(is_bundled_proxy_address("http://localhost:18787/v1", 18787));
}

#[test]
fn is_bundled_proxy_address_rejects_a_genuinely_foreign_port() {
    // Same host, wrong port — e.g. an Ollama/LM-Studio style local LLM the
    // user deliberately pointed `apiUrl` at (the #662 scenario). Must never
    // be classified as the bundled proxy.
    assert!(!is_bundled_proxy_address(
        "http://127.0.0.1:11434/v1",
        18787
    ));
}

#[test]
fn is_bundled_proxy_address_rejects_a_genuinely_foreign_host() {
    assert!(!is_bundled_proxy_address(
        "http://192.168.1.50:18787/v1",
        18787
    ));
}

#[test]
fn is_bundled_proxy_address_rejects_https() {
    // The bundled proxy is plain http only — an https URL on the exact
    // same host/port/path is still not the bundled proxy's own address.
    assert!(!is_bundled_proxy_address(
        "https://127.0.0.1:18787/v1",
        18787
    ));
}

#[test]
fn is_bundled_proxy_address_rejects_a_different_path() {
    assert!(!is_bundled_proxy_address(
        "http://127.0.0.1:18787/v1/chat/completions",
        18787
    ));
    assert!(!is_bundled_proxy_address("http://127.0.0.1:18787/", 18787));
}

#[test]
fn is_bundled_proxy_address_tolerates_a_trailing_slash() {
    assert!(is_bundled_proxy_address(
        "http://127.0.0.1:18787/v1/",
        18787
    ));
}

#[test]
fn is_bundled_proxy_address_tolerates_host_case_differences() {
    // `Url::parse` normalizes the host to lowercase per the URL spec, so an
    // operator hand-editing the stored value with different casing must
    // still match.
    assert!(is_bundled_proxy_address("http://LOCALHOST:18787/v1", 18787));
}

#[test]
fn is_bundled_proxy_address_rejects_an_unparseable_url() {
    assert!(!is_bundled_proxy_address("not a url at all", 18787));
    assert!(!is_bundled_proxy_address("", 18787));
}

#[test]
fn is_bundled_proxy_address_rejects_when_no_port_is_present() {
    // A bare `http://localhost/v1` (no explicit port) never equals
    // `Some(proxy_port)` — the bundled proxy always listens on a
    // non-default HTTP port, so a URL with none explicitly stated can never
    // be its address.
    assert!(!is_bundled_proxy_address("http://localhost/v1", 18787));
}

// #683 review: the doc comment on `is_bundled_proxy_address` claims a
// trailing slash or different host/SCHEME casing can't defeat the match —
// only host casing had a test. Pin the scheme half too.

#[test]
fn is_bundled_proxy_address_tolerates_scheme_case_differences() {
    assert!(is_bundled_proxy_address("HTTP://127.0.0.1:18787/v1", 18787));
}

// #683 review: `Url::path()` excludes the query string and `host_str()`
// ignores userinfo, so a matched URL carrying either still classifies as
// bundled — pinning this explicitly, since it's also why
// `update_settings`'s self-heal would silently DROP userinfo/query on
// rewrite (mitigated by the BUNDLED_PROXY_PLACEHOLDER guard added in the
// same review round, which now only rewrites when there is no
// PRESENTER_AI_API_URL override in effect).

#[test]
fn is_bundled_proxy_address_ignores_userinfo_and_query() {
    assert!(is_bundled_proxy_address(
        "http://u:p@127.0.0.1:18787/v1",
        18787
    ));
    assert!(is_bundled_proxy_address(
        "http://127.0.0.1:18787/v1?x=1",
        18787
    ));
}

// #683 review: `should_self_heal_to_canonical`'s own truth table — proves
// the env-override guard directly (no env mutation needed, since it's
// parameterized on `canonical` rather than reading `AiSettings::default()`
// itself). This is the exact bug the reviewer found: without the
// `canonical == BUNDLED_PROXY_PLACEHOLDER` guard, a box running
// `PRESENTER_AI_API_URL` pointed at a foreign endpoint would have
// `update_settings` silently rewrite a matched bundled row to that foreign
// value on the very next ordinary save.

#[test]
fn should_self_heal_to_canonical_fires_when_canonical_is_the_literal_placeholder() {
    assert!(should_self_heal_to_canonical(
        "http://127.0.0.1:18787/v1",
        18787,
        "http://localhost:8787/v1"
    ));
}

#[test]
fn should_self_heal_to_canonical_never_fires_under_a_presenter_ai_api_url_override() {
    // `canonical` here is whatever AiSettings::default() resolves to under
    // a PRESENTER_AI_API_URL override — a genuinely foreign string, never
    // the literal placeholder. The self-heal must stay a no-op, even
    // though the stored value still structurally matches the bundled
    // proxy's own address.
    assert!(!should_self_heal_to_canonical(
        "http://127.0.0.1:18787/v1",
        18787,
        "http://10.0.0.5:9000/v1"
    ));
}

#[test]
fn should_self_heal_to_canonical_never_fires_for_a_genuinely_foreign_stored_url() {
    assert!(!should_self_heal_to_canonical(
        "http://192.168.1.50:11434/v1",
        18787,
        "http://localhost:8787/v1"
    ));
}
