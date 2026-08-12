use super::integrations::extract_actor;
use super::AppError;
use crate::ai::agent::ProgressEvent;
use crate::ai::proxy::ProxyStatus;
use crate::ai::{AiAgentError, AiSettings, ToolAction, AI_SETTINGS_KEY};
use crate::state::AppState;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, Sse};
use axum::response::IntoResponse;
use axum::Json;
use presenter_persistence::SettingsAuditSource;
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::time::{Duration, SystemTime};
use tracing::{error, instrument, warn};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ChatRequest {
    pub message: String,
}

/// Idle window after which the shared AI conversation is auto-cleared on the
/// next `chat()` call, when `PRESENTER_AI_IDLE_CLEAR_MINUTES` is unset. A
/// global conversation (one process, every operator/tab/session — see
/// `AppState::ai_conversation`) left untouched between services shouldn't
/// silently keep growing toward the next operator's very first question
/// (#665).
pub(super) const DEFAULT_IDLE_CLEAR_MINUTES: u64 = 30;

/// Pure parser for the idle-clear-window env var — takes the raw value
/// directly rather than reading `std::env` itself, so it is unit-testable
/// without mutating process-global state (which would race against every
/// OTHER test in this binary reading the same key; #665 review finding,
/// same rationale as `context_budget::parse_context_budget_bytes`).
pub(super) fn parse_idle_clear_minutes(raw: Option<&str>) -> u64 {
    raw.and_then(|v| v.parse::<u64>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_IDLE_CLEAR_MINUTES)
}

/// Read the configured idle-clear window: env override, else the
/// conservative default. `saturating_mul` avoids an overflow panic (debug)
/// or silent wraparound (release) if an operator sets an absurdly large
/// minute count.
pub(super) fn idle_clear_window() -> Duration {
    let minutes = parse_idle_clear_minutes(
        std::env::var("PRESENTER_AI_IDLE_CLEAR_MINUTES")
            .ok()
            .as_deref(),
    );
    Duration::from_secs(minutes.saturating_mul(60))
}

/// Pure decision function: given when the AI conversation was last touched
/// and "now", should it be cleared before appending the new message?
/// Extracted so a test can inject both timestamps directly (#665 AC6)
/// without needing a real clock or a live handler.
pub(super) fn should_idle_clear(
    last_activity: Option<SystemTime>,
    now: SystemTime,
    idle_window: Duration,
) -> bool {
    match last_activity {
        // Never used yet (fresh conversation) — nothing to clear.
        None => false,
        Some(last) => now.duration_since(last).unwrap_or(Duration::ZERO) > idle_window,
    }
}

/// Translate a `run_agent` failure into the message shown to the operator.
/// `AiAgentError::ContextBudgetExceeded` already carries a friendly message
/// (see its `#[error(...)]`); every other failure is forwarded via its
/// normal `Display` as before. This is the ONE place that used to leak the
/// provider's raw "prompt is too long" text straight to the browser — the
/// typed-error downcast (same pattern as `.claude/rules/repository-error-pattern.md`)
/// ensures that specific case now shows the friendly wording instead (#665).
pub(super) fn friendly_ai_error_message(e: &anyhow::Error) -> String {
    match e.downcast_ref::<AiAgentError>() {
        Some(agent_err) => agent_err.to_string(),
        None => e.to_string(),
    }
}

