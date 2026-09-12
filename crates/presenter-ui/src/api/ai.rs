use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatRequest {
    pub message: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolAction {
    pub tool: String,
    pub result_preview: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSettingsResponse {
    pub api_url: String,
    pub api_key_set: bool,
    pub model: String,
    pub system_prompt_extra: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAiSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt_extra: Option<String>,
}

/// Permissive fallback for the `#[serde(default = "default_true")]`
/// `model_valid` field so an OLDER server payload (before the field existed)
/// still deserializes cleanly — the #600 lesson.
fn default_true() -> bool {
    true
}

/// Backend-agnostic `/ai/status` response (#762 removed the bundled proxy +
/// Claude OAuth, so there is no `proxy` object and no `requiresClaudeAuth`).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiStatusResponse {
    pub connected: bool,
    pub error: Option<String>,
    /// Whether the CONFIGURED model is present in the backend's model catalog
    /// (#661) — mirrors the server's `StatusResponse::model_valid`.
    #[serde(default = "default_true")]
    pub model_valid: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationMessage {
    pub role: String,
    pub content: String,
    pub actions: Vec<ToolAction>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationResponse {
    pub messages: Vec<ConversationMessage>,
}

/// SSE progress event from the streaming chat endpoint.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ProgressEvent {
    ToolStart {
        tool: String,
    },
    ToolDone {
        tool: String,
        preview: String,
    },
    Response {
        response: String,
        actions: Vec<ToolAction>,
    },
    Error {
        message: String,
    },
}

use super::ApiError;

pub async fn get_settings() -> Result<AiSettingsResponse, ApiError> {
    super::get_json("/ai/settings").await
}

pub async fn update_settings(settings: &UpdateAiSettings) -> Result<(), ApiError> {
    super::put_no_content("/ai/settings", settings).await
}

pub async fn clear_conversation() -> Result<(), ApiError> {
    super::post_no_content("/ai/clear", &serde_json::json!({})).await
}

pub async fn check_status() -> Result<AiStatusResponse, ApiError> {
    super::get_json("/ai/status").await
}

pub async fn get_conversation() -> Result<ConversationResponse, ApiError> {
    super::get_json("/ai/conversation").await
}
