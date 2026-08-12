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
/// itself a conservative OVER-estimate of token count (a token is on
/// average several bytes, so counting raw bytes never UNDER-counts), so
/// this stays safely conservative without needing a `len()/4` fudge factor
/// that could under-count.
pub(crate) const DEFAULT_CONTEXT_BUDGET_BYTES: usize = 300_000;

/// Marker content used to replace an evicted tool result. The message
/// itself (role, `tool_call_id`, `name`) is NEVER removed — only its
/// `content` is replaced — so no `tool_call`/`tool_result` pair is ever
/// orphaned, which would break the OpenAI-compatible API contract (every
/// `tool_calls` entry on an assistant message MUST have a matching
/// `role:"tool"` reply with a matching `tool_call_id`).
pub(crate) const TRUNCATED_STUB: &str = "[result truncated — call the tool again if needed]";

/// Marker used to replace the `arguments` of an already-answered assistant
/// `tool_calls` entry when ITS payload (not just the tool's reply) is what
/// makes an OLD round oversized — e.g. a tool called with a large argument
/// (#665 review finding: the original design only shrank `role:"tool"`
/// messages, so a turn whose SIZE came from the model's own tool-call
/// arguments — rather than from tool results — could never be brought
/// under budget and would hard-refuse instead of completing, failing the
/// ticket's own AC2 ["the turn completes"]). Only the `arguments` string is
/// replaced; `id`/`type`/`name` are untouched, so the pairing with the
/// (already-received) tool result is unaffected — the model already has
/// that answer and does not need the original request payload to keep
/// reasoning about the rest of the turn.
pub(crate) const TRUNCATED_ARGUMENTS: &str = "[arguments truncated]";

/// Pure parser for the context-budget env var — takes the raw value
/// directly (rather than reading `std::env` itself) so it is unit-testable
/// without mutating process-global state, which would race against every
/// OTHER test in this binary that reads the SAME env var concurrently
/// (Rust runs `#[test]`s in parallel by default; #665 review finding).
fn parse_context_budget_bytes(raw: Option<&str>) -> usize {
    raw.and_then(|v| v.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_CONTEXT_BUDGET_BYTES)
}

/// Read the configured context budget: env override, else the conservative
/// default. Re-read on every call (cheap, and lets ops override without a
/// restart-sensitive cache). Thin env-reading wrapper around the pure,
/// directly-testable [`parse_context_budget_bytes`].
pub(crate) fn context_budget_bytes() -> usize {
    parse_context_budget_bytes(
        std::env::var("PRESENTER_AI_CONTEXT_BUDGET_BYTES")
            .ok()
            .as_deref(),
    )
}

/// Cheap size estimate for one message: its `content` string plus its
/// serialized `tool_calls` (if any). This mirrors exactly what
/// `build_api_messages` puts on the wire for that message — the system
/// prompt and the per-request tool schema are deliberately NOT included
/// here, since it is the CONVERSATION that grows unbounded across a turn,
/// not those two. They are NOT literally fixed-size (the system prompt
/// lists every worship library, and the tool schema is tens of KB), but
/// they stay well inside the default budget's margin — `DEFAULT_CONTEXT_BUDGET_BYTES`
/// is a conservative byte count (see its own doc), not a token-exact
/// ceiling, and `client.rs` maps a provider rejection through the same
/// belt-and-braces path if the estimate ever under-shoots in practice.
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

/// Index of the START of the "current round" — the most recently appended
/// assistant `tool_calls` message, and everything from it to the end of the
/// conversation (that message's own tool results, appended right after it).
/// Returns `conversation.len()` (protects nothing) when no `tool_calls`
/// message exists yet, e.g. the very first iteration of a turn.
///
/// This round is NEVER evicted by [`enforce_context_budget`] — it is what
/// the model just asked for or just received THIS iteration. Evicting it
/// would show the model a "truncated" stub for the very call it just made,
/// which makes it re-call the same tool and livelock toward
/// `MAX_ITERATIONS` instead of making progress (#665 review finding).
fn current_round_start(conversation: &[ChatMessage]) -> usize {
    conversation
        .iter()
        .rposition(|m| m.role == "assistant" && m.tool_calls.is_some())
        .unwrap_or(conversation.len())
}

