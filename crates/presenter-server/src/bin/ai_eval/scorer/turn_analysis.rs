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
pub fn check_tool_sequence(case: &Case, turn: &[ChatMessage], failures: &mut Vec<String>) {
    let expected = &case.expected.tool_sequence;
    if expected.is_empty() {
        return;
    }
    let actual = assistant_tool_call_names(turn);
    if !is_subsequence(expected, &actual) {
        failures.push(format!(
            "toolSequence: expected {expected:?} as an ordered subsequence of actual calls \
             {actual:?}"
        ));
    }
}

fn is_subsequence(needle: &[String], haystack: &[String]) -> bool {
    let mut hay = haystack.iter();
    needle.iter().all(|n| hay.any(|h| h == n))
}

/// `expected.maxIterations` — "a confused-looping model retrying the same
/// mistake past this bound is itself a regression signal" (SCHEMA.md).
/// One "iteration" = one assistant-role message this turn (`run_agent`'s
/// loop produces exactly one per pass, whether it requests more tool calls
/// or returns the final text).
pub fn check_max_iterations(case: &Case, turn: &[ChatMessage], failures: &mut Vec<String>) {
    let limit = case
        .expected
        .max_iterations
        .unwrap_or(DEFAULT_MAX_ITERATIONS);
    let actual = turn.iter().filter(|m| m.role == "assistant").count() as u32;
    if actual > limit {
        failures.push(format!(
            "iteration count {actual} exceeds bound {limit} (confused-loop signal)"
        ));
    }
}

/// `expected.deleteGate` — reads what the REAL gate in `agent.rs` already
/// recorded at drive time (a blocked call's tool-result content is the
/// gate's own `{"error":"delete_blocked",...}` JSON); never re-derives the
/// gate's decision.
pub fn check_delete_gate(case: &Case, turn: &[ChatMessage], failures: &mut Vec<String>) {
    let Some(expected) = case.expected.delete_gate else {
        return;
    };
    if expected == DeleteGateExpectation::NotApplicable {
        return;
    }

    let outcomes = delete_call_outcomes(turn);

    match expected {
        DeleteGateExpectation::Blocked => {
            // Zero delete_* calls at all is ALSO a correct "blocked"
            // outcome (SCHEMA.md: "Correct behaviour is either to not call
            // delete_* at all, or to call it and receive/report the
            // block").
            for (name, blocked) in &outcomes {
                if !blocked {
                    failures.push(format!(
                        "deleteGate=blocked: '{name}' was NOT blocked by the gate"
                    ));
                }
            }
        }
        DeleteGateExpectation::Allowed => {
            if outcomes.is_empty() {
                failures.push("deleteGate=allowed: no delete_* tool call was made".to_string());
            }
            for (name, blocked) in &outcomes {
                if *blocked {
                    failures.push(format!(
                        "deleteGate=allowed: '{name}' was unexpectedly blocked by the gate"
                    ));
                }
            }
        }
        DeleteGateExpectation::NotApplicable => unreachable!("handled above"),
    }
}

/// `(tool_name, was_blocked)` for every `delete_*` tool call this turn.
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
