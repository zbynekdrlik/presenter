//! Regression test for #665: the AI assistant dies with "prompt is too
//! long" during use (a real 2026-08-09 production outage).
//!
//! `run_agent`'s per-turn conversation is rebuilt and resent on EVERY loop
//! iteration (up to `MAX_ITERATIONS = 100`), but nothing bounds its size —
//! a turn that makes a few tool calls, each returning a sizeable result,
//! resends an EVER-GROWING payload on every one of those calls (quadratic
//! growth within a single turn). Eventually the provider rejects the
//! request outright; before this fix, that raw rejection propagated with no
//! server-side log line at all (see `client.rs`), which is why the outage
//! was retroactively undiagnosable.
//!
//! This test proves the growth is unbounded on today's code: two tool-call
//! rounds, each contributing roughly 100KB of tool_calls + tool result to
//! the conversation, should NOT cause the 3rd request's body to balloon to
//! the raw, un-evicted ~400KB total — but on current code, with nothing
//! trimming the conversation mid-turn, it does.

use crate::ai::agent::run_agent;
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

fn tool_call_body(id: &str, name: &str, arguments: &str) -> serde_json::Value {
    serde_json::json!({
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
    })
}

fn text_body(text: &str) -> serde_json::Value {
    serde_json::json!({
        "choices": [{
            "message": {"role": "assistant", "content": text, "tool_calls": null},
            "finish_reason": "stop"
        }]
    })
}

/// Replies with a `create_library` tool call for the first `tool_rounds`
/// requests, then a plain text response — driving `run_agent` through
/// several real tool-call iterations before it finishes.
struct ToolThenText {
    counter: Arc<AtomicUsize>,
    tool_rounds: usize,
    big_name: String,
}

impl Respond for ToolThenText {
    fn respond(&self, _req: &wiremock::Request) -> ResponseTemplate {
        let n = self.counter.fetch_add(1, Ordering::SeqCst);
        if n < self.tool_rounds {
            let args = serde_json::json!({"name": format!("{}-{n}", self.big_name)}).to_string();
            ResponseTemplate::new(200).set_body_json(tool_call_body(
                &format!("call_{n}"),
                "create_library",
                &args,
            ))
        } else {
            ResponseTemplate::new(200).set_body_json(text_body("Created the requested libraries."))
        }
    }
}

#[tokio::test]
async fn run_agent_does_not_resend_an_ever_growing_conversation_within_one_turn() {
    // Each tool-call round's assistant tool_calls + tool result together are
    // ~100KB (a 100_000-char library name echoed back by create_library's
    // own tool result). Two rounds with NOTHING bounding growth would make
    // the 3rd request's body balloon to the raw, un-evicted total (~400KB).
    let mock_server = MockServer::start().await;
    let big_name = "N".repeat(100_000);
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ToolThenText {
            counter: Arc::new(AtomicUsize::new(0)),
            tool_rounds: 2,
            big_name,
        })
        .mount(&mock_server)
        .await;

    let state = AppState::in_memory().await.unwrap();
    let settings = settings_for(&mock_server);
    let mut conversation: Vec<ChatMessage> = Vec::new();

    let result = run_agent(
        "create two libraries for me",
        &mut conversation,
        &state,
        &settings,
        None,
    )
    .await;
    assert!(
        result.is_ok(),
        "the turn should complete successfully once growth is bounded, got: {result:?}"
    );

    let requests = mock_server.received_requests().await.unwrap();
    assert_eq!(
        requests.len(),
        3,
        "expected 2 tool-call round trips + 1 final text response, got {}",
        requests.len()
    );

    // The bug: nothing bounds mid-turn growth, so the 3rd request carries
    // the raw, un-evicted conversation (~400KB — both tool-call rounds'
    // full, un-stubbed content). A bounded implementation must keep this
    // well under that raw total.
    assert!(
        requests[2].body.len() < 250_000,
        "the 3rd request must not carry the raw, un-evicted ~400KB conversation \
         (two tool-call rounds of ~100KB each with nothing trimmed mid-turn); \
         got {} bytes",
        requests[2].body.len()
    );
}
