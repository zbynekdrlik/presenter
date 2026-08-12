//! Integration tests for the AI agent loop's context-budget wiring (#665).
//!
//! `context_budget.rs` and `client.rs` unit-test the pure eviction and
//! serialization functions in isolation. These tests drive the REAL
//! `run_agent` loop end to end (real `AppState`, real tool dispatch, a
//! mocked provider) to prove the wiring in `agent.rs` actually fires where
//! the ticket says it must:
//!
//!   - A turn whose OWN first message already exceeds the budget, with
//!     nothing yet to evict, must be refused BEFORE ever calling the
//!     provider — never let the provider's raw "prompt is too long"
//!     rejection reach the browser.
//!   - `enforce_budget_or_refuse` must run on EVERY iteration of the loop,
//!     not just once at the end of a turn — a turn that makes several tool
//!     calls must not resend an ever-growing conversation on every one of
//!     those calls (the quadratic-within-one-turn growth behind the
//!     2026-08-09 outage).
//!   - The `MAX_ITERATIONS`-exhausted fallback path must trim stale PRIOR
//!     turns before returning, instead of writing an ever-growing
//!     conversation back to global state (`router/ai.rs`) and poisoning the
//!     next operator's turn.

use crate::ai::agent::run_agent;
use crate::ai::context_budget::{DEFAULT_CONTEXT_BUDGET_BYTES, TRUNCATED_STUB};
use crate::ai::{AiAgentError, AiSettings, ChatMessage};
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

fn empty_message() -> ChatMessage {
    ChatMessage {
        role: String::new(),
        content: None,
        tool_calls: None,
        tool_call_id: None,
        name: None,
        preview: None,
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

// --- regression guard (RED before this fix): a turn that makes a couple of
// tool calls must not resend an ever-growing conversation on every one of
// them (quadratic growth within a single turn) — the root cause of the
// 2026-08-09 "prompt is too long" outage. This is the ORIGINAL failing test
// written before any fix code existed; kept as the permanent regression
// guard now that it passes. ---

#[tokio::test]
async fn run_agent_does_not_resend_an_ever_growing_conversation_within_one_turn() {
    // Each tool-call round's assistant tool_calls + tool result together are
    // ~100KB (a 100_000-char library name echoed back by create_library's
    // own tool result). Two rounds with NOTHING bounding growth would make
    // the 3rd request's body balloon to the raw, un-evicted total (~400KB) —
    // this failed on pre-fix code and must stay well under that now.
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
    assert!(
        requests[2].body.len() < 250_000,
        "the 3rd request must not carry the raw, un-evicted ~400KB conversation \
         (two tool-call rounds of ~100KB each with nothing trimmed mid-turn); \
         got {} bytes",
        requests[2].body.len()
    );
}

// --- refusal: a turn whose own first message already exceeds the budget,
// with nothing to evict, must be refused before the provider is ever called ---

#[tokio::test]
async fn run_agent_refuses_before_calling_the_provider_when_the_first_message_alone_is_oversized() {
    // No mocks mounted at all: if run_agent called the provider even once,
    // wiremock's default "no matching mock" response would make `result`
    // an Err for the WRONG reason. The real proof is `received_requests()`
    // being empty, asserted below alongside the typed error.
    let mock_server = MockServer::start().await;
    let state = AppState::in_memory().await.unwrap();
    let settings = settings_for(&mock_server);
    let mut conversation: Vec<ChatMessage> = Vec::new();

    // A single user turn (nothing to stub, and only one turn so nothing to
    // drop) whose own content already exceeds the default budget.
    let huge_message = "Z".repeat(DEFAULT_CONTEXT_BUDGET_BYTES + 50_000);

    let result = run_agent(&huge_message, &mut conversation, &state, &settings, None).await;

    let err = result.expect_err("an unevictable oversized turn must be refused, not sent");
    assert!(
        matches!(
            err.downcast_ref::<AiAgentError>(),
            Some(AiAgentError::ContextBudgetExceeded)
        ),
        "refusal must be the typed AiAgentError::ContextBudgetExceeded, got: {err:?}"
    );

    let requests = mock_server.received_requests().await.unwrap();
    assert!(
        requests.is_empty(),
        "the provider must never be called once eviction cannot fit the turn, got {} requests",
        requests.len()
    );
}

// --- per-iteration enforcement: eviction must fire mid-turn, not only once
// at the end ---

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
async fn run_agent_evicts_oversized_tool_results_mid_turn_not_only_at_the_end() {
    // Two tool-call rounds, each round's assistant tool_calls + tool result
    // together are roughly a third of DEFAULT_CONTEXT_BUDGET_BYTES — the
    // raw, un-evicted total after round 2 exceeds the budget, so eviction
    // MUST fire before the 3rd request is sent. Proven by finding
    // TRUNCATED_STUB in that 3rd request's body, sent by the REAL run_agent
    // loop to its mocked provider — not just a unit test of
    // enforce_context_budget in isolation.
    let mock_server = MockServer::start().await;
    let big_name = "N".repeat(DEFAULT_CONTEXT_BUDGET_BYTES / 3);
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
        "eviction must keep the turn under budget, got: {result:?}"
    );

    let requests = mock_server.received_requests().await.unwrap();
    assert_eq!(
        requests.len(),
        3,
        "expected 2 tool-call round trips + 1 final text response, got {}",
        requests.len()
    );
    let first_two_clean = requests[..2]
        .iter()
        .all(|r| !String::from_utf8_lossy(&r.body).contains(TRUNCATED_STUB));
    assert!(
        first_two_clean,
        "eviction must not fire before the budget is actually exceeded"
    );
    let last_body = String::from_utf8_lossy(&requests[2].body);
    assert!(
        last_body.contains(TRUNCATED_STUB),
        "the 3rd request must show per-iteration eviction already stubbed an older tool result"
    );
}

// --- AC2: many tool calls in one turn — every request stays bounded, and
// growth is not monotonic (eviction actually shrinks later requests) ---

#[tokio::test]
async fn run_agent_keeps_every_request_bounded_across_many_tool_call_iterations() {
    // 30 tool-call rounds, each contributing ~40KB raw (assistant tool_calls
    // + tool result echoing a 20_000-char library name). The raw, un-evicted
    // sum after 30 rounds would be ~1.2MB — nothing like the ~300KB budget.
    // If per-iteration eviction genuinely runs on every one of the 30
    // rounds (not just once at the end), every captured request stays
    // bounded AND at least one later request must be SMALLER than an
    // earlier one (proof growth is not simply monotonic/unbounded).
    let mock_server = MockServer::start().await;
    let big_name = "N".repeat(20_000);
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ToolThenText {
            counter: Arc::new(AtomicUsize::new(0)),
            tool_rounds: 30,
            big_name,
        })
        .mount(&mock_server)
        .await;

    let state = AppState::in_memory().await.unwrap();
    let settings = settings_for(&mock_server);
    let mut conversation: Vec<ChatMessage> = Vec::new();

    let result = run_agent(
        "build a large set list for me",
        &mut conversation,
        &state,
        &settings,
        None,
    )
    .await;
    assert!(
        result.is_ok(),
        "the turn must still complete, got: {result:?}"
    );

    let requests = mock_server.received_requests().await.unwrap();
    assert_eq!(
        requests.len(),
        31,
        "expected 30 tool-call rounds + 1 final text response"
    );

    let sizes: Vec<usize> = requests.iter().map(|r| r.body.len()).collect();
    let peak = sizes.iter().copied().max().unwrap_or(0);
    assert!(
        peak < 600_000,
        "no single request across all 30 iterations may carry the raw, \
         un-evicted ~1.2MB conversation; got a peak request size of {peak} bytes, sizes: {sizes:?}"
    );

    let ever_shrinks = sizes.windows(2).any(|w| w[1] < w[0]);
    assert!(
        ever_shrinks,
        "eviction must make at least one later request SMALLER than an earlier \
         one — a purely monotonic size sequence means nothing is ever being \
         evicted mid-turn; got sizes: {sizes:?}"
    );
}

