use super::AppError;
use crate::ai::proxy::ProxyStatus;
use crate::ai::{AiSettings, ToolAction, AI_SETTINGS_KEY};
use crate::state::AppState;
use axum::{extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use tracing::instrument;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ChatRequest {
    pub message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ChatResponse {
    pub response: String,
    pub actions: Vec<ToolAction>,
}

#[instrument(skip_all)]
pub(super) async fn chat(
    State(state): State<AppState>,
    Json(payload): Json<ChatRequest>,
) -> Result<Json<ChatResponse>, AppError> {
    if payload.message.trim().is_empty() {
        return Err(AppError::bad_request_message("message cannot be empty"));
    }

    let settings = get_settings_internal(&state).await?;

    let mut conversation = {
        let guard = state.ai_conversation().read().await;
        guard.clone()
    };

    let (response, actions) =
        crate::ai::agent::run_agent(&payload.message, &mut conversation, &state, &settings)
            .await
            .map_err(|e| AppError::internal(format!("AI error: {e}")))?;

    {
        let mut guard = state.ai_conversation().write().await;
        *guard = conversation;
    }

    Ok(Json(ChatResponse { response, actions }))
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
        .await
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
