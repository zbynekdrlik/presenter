//! Size-budgeted context eviction for the AI agent loop (#665).
//!
//! `run_agent` calls [`enforce_context_budget`] on EVERY iteration of its
//! up-to-`MAX_ITERATIONS` loop, right before building the outgoing request —
//! not just once at the end of a turn. Without this, a single turn that
//! makes many tool calls resends an ever-growing conversation on every one
//! of those calls (quadratic growth within ONE turn), which is how the
//! 2026-08-09 outage happened: the payload crossed the provider's context
//! window mid-turn, with no warning and nothing in the logs.

use super::ChatMessage;

/// Conservative default budget (bytes of message `content` plus serialized
/// `tool_calls`, summed across the conversation) when
/// `PRESENTER_AI_CONTEXT_BUDGET_BYTES` is unset.
///
/// No tokenizer dependency, per the design: a plain UTF-8 byte count is
/// itself a conservative OVER-estimate of token count for the mixed
/// Slovak/English text this app sends (a token is rarely more than ~1 byte
/// on average even with diacritics), so this stays safely conservative
/// without needing a `len()/4` fudge factor that could under-count.
pub(crate) const DEFAULT_CONTEXT_BUDGET_BYTES: usize = 300_000;

/// Marker content used to replace an evicted tool result. The message
/// itself (role, `tool_call_id`, `name`) is NEVER removed — only its
/// `content` is replaced — so no `tool_call`/`tool_result` pair is ever
/// orphaned, which would break the OpenAI-compatible API contract (every
/// `tool_calls` entry on an assistant message MUST have a matching
/// `role:"tool"` reply with a matching `tool_call_id`).
pub(crate) const TRUNCATED_STUB: &str = "[result truncated — call the tool again if needed]";

