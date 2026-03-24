pub(crate) mod agent;
pub(crate) mod client;
pub(crate) mod tools;

use serde::{Deserialize, Serialize};

pub(crate) const AI_SETTINGS_KEY: &str = "ai-settings";

/// AI configuration settings persisted in app_settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSettings {
    pub api_url: String,
    #[serde(default)]
    pub api_key: Option<String>,
    pub model: String,
    #[serde(default)]
    pub system_prompt_extra: Option<String>,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            api_url: std::env::var("PRESENTER_AI_API_URL")
                .unwrap_or_else(|_| "http://localhost:8787/v1".to_string()),
            api_key: std::env::var("PRESENTER_AI_API_KEY").ok(),
            model: std::env::var("PRESENTER_AI_MODEL")
                .unwrap_or_else(|_| "claude-sonnet-4-20250514".to_string()),
            system_prompt_extra: None,
        }
    }
}

/// A single message in the OpenAI chat format.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub role: String,
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCallMessage>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallMessage {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: ToolCallFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallFunction {
    pub name: String,
    pub arguments: String,
}

/// Summary of a tool execution for the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolAction {
    pub tool: String,
    pub result_preview: String,
}
