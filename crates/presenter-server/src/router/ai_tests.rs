//! Router-level tests for `/ai/status` (#597): the `connected` field MUST NOT
//! report `true` when `proxy.claudeAuthenticated == false`. The TCP-only
//! connectivity check is necessary but not sufficient — actual AI readiness
//! requires a valid Claude OAuth session as well.

use crate::ai::AiAgentError;
use crate::router::ai::{
    compute_ai_connected, compute_ai_status_error, friendly_ai_error_message,
    get_settings_internal, is_bundled_proxy_address, parse_idle_clear_minutes,
    render_connectivity_error, should_idle_clear, should_self_heal_to_canonical,
    DEFAULT_IDLE_CLEAR_MINUTES,
};
use std::time::{Duration, SystemTime};

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

// #661 item 3: `PUT /ai/settings` used to write through the bare
// `set_app_setting` upsert with zero audit hook — the ONE settings family
// that could never produce a `settings_audit` row, which is why the
// undiagnosed 2026-08-02 prod model-id edit left no forensic trail. Drives
// the REAL HTTP handler through the full router (not just a pure
// function), matching the `post_ai_chat_...` tests' own style below.

#[tokio::test]
async fn put_ai_settings_records_a_settings_audit_row_with_the_actor() {
    use crate::router::build_router;
    use crate::state::AppState;
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use presenter_persistence::SettingsAuditSource;
    use tower::ServiceExt;

    let state = AppState::in_memory().await.unwrap();
    let app = build_router(state.clone());

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/ai/settings")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .header("x-forwarded-for", "10.1.2.3, 10.0.0.1")
                .body(Body::from(
                    serde_json::json!({"model": "claude-sonnet-4-6"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let rows = state
        .repository()
        .list_settings_audit(
            Some("app_settings"),
            Some(crate::ai::AI_SETTINGS_KEY),
            None,
            10,
        )
        .await
        .unwrap();
    assert_eq!(
        rows.len(),
        1,
        "PUT /ai/settings must record exactly one audit row: {rows:?}"
    );
    assert_eq!(
        rows[0].actor, "10.1.2.3",
        "actor must be extracted from the FIRST X-Forwarded-For hop"
    );
    assert_eq!(rows[0].source, SettingsAuditSource::HttpSetter);
    assert_eq!(
        rows[0].after_json.get("model").and_then(|v| v.as_str()),
        Some("claude-sonnet-4-6"),
        "the audit row must carry the NEW model value: {:?}",
        rows[0].after_json
    );
}

#[tokio::test]
async fn put_ai_settings_falls_back_to_anonymous_actor_with_no_forwarding_headers() {
    use crate::router::build_router;
    use crate::state::AppState;
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use tower::ServiceExt;

    let state = AppState::in_memory().await.unwrap();
    let app = build_router(state.clone());

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/ai/settings")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({"model": "claude-sonnet-4-6"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let rows = state
        .repository()
        .list_settings_audit(
            Some("app_settings"),
            Some(crate::ai::AI_SETTINGS_KEY),
            None,
            10,
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].actor, "anonymous");
}

// #679 review finding 1: an ORDINARY `PUT /ai/settings` that omits `apiUrl`
// entirely (exactly the payload the two tests above already send) used to
// re-persist whatever `get_settings_internal` had returned — which, before
// this fix, could already be the SUBSTITUTED live-proxy URL. Drives the
// REAL `PUT` then `GET` handlers through the full router (not just the
// pure `get_settings_internal` unit test above) to prove the round-trip
// itself preserves the literal default `apiUrl`.

#[tokio::test]
async fn put_ai_settings_with_no_api_url_never_mutates_the_stored_default() {
    use crate::router::build_router;
    use crate::state::AppState;
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use tower::ServiceExt;

    let state = AppState::in_memory().await.unwrap();
    let app = build_router(state.clone());

    let put_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/ai/settings")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({"model": "claude-sonnet-4-6"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(put_response.status(), StatusCode::NO_CONTENT);

    let get_response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/ai/settings")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(get_response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(get_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        body.get("apiUrl").and_then(|v| v.as_str()),
        Some(crate::ai::AiSettings::default().api_url.as_str()),
        "a PUT that never mentioned apiUrl must leave the stored value at \
         the literal default, not a substituted live-proxy URL: {body:?}"
    );
}

// #683 (optional hardening): a legitimate PUT /ai/settings save must
// self-heal a historically-poisoned row — even one where the payload never
// mentions `apiUrl` at all — back to the canonical literal default string,
// so `update_settings`'s persisted value and `check_status`'s live
// classification can never end up disagreeing about what "bundled" means.

#[tokio::test]
async fn put_ai_settings_normalizes_a_historically_substituted_bundled_url_to_the_canonical_default(
) {
    use crate::ai::AI_SETTINGS_KEY;
    use crate::router::build_router;
    use crate::state::AppState;
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
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

    // An ORDINARY settings save that never mentions apiUrl at all.
    let put_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/ai/settings")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({"model": "claude-sonnet-4-6"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(put_response.status(), StatusCode::NO_CONTENT);

    let get_response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/ai/settings")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(get_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        body.get("apiUrl").and_then(|v| v.as_str()),
        Some(crate::ai::AiSettings::default().api_url.as_str()),
        "a historically-poisoned api_url must self-heal to the canonical \
         default string on the next ordinary settings save: {body:?}"
    );
}

// #683: the normalize-on-save hardening above must NEVER touch a
// genuinely-foreign `apiUrl` (the #662 local-LLM scenario) — an ordinary
// save that doesn't mention `apiUrl` must leave a real non-bundled endpoint
// exactly as the operator configured it.

#[tokio::test]
async fn put_ai_settings_never_touches_a_genuinely_non_bundled_api_url() {
    use crate::ai::AI_SETTINGS_KEY;
    use crate::router::build_router;
    use crate::state::AppState;
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use tower::ServiceExt;

    const FOREIGN_URL: &str = "http://192.168.1.50:11434/v1";

    let state = AppState::in_memory().await.unwrap();
    let non_bundled = crate::ai::AiSettings {
        api_url: FOREIGN_URL.to_string(),
        api_key: None,
        model: "llama3".to_string(),
        system_prompt_extra: None,
    };
    state
        .repository()
        .set_app_setting(
            AI_SETTINGS_KEY,
            &serde_json::to_string(&non_bundled).unwrap(),
        )
        .await
        .unwrap();

    let app = build_router(state);

    let put_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/ai/settings")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({"model": "llama3.1"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(put_response.status(), StatusCode::NO_CONTENT);

    let get_response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/ai/settings")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(get_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        body.get("apiUrl").and_then(|v| v.as_str()),
        Some(FOREIGN_URL),
        "a genuinely non-bundled apiUrl must never be rewritten by the \
         normalize-on-save hardening: {body:?}"
    );
}

