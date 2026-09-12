//! Router-level tests for the backend-agnostic AI connectivity/status
//! computation (#661, #762, #764).
//!
//! Since #762 removed the bundled CLIProxyAPI proxy + Claude OAuth, `/ai/status`
//! carries only `{connected, error, modelValid}` — no `proxy` object, no
//! `requiresClaudeAuth`, no bundled-proxy classification. This file holds the
//! pure `compute_ai_connected`/`compute_ai_status_error` truth tables, the
//! `get_settings_internal` raw-read invariant, the #764 last-completion fold
//! driven end-to-end through the real handler against a wiremock backend, and
//! the #762 payload-shape regression. `ai_tests.rs` keeps the
//! chat/settings-audit/idle-clear tests.

use crate::router::ai::{compute_ai_connected, compute_ai_status_error, get_settings_internal};

// ── compute_ai_connected: connected IFF connectivity AND model both OK ──

#[test]
fn connected_is_true_only_when_connectivity_and_model_are_ok() {
    assert!(
        compute_ai_connected(true, true),
        "connected is true only when the backend is reachable AND the configured model is valid"
    );
}

#[test]
fn connected_is_false_when_connectivity_fails() {
    assert!(
        !compute_ai_connected(false, true),
        "connected must be false when the backend is unreachable, regardless of model validity"
    );
}

#[test]
fn connected_is_false_when_the_configured_model_is_not_in_the_catalog() {
    // #661: the incident this fixes — an invalid `model` id sat
    // `connected: true` for 4 days because nothing checked it. `model_valid=false`
    // must flip `connected` to false even when connectivity is fine.
    assert!(
        !compute_ai_connected(true, false),
        "connected must be false when connectivity is fine but the configured \
         model is not one the backend actually serves"
    );
}

// ── compute_ai_status_error branch table (#624/#661, OAuth branch removed #762) ──

#[test]
fn status_error_is_none_when_connected() {
    assert_eq!(
        compute_ai_status_error(true, true, "google/gemini-3.8-flash", None),
        None
    );
}

#[test]
fn status_error_names_the_invalid_model_when_connectivity_is_fine() {
    let error = compute_ai_status_error(false, false, "bad/model-id", None);
    assert_eq!(
        error,
        Some(
            "Configured AI model 'bad/model-id' is not available in the backend's model catalog — check AI settings"
                .to_string()
        )
    );
}

#[test]
fn status_error_surfaces_the_real_connectivity_failure_message() {
    // The regression #624 fixes: a 401 from the backend must be visible to the
    // caller, not silently replaced by a generic "unreachable" string.
    let error = compute_ai_status_error(false, true, "m", Some("AI API returned status 401"));
    assert_eq!(
        error,
        Some("AI backend unreachable: AI API returned status 401".to_string()),
        "the real connectivity error must be surfaced, not replaced by a generic message"
    );
}

#[test]
fn status_error_falls_back_to_generic_message_when_no_error_string_available() {
    // Defensive branch: connectivity failed with no captured error string.
    assert_eq!(
        compute_ai_status_error(false, true, "m", None),
        Some("AI backend unreachable".to_string())
    );
}

// ── get_settings_internal: a pure DB read returning the raw stored/default ──

#[tokio::test]
async fn get_settings_internal_returns_the_default_when_nothing_stored() {
    // With nothing stored yet, the raw read returns exactly the literal
    // default (since #762: `https://openrouter.ai/api/v1`) — never a
    // substituted/mutated value. `get_settings_internal` never touches
    // `api_url` (the persist/display path stays raw; env overrides live in
    // `resolve_effective_settings`).
    let state = crate::state::AppState::in_memory().await.unwrap();
    let settings = get_settings_internal(&state).await.unwrap();
    assert_eq!(
        settings.api_url,
        crate::ai::AiSettings::default().api_url,
        "with nothing stored yet, api_url must be exactly the literal default"
    );
}

// ── #764: `connected` must reflect the last REAL completion, not just the
// `list_models` probe ──────────────────────────────────────────────────────
//
// A metered backend (OpenRouter, #761) serves `GET /models` 200 even when the
// workspace budget is exhausted / the key is revoked, while `POST
// /chat/completions` 403s. Before #764 `evaluate_ai_status` derived
// `connected` solely from `list_models`, so `/ai/status` and `/healthz.ai`
// reported a false `connected:true` during a budget outage — exactly the
// failure #760 was meant to surface. These drive the FULL production path
// (`POST /ai/chat` -> `run_agent` -> `call_chat_completions`) against a
// wiremock backend, then read the REAL `/ai/status` handler.

/// A `/models` catalog body containing `model` (so the `list_models` probe
/// succeeds AND the configured model validates — isolating the completion
/// signal as the only thing that can flip `connected`).
fn models_catalog(model: &str) -> serde_json::Value {
    serde_json::json!({
        "object": "list",
        "data": [{"id": model, "object": "model", "owned_by": "test"}],
    })
}

/// A minimal successful chat-completion body (`run_agent` reads `choices[0]`).
fn ok_completion_body(text: &str) -> serde_json::Value {
    serde_json::json!({
        "choices": [{
            "message": {"role": "assistant", "content": text, "tool_calls": null},
            "finish_reason": "stop"
        }]
    })
}

/// The live 403 budget body from the issue's real dev repro.
const BUDGET_403_BODY: &str =
    "{\"error\":{\"message\":\"Workspace daily budget of $1.00 exceeded. Contact your org admin.\",\"code\":403}}";

