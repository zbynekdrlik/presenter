use super::AiSettings;
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use tracing::error;

/// Shared HTTP client for `/ai/status` connectivity probes ONLY (#622
/// post-merge review finding 3a). `check_connectivity` is polled every 5s by
/// the operator-header status chip (`ai_status.rs`) — building a fresh
/// `reqwest::Client` (own connection pool + TLS setup) on every single poll
/// was needless per-call cost. `call_chat_completions` keeps its OWN client:
/// a chat call is a one-off with a much longer 120s timeout, so reuse would
/// not meaningfully help there.
static CONNECTIVITY_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn connectivity_client() -> &'static reqwest::Client {
    CONNECTIVITY_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(3))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

/// Conservative default response budget when `PRESENTER_AI_MAX_TOKENS` is
/// unset. This bounds the PROVIDER's REPLY size — distinct from (and much
/// smaller than) the request-side context budget in `ai::context_budget` —
/// so a single runaway completion can't itself blow the context window on
/// the next iteration, and a request that would otherwise have no ceiling
/// at all always carries one (#665).
pub(crate) const DEFAULT_MAX_TOKENS: u32 = 8192;

/// Read the configured max-tokens response cap: env override, else the
/// conservative default. Re-read on every call, same rationale as
/// `context_budget::context_budget_bytes`.
pub(crate) fn max_tokens() -> u32 {
    std::env::var("PRESENTER_AI_MAX_TOKENS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_MAX_TOKENS)
}

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
    /// Always sent (never omitted) — an unbounded response is exactly the
    /// kind of thing that can grow the conversation past the context
    /// budget on the next iteration (#665).
    pub max_tokens: u32,
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
        max_tokens: max_tokens(),
    };

    let client = reqwest::Client::new();
    let mut req = client.post(&url).json(&request);

    if let Some(key) = &settings.api_key {
        if !key.is_empty() {
            req = req.header("Authorization", format!("Bearer {key}"));
        }
    }

    // Every provider-side failure below is logged server-side at WARN/ERROR
    // BEFORE the error is returned. Before #665 nothing here logged at all —
    // the raw error reached the SSE stream to the browser and journalctl had
    // zero trace of it, which is why the 2026-08-09 outage was
    // retroactively undiagnosable (a full-month journalctl grep on PP found
    // zero matches for "prompt is too long" / "invalid_request_error" /
    // "context_length" / any 4xx/5xx).
    let response = match req
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            error!(error = %e, url, "AI provider request failed to send (network/timeout)");
            return Err(anyhow::Error::from(e).context("failed to reach AI API"));
        }
    };

    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "no body".to_string());
        error!(
            status = %status,
            body = %body,
            url,
            "AI provider chat completion request failed"
        );
        anyhow::bail!("AI API returned {status}: {body}");
    }

    response
        .json::<ChatCompletionResponse>()
        .await
        .context("failed to parse AI API response")
}