/// Read the configured context budget: env override, else the conservative
/// default. Re-read on every call (cheap, and lets ops/tests override
/// without a restart-sensitive cache).
pub(crate) fn context_budget_bytes() -> usize {
    std::env::var("PRESENTER_AI_CONTEXT_BUDGET_BYTES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_CONTEXT_BUDGET_BYTES)
}

/// Cheap size estimate for one message: its `content` string plus its
/// serialized `tool_calls` (if any). This mirrors exactly what
/// `build_api_messages` puts on the wire for that message — the system
/// prompt and the per-request tool schema are deliberately NOT included
/// here, since they're small and roughly fixed-size, unlike the
/// conversation, which is what actually grows unbounded.
fn estimate_message_bytes(msg: &ChatMessage) -> usize {
    let content_len = msg.content.as_deref().map(str::len).unwrap_or(0);
    let tool_calls_len = msg
        .tool_calls
        .as_ref()
        .and_then(|tc| serde_json::to_string(tc).ok())
        .map(|s| s.len())
        .unwrap_or(0);
    content_len + tool_calls_len
}

/// Sum of [`estimate_message_bytes`] across the whole conversation.
pub(crate) fn estimate_conversation_bytes(conversation: &[ChatMessage]) -> usize {
    conversation.iter().map(estimate_message_bytes).sum()
}

/// Enforce `budget_bytes` on `conversation` IN PLACE. Returns `true` if
/// anything was modified.
///
/// Eviction order:
/// 1. Stub the CONTENT of the OLDEST `role:"tool"` messages first (oldest to
///    newest) — the message, its `tool_call_id`, and its `name` are always
///    kept, only the payload is replaced — until the estimate fits or every
///    tool result has already been stubbed.
/// 2. If still over budget, drop whole oldest user TURNS (never a partial
///    turn — the same turn-boundary rule `trim_conversation` uses: a "turn"
///    is a user message plus everything up to the next user message), so a
///    tool result is never orphaned from its originating tool call. Stops
///    once only the current turn remains — that turn's own tool results
///    were already exhausted in step 1, so there is nothing left to safely
///    evict without losing the request currently in flight.
///
/// If the conversation is still over budget when this returns, the caller
/// (`run_agent`) refuses to send the request at all rather than risk the
/// provider's own hard rejection.
pub(crate) fn enforce_context_budget(
    conversation: &mut Vec<ChatMessage>,
    budget_bytes: usize,
) -> bool {
    let mut mutated = false;

    // Phase 1: stub oldest-first tool results until the estimate fits, or
    // there is nothing left to stub.
    let mut idx = 0;
    while estimate_conversation_bytes(conversation) > budget_bytes && idx < conversation.len() {
        let msg = &mut conversation[idx];
        let already_small = msg
            .content
            .as_deref()
            .is_some_and(|c| c.len() <= TRUNCATED_STUB.len());
        if msg.role == "tool" && msg.content.as_deref() != Some(TRUNCATED_STUB) && !already_small {
            msg.content = Some(TRUNCATED_STUB.to_string());
            mutated = true;
        }
        idx += 1;
    }

    // Phase 2: still over budget — drop whole oldest user turns.
    while estimate_conversation_bytes(conversation) > budget_bytes {
        let user_positions: Vec<usize> = conversation
            .iter()
            .enumerate()
            .filter(|(_, m)| m.role == "user")
            .map(|(i, _)| i)
            .collect();
        if user_positions.len() < 2 {
            // Only the current turn (or nothing) is left — can't drop more
            // without discarding the request in flight. Stop; the caller
            // decides what to do if still over budget.
            break;
        }
        let next_user = user_positions[1];
        conversation.drain(0..next_user);
        mutated = true;
    }

    mutated
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{ToolCallFunction, ToolCallMessage};

    fn user_msg(content: &str) -> ChatMessage {
        ChatMessage {
            role: "user".to_string(),
            content: Some(content.to_string()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
            preview: None,
        }
    }

    fn assistant_tool_call_msg(id: &str) -> ChatMessage {
        ChatMessage {
            role: "assistant".to_string(),
            content: None,
            tool_calls: Some(vec![ToolCallMessage {
                id: id.to_string(),
                call_type: "function".to_string(),
                function: ToolCallFunction {
                    name: "get_presentation".to_string(),
                    arguments: "{}".to_string(),
                },
            }]),
            tool_call_id: None,
            name: None,
            preview: None,
        }
    }

    fn tool_result_msg(id: &str, content: &str) -> ChatMessage {
        ChatMessage {
            role: "tool".to_string(),
            content: Some(content.to_string()),
            tool_calls: None,
            tool_call_id: Some(id.to_string()),
            name: Some("get_presentation".to_string()),
            preview: Some("done".to_string()),
        }
    }

    /// Every `tool_call_id` in the conversation must be paired with an
    /// earlier `tool_calls` entry carrying the same id — the invariant that
    /// eviction (stubbing OR turn-dropping) must never break.
    fn assert_no_orphans(conv: &[ChatMessage]) {
        for (idx, msg) in conv.iter().enumerate() {
            if let Some(ref tcid) = msg.tool_call_id {
                let has_call = conv[..idx].iter().any(|m| {
                    m.tool_calls
                        .as_ref()
                        .map(|tcs| tcs.iter().any(|t| &t.id == tcid))
                        .unwrap_or(false)
                });
                assert!(
                    has_call,
                    "tool result {tcid:?} has no matching tool_call earlier in the conversation"
                );
            }
        }
    }

    // --- AC1: oversized tool results, budget respected, newest kept, no orphans ---

    #[test]
    fn enforce_budget_stubs_oldest_oversized_tool_results_keeps_newest_intact_no_orphans() {
        let big = "X".repeat(5000);
        let mut conv = vec![user_msg("build me a set list")];
        for i in 0..10 {
            conv.push(assistant_tool_call_msg(&format!("call_{i}")));
            conv.push(tool_result_msg(&format!("call_{i}"), &big));
        }
        // Budget room for roughly the newest 2 results + the small
        // surrounding messages — deliberately smaller than the un-evicted
        // total (~50_000 bytes), so eviction MUST engage.
        let budget = big.len() * 2 + 1000;

        let mutated = enforce_context_budget(&mut conv, budget);

        assert!(
            mutated,
            "eviction must have modified the oversized conversation"
        );
        assert!(
            estimate_conversation_bytes(&conv) <= budget,
            "conversation must fit the budget after enforcement"
        );

        // Newest user message is untouched.
        assert_eq!(conv[0].content.as_deref(), Some("build me a set list"));

        // Newest tool result (call_9) must be intact, not stubbed.
        let newest_tool = conv.iter().rev().find(|m| m.role == "tool").unwrap();
        assert_eq!(
            newest_tool.tool_call_id.as_deref(),
            Some("call_9"),
            "the newest tool result must be the last one in the conversation"
        );
        assert_eq!(
            newest_tool.content.as_deref(),
            Some(big.as_str()),
            "the newest tool result must remain intact"
        );

        // Oldest tool result (call_0) must have been stubbed.
        let oldest_tool = conv.iter().find(|m| m.role == "tool").unwrap();
        assert_eq!(
            oldest_tool.content.as_deref(),
            Some(TRUNCATED_STUB),
            "the oldest tool result must be stubbed"
        );

        assert_no_orphans(&conv);
    }

    #[test]
    fn enforce_budget_is_noop_when_already_under_budget() {
        let mut conv = vec![user_msg("hi"), tool_result_msg("call_0", "small")];
        let before = conv.clone();
        let mutated = enforce_context_budget(&mut conv, 1_000_000);
        assert!(!mutated);
        assert_eq!(conv.len(), before.len());
        assert_eq!(conv[1].content, before[1].content);
    }

    #[test]
    fn enforce_budget_drops_whole_oldest_turns_when_stubbing_alone_is_not_enough() {
        // Each turn has ONE small tool result (nothing to usefully stub —
        // stubbing wouldn't shrink it much), but the USER TEXT itself is
        // large, so only dropping whole turns can bring it under budget.
        let big_user_text = "Y".repeat(4000);
        let mut conv = Vec::new();
        for i in 0..6 {
            conv.push(user_msg(&format!("{big_user_text} turn {i}")));
            conv.push(assistant_tool_call_msg(&format!("call_{i}")));
            conv.push(tool_result_msg(&format!("call_{i}"), "ok"));
        }
        let budget = big_user_text.len() + 200; // room for ~1 turn only

        let mutated = enforce_context_budget(&mut conv, budget);

        assert!(mutated);
        assert!(estimate_conversation_bytes(&conv) <= budget);
        // Only the newest turn should remain (dropping whole turns, never
        // partial ones).
        assert_eq!(conv[0].role, "user");
        assert!(conv[0].content.as_deref().unwrap().ends_with("turn 5"));
        assert_no_orphans(&conv);
    }

    #[test]
    fn enforce_budget_leaves_conversation_over_budget_when_nothing_left_to_evict() {
        // A single turn whose own user message already exceeds the budget —
        // there is nothing to stub (no tool results) and nothing to drop
        // (only one turn). The function must not panic or infinite-loop; it
        // just returns with the conversation still (harmlessly) over
        // budget, and the CALLER is responsible for refusing to send.
        let mut conv = vec![user_msg(&"Z".repeat(10_000))];
        let mutated = enforce_context_budget(&mut conv, 10);
        assert!(
            !mutated,
            "nothing could be evicted, so nothing should change"
        );
        assert!(estimate_conversation_bytes(&conv) > 10);
        assert_eq!(conv.len(), 1);
    }

    #[test]
    fn context_budget_bytes_env_override_and_default() {
        let key = "PRESENTER_AI_CONTEXT_BUDGET_BYTES";
        let original = std::env::var(key).ok();

        std::env::remove_var(key);
        assert_eq!(context_budget_bytes(), DEFAULT_CONTEXT_BUDGET_BYTES);

        std::env::set_var(key, "12345");
        assert_eq!(context_budget_bytes(), 12345);

        // Invalid / zero values fall back to the default rather than
        // disabling the budget entirely.
        std::env::set_var(key, "not-a-number");
        assert_eq!(context_budget_bytes(), DEFAULT_CONTEXT_BUDGET_BYTES);
        std::env::set_var(key, "0");
        assert_eq!(context_budget_bytes(), DEFAULT_CONTEXT_BUDGET_BYTES);

        match original {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }
}
