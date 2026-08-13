use super::{AiAgentError, AiSettings};
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use tracing::error;

/// Cap on how much of a provider's error BODY is written to the server log.
/// The real outage this ticket fixes left zero trace in journalctl because
/// nothing logged at all — but an UNBOUNDED body (a proxy returning a large
/// HTML/JSON error page) would just trade that failure for flooding
/// journald instead, so this stays generous but bounded.
const LOGGED_BODY_MAX_CHARS: usize = 2000;

/// Substrings that indicate the PROVIDER itself rejected the request for
/// being too large — as opposed to any other 4xx (bad model name, invalid
/// auth, malformed JSON, etc.). Matched case-insensitively against the raw
/// error body. When one of these fires, the operator sees the SAME friendly
/// "conversation grew too large" wording as the client-side budget refusal
/// (`AiAgentError::ContextBudgetExceeded`) instead of the provider's raw
/// text — belt-and-braces alongside the request-side budget, since the
/// budget is a conservative BYTE estimate (excludes the system prompt and
/// tool schemas) and could in principle still under-shoot the provider's
/// own real token-based limit (#665 review finding).
const CONTEXT_LENGTH_ERROR_MARKERS: &[&str] = &[
    "prompt is too long",
    "context_length",
    "maximum context length",
    "too many tokens",
];

fn looks_like_context_length_error(body: &str) -> bool {
    let lower = body.to_lowercase();
    CONTEXT_LENGTH_ERROR_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
}

/// Shared HTTP client for `/ai/status` connectivity probes ONLY (#622
/// post-merge review finding 3a). `list_models` (the successor to
/// `check_connectivity`, removed #661 once this call also became the model-
/// catalog check) is polled every 5s by the operator-header status chip
/// (`ai_status.rs`) — building a fresh `reqwest::Client` (own connection
/// pool + TLS setup) on every single poll was needless per-call cost.
/// `call_chat_completions` keeps its OWN client: a chat call is a one-off
/// with a much longer 120s timeout, so reuse would not meaningfully help
/// there.
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

/// Pure parser for the max-tokens env var — takes the raw value directly
/// rather than reading `std::env` itself, so it is unit-testable without
/// mutating process-global state (which would race against every OTHER
/// test in this binary reading the same key; #665 review finding, same
/// rationale as `context_budget::parse_context_budget_bytes`).
fn parse_max_tokens(raw: Option<&str>) -> u32 {
    raw.and_then(|v| v.parse::<u32>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_MAX_TOKENS)
}

/// Read the configured max-tokens response cap: env override, else the
/// conservative default. Re-read on every call, same rationale as
/// `context_budget::context_budget_bytes`.
pub(crate) fn max_tokens() -> u32 {
    parse_max_tokens(std::env::var("PRESENTER_AI_MAX_TOKENS").ok().as_deref())
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
    /// Per-call token usage, when the provider included it (#687). Every
    /// OpenAI-compatible candidate this project has actually run against
    /// (llama.cpp, CLIProxyAPI) returns this object on every response, but
    /// the spec permits a provider to omit it entirely — `agent::run_agent`
    /// treats a missing object as "this call didn't say", never as "this
    /// call used zero tokens".
    #[serde(default)]
    pub usage: Option<Usage>,
}

