use super::AppError;
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

    // Save updated conversation back
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
}

#[instrument(skip_all)]
pub(super) async fn check_status(
    State(state): State<AppState>,
) -> Result<Json<StatusResponse>, AppError> {
    let settings = get_settings_internal(&state).await?;
    match crate::ai::client::check_connectivity(&settings).await {
        Ok(()) => Ok(Json(StatusResponse {
            connected: true,
            error: None,
        })),
        Err(e) => Ok(Json(StatusResponse {
            connected: false,
            error: Some(e.to_string()),
        })),
    }
}

async fn get_settings_internal(state: &AppState) -> anyhow::Result<AiSettings> {
    match state.repository().get_app_setting(AI_SETTINGS_KEY).await? {
        Some(json) => Ok(serde_json::from_str(&json)?),
        None => Ok(AiSettings::default()),
    }
}
