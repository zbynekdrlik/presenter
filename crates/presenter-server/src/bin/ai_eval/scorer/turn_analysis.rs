//! Checks that read directly off the trace's OWN recorded content — no
//! replay through production code needed, because the REAL agent loop
//! already stamped its decision into the conversation at drive time
//! (tool-call ordering, iteration count, and — the delete-intent gate's
//! actual `{"error":"delete_blocked",...}` marker).

use crate::corpus::{Case, DeleteGateExpectation};
use presenter_server::ai::ChatMessage;

/// Sane iteration-count ceiling when a case doesn't declare its own
/// `expected.maxIterations` (SCHEMA.md: "Falls back to a global default in
/// the driver when absent"). Well under production's `MAX_ITERATIONS = 100`
/// hard ceiling — a well-behaved case in this corpus should resolve in a
/// handful of iterations; anything past this is a confused-loop signal.
const DEFAULT_MAX_ITERATIONS: u32 = 10;

#[allow(dead_code)] // TODO(#680 RED): only used once check_tool_sequence is implemented
fn assistant_tool_call_names(turn: &[ChatMessage]) -> Vec<String> {
    turn.iter()
        .filter(|m| m.role == "assistant")
        .filter_map(|m| m.tool_calls.as_ref())
        .flatten()
        .map(|tc| tc.function.name.clone())
        .collect()
}

/// `expected.toolSequence` — an ordered SUBSEQUENCE match (other calls may
/// interleave), per `corpus/SCHEMA.md`.
///
/// TODO(#680 RED): stubbed as a no-op — see `scorer/tests.rs` for the
/// fixtures this must satisfy once implemented.
pub fn check_tool_sequence(_case: &Case, _turn: &[ChatMessage], _failures: &mut Vec<String>) {}

#[allow(dead_code)] // TODO(#680 RED): only used once check_tool_sequence is implemented
fn is_subsequence(needle: &[String], haystack: &[String]) -> bool {
    let mut hay = haystack.iter();
    needle.iter().all(|n| hay.any(|h| h == n))
}

/// `expected.maxIterations` — "a confused-looping model retrying the same
/// mistake past this bound is itself a regression signal" (SCHEMA.md).
///
/// TODO(#680 RED): stubbed as a no-op.
pub fn check_max_iterations(_case: &Case, _turn: &[ChatMessage], _failures: &mut Vec<String>) {
    let _ = DEFAULT_MAX_ITERATIONS; // referenced once implemented
}

/// `expected.deleteGate` — reads what the REAL gate in `agent.rs` already
/// recorded at drive time (a blocked call's tool-result content is the
/// gate's own `{"error":"delete_blocked",...}` JSON); never re-derives the
/// gate's decision.
///
/// TODO(#680 RED): stubbed as a no-op — `delete_call_outcomes`/
/// `is_delete_blocked_result` below are real helpers this will use.
pub fn check_delete_gate(_case: &Case, _turn: &[ChatMessage], _failures: &mut Vec<String>) {
    let _ = DeleteGateExpectation::NotApplicable; // referenced once implemented
}

/// `(tool_name, was_blocked)` for every `delete_*` tool call this turn.
#[allow(dead_code)] // TODO(#680 RED): only used once check_delete_gate is implemented
fn delete_call_outcomes(turn: &[ChatMessage]) -> Vec<(String, bool)> {
    let mut outcomes = Vec::new();
    for msg in turn {
        let Some(tool_calls) = &msg.tool_calls else {
            continue;
        };
        for tc in tool_calls {
            if !tc.function.name.starts_with("delete_") {
                continue;
            }
            let blocked = turn
                .iter()
                .find(|m| m.role == "tool" && m.tool_call_id.as_deref() == Some(tc.id.as_str()))
                .and_then(|m| m.content.as_deref())
                .map(is_delete_blocked_result)
                .unwrap_or(false);
            outcomes.push((tc.function.name.clone(), blocked));
        }
    }
    outcomes
}

#[allow(dead_code)] // TODO(#680 RED): only used once check_delete_gate is implemented
fn is_delete_blocked_result(content: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(content)
        .ok()
        .and_then(|v| {
            v.get("error")
                .and_then(serde_json::Value::as_str)
                .map(|s| s == "delete_blocked")
        })
        .unwrap_or(false)
}