/// SSE streaming chat endpoint. Sends progress events as tools execute,
/// then a final response event with the assistant's reply.
#[instrument(skip_all)]
pub(super) async fn chat(
    State(state): State<AppState>,
    Json(payload): Json<ChatRequest>,
) -> Result<impl IntoResponse, AppError> {
    if payload.message.trim().is_empty() {
        return Err(AppError::bad_request_message("message cannot be empty"));
    }

    let settings = get_settings_internal(&state).await?;

    // Idle auto-clear (#665): if the shared conversation hasn't been
    // touched in a while, clear it before appending the new message rather
    // than let it silently keep growing toward the next operator's very
    // first question.
    let now = SystemTime::now();
    {
        let mut last_activity = state.ai_last_activity().write().await;
        if should_idle_clear(*last_activity, now, idle_clear_window()) {
            state.ai_conversation().write().await.clear();
        }
        *last_activity = Some(now);
    }

    let mut conversation = {
        let guard = state.ai_conversation().read().await;
        guard.clone()
    };

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ProgressEvent>();

    // Spawn the agent loop in a background task
    let state_clone = state.clone();
    let message = payload.message.clone();
    let agent_handle = tokio::spawn(async move {
        let result = crate::ai::agent::run_agent(
            &message,
            &mut conversation,
            &state_clone,
            &settings,
            Some(tx),
        )
        .await;

        // Store updated conversation back
        {
            let mut guard = state_clone.ai_conversation().write().await;
            *guard = conversation;
        }
        // Re-stamp activity now that the turn actually finished — a long
        // turn (many tool-call iterations) followed by a pause must not be
        // treated as idle from the moment it STARTED; otherwise a turn that
        // took, say, 20 minutes followed by a 15-minute pause would trigger
        // an idle-clear on the very next message despite the conversation
        // having just been used (#665 review finding).
        *state_clone.ai_last_activity().write().await = Some(SystemTime::now());

        result
    });

    // Build SSE stream from the progress channel
    let stream = async_stream::stream! {
        // Yield progress events as they arrive
        while let Some(event) = rx.recv().await {
            let json = serde_json::to_string(&event).unwrap_or_default();
            yield Ok::<_, Infallible>(Event::default().event("progress").data(json));
        }

        // Agent is done — get the final result
        match agent_handle.await {
            Ok(Ok((response, actions))) => {
                let done = serde_json::json!({
                    "type": "response",
                    "response": response,
                    "actions": actions,
                });
                yield Ok(Event::default().event("done").data(done.to_string()));
            }
            Ok(Err(e)) => {
                // #665: translate through friendly_ai_error_message so a
                // context-budget refusal shows its friendly wording instead
                // of a raw provider/internal error string, and log the real
                // error server-side — before this fix an AI failure reached
                // only the browser's SSE stream with nothing in journalctl,
                // which is why the 2026-08-09 outage was undiagnosable.
                error!(error = %e, "AI chat request failed");
                let message = friendly_ai_error_message(&e);
                let err = serde_json::json!({"type": "error", "message": message});
                yield Ok(Event::default().event("error").data(err.to_string()));
            }
            Err(e) => {
                error!(error = %e, "AI chat request task panicked or was cancelled");
                let err = serde_json::json!({"type": "error", "message": e.to_string()});
                yield Ok(Event::default().event("error").data(err.to_string()));
            }
        }
    };

    Ok(Sse::new(stream))
}