/// Ping the AI API to verify connectivity. Uses the shared, lazily-built
/// `connectivity_client()` (3s timeout) rather than a fresh client per call —
/// this is polled every 5s by the status chip (#622 post-merge review
/// finding 3a).
pub async fn check_connectivity(settings: &AiSettings) -> anyhow::Result<()> {
    let url = format!("{}/models", settings.api_url.trim_end_matches('/'));
    let mut req = connectivity_client().get(&url);

    if let Some(key) = &settings.api_key {
        if !key.is_empty() {
            req = req.header("Authorization", format!("Bearer {key}"));
        }
    }

    let response = req.send().await.context("failed to reach AI API")?;

    if !response.status().is_success() {
        anyhow::bail!("AI API returned status {}", response.status());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // --- AC4: max_tokens is always present on the serialized request ---

    #[test]
    fn chat_completion_request_serializes_a_max_tokens_field() {
        let request = ChatCompletionRequest {
            model: "test-model".to_string(),
            messages: vec![serde_json::json!({"role": "user", "content": "hi"})],
            tools: None,
            tool_choice: None,
            max_tokens: 4321,
        };
        let value = serde_json::to_value(&request).expect("must serialize");
        assert_eq!(
            value.get("max_tokens"),
            Some(&serde_json::json!(4321)),
            "serialized request must carry a max_tokens field: {value}"
        );
    }

    #[test]
    fn max_tokens_env_override_and_default() {
        let key = "PRESENTER_AI_MAX_TOKENS";
        let original = std::env::var(key).ok();

        std::env::remove_var(key);
        assert_eq!(max_tokens(), DEFAULT_MAX_TOKENS);

        std::env::set_var(key, "2048");
        assert_eq!(max_tokens(), 2048);

        std::env::set_var(key, "0");
        assert_eq!(max_tokens(), DEFAULT_MAX_TOKENS);
        std::env::set_var(key, "not-a-number");
        assert_eq!(max_tokens(), DEFAULT_MAX_TOKENS);

        match original {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }

    // --- AC7: a provider-side failure logs a server-side WARN/ERROR line
    // carrying the provider's error text ---

    /// A minimal `tracing::Subscriber` that captures the formatted fields of
    /// every ERROR (or higher-severity) event, so a test can assert a log
    /// line was actually emitted and what it said — without a real log
    /// sink. Same shape as `resolume::backoff_tests::ErrorCounter`, extended
    /// to capture message TEXT (not just a count) since AC7 requires
    /// asserting the provider's error body reached the log line.
    struct CapturedLogs {
        lines: Arc<Mutex<Vec<String>>>,
    }

    struct FieldVisitor(String);
    impl tracing::field::Visit for FieldVisitor {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            self.0.push_str(&format!("{}={:?} ", field.name(), value));
        }
    }

    impl tracing::Subscriber for CapturedLogs {
        fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
            *metadata.level() <= tracing::Level::WARN
        }
        fn new_span(&self, _span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _span: &tracing::span::Id, _values: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _span: &tracing::span::Id, _follows: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            if *event.metadata().level() <= tracing::Level::WARN {
                let mut visitor = FieldVisitor(String::new());
                event.record(&mut visitor);
                self.lines.lock().unwrap().push(visitor.0);
            }
        }
        fn enter(&self, _span: &tracing::span::Id) {}
        fn exit(&self, _span: &tracing::span::Id) {}
    }

    #[tokio::test]
    async fn provider_error_response_is_logged_server_side_with_the_error_body() {
        let lines = Arc::new(Mutex::new(Vec::new()));
        let _guard = tracing::subscriber::set_default(CapturedLogs {
            lines: lines.clone(),
        });

        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "error": {"message": "prompt is too long: 512000 tokens > 200000 maximum"}
            })))
            .mount(&mock_server)
            .await;

        let settings = AiSettings {
            api_url: mock_server.uri(),
            api_key: None,
            model: "test-model".to_string(),
            system_prompt_extra: None,
        };
        let messages = vec![serde_json::json!({"role": "user", "content": "hi"})];

        let result = call_chat_completions(&messages, None, &settings).await;
        assert!(result.is_err(), "a 400 response must propagate as an error");

        let captured = lines.lock().unwrap();
        let joined = captured.join("\n");
        assert!(
            !captured.is_empty(),
            "a provider-side failure must emit at least one WARN/ERROR log line"
        );
        assert!(
            joined.contains("prompt is too long"),
            "the log line must carry the provider's actual error body, got: {joined}"
        );
    }

    #[tokio::test]
    async fn network_failure_is_also_logged_server_side() {
        let lines = Arc::new(Mutex::new(Vec::new()));
        let _guard = tracing::subscriber::set_default(CapturedLogs {
            lines: lines.clone(),
        });

        let settings = AiSettings {
            // Reserved/unbound port — connection refused immediately, no
            // real network access required.
            api_url: "http://127.0.0.1:1".to_string(),
            api_key: None,
            model: "test-model".to_string(),
            system_prompt_extra: None,
        };
        let messages = vec![serde_json::json!({"role": "user", "content": "hi"})];

        let result = call_chat_completions(&messages, None, &settings).await;
        assert!(result.is_err());

        let captured = lines.lock().unwrap();
        assert!(
            !captured.is_empty(),
            "a network-level failure to reach the AI provider must also be logged server-side"
        );
    }
}