// #675 review finding 3: `cd7c0f56` replaced `serde_json::to_value(&settings)`
// with `redact_settings_for_audit()` to stop the raw `api_key` from being
// persisted into `settings_audit` (`AiSettings` derives `Serialize`, so the
// naive snapshot would have written the literal key into the DB). But BOTH
// existing PUT-handler tests above leave `api_key: None` the whole time, so
// they pass identically with the fix reverted — this test actually sets a
// real key and asserts the audit row never carries it.
#[tokio::test]
async fn put_ai_settings_with_a_real_api_key_never_writes_it_into_the_audit_row() {
    use crate::router::build_router;
    use crate::state::AppState;
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use tower::ServiceExt;

    const SECRET_KEY: &str = "sk-ant-super-secret-do-not-leak-4f8e2b91";

    let state = AppState::in_memory().await.unwrap();
    let app = build_router(state.clone());

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/ai/settings")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({"apiKey": SECRET_KEY}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let rows = state
        .repository()
        .list_settings_audit(
            Some("app_settings"),
            Some(crate::ai::AI_SETTINGS_KEY),
            None,
            10,
        )
        .await
        .unwrap();
    assert_eq!(
        rows.len(),
        1,
        "PUT /ai/settings must record exactly one audit row: {rows:?}"
    );

    // The redacted-away signal: `apiKeySet` must be true (a key WAS set),
    // and it must be the ONLY thing the audit row says about the key.
    assert_eq!(
        rows[0]
            .after_json
            .get("apiKeySet")
            .and_then(|v| v.as_bool()),
        Some(true),
        "after_json must record that a key IS set: {:?}",
        rows[0].after_json
    );

    // The regression: neither snapshot may contain the raw secret anywhere,
    // and neither may carry a raw `apiKey` field at all — only the boolean
    // `apiKeySet`. Stringifying the whole JSON value (rather than checking
    // one named field) is deliberate: it also catches the raw key leaking
    // in via any OTHER field name a future edit might introduce.
    let before_str = rows[0]
        .before_json
        .as_ref()
        .map(serde_json::Value::to_string)
        .unwrap_or_default();
    let after_str = rows[0].after_json.to_string();
    assert!(
        !before_str.contains(SECRET_KEY) && !after_str.contains(SECRET_KEY),
        "the raw API key must never be persisted into the settings_audit row: \
         before={before_str} after={after_str}"
    );
    assert!(
        !after_str.contains("\"apiKey\""),
        "the audit snapshot must never carry a raw `apiKey` field, only the \
         redacted `apiKeySet` boolean: {after_str}"
    );
}