/// Raw per-call token counts as the provider reported them (#687). Every
/// field is independently `Option<u32>` — a provider may report only some
/// of the three counts even when it does include the `usage` object, and a
/// missing count must never be read as `0` (that would misreport "this
/// call used no tokens" as opposed to "the provider didn't say").
/// `agent::run_agent` sums these across every call it makes in one turn
/// into `agent::TokenUsage` — see that type's own doc comment for why the
/// aggregate lives there rather than here (this struct mirrors the wire
/// format exactly and is never itself serialized).
#[derive(Debug, Clone, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub prompt_tokens: Option<u32>,
    #[serde(default)]
    pub completion_tokens: Option<u32>,
    #[serde(default)]
    pub total_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct Choice {
    pub message: ResponseMessage,
    /// The provider's stop reason (`"stop"`, `"length"`, `"tool_calls"`,
    /// ...). Read by `agent::run_agent` into `TurnMetadata::finish_reason`
    /// on every iteration (#662 defect 6) — a `"length"` value means the
    /// response was cut off by the provider's own context/token ceiling,
    /// which used to be silently indistinguishable from a deliberately
    /// short/empty response anywhere downstream.
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ResponseMessage {
    #[allow(dead_code)]
    pub role: String,
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ResponseToolCall>>,
    /// A "thinking"-mode candidate's reasoning trace, when the provider
    /// separates it from `content` (llama.cpp's `--reasoning-format auto`,
    /// confirmed via a direct probe to correctly keep this OUT of
    /// `content` — see #662's reasoning-on rerun comment). Only the LENGTH
    /// is surfaced into the trace (`agent::TurnMetadata::reasoning_content_len`)
    /// — never the full text, which would bloat eval traces for no
    /// diagnostic gain beyond "was there a long reasoning block here".
    #[serde(default)]
    pub reasoning_content: Option<String>,
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

/// OpenAI-compatible `/models` list response (#661). Only the `id` field is
/// used — the proxy's own catalog entries carry more (`object`, `created`,
/// `owned_by`), but this is the ONLY thing `list_models` needs to answer
/// "is the configured model actually served".
#[derive(Debug, Deserialize)]
pub struct ModelsResponse {
    pub data: Vec<ModelEntry>,
}

#[derive(Debug, Deserialize)]
pub struct ModelEntry {
    pub id: String,
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
        // Log the FULL body server-side (this is what makes the next outage
        // diagnosable from journalctl), but cap what actually reaches the
        // log line so a large provider error page can't flood journald.
        let logged_body: String = body.chars().take(LOGGED_BODY_MAX_CHARS).collect();
        error!(
            status = %status,
            body = %logged_body,
            url,
            "AI provider chat completion request failed"
        );
        if looks_like_context_length_error(&body) {
            // The provider ITSELF rejected the request as too large — the
            // request-side budget (a conservative byte estimate that
            // excludes the system prompt/tool schemas) can in principle
            // still under-shoot the provider's real limit. Show the same
            // friendly wording as the client-side refusal rather than the
            // provider's raw text.
            return Err(AiAgentError::ContextBudgetExceeded.into());
        }
        anyhow::bail!("AI API returned {status}: {body}");
    }

    response
        .json::<ChatCompletionResponse>()
        .await
        .context("failed to parse AI API response")
}

/// List the model ids the proxy currently serves (#661). Used both to
/// verify basic connectivity AND to validate the CONFIGURED model actually
/// exists — before this, `/ai/status` only pinged this same endpoint and
/// discarded the body, so an invalid `settings.model` string sat
/// `connected: true` for four days until a real chat call 502'd.
///
/// Uses the shared, lazily-built `connectivity_client()` (3s timeout)
/// rather than a fresh client per call — this is polled every 5s by the
/// status chip (#622 post-merge review finding 3a).
pub async fn list_models(settings: &AiSettings) -> anyhow::Result<Vec<String>> {
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

    let parsed: ModelsResponse = response
        .json()
        .await
        .context("failed to parse AI /models response")?;
    Ok(parsed.data.into_iter().map(|m| m.id).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex, Once};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // --- #661: list_models() parses the proxy's real model catalog ---

    fn test_settings(api_url: &str, model: &str) -> AiSettings {
        AiSettings {
            api_url: api_url.to_string(),
            api_key: None,
            model: model.to_string(),
            system_prompt_extra: None,
        }
    }

    #[tokio::test]
    async fn list_models_parses_ids_from_a_real_looking_models_response() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "object": "list",
                "data": [
                    {"id": "claude-opus-4-6", "object": "model", "owned_by": "anthropic"},
                    {"id": "claude-sonnet-4-6", "object": "model", "owned_by": "anthropic"}
                ]
            })))
            .mount(&mock_server)
            .await;

        let settings = test_settings(&mock_server.uri(), "claude-opus-4-6");
        let models = list_models(&settings).await.expect("must succeed");
        assert_eq!(
            models,
            vec![
                "claude-opus-4-6".to_string(),
                "claude-sonnet-4-6".to_string()
            ]
        );
    }

    #[tokio::test]
    async fn list_models_fails_on_a_non_success_status() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&mock_server)
            .await;

        let settings = test_settings(&mock_server.uri(), "claude-opus-4-6");
        let err = list_models(&settings)
            .await
            .expect_err("a 401 must be an error");
        assert!(err.to_string().contains("401"));
    }

    #[tokio::test]
    async fn list_models_fails_on_a_malformed_body() {
        // A 2xx with a body that doesn't match the expected shape must be
        // an error, not silently treated as "connected" the way the old
        // bare-GET check_connectivity (removed #661 -- superseded by
        // list_models) would have: it discarded the body entirely, so a
        // malformed/incompatible proxy response could never be
        // distinguished from a healthy one.
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&mock_server)
            .await;

        let settings = test_settings(&mock_server.uri(), "claude-opus-4-6");
        list_models(&settings)
            .await
            .expect_err("a malformed /models body must be an error");
    }

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
        // Pure-parser coverage, no env mutation (a mutated env var races
        // against every other test in this binary that reads the same key —
        // #665 review finding).
        assert_eq!(parse_max_tokens(None), DEFAULT_MAX_TOKENS);
        assert_eq!(parse_max_tokens(Some("2048")), 2048);
        assert_eq!(parse_max_tokens(Some("0")), DEFAULT_MAX_TOKENS);
        assert_eq!(parse_max_tokens(Some("not-a-number")), DEFAULT_MAX_TOKENS);
    }

    // --- #687: the response's `usage` object is captured when the provider
    // returns one, and stays `None` (never a fabricated 0) when it doesn't ---

    #[tokio::test]
    async fn call_chat_completions_captures_usage_when_the_provider_returns_it() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{
                    "message": {"role": "assistant", "content": "hi"},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 12, "completion_tokens": 3, "total_tokens": 15}
            })))
            .mount(&mock_server)
            .await;

        let settings = test_settings(&mock_server.uri(), "test-model");
        let messages = vec![serde_json::json!({"role": "user", "content": "hi"})];
        let response = call_chat_completions(&messages, None, &settings)
            .await
            .expect("must succeed");

        let usage = response
            .usage
            .expect("usage must be captured when the provider returns it");
        assert_eq!(usage.prompt_tokens, Some(12));
        assert_eq!(usage.completion_tokens, Some(3));
        assert_eq!(usage.total_tokens, Some(15));
    }

    #[tokio::test]
    async fn call_chat_completions_usage_is_none_when_the_provider_omits_it() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{
                    "message": {"role": "assistant", "content": "hi"},
                    "finish_reason": "stop"
                }]
            })))
            .mount(&mock_server)
            .await;

        let settings = test_settings(&mock_server.uri(), "test-model");
        let messages = vec![serde_json::json!({"role": "user", "content": "hi"})];
        let response = call_chat_completions(&messages, None, &settings)
            .await
            .expect("must succeed");

        assert!(
            response.usage.is_none(),
            "a response with no usage object at all must deserialize to None, not error"
        );
    }

    // --- AC7: a provider-side failure logs a server-side WARN/ERROR line
    // carrying the provider's error text ---

    struct FieldVisitor(String);
    impl tracing::field::Visit for FieldVisitor {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            self.0.push_str(&format!("{}={:?} ", field.name(), value));
        }
    }

    /// Process-lifetime slot for whichever capture test currently holds
    /// `CAPTURE_TEST_LOCK` below. A `Mutex`, not a `thread_local!` — this
    /// used to be a `thread_local!`, which only worked because
    /// `#[tokio::test]`'s default (`current_thread`) flavor happens to pin
    /// an entire test body, including every `.await`, to one dedicated OS
    /// thread. Nothing enforced that: switching either capture test below
    /// to `flavor = "multi_thread"`, or introducing an `.await` whose
    /// continuation resumes on a different worker thread, would have made
    /// captured events silently vanish (the event fires on a thread whose
    /// thread-local was never opted in) with no error — correct today,
    /// silently wrong the moment someone touches test flavor. A process-
    /// wide `Mutex` has no thread affinity: it is checked from whichever
    /// OS thread the event happens to fire on. See `CAPTURE_TEST_LOCK` for
    /// how the two capture tests below are kept from cross-contaminating
    /// this one shared slot.
    static CAPTURE_SINK: Mutex<Option<Arc<Mutex<Vec<String>>>>> = Mutex::new(None);

    /// Serializes the capture tests below against EACH OTHER: only one
    /// `CaptureGuard` may be alive at a time, so only one test's events are
    /// ever routed into `CAPTURE_SINK` at once. `cargo test` runs tests in
    /// parallel OS threads within one process, and `CAPTURE_SINK` above is
    /// now shared process-wide rather than thread-scoped — without this
    /// lock, two concurrently-running capture tests would interleave their
    /// events into the same buffer.
    static CAPTURE_TEST_LOCK: Mutex<()> = Mutex::new(());

    /// Captures every WARN/ERROR-level tracing event emitted anywhere in
    /// the process into `CAPTURE_SINK`, whenever that slot holds a sink —
    /// i.e. for as long as a `CaptureGuard` (below) is alive — so a test
    /// can assert a log line was actually produced and what it said,
    /// without a real log sink.
    ///
    /// Installed ONCE, GLOBALLY (`capture_logs_on_this_thread`, below) —
    /// deliberately NOT via the per-test scoped `subscriber::set_default`
    /// this used to use. Reason: the `error!(...)` call this test exercises
    /// (in `call_chat_completions`'s non-success branch) is ALSO hit by
    /// `provider_context_length_rejection_is_translated_to_the_friendly_error`
    /// and `provider_non_context_length_4xx_stays_a_generic_error`, two
    /// sibling tests that never install ANY capturing subscriber.
    /// `tracing-core` caches each callsite's dispatch `Interest`
    /// (never/sometimes/always) starting from whichever test's thread
    /// reaches it FIRST, so repeatedly constructing a fresh, test-scoped
    /// `Dispatch` (one per `subscriber::set_default` call) made this test's
    /// own capture depend on `cargo test`'s default parallel scheduling
    /// order across siblings — exactly the cross-test race that produced
    /// the CI failure this fixes. A SINGLE, process-lifetime subscriber
    /// sidesteps it: the callsite is registered against this ONE
    /// subscriber exactly once, for good.
    struct GlobalCapture;
    impl tracing::Subscriber for GlobalCapture {
        fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
            *metadata.level() <= tracing::Level::WARN
        }
        fn new_span(&self, _span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _span: &tracing::span::Id, _values: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _span: &tracing::span::Id, _follows: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            if *event.metadata().level() > tracing::Level::WARN {
                return;
            }
            let sink_slot = CAPTURE_SINK
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(sink) = sink_slot.as_ref() {
                let mut visitor = FieldVisitor(String::new());
                event.record(&mut visitor);
                sink.lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(visitor.0);
            }
        }
        fn enter(&self, _span: &tracing::span::Id) {}
        fn exit(&self, _span: &tracing::span::Id) {}
    }

    /// RAII opt-in: while alive, holds `CAPTURE_TEST_LOCK` (so no other
    /// capture test can become active concurrently) and keeps this test's
    /// buffer installed in `CAPTURE_SINK`. Dropping releases BOTH — clears
    /// the sink, then (via the field's own `Drop`) unlocks the test lock —
    /// including when dropped during a panicking test's unwind, so one
    /// panicking capture test can never wedge every later one, and a
    /// later, unrelated test never inherits a stale sink. Both locks are
    /// accessed poison-tolerantly throughout (`unwrap_or_else(|p|
    /// p.into_inner())`) for the same reason: a panic while a lock is held
    /// must not turn into every subsequent capture test failing too.
    struct CaptureGuard {
        _test_lock: std::sync::MutexGuard<'static, ()>,
    }
    impl Drop for CaptureGuard {
        fn drop(&mut self) {
            *CAPTURE_SINK
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        }
    }

    fn capture_logs_on_this_thread() -> (CaptureGuard, Arc<Mutex<Vec<String>>>) {
        static INSTALLED: Once = Once::new();
        INSTALLED.call_once(|| {
            // `Dispatch::new` (not a bare `set_default`) so this
            // registration triggers tracing-core's own callsite-interest
            // rebuild exactly once, then stays alive — and therefore
            // correct — for the rest of the process. See the doc comment
            // on `GlobalCapture` above. `.expect(...)`, not `let _ =`: if
            // some OTHER global default ever gets installed first (a test
            // ordering change, a new test file), silently discarding this
            // `Err` would leave `GlobalCapture` never installed, and BOTH
            // capture tests below would fail on their generic "a
            // provider-side failure must emit at least one WARN/ERROR log
            // line" assertion — a confusing symptom pointing nowhere near
            // the real cause. Fail loudly here instead, naming the cause.
            let dispatch = tracing::dispatcher::Dispatch::new(GlobalCapture);
            tracing::dispatcher::set_global_default(dispatch).expect(
                "GlobalCapture must be the first global tracing dispatcher installed in \
                 this test binary — set_global_default failing here means some other test \
                 already installed a different one first, which would make the ai::client \
                 WARN/ERROR log-capture tests silently see zero events instead of failing \
                 with a clear cause",
            );
        });

        // Serialize against any other currently-active capture test BEFORE
        // touching CAPTURE_SINK, so two capture tests never interleave
        // their events into the same shared buffer.
        let test_lock = CAPTURE_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let sink = Arc::new(Mutex::new(Vec::new()));
        *CAPTURE_SINK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(sink.clone());
        (
            CaptureGuard {
                _test_lock: test_lock,
            },
            sink,
        )
    }

    #[tokio::test]
    async fn provider_error_response_is_logged_server_side_with_the_error_body() {
        let (_capture_guard, lines) = capture_logs_on_this_thread();

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
        let err = result.expect_err("a 400 response must propagate as an error");
        // A "prompt is too long"-shaped rejection is translated to the
        // typed ContextBudgetExceeded error (see the belt-and-braces
        // provider-side detection below) — the raw body must still reach
        // the LOG line even though the returned error is now typed/friendly.
        assert!(
            matches!(
                err.downcast_ref::<AiAgentError>(),
                Some(AiAgentError::ContextBudgetExceeded)
            ),
            "a context-length-shaped provider rejection must be the typed \
             ContextBudgetExceeded error, got: {err:?}"
        );

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

    // --- #665 review finding: the provider can reject a request as too
    // large on its OWN (the request-side byte budget is a conservative
    // estimate and could in principle under-shoot the provider's real
    // limit) — that rejection must ALSO show the friendly wording, not the
    // provider's raw text, but an UNRELATED 4xx must not be swallowed into
    // the same friendly message. ---

    #[tokio::test]
    async fn provider_context_length_rejection_is_translated_to_the_friendly_error() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(400).set_body_json(serde_json::json!({
                "error": {"message": "maximum context length exceeded"}
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
        let err = result.expect_err("a context-length-shaped 4xx must be an error");
        assert!(
            matches!(
                err.downcast_ref::<AiAgentError>(),
                Some(AiAgentError::ContextBudgetExceeded)
            ),
            "must be translated to the typed ContextBudgetExceeded error, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn provider_non_context_length_4xx_stays_a_generic_error() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
                "error": {"message": "invalid api key"}
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
        let err = result.expect_err("an auth failure must still be an error");
        assert!(
            err.downcast_ref::<AiAgentError>().is_none(),
            "an unrelated 4xx must NOT be translated to ContextBudgetExceeded, got: {err:?}"
        );
        assert!(err.to_string().contains("invalid api key"));
    }

    #[tokio::test]
    async fn network_failure_is_also_logged_server_side() {
        let (_capture_guard, lines) = capture_logs_on_this_thread();

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
        let err = result.expect_err("an unreachable provider must be an error");
        assert!(
            err.downcast_ref::<AiAgentError>().is_none(),
            "a network failure must not be mistaken for a context-length rejection, got: {err:?}"
        );

        let captured = lines.lock().unwrap();
        let joined = captured.join("\n");
        assert!(
            !captured.is_empty(),
            "a network-level failure to reach the AI provider must also be logged server-side"
        );
        assert!(
            joined.contains("127.0.0.1:1") || joined.to_lowercase().contains("connect"),
            "the log line must carry the underlying connect failure or target, got: {joined}"
        );
    }
}
