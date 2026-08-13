//! Integration tests for `run_agent`'s token-usage capture and aggregation
//! (#687).
//!
//! `client.rs` unit-tests the pure `ChatCompletionResponse` deserialization
//! in isolation (`usage` present / absent on a single response). These
//! tests drive the REAL `run_agent` loop end to end (real `AppState`, real
//! tool dispatch, a mocked provider) to prove the AGGREGATION wiring in
//! `agent.rs` itself: usage summed correctly across a turn that makes more
//! than one LLM call, and a call that omits `usage` altogether just doesn't
//! contribute to the running sum rather than resetting it — same "drive the
//! real loop, not just the pure function" discipline `agent_budget_tests.rs`
//! established for context-budget wiring.

use crate::ai::agent::{run_agent, TokenUsage};
use crate::ai::{AiSettings, ChatMessage};
use crate::state::AppState;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Respond, ResponseTemplate};

fn settings_for(mock_server: &MockServer) -> AiSettings {
    AiSettings {
        api_url: mock_server.uri(),
        api_key: None,
        model: "test-model".to_string(),
        system_prompt_extra: None,
    }
}

fn usage_json(prompt: u32, completion: u32, total: u32) -> serde_json::Value {
    serde_json::json!({
        "prompt_tokens": prompt,
        "completion_tokens": completion,
        "total_tokens": total,
    })
}

fn text_body_with_usage(text: &str, usage: Option<serde_json::Value>) -> serde_json::Value {
    let mut body = serde_json::json!({
        "choices": [{
            "message": {"role": "assistant", "content": text, "tool_calls": null},
            "finish_reason": "stop"
        }]
    });
    if let Some(u) = usage {
        body["usage"] = u;
    }
    body
}

fn tool_call_body_with_usage(
    id: &str,
    name: &str,
    arguments: &str,
    usage: Option<serde_json::Value>,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": id,
                    "type": "function",
                    "function": {"name": name, "arguments": arguments}
                }]
            },
            "finish_reason": "tool_calls"
        }]
    });
    if let Some(u) = usage {
        body["usage"] = u;
    }
    body
}

#[tokio::test]
async fn run_agent_captures_usage_when_the_provider_returns_it() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(text_body_with_usage(
                "hi there",
                Some(usage_json(42, 8, 50)),
            )),
        )
        .mount(&mock_server)
        .await;

    let state = AppState::in_memory().await.unwrap();
    let settings = settings_for(&mock_server);
    let mut conversation: Vec<ChatMessage> = Vec::new();

    let (_, _, _, usage) = run_agent("hello", &mut conversation, &state, &settings, None)
        .await
        .expect("run_agent must succeed");

    assert_eq!(
        usage,
        Some(TokenUsage {
            prompt_tokens: Some(42),
            completion_tokens: Some(8),
            total_tokens: Some(50),
        }),
        "a single-call turn's usage must land verbatim from the provider's response, got {usage:?}"
    );
}

#[tokio::test]
async fn run_agent_usage_is_none_when_the_provider_never_returns_it() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(text_body_with_usage("hi", None)))
        .mount(&mock_server)
        .await;

    let state = AppState::in_memory().await.unwrap();
    let settings = settings_for(&mock_server);
    let mut conversation: Vec<ChatMessage> = Vec::new();

    let (_, _, _, usage) = run_agent("hello", &mut conversation, &state, &settings, None)
        .await
        .expect("run_agent must succeed");

    assert!(
        usage.is_none(),
        "no call in the turn reported usage — the aggregate must stay None, \
         never a fabricated 0: got {usage:?}"
    );
}

/// Two tool-call rounds (the 2nd omits `usage` entirely) then a final text
/// response — proves `run_agent` SUMS usage across every
/// `call_chat_completions` call in the turn, and that a call missing
/// `usage` altogether just doesn't contribute rather than wiping out what
/// was already accumulated.
struct UsageAcrossRounds {
    counter: Arc<AtomicUsize>,
}

impl Respond for UsageAcrossRounds {
    fn respond(&self, _req: &wiremock::Request) -> ResponseTemplate {
        let n = self.counter.fetch_add(1, Ordering::SeqCst);
        let body = match n {
            0 => tool_call_body_with_usage(
                "call_0",
                "create_library",
                r#"{"name":"Library One"}"#,
                Some(usage_json(100, 20, 120)),
            ),
            1 => tool_call_body_with_usage(
                "call_1",
                "create_library",
                r#"{"name":"Library Two"}"#,
                None,
            ),
            _ => text_body_with_usage("Created both libraries.", Some(usage_json(50, 30, 80))),
        };
        ResponseTemplate::new(200).set_body_json(body)
    }
}

#[tokio::test]
async fn run_agent_sums_usage_across_tool_call_iterations() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(UsageAcrossRounds {
            counter: Arc::new(AtomicUsize::new(0)),
        })
        .mount(&mock_server)
        .await;

    let state = AppState::in_memory().await.unwrap();
    let settings = settings_for(&mock_server);
    let mut conversation: Vec<ChatMessage> = Vec::new();

    let (_, _, _, usage) = run_agent(
        "create two libraries for me",
        &mut conversation,
        &state,
        &settings,
        None,
    )
    .await
    .expect("run_agent must succeed");

    assert_eq!(
        usage,
        Some(TokenUsage {
            prompt_tokens: Some(150),
            completion_tokens: Some(50),
            total_tokens: Some(200),
        }),
        "usage must be SUMMED across all 3 calls (round 2's missing usage \
         must contribute nothing, not reset the running total): got {usage:?}"
    );
}