// #624 follow-up: `.to_string()` on an anyhow error only renders the
// OUTERMOST `.context(...)` layer — `check_connectivity`'s real transport
// failure (DNS/TLS/timeout/connection-refused) gets silently dropped behind
// the "failed to reach AI API" wrapper context. `render_connectivity_error`
// must render the FULL chain so the underlying cause stays visible.
#[test]
fn connectivity_error_renders_full_anyhow_chain() {
    let err = anyhow::anyhow!("connection refused").context("failed to reach AI API");
    let rendered = render_connectivity_error(&err);
    assert!(
        rendered.contains("failed to reach AI API"),
        "rendered error must keep the outer context: {rendered}"
    );
    assert!(
        rendered.contains("connection refused"),
        "rendered error must keep the underlying cause, not just the outer context: {rendered}"
    );
}

// #665: the shared AI conversation is auto-cleared on the next `chat()` call
// once it has sat idle past the configured window — `should_idle_clear` is
// the pure decision function so this is testable without a real clock or a
// live handler.

#[test]
fn idle_clear_is_false_for_a_fresh_conversation_never_touched() {
    assert!(
        !should_idle_clear(None, SystemTime::now(), Duration::from_secs(1800)),
        "a conversation that has never been touched has nothing to clear"
    );
}

#[test]
fn idle_clear_is_false_while_still_within_the_idle_window() {
    let now = SystemTime::now();
    let last_activity = now - Duration::from_secs(300); // 5 minutes ago
    assert!(
        !should_idle_clear(Some(last_activity), now, Duration::from_secs(1800)),
        "5 minutes of idle must not clear a 30-minute window"
    );
}

#[test]
fn idle_clear_is_true_once_the_idle_window_has_elapsed() {
    let now = SystemTime::now();
    let last_activity = now - Duration::from_secs(1801); // just over 30 minutes ago
    assert!(
        should_idle_clear(Some(last_activity), now, Duration::from_secs(1800)),
        "a conversation idle longer than the window must be cleared"
    );
}

#[test]
fn idle_clear_is_false_exactly_at_the_boundary() {
    // The comparison is a strict `>`, not `>=` — exactly at the boundary
    // must NOT clear. Kills the off-by-one mutant that would flip this to
    // `>=` and clear one tick too early.
    let now = SystemTime::now();
    let last_activity = now - Duration::from_secs(1800);
    assert!(
        !should_idle_clear(Some(last_activity), now, Duration::from_secs(1800)),
        "exactly at the idle window boundary must not yet clear"
    );
}

