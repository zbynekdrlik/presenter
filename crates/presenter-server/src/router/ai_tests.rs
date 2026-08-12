//! Router-level tests for `/ai/status` (#597): the `connected` field MUST NOT
//! report `true` when `proxy.claudeAuthenticated == false`. The TCP-only
//! connectivity check is necessary but not sufficient — actual AI readiness
//! requires a valid Claude OAuth session as well.

use crate::ai::AiAgentError;
use crate::router::ai::{
    compute_ai_connected, compute_ai_status_error, friendly_ai_error_message,
    parse_idle_clear_minutes, render_connectivity_error, should_idle_clear,
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