// ── Conversation history ──

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ConversationMessage {
    pub role: String,
    pub content: String,
    pub actions: Vec<ToolAction>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ConversationResponse {
    pub messages: Vec<ConversationMessage>,
}

/// Return the current conversation as display-ready messages.
/// Filters out internal tool messages, only returns user + assistant.
#[instrument(skip_all)]
pub(super) async fn get_conversation(
    State(state): State<AppState>,
) -> Result<Json<ConversationResponse>, AppError> {
    let guard = state.ai_conversation().read().await;
    let mut display_messages = Vec::new();

    // Walk through messages, collecting tool actions for assistant messages
    let mut pending_actions: Vec<ToolAction> = Vec::new();

    for msg in guard.iter() {
        match msg.role.as_str() {
            "user" => {
                display_messages.push(ConversationMessage {
                    role: "user".to_string(),
                    content: msg.content.clone().unwrap_or_default(),
                    actions: Vec::new(),
                });
            }
            "assistant" => {
                // If this assistant message has tool_calls, collect action names
                // and wait for the next text-only assistant message
                if msg.tool_calls.is_some() {
                    // Tool call message — actions will be filled from subsequent tool results
                    continue;
                }
                // Text response from assistant — include accumulated actions
                display_messages.push(ConversationMessage {
                    role: "assistant".to_string(),
                    content: msg.content.clone().unwrap_or_default(),
                    actions: std::mem::take(&mut pending_actions),
                });
            }
            "tool" => {
                // Accumulate tool results as actions for the next assistant text.
                // Prefer the persisted preview field (populated in agent.rs at tool
                // execution time). Fall back to extracting from content for legacy
                // messages that were stored before the preview field existed.
                if let Some(ref name) = msg.name {
                    let preview = msg.preview.clone().unwrap_or_else(|| {
                        // Legacy fallback: best-effort extraction from tool result JSON.
                        let Some(content) = msg.content.as_deref() else {
                            return "done".to_string();
                        };
                        let Ok(json) = serde_json::from_str::<serde_json::Value>(content) else {
                            return "done".to_string();
                        };
                        if let Some(err) = json.get("error").and_then(|v| v.as_str()) {
                            return format!("Error: {err}");
                        }
                        if let Some(arr) = json.as_array() {
                            return format!("{} results", arr.len());
                        }
                        "done".to_string()
                    });
                    pending_actions.push(ToolAction {
                        tool: name.clone(),
                        result_preview: preview,
                    });
                }
            }
            _ => {}
        }
    }

    Ok(Json(ConversationResponse {
        messages: display_messages,
    }))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SettingsResponse {
    pub api_url: String,
    pub api_key_set: bool,
    pub model: String,
    pub system_prompt_extra: Option<String>,
}

#[instrument(skip_all)]
pub(super) async fn get_settings(
    State(state): State<AppState>,
) -> Result<Json<SettingsResponse>, AppError> {
    let settings = get_settings_internal(&state).await?;
    Ok(Json(SettingsResponse {
        api_url: settings.api_url,
        api_key_set: settings.api_key.as_ref().is_some_and(|k| !k.is_empty()),
        model: settings.model,
        system_prompt_extra: settings.system_prompt_extra,
    }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UpdateSettingsRequest {
    pub api_url: Option<String>,
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub system_prompt_extra: Option<String>,
}

/// The generic `app_settings` table's actual name — used as the audit
/// trail's `setting_table` label (#661). ai-settings has no dedicated table
/// of its own (unlike ableset/osc/resolume/android/video-source), so this
/// names the REAL underlying table rather than a domain name that doesn't
/// exist in the schema.
const AI_SETTINGS_AUDIT_TABLE: &str = "app_settings";

/// Audit-safe JSON snapshot of `AiSettings` (#661) — `api_key` is NEVER
/// persisted verbatim into the `settings_audit` table. This project's
/// standing "never log token contents" discipline (see `ai/proxy.rs`'s
/// Claude OAuth handling) applies equally to the AI provider API key: the
/// audit trail's whole point is a forensic "who changed what, when", not a
/// second place a credential could leak from. Only WHETHER a key is set is
/// preserved — the same signal `GET /ai/settings`'s `SettingsResponse`
/// already exposes as `api_key_set`, never the raw value.
fn redact_settings_for_audit(settings: &AiSettings) -> serde_json::Value {
    serde_json::json!({
        "apiUrl": settings.api_url,
        "apiKeySet": settings.api_key.as_ref().is_some_and(|k| !k.is_empty()),
        "model": settings.model,
        "systemPromptExtra": settings.system_prompt_extra,
    })
}

#[instrument(skip_all)]
pub(super) async fn update_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<UpdateSettingsRequest>,
) -> Result<StatusCode, AppError> {
    let mut settings = get_settings_internal(&state).await?;
    let before_json = redact_settings_for_audit(&settings);

    if let Some(url) = payload.api_url {
        settings.api_url = url;
    }
    if let Some(key) = payload.api_key {
        settings.api_key = if key.is_empty() { None } else { Some(key) };
    }
    if let Some(model) = payload.model {
        settings.model = model;
    }
    if payload.system_prompt_extra.is_some() {
        settings.system_prompt_extra = payload.system_prompt_extra;
    }

    let json = serde_json::to_string(&settings).map_err(|e| anyhow::anyhow!(e))?;
    state
        .repository()
        .set_app_setting(AI_SETTINGS_KEY, &json)
        .await?;

    // #661: bring ai-settings under the same append-only audit trail every
    // OTHER settings family already has — before this, ai-settings was the
    // ONE family that could never produce a settings_audit row, which is
    // why the undiagnosed 2026-08-02 prod model-id edit left no forensic
    // trail of who/what made it.
    //
    // Deliberately NOT wrapped in the same transaction as the
    // `set_app_setting` write above (unlike the `*_on` variants
    // ableset/osc use) — `set_app_setting` is a generic single-statement
    // upsert with no transactional variant, and adding one purely for this
    // call is out of this ticket's scope (see the design comment). A best-
    // effort audit failure is therefore logged, not propagated: the
    // setting write itself already succeeded by this point, and returning
    // a 500 to the caller here would misleadingly suggest their change was
    // rejected when it was not.
    let actor = extract_actor(&headers);
    let after_json = redact_settings_for_audit(&settings);
    if let Err(err) = state
        .repository()
        .record_settings_audit(
            AI_SETTINGS_AUDIT_TABLE,
            AI_SETTINGS_KEY,
            SettingsAuditSource::HttpSetter,
            &actor,
            Some(before_json),
            after_json,
        )
        .await
    {
        warn!(
            ?err,
            "failed to record ai-settings audit row (the settings write itself already succeeded)"
        );
    }

    Ok(StatusCode::NO_CONTENT)
}

#[instrument(skip_all)]
pub(super) async fn clear_conversation(
    State(state): State<AppState>,
) -> Result<StatusCode, AppError> {
    let mut guard = state.ai_conversation().write().await;
    guard.clear();
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StatusResponse {
    pub connected: bool,
    pub error: Option<String>,
    pub proxy: ProxyStatus,
    /// Whether the CONFIGURED `model` is present in the proxy's own catalog
    /// (#661), surfaced as its OWN field (#675 review finding 2) — a caller
    /// that ANDs it into `connected` (like the deploy workflows used to)
    /// cannot tell "the model is misconfigured" apart from "the Claude OAuth
    /// token is merely stale right now", even though only the former is a
    /// regression a CODE/CONFIG change could have caused. Permissive
    /// default `true` when the catalog itself couldn't be fetched — see
    /// `check_status` below — so a caller gating on this field alone never
    /// mistakes "couldn't check" for "checked and invalid".
    pub model_valid: bool,
}

/// Compute AI `connected` status by ANDing THREE signals: TCP-level
/// connectivity, Claude OAuth validity (#597), and whether the CONFIGURED
/// `model` is actually present in the proxy's own model catalog (#661).
///
/// The connectivity check (`list_models`) pings the
/// proxy's `/models` endpoint — it succeeds whenever the CLIProxyAPI
/// process is running and answering on its port, regardless of whether the
/// underlying Claude OAuth token is still valid OR whether the configured
/// model id is one the proxy actually serves. A real incident
/// (2026-08-02 → 2026-08-06) had `connected: true` for four days with an
/// invalid model id, discovered only when a real chat call 502'd. This
/// function ensures `connected` is `true` ONLY when all three signals are
/// healthy.
///
/// `model_valid` is the caller's job to compute FROM the same `/models`
/// response `connectivity_ok` came from — when connectivity itself failed
/// (no model list to check against), the caller should pass `true` here so
/// this function's result still reflects the CONNECTIVITY failure, not a
/// misleading "model not found".
///
/// Extracted as a pure function so the truth-table is unit-testable without
/// constructing a live ProxyManager + network connectivity.
pub(super) fn compute_ai_connected(
    connectivity_ok: bool,
    claude_authenticated: bool,
    model_valid: bool,
) -> bool {
    connectivity_ok && claude_authenticated && model_valid
}

/// Build the `/ai/status` `error` message (#624, extended #661).
///
/// `check_status` used to discard the real underlying error from
/// `check_connectivity` (removed #661 -- superseded by `list_models`) via
/// `.is_ok()`, replacing it with a constant "AI proxy unreachable" string —
/// which is misleading when the actual failure is an HTTP-level error such
/// as 401 (bad/expired API key) or 500 (proxy-side crash). `connectivity_error`
/// carries that real message (`list_models`'s `anyhow::Error` rendered via
/// `render_connectivity_error`, see below) so the caller can see WHY the
/// proxy is unreachable, not just that it is.
///
/// `model_valid`/`configured_model` (#661): a THIRD branch names the exact
/// invalid model id when connectivity and auth are both fine but the
/// configured model isn't in the proxy's catalog — this is the case that
/// used to sit silently `connected: true` for days.
///
/// Extracted as a pure function so the branch logic is unit-testable without
/// constructing a live ProxyManager + network connectivity (same rationale
/// as `compute_ai_connected` above).
pub(super) fn compute_ai_status_error(
    connected: bool,
    claude_authenticated: bool,
    model_valid: bool,
    configured_model: &str,
    connectivity_error: Option<&str>,
) -> Option<String> {
    if connected {
        None
    } else if !claude_authenticated {
        Some("Claude not authenticated — run /ai/proxy/login to re-authorize".to_string())
    } else if !model_valid {
        Some(format!(
            "Configured AI model '{configured_model}' is not available in the proxy's model catalog — check AI settings"
        ))
    } else {
        Some(match connectivity_error {
            Some(err) => format!("AI proxy unreachable: {err}"),
            None => "AI proxy unreachable".to_string(),
        })
    }
}

/// Render a `list_models` connectivity failure for the `/ai/status` `error` field (#624).
///
/// `list_models` wraps the underlying transport failure (DNS, TLS,
/// timeout, connection refused) in an outer `.context("failed to reach AI
/// API")`. Rendering it with `.to_string()` shows only that outermost
/// context and silently drops the real cause — this uses anyhow's
/// alternate-mode formatting (`{:#}`) to render the full chain instead.
pub(super) fn render_connectivity_error(e: &anyhow::Error) -> String {
    format!("{e:#}")
}

#[instrument(skip_all)]
pub(super) async fn check_status(
    State(state): State<AppState>,
) -> Result<Json<StatusResponse>, AppError> {
    let settings = get_settings_internal(&state).await?;
    let proxy_status = state.ai_proxy().status().await;

    // #661: list_models (not the old bare check_connectivity) so the SAME
    // HTTP round trip that proves the proxy is reachable also gives us the
    // catalog to validate the configured model against — one call, two
    // signals, keeping the 5s-polled status chip's cost unchanged.
    let models_result = crate::ai::client::list_models(&settings).await;
    let connectivity_ok = models_result.is_ok();
    let connectivity_err_msg = models_result.as_ref().err().map(render_connectivity_error);
    // Permissive default (`true`) when the model list itself couldn't be
    // fetched — the CONNECTIVITY branch already covers that case, and a
    // defaulted-false model_valid here would produce a misleading "model
    // not found" message instead of the real connectivity failure.
    let model_valid = models_result
        .as_ref()
        .map(|models| models.iter().any(|id| id == &settings.model))
        .unwrap_or(true);

    let connected = compute_ai_connected(
        connectivity_ok,
        proxy_status.claude_authenticated,
        model_valid,
    );

    let error = compute_ai_status_error(
        connected,
        proxy_status.claude_authenticated,
        model_valid,
        &settings.model,
        connectivity_err_msg.as_deref(),
    );

    Ok(Json(StatusResponse {
        connected,
        error,
        proxy: proxy_status,
        model_valid,
    }))
}

// ── Proxy management ──

#[instrument(skip_all)]
pub(super) async fn proxy_start(
    State(state): State<AppState>,
) -> Result<Json<ProxyStatus>, AppError> {
    state
        .ai_proxy()
        .start()
        .await
        .map_err(|e| AppError::internal(format!("Failed to start proxy: {e}")))?;
    Ok(Json(state.ai_proxy().status().await))
}

#[instrument(skip_all)]
pub(super) async fn proxy_stop(
    State(state): State<AppState>,
) -> Result<Json<ProxyStatus>, AppError> {
    state
        .ai_proxy()
        .stop()
        .await
        .map_err(|e| AppError::internal(format!("Failed to stop proxy: {e}")))?;
    Ok(Json(state.ai_proxy().status().await))
}

// ── Claude OAuth ──

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LoginResponse {
    pub login_url: String,
}

/// Start the Claude login flow. Returns the auth URL for the user to open.
#[instrument(skip_all)]
pub(super) async fn proxy_login(
    State(state): State<AppState>,
) -> Result<Json<LoginResponse>, AppError> {
    let url = state
        .ai_proxy()
        .claude_login()
        .await
        .map_err(|e| AppError::internal(format!("Login failed: {e}")))?;
    Ok(Json(LoginResponse { login_url: url }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CompleteLoginRequest {
    pub callback_url: String,
}

/// Complete the login by forwarding the callback URL to CLIProxyAPI.
#[instrument(skip_all)]
pub(super) async fn proxy_complete_login(
    State(state): State<AppState>,
    Json(payload): Json<CompleteLoginRequest>,
) -> Result<Json<ProxyStatus>, AppError> {
    let url = payload.callback_url.trim();
    if url.is_empty() {
        return Err(AppError::bad_request_message(
            "callback URL cannot be empty",
        ));
    }

    state
        .ai_proxy()
        .complete_login(url)
        .await
        .map_err(|e| AppError::internal(format!("Login completion failed: {e}")))?;

    Ok(Json(state.ai_proxy().status().await))
}

async fn get_settings_internal(state: &AppState) -> anyhow::Result<AiSettings> {
    let mut settings = match state.repository().get_app_setting(AI_SETTINGS_KEY).await? {
        Some(json) => serde_json::from_str(&json)?,
        None => AiSettings::default(),
    };

    // If no custom API URL set and proxy is running, use proxy URL
    if settings.api_url == AiSettings::default().api_url {
        let proxy_status = state.ai_proxy().status().await;
        if proxy_status.running {
            settings.api_url = proxy_status.api_url;
        }
    }

    Ok(settings)
}