async fn seed_openrouter_like_state(mock_uri: &str, model: &str) -> crate::state::AppState {
    use crate::ai::AI_SETTINGS_KEY;
    let state = crate::state::AppState::in_memory().await.unwrap();
    let settings = crate::ai::AiSettings {
        api_url: mock_uri.to_string(),
        api_key: None,
        model: model.to_string(),
        system_prompt_extra: None,
    };
    state
        .repository()
        .set_app_setting(AI_SETTINGS_KEY, &serde_json::to_string(&settings).unwrap())
        .await
        .unwrap();
    state
}

/// POST `/ai/chat` and fully drain the SSE response, so `run_agent` (and its
/// completion-health recording) has genuinely finished before we read status.
async fn drive_ai_chat(state: &crate::state::AppState) {
    use crate::router::build_router;
    use axum::body::Body;
    use axum::http::{Method, Request};
    use tower::ServiceExt;

    let app = build_router(state.clone());
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/ai/chat")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({"message": "Odpovedz iba: OK"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let _ = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
}

/// GET `/ai/status` through the real handler and return the parsed JSON.
async fn read_ai_status(state: &crate::state::AppState) -> serde_json::Value {
    use crate::router::build_router;
    use axum::body::Body;
    use axum::http::{Method, Request};
    use tower::ServiceExt;

    let app = build_router(state.clone());
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
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn ai_status_connected_is_false_after_a_completion_403_even_though_models_200() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(models_catalog("test-model")))
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(403).set_body_string(BUDGET_403_BODY))
        .mount(&mock)
        .await;

    let state = seed_openrouter_like_state(&mock.uri(), "test-model").await;
    drive_ai_chat(&state).await; // one failed completion -> records the failure

    let status = read_ai_status(&state).await;
    assert_eq!(
        status.get("connected").and_then(|v| v.as_bool()),
        Some(false),
        "a completion 403 (exhausted budget) must flip connected:false even though \
         /models still 200s: {status:?}"
    );
    let error = status.get("error").and_then(|v| v.as_str()).unwrap_or("");
    assert!(
        error.contains("budget"),
        "the status error must carry the backend budget message: {status:?}"
    );
    // #760 guard: /models 200 means the model is still valid — the flip must
    // come from the completion signal, not a spurious modelValid:false.
    assert_eq!(
        status.get("modelValid").and_then(|v| v.as_bool()),
        Some(true),
        "modelValid must stay true (the model is in the catalog): {status:?}"
    );
}

#[tokio::test]
async fn ai_status_connected_recovers_after_a_subsequent_successful_completion() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(models_catalog("test-model")))
        .mount(&mock)
        .await;
    // First completion 403s (budget), then the mock is exhausted and the
    // second completion falls through to the 200 mock below — no mid-test
    // remount needed.
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(403).set_body_string(BUDGET_403_BODY))
        .up_to_n_times(1)
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ok_completion_body("OK")))
        .mount(&mock)
        .await;

    let state = seed_openrouter_like_state(&mock.uri(), "test-model").await;

    drive_ai_chat(&state).await; // #1: 403 -> records the failure
    let after_failure = read_ai_status(&state).await;
    assert_eq!(
        after_failure.get("connected").and_then(|v| v.as_bool()),
        Some(false),
        "the failed completion must have flipped connected:false: {after_failure:?}"
    );

    drive_ai_chat(&state).await; // #2: 200 -> clears the failure
    let after_success = read_ai_status(&state).await;
    assert_eq!(
        after_success.get("connected").and_then(|v| v.as_bool()),
        Some(true),
        "a subsequent successful completion must clear the failure and restore \
         connected:true: {after_success:?}"
    );
    assert!(
        after_success
            .get("error")
            .map(|v| v.is_null())
            .unwrap_or(false),
        "error must be null once connected again: {after_success:?}"
    );
}

// #762 [red->green]: after removing the bundled CLIProxyAPI proxy + Claude
// OAuth, the `/ai/status` payload is backend-agnostic — it must NO LONGER carry
// the nested `proxy` object or the `requiresClaudeAuth` flag (an API-key backend
// like OpenRouter has no bundled proxy and no Claude login concept). It keeps
// only `connected`, `error`, and `modelValid`. Seeded with an unreachable
// endpoint so the shape assertion is deterministic (no live network, connectivity
// simply fails fast) — the JSON key set is independent of the verdict.
#[tokio::test]
async fn ai_status_has_no_proxy_or_requires_claude_auth_fields() {
    let state = seed_openrouter_like_state("http://127.0.0.1:1/v1", "any-model").await;
    let body = read_ai_status(&state).await;

    assert!(
        body.get("proxy").is_none(),
        "the backend-agnostic /ai/status must not carry a nested `proxy` object: {body:?}"
    );
    assert!(
        body.get("requiresClaudeAuth").is_none(),
        "the backend-agnostic /ai/status must not carry `requiresClaudeAuth`: {body:?}"
    );
    // The backend-agnostic shape it DOES keep.
    assert!(
        body.get("connected").is_some(),
        "must keep `connected`: {body:?}"
    );
    assert!(
        body.get("error").is_some(),
        "must keep `error` key: {body:?}"
    );
    assert!(
        body.get("modelValid").is_some(),
        "must keep `modelValid`: {body:?}"
    );
}