#[test]
fn idle_clear_minutes_env_override_and_default() {
    // Pure-parser coverage, no env mutation (a mutated env var races
    // against every other test in this binary that reads the same key —
    // #665 review finding).
    assert_eq!(parse_idle_clear_minutes(None), DEFAULT_IDLE_CLEAR_MINUTES);
    assert_eq!(parse_idle_clear_minutes(Some("5")), 5);
    // Invalid / zero values fall back to the default rather than disabling
    // the idle-clear entirely.
    assert_eq!(
        parse_idle_clear_minutes(Some("0")),
        DEFAULT_IDLE_CLEAR_MINUTES
    );
    assert_eq!(
        parse_idle_clear_minutes(Some("not-a-number")),
        DEFAULT_IDLE_CLEAR_MINUTES
    );
}

// #665: `chat()`'s SSE error branch used to leak the provider's raw
// "prompt is too long" text straight to the operator. `friendly_ai_error_message`
// is the one place that must show `AiAgentError::ContextBudgetExceeded`'s
// friendly wording instead — and must find it even under additional
// `.context(...)` layers, which is what makes it meaningfully different from
// a bare `.to_string()` (see `.claude/rules/repository-error-pattern.md`).

#[test]
fn friendly_ai_error_message_finds_context_budget_exceeded_even_under_added_context() {
    let err = anyhow::Error::from(AiAgentError::ContextBudgetExceeded)
        .context("while streaming the AI response");
    let message = friendly_ai_error_message(&err);
    assert!(
        message.contains("Click \"Clear\""),
        "must show the friendly Click-Clear wording even under added context, got: {message}"
    );
    assert_ne!(
        message,
        err.to_string(),
        "must differ from the raw outer-context .to_string(), which would show only \
         'while streaming the AI response' and hide the friendly wording"
    );
}

#[test]
fn friendly_ai_error_message_forwards_other_errors_via_display() {
    let err = anyhow::anyhow!("connection refused");
    assert_eq!(friendly_ai_error_message(&err), "connection refused");
}

// #665 AC5: `POST /ai/chat` must return the friendly operator-facing error —
// never the provider's raw "prompt is too long" text — when the budget
// cannot be met. Drives the REAL HTTP endpoint through the full router
// (not just the pure `friendly_ai_error_message` function above).

#[tokio::test]
async fn post_ai_chat_never_leaks_prompt_is_too_long_when_the_budget_is_exceeded() {
    use crate::ai::AI_SETTINGS_KEY;
    use crate::router::build_router;
    use crate::state::AppState;
    use axum::body::Body;
    use axum::http::{Method, Request};
    use tower::ServiceExt;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // A mock that WOULD return a raw provider "prompt is too long" rejection
    // if it were ever called — the point of this test is that it must NOT
    // be, because the oversized turn is refused before any provider call.
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
            "error": {"message": "prompt is too long: 512000 tokens > 200000 maximum"}
        })))
        .mount(&mock_server)
        .await;

    let state = AppState::in_memory().await.unwrap();
    let settings = crate::ai::AiSettings {
        api_url: mock_server.uri(),
        api_key: None,
        model: "test-model".to_string(),
        system_prompt_extra: None,
    };
    state
        .repository()
        .set_app_setting(AI_SETTINGS_KEY, &serde_json::to_string(&settings).unwrap())
        .await
        .unwrap();

    let app = build_router(state);
    let huge_message = "Z".repeat(400_000);
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/ai/chat")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({"message": huge_message}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_text = String::from_utf8_lossy(&bytes);

    assert!(
        !body_text.contains("prompt is too long"),
        "the raw provider rejection text must never reach the SSE stream, got: {body_text}"
    );
    assert!(
        body_text.contains("conversation grew too large"),
        "the friendly Click-Clear message must be shown instead, got: {body_text}"
    );

    let requests = mock_server.received_requests().await.unwrap();
    assert!(
        requests.is_empty(),
        "the provider must never be called once the turn's own content exceeds the budget"
    );
}

