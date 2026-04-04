use super::AiSettings;
use anyhow::Context;
use serde::{Deserialize, Serialize};

/// OpenAI-compatible chat completion request.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<String>,
}

/// OpenAI-compatible chat completion response.
#[derive(Debug, Deserialize)]
pub struct ChatCompletionResponse {
    pub choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
pub struct Choice {
    pub message: ResponseMessage,
    #[allow(dead_code)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ResponseMessage {
    #[allow(dead_code)]
    pub role: String,
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ResponseToolCall>>,
}

#[derive(Debug, Deserialize)]
pub struct ResponseToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: ResponseFunction,
}

#[derive(Debug, Deserialize)]
pub struct ResponseFunction {
    pub name: String,
    pub arguments: String,
}

/// Call an OpenAI-compatible chat completions endpoint.
pub async fn call_chat_completions(
    messages: &[serde_json::Value],
    tools: Option<&[serde_json::Value]>,
    settings: &AiSettings,
) -> anyhow::Result<ChatCompletionResponse> {
    let url = format!(
        "{}/chat/completions",
        settings.api_url.trim_end_matches('/')
    );

    let request = ChatCompletionRequest {
        model: settings.model.clone(),
        messages: messages.to_vec(),
        tools: tools.map(|t| t.to_vec()),
        tool_choice: tools.map(|_| "auto".to_string()),
    };

    let client = reqwest::Client::new();
    let mut req = client.post(&url).json(&request);

    if let Some(key) = &settings.api_key {
        if !key.is_empty() {
            req = req.header("Authorization", format!("Bearer {key}"));
        }
    }

    let response = req
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await
        .context("failed to reach AI API")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "no body".to_string());
        anyhow::bail!("AI API returned {status}: {body}");
    }

    response
        .json::<ChatCompletionResponse>()
        .await
        .context("failed to parse AI API response")
}

/// Ping the AI API to verify connectivity.
pub async fn check_connectivity(settings: &AiSettings) -> anyhow::Result<()> {
    let url = format!("{}/models", settings.api_url.trim_end_matches('/'));
    let client = reqwest::Client::new();
    let mut req = client.get(&url);

    if let Some(key) = &settings.api_key {
        if !key.is_empty() {
            req = req.header("Authorization", format!("Bearer {key}"));
        }
    }

    let response = req
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .context("failed to reach AI API")?;

    if !response.status().is_success() {
        anyhow::bail!("AI API returned status {}", response.status());
    }

    Ok(())
}