/// Try to shrink ONE message in place (a `role:"tool"` result's content, or
/// an assistant `tool_calls` entry's `arguments`). Returns `true` if it
/// changed. Extracted so [`enforce_context_budget`]'s phase-1 loop stays
/// readable; used for both eviction targets it covers.
fn shrink_message(msg: &mut ChatMessage) -> bool {
    match msg.role.as_str() {
        "tool" => {
            let already_small = msg
                .content
                .as_deref()
                .is_some_and(|c| c.len() <= TRUNCATED_STUB.len());
            // `already_small` alone is the whole guard: a message whose
            // content already equals TRUNCATED_STUB always satisfies
            // `c.len() <= TRUNCATED_STUB.len()` too, so a separate
            // `content != Some(TRUNCATED_STUB)` check would be redundant.
            if already_small {
                false
            } else {
                msg.content = Some(TRUNCATED_STUB.to_string());
                true
            }
        }
        "assistant" => {
            let Some(tool_calls) = msg.tool_calls.as_mut() else {
                return false;
            };
            let mut changed = false;
            for tc in tool_calls.iter_mut() {
                // Same already-small guard as the "tool" branch above: a
                // call with genuinely small arguments (the common case —
                // most tool calls carry an id or a short field, not a huge
                // literal) must be left alone. Without this, replacing
                // already-tiny arguments (e.g. `"{}"`, 2 bytes) with the
                // 22-byte TRUNCATED_ARGUMENTS marker would GROW the
                // message instead of shrinking it.
                let already_small = tc.function.arguments.len() <= TRUNCATED_ARGUMENTS.len();
                if !already_small {
                    tc.function.arguments = TRUNCATED_ARGUMENTS.to_string();
                    changed = true;
                }
            }
            changed
        }
        _ => false,
    }
}