// #665 review finding: `should_idle_clear` is well covered as a pure
// function, but nothing drove `chat()`'s ACTUAL idle-clear branch —
// deleting the clear-on-idle call, deleting the activity re-stamp (which
// would disable idle-clear forever), or inverting the condition would all
// silently pass the rest of the suite. These drive the REAL handler
// through the full router.

fn text_response_body(text: &str) -> serde_json::Value {
    serde_json::json!({
        "choices": [{
            "message": {"role": "assistant", "content": text, "tool_calls": null},
            "finish_reason": "stop"
        }]
    })
}

/// Build a router whose `/ai/chat` provider always replies with `reply`,
/// seed `ai_conversation` with one message carrying `stale_content`, seed
/// `ai_last_activity` to `age_ago` in the past, then POST a fresh message
/// and drain the SSE response fully (so the spawned write-back task has
/// genuinely finished) before returning the shared `AppState` for
/// inspection.
async fn drive_idle_clear_scenario(
    stale_content: &str,
    age_ago: std::time::Duration,
    reply: &str,
) -> crate::state::AppState {
    use crate::ai::{ChatMessage, AI_SETTINGS_KEY};
    use crate::router::build_router;
    use crate::state::AppState;
    use axum::body::Body;
    use axum::http::{Method, Request};
    use tower::ServiceExt;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(text_response_body(reply)))
        .mount(&mock_server)
        .await;

    let state = AppState::in_memory().await.unwrap();
    let settings = crate::ai::AiSettings {
        api_url: mock_server.uri(),
        api_key: None,
        model: "test-model".to_string(),
        system_prompt_extra: None,
    };
    state
        .repository()
        .set_app_setting(AI_SETTINGS_KEY, &serde_json::to_string(&settings).unwrap())
        .await
        .unwrap();

    state.ai_conversation().write().await.push(ChatMessage {
        role: "user".to_string(),
        content: Some(stale_content.to_string()),
        tool_calls: None,
        tool_call_id: None,
        name: None,
        preview: None,
    });
    *state.ai_last_activity().write().await = Some(SystemTime::now() - age_ago);

    let app = build_router(state.clone());
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/ai/chat")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({"message": "new question"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let _ = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();

    state
}

#[tokio::test]
async fn post_ai_chat_clears_a_conversation_that_has_gone_idle_past_the_window() {
    let state = drive_idle_clear_scenario(
        "STALE MESSAGE FROM AN HOUR AGO",
        Duration::from_secs(3600), // well past the 30-minute default window
        "sure, done",
    )
    .await;

    let conversation = state.ai_conversation().read().await;
    assert!(
        conversation
            .iter()
            .all(|m| m.content.as_deref() != Some("STALE MESSAGE FROM AN HOUR AGO")),
        "the stale prior message must be cleared once the conversation has gone idle: {conversation:?}"
    );
    assert!(
        conversation
            .iter()
            .any(|m| m.role == "user" && m.content.as_deref() == Some("new question")),
        "the new message must still be appended after the clear: {conversation:?}"
    );
}

#[tokio::test]
async fn post_ai_chat_does_not_clear_a_conversation_that_is_still_fresh() {
    let state = drive_idle_clear_scenario(
        "RECENT MESSAGE FROM JUST NOW",
        Duration::from_secs(60), // well within the 30-minute default window
        "sure, done",
    )
    .await;

    let conversation = state.ai_conversation().read().await;
    assert!(
        conversation
            .iter()
            .any(|m| m.content.as_deref() == Some("RECENT MESSAGE FROM JUST NOW")),
        "a conversation that is still within the idle window must NOT be cleared: {conversation:?}"
    );
}