// --- MAX_ITERATIONS exhausted: stale prior turns must be trimmed, not
// carried forward to poison the next turn ---

/// Always replies with a `list_libraries` tool call — never a text response
/// — so `run_agent` is forced through every one of its `MAX_ITERATIONS`
/// iterations without ever converging.
struct AlwaysToolCall {
    counter: Arc<AtomicUsize>,
}

impl Respond for AlwaysToolCall {
    fn respond(&self, _req: &wiremock::Request) -> ResponseTemplate {
        let n = self.counter.fetch_add(1, Ordering::SeqCst);
        ResponseTemplate::new(200).set_body_json(tool_call_body(
            &format!("call_{n}"),
            "list_libraries",
            "{}",
        ))
    }
}

/// A small completed prior turn: `[user, assistant]`. Content is tiny on
/// purpose — this test is about turn COUNT/identity surviving trim, not
/// byte-budget eviction (which is exercised separately above).
fn seeded_turn(n: usize) -> Vec<ChatMessage> {
    vec![
        ChatMessage {
            role: "user".to_string(),
            content: Some(format!("old question {n}")),
            ..empty_message()
        },
        ChatMessage {
            role: "assistant".to_string(),
            content: Some(format!("old answer {n}")),
            ..empty_message()
        },
    ]
}

#[tokio::test]
async fn run_agent_trims_stale_prior_turns_when_max_iterations_is_exhausted() {
    // Before the fix, the MAX_ITERATIONS-exhausted fallback path returned
    // WITHOUT trimming — router/ai.rs then wrote that oversized conversation
    // straight back to the shared global state, poisoning the NEXT
    // operator's very first message with an already-huge baseline. This
    // seeds 15 completed prior turns, then drives a NEW turn that never
    // returns a text response (the provider always replies with a tool
    // call, forcing all 100 iterations), and proves the stale prior turns
    // are gone afterward rather than compounding into the next turn.
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(AlwaysToolCall {
            counter: Arc::new(AtomicUsize::new(0)),
        })
        .mount(&mock_server)
        .await;

    let state = AppState::in_memory().await.unwrap();
    let settings = settings_for(&mock_server);
    let mut conversation: Vec<ChatMessage> = Vec::new();
    for n in 0..15 {
        conversation.extend(seeded_turn(n));
    }

    let new_message = "keep looking, never stop";
    let result = run_agent(new_message, &mut conversation, &state, &settings, None).await;
    assert!(
        result.is_ok(),
        "hitting MAX_ITERATIONS must still return Ok with the canned message"
    );

    assert!(
        conversation
            .iter()
            .all(|m| m.content.as_deref().is_none_or(|c| !c.starts_with("old "))),
        "all 15 stale prior turns must be trimmed away, not compounded into the next turn's baseline"
    );
    let user_messages: Vec<&ChatMessage> =
        conversation.iter().filter(|m| m.role == "user").collect();
    assert_eq!(
        user_messages.len(),
        1,
        "only the current turn's own user message should remain"
    );
    assert_eq!(user_messages[0].content.as_deref(), Some(new_message));
    assert!(
        conversation.len() < 231,
        "the conversation must actually shrink from the untrimmed 231 messages \
         (15 old turns + the 201-message new turn), got {}",
        conversation.len()
    );
}
