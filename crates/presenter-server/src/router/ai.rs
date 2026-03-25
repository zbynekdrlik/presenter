use super::AppError;
use crate::ai::agent::ProgressEvent;
use crate::ai::proxy::ProxyStatus;
use crate::ai::{AiSettings, ToolAction, AI_SETTINGS_KEY};
use crate::state::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::sse::{Event, Sse};
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use tracing::instrument;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ChatRequest {
    pub message: String,
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
                let err = serde_json::json!({"type": "error", "message": e.to_string()});
                yield Ok(Event::default().event("error").data(err.to_string()));
            }
            Err(e) => {
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
                // Accumulate tool results as actions for the next assistant text
                if let Some(ref name) = msg.name {
                    let preview = msg
                        .content
                        .as_deref()
                        .and_then(|c| {
                            // Try to extract a short preview from the result
                            serde_json::from_str::<serde_json::Value>(c)
                                .ok()
                                .and_then(|v| {
                                    if let Some(arr) = v.as_array() {
                                        Some(format!("{} results", arr.len()))
                                    } else {
                                        v.get("error").map(|err| format!("Error: {err}"))
                                    }
                                })
                        })
                        .unwrap_or_else(|| "done".to_string());
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

#[instrument(skip_all)]
pub(super) async fn update_settings(
    State(state): State<AppState>,
    Json(payload): Json<UpdateSettingsRequest>,
) -> Result<StatusCode, AppError> {
    let mut settings = get_settings_internal(&state).await?;

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
}

#[instrument(skip_all)]
pub(super) async fn check_status(
    State(state): State<AppState>,
) -> Result<Json<StatusResponse>, AppError> {
    let settings = get_settings_internal(&state).await?;
    let proxy_status = state.ai_proxy().status().await;

    let connection = crate::ai::client::check_connectivity(&settings).await;
    match connection {
        Ok(()) => Ok(Json(StatusResponse {
            connected: true,
            error: None,
            proxy: proxy_status,
        })),
        Err(e) => Ok(Json(StatusResponse {
            connected: false,
            error: Some(e.to_string()),
            proxy: proxy_status,
        })),
    }
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

/// Generate the Claude OAuth URL. User opens it, authorizes, gets a code.
#[instrument(skip_all)]
pub(super) async fn proxy_login(
    State(state): State<AppState>,
) -> Result<Json<LoginResponse>, AppError> {
    let url = state
        .ai_proxy()
        .generate_oauth_url()
        .map_err(|e| AppError::internal(format!("Login failed: {e}")))?;
    Ok(Json(LoginResponse { login_url: url }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CompleteLoginRequest {
    pub code: String,
}

/// Exchange the authorization code for a token and store it.
#[instrument(skip_all)]
pub(super) async fn proxy_complete_login(
    State(state): State<AppState>,
    Json(payload): Json<CompleteLoginRequest>,
) -> Result<Json<ProxyStatus>, AppError> {
    let code = payload.code.trim();
    if code.is_empty() {
        return Err(AppError::bad_request_message("code cannot be empty"));
    }

    let token = state
        .ai_proxy()
        .exchange_code(code)
        .await
        .map_err(|e| AppError::internal(format!("Token exchange failed: {e}")))?;

    state
        .ai_proxy()
        .store_token_and_restart(&token)
        .await
        .map_err(|e| AppError::internal(format!("Failed to store token: {e}")))?;

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
