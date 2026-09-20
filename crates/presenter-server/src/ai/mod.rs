// `agent`, `bible_validator` and `tools` are `pub` (not `pub(crate)`) so the
// #680 `ai_eval` binary (a separate crate root — see `lib.rs`'s doc comment)
// can reuse the REAL `run_agent`/`execute_tool`/packer/validator instead of
// re-implementing them. Everything else here stays crate-private; nothing
// else is needed outside this crate.
pub mod agent;
pub(crate) mod agent_guard;
pub mod bible_validator;
pub(crate) mod client;
pub(crate) mod context_budget;
pub(crate) mod health_cache;
pub(crate) mod last_error;
pub(crate) mod preflight;
pub(crate) mod redact;
pub(crate) mod tool_defs;
pub mod tools;

// Minimal widening for the #680 `ai_eval` binary's constrained-output mode
// (#662 step 2): expose ONLY the options-carrying chat call it needs, keeping
// the rest of `client` crate-private. The returned `ChatCompletionResponse`'s
// fields are all `pub`, so the harness reads them by field access (via type
// inference) without ever naming the type.
pub use client::call_chat_completions_with_options;

#[cfg(test)]
mod agent_budget_tests;
#[cfg(test)]
mod agent_usage_tests;

use serde::{Deserialize, Serialize};

pub(crate) const AI_SETTINGS_KEY: &str = "ai-settings";

/// Default AI provider endpoint used when neither a DB `ai-settings` row nor
/// the `PRESENTER_AI_API_URL` env var is set. Since #762 removed the bundled
/// CLIProxyAPI proxy, the effective `apiUrl` is simply env → DB → this default:
/// OpenRouter (`https://openrouter.ai/api/v1`), the OpenAI-compatible backend
/// deployed instances point at via `/etc/presenter/ai.env` (#761).
pub(crate) const DEFAULT_AI_API_URL: &str = "https://openrouter.ai/api/v1";

/// Hardcoded default AI model used when neither a DB override nor the
/// `PRESENTER_AI_MODEL` env var is set. Since #761 the AI backend is OpenRouter
/// (`https://openrouter.ai/api/v1`), so this is an OpenRouter model slug —
/// `google/gemini-3.8-flash` (owner ROZHODNUTÉ 2026-09-12, verified present on
/// the public `/api/v1/models` catalog). In practice the deployed instances set
/// `PRESENTER_AI_MODEL` via `/etc/presenter/ai.env` from the GH Actions
/// `AI_MODEL` variable, so changing the model in production is a variable edit +
/// redeploy, not a code change; this const is the fallback when that env var is
/// unset. Must be a slug OpenRouter's catalog serves or the post-deploy
/// `modelValid` gate (#661) fails (superseded #437's proxy-only-id rule).
pub(crate) const DEFAULT_AI_MODEL: &str = "google/gemini-3.8-flash";

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
                .unwrap_or_else(|_| DEFAULT_AI_API_URL.to_string()),
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

/// Agent-loop errors that need a specific, friendly translation at the HTTP
/// boundary — unlike most agent failures (network errors, provider 5xx),
/// which are simply forwarded via `anyhow`'s `Display` as before.
///
/// Currently one variant: the context-budget refusal (#665). Its whole
/// point is to NEVER let the provider's raw "prompt is too long" text (or
/// anything resembling it) reach the operator UI — `run_agent` returns this
/// BEFORE ever calling the provider once eviction can no longer bring the
/// conversation under budget, and `router/ai.rs`'s `chat()` handler
/// downcasts on it (the same typed-error-then-map-in-handler pattern used
/// throughout this codebase, see `.claude/rules/repository-error-pattern.md`)
/// to show this message instead of the generic `.to_string()`.
#[derive(Debug, thiserror::Error)]
pub(crate) enum AiAgentError {
    #[error(
        "The conversation grew too large to send, even after trimming older tool results. \
         Click \"Clear\" to reset the AI conversation, then try again."
    )]
    ContextBudgetExceeded,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression for #437 + #761: the hardcoded default AI model must NOT be
    /// the retired `claude-opus-4-20250514` (retired at Anthropic 2026-06-15 →
    /// 404) and — since #761 switched the AI backend to OpenRouter — must be the
    /// OpenRouter slug `google/gemini-3.8-flash` (owner ROZHODNUTÉ 2026-09-12,
    /// verified present on `https://openrouter.ai/api/v1/models`). The previous
    /// pin (`claude-opus-4-6`) was a CLIProxyAPI proxy-only id that OpenRouter's
    /// catalog does not serve, so it would fail the post-deploy `modelValid`
    /// gate against the new backend (#661).
    #[test]
    fn default_model_is_not_retired() {
        assert_ne!(
            DEFAULT_AI_MODEL, "claude-opus-4-20250514",
            "default AI model must not be the retired claude-opus-4-20250514"
        );
        assert_eq!(
            DEFAULT_AI_MODEL, "google/gemini-3.8-flash",
            "default AI model must be the OpenRouter slug google/gemini-3.8-flash (#761)"
        );
    }
}