/// Enforce `budget_bytes` on `conversation` IN PLACE. Returns `true` if
/// anything was modified.
///
/// Eviction order:
/// 1. Oldest-first, BEFORE the current round ([`current_round_start`]):
///    shrink whichever of the two possible payloads is oversized — the
///    CONTENT of `role:"tool"` messages, or the `arguments` of an
///    already-answered assistant `tool_calls` entry. The message itself
///    (role, `tool_call_id`/`id`, `name`) is never removed, so no
///    `tool_call`/`tool_result` pair is ever orphaned. The current round is
///    never touched (see [`current_round_start`]).
/// 2. If still over budget, drop whole oldest user TURNS (never a partial
///    turn — the same turn-boundary rule `trim_conversation` uses: a "turn"
///    is a user message plus everything up to the next user message).
///    Stops once only the current turn remains.
///
/// If the conversation is still over budget when this returns — either
/// because the current round alone doesn't fit, or nothing was left to
/// evict — the caller (`run_agent`) refuses to send the request at all
/// rather than risk the provider's own hard rejection.
pub(crate) fn enforce_context_budget(
    conversation: &mut Vec<ChatMessage>,
    budget_bytes: usize,
) -> bool {
    let mut mutated = false;

    // Phase 1. A running total (rather than re-summing the whole
    // conversation on every step) keeps this O(n) instead of O(n²) — this
    // loop runs once per message and each step used to re-serialize every
    // `tool_calls` message in the conversation again (#665 review finding),
    // which matters because it runs on every one of up to 100 iterations,
    // exactly during the over-budget case this code exists to handle.
    let protect_from = current_round_start(conversation);
    let mut sizes: Vec<usize> = conversation.iter().map(estimate_message_bytes).collect();
    let mut total: usize = sizes.iter().sum();

    let mut idx = 0;
    while total > budget_bytes && idx < protect_from {
        if shrink_message(&mut conversation[idx]) {
            let new_size = estimate_message_bytes(&conversation[idx]);
            total = total - sizes[idx] + new_size;
            sizes[idx] = new_size;
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

    fn assistant_tool_call_msg_with_args(id: &str, arguments: &str) -> ChatMessage {
        ChatMessage {
            role: "assistant".to_string(),
            content: None,
            tool_calls: Some(vec![ToolCallMessage {
                id: id.to_string(),
                call_type: "function".to_string(),
                function: ToolCallFunction {
                    name: "get_presentation".to_string(),
                    arguments: arguments.to_string(),
                },
            }]),
            tool_call_id: None,
            name: None,
            preview: None,
        }
    }

    fn assistant_tool_call_msg(id: &str) -> ChatMessage {
        assistant_tool_call_msg_with_args(id, "{}")
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
    /// earlier `tool_calls` entry carrying the same id, AND every
    /// `tool_calls` entry on an assistant message has a matching LATER
    /// `role:"tool"` reply — the invariant, in BOTH directions, that
    /// eviction (stubbing OR turn-dropping) must never break. Checking only
    /// one direction would miss the OpenAI-compatible API contract's other
    /// half: an assistant `tool_calls` entry with no reply is rejected too.
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
            if let Some(ref tool_calls) = msg.tool_calls {
                for tc in tool_calls {
                    let has_reply = conv[idx + 1..]
                        .iter()
                        .any(|m| m.tool_call_id.as_deref() == Some(tc.id.as_str()));
                    assert!(
                        has_reply,
                        "assistant tool_calls entry {:?} has no matching tool reply later in the conversation",
                        tc.id
                    );
                }
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
        // Deliberately smaller than the un-evicted total (~51_000 bytes):
        // only the current (protected) round survives intact, so every
        // OLDER tool result (9 of the 10) must be stubbed to fit.
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

        // Newest tool result (call_9) must be intact, not stubbed — it
        // belongs to the current (protected) round.
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
    fn shrink_message_leaves_an_already_small_tool_result_untouched() {
        // A tool result that is already at or below TRUNCATED_STUB's own
        // length must be left EXACTLY as-is, not replaced by the (longer)
        // stub text — replacing it would grow the conversation instead of
        // shrinking it. Proves the `already_small` guard actually matters,
        // not just that SOME message got left alone by coincidence.
        let mut msg = tool_result_msg("call_0", "ok");
        let changed = shrink_message(&mut msg);
        assert!(
            !changed,
            "an already-tiny tool result must not be rewritten to the longer stub"
        );
        assert_eq!(msg.content.as_deref(), Some("ok"));
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

    // --- #665 review findings: eviction must also reach oversized
    // ASSISTANT tool_calls arguments (not just tool results), and must
    // NEVER touch the current (most recently appended) round ---

    #[test]
    fn enforce_budget_shrinks_oversized_assistant_tool_call_arguments_when_that_is_what_is_big() {
        // The model itself can send a large `arguments` payload (e.g. a
        // tool called with a big literal value) — this is what made a
        // "60 tool calls in one turn" AC2 scenario hard-refuse instead of
        // completing on the original design, which only ever shrank
        // role:"tool" results. Each OLD round here is big because of its
        // tool_calls arguments, not its (tiny) tool result.
        let big_args = format!("{{\"name\":\"{}\"}}", "N".repeat(20_000));
        let mut conv = vec![user_msg("build a large set list")];
        for i in 0..10 {
            conv.push(assistant_tool_call_msg_with_args(
                &format!("call_{i}"),
                &big_args,
            ));
            conv.push(tool_result_msg(&format!("call_{i}"), "ok"));
        }
        // Budget room for roughly the current round + a handful of shrunk
        // older ones — far below the ~200_000-byte un-evicted total.
        let budget = 25_000;

        let mutated = enforce_context_budget(&mut conv, budget);

        assert!(mutated, "eviction must have shrunk the oversized arguments");
        assert!(
            estimate_conversation_bytes(&conv) <= budget,
            "conversation must fit the budget after enforcement"
        );
        assert_no_orphans(&conv);

        // The oldest round's arguments were shrunk...
        let oldest_tc = conv
            .iter()
            .find(|m| m.role == "assistant" && m.tool_calls.is_some())
            .unwrap();
        assert_eq!(
            oldest_tc.tool_calls.as_ref().unwrap()[0].function.arguments,
            TRUNCATED_ARGUMENTS,
            "the oldest assistant tool_calls arguments must be shrunk"
        );

        // ...but the NEWEST round's arguments are untouched (protected).
        let newest_tc = conv
            .iter()
            .rev()
            .find(|m| m.role == "assistant" && m.tool_calls.is_some())
            .unwrap();
        assert_eq!(
            newest_tc.tool_calls.as_ref().unwrap()[0].function.arguments,
            big_args,
            "the current round's own arguments must never be evicted"
        );
    }

    #[test]
    fn enforce_budget_never_touches_the_current_round_even_if_it_alone_exceeds_budget() {
        // A single (protected) round whose own content already exceeds the
        // budget, with nothing older to evict. Must be left over budget
        // rather than stubbing the very call that was just made — the
        // caller (enforce_budget_or_refuse) is responsible for refusing.
        let mut conv = vec![
            user_msg("build a large set list"),
            assistant_tool_call_msg(&"call_0".to_string()),
            tool_result_msg("call_0", &"X".repeat(10_000)),
        ];
        let mutated = enforce_context_budget(&mut conv, 100);

        assert!(
            !mutated,
            "the current round must never be evicted, even under extreme pressure"
        );
        assert_eq!(
            conv[2].content.as_deref(),
            Some("X".repeat(10_000).as_str()),
            "the current round's tool result must remain intact"
        );
        assert!(estimate_conversation_bytes(&conv) > 100);
    }

    // --- context_budget_bytes: pure-parser coverage, no env mutation (a
    // mutated env var races against every other test in this binary that
    // reads the same key — #665 review finding) ---

    #[test]
    fn parse_context_budget_bytes_env_override_and_default() {
        assert_eq!(
            parse_context_budget_bytes(None),
            DEFAULT_CONTEXT_BUDGET_BYTES
        );
        assert_eq!(parse_context_budget_bytes(Some("12345")), 12345);
        // Invalid / zero values fall back to the default rather than
        // disabling the budget entirely.
        assert_eq!(
            parse_context_budget_bytes(Some("not-a-number")),
            DEFAULT_CONTEXT_BUDGET_BYTES
        );
        assert_eq!(
            parse_context_budget_bytes(Some("0")),
            DEFAULT_CONTEXT_BUDGET_BYTES
        );
    }
}
