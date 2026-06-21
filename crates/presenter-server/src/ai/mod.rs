pub(crate) mod agent;
pub(crate) mod bible_validator;
pub(crate) mod client;
pub(crate) mod proxy;
pub(crate) mod tool_defs;
pub(crate) mod tools;

use serde::{Deserialize, Serialize};

pub(crate) const AI_SETTINGS_KEY: &str = "ai-settings";

/// Hardcoded default AI model used when neither a DB override nor the
/// `PRESENTER_AI_MODEL` env var is set. Must be a model the bundled on-device
/// CLIProxyAPI catalog actually serves (see #437).
pub(crate) const DEFAULT_AI_MODEL: &str = "claude-opus-4-20250514";

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
                .unwrap_or_else(|_| DEFAULT_AI_MODEL.to_string()),
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
    /// Human-readable summary of a tool result. Only set on role="tool"
    /// messages. This field is in-memory / internal state only and is
    /// NEVER sent to the LLM. The wire format built in `agent.rs` explicitly
    /// reads only the 5 other fields, so adding fields here cannot leak.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression for #437: the hardcoded default AI model must NOT be the
    /// retired `claude-opus-4-20250514` (retired at Anthropic 2026-06-15 → 404)
    /// and must be `claude-opus-4-6` — the newest Opus the bundled on-device
    /// CLIProxyAPI catalog serves (4-8 is not in the proxy catalog → would 404).
    #[test]
    fn default_model_is_not_retired() {
        assert_ne!(
            DEFAULT_AI_MODEL, "claude-opus-4-20250514",
            "default AI model must not be the retired claude-opus-4-20250514"
        );
        assert_eq!(
            DEFAULT_AI_MODEL, "claude-opus-4-6",
            "default AI model must be claude-opus-4-6"
        );
    }
}
