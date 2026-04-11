# AI Mode Cleanup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix five known bugs in the AI chat mode: library type confusion, preview loss on reload, missing delete protection, conversation trim corruption, and system prompt bloat.

**Architecture:** Keep the current agent loop + tool architecture unchanged. Make surgical additions: a `preview` field on `ChatMessage`, a `delete_intent_allowed` gate, a turn-boundary `trim_conversation`, a new `get_style_guide` tool, and a rewritten system prompt. The LLM wire format is unchanged because `agent.rs` manually constructs messages for the API and doesn't use serde on `ChatMessage` for outgoing calls.

**Tech Stack:** Rust (tokio, serde), Claude via CLIProxyAPI (OpenAI-compatible function-calling)

**Spec:** `docs/superpowers/specs/2026-04-11-ai-mode-cleanup-design.md`

---

## Context

The AI chat has five observed bugs, documented in detail in the spec:

1. **Library type confusion** — AI creates bible presentations as worship presentations because the system prompt lists libraries without type info and has no section for bible presentations
2. **Preview loss on reload** — `router/ai.rs:149-164` recomputes preview from tool result JSON on every `GET /ai/conversation`, falling back to literal `"done"` for most tool results
3. **Unprotected destructive operations** — `delete_*` tools execute immediately on model hallucination, causing data loss
4. **Trim drops tool call/result pairs** — `agent.rs:327-333` naively drains oldest N messages, splitting tool_call/tool_result pairs and breaking the OpenAI API contract
5. **System prompt bloat** — 125 lines of Slovak book abbreviations and formatting rules on every request

**Key existing code:**
- `crates/presenter-server/src/ai/mod.rs` — `ChatMessage`, `AiSettings`, `ToolAction` type definitions
- `crates/presenter-server/src/ai/agent.rs` — agent loop, system prompt, trimming
- `crates/presenter-server/src/ai/client.rs` — OpenAI-compatible API client
- `crates/presenter-server/src/ai/tools.rs` — 27 tool definitions and handlers
- `crates/presenter-server/src/router/ai.rs` — HTTP endpoints for chat, conversation, settings
- `crates/presenter-server/src/ai/mod.rs:8` — `AI_SETTINGS_KEY` constant used for persistence

**Important:** `agent.rs:186-201` manually builds LLM messages by reading 5 specific fields from each `ChatMessage` (role, content, tool_calls, tool_call_id, name). New fields added to `ChatMessage` are automatically excluded from the wire format — no `WireMessage` struct is needed, but a comment MUST document this guarantee so future readers don't accidentally switch to `serde_json::to_value(&msg)` and leak UI state.

---

## File Structure

### Modified Files

| File | Change |
|------|--------|
| `crates/presenter-server/src/ai/mod.rs` | Add `preview: Option<String>` field to `ChatMessage` |
| `crates/presenter-server/src/ai/agent.rs` | Add `delete_intent_allowed()` and unit tests, rewrite `trim_conversation`, add delete gate in dispatch loop, populate `preview` on tool-result messages, rewrite `build_system_prompt` |
| `crates/presenter-server/src/router/ai.rs` | Replace preview re-extraction block with `msg.preview.clone()` fallback |
| `crates/presenter-server/src/ai/tools.rs` | Add `get_style_guide` tool definition + handler |
| `Cargo.toml` | Bump version 0.4.16 → 0.4.17 |

### New Files

| File | Purpose |
|------|---------|
| `crates/presenter-server/src/ai/style_guide.md` | Detailed formatting rules loaded on demand via `get_style_guide` tool |

---

## Task 1: Add `preview` field to `ChatMessage`

**Files:**
- Modify: `crates/presenter-server/src/ai/mod.rs:38-47`

- [ ] **Step 1: Add preview field to ChatMessage**

In `crates/presenter-server/src/ai/mod.rs`, modify the `ChatMessage` struct. The current definition is:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub role: String,
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCallMessage>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}
```

Replace with:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub role: String,
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCallMessage>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Human-readable summary of a tool result. Only set on role="tool"
    /// messages. This field is in-memory / internal state only and is
    /// NEVER sent to the LLM. The wire format built in `agent.rs` explicitly
    /// reads only the 5 other fields, so adding fields here cannot leak.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
}
```

- [ ] **Step 2: Verify the struct compiles**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
cargo check -p presenter-server 2>&1 | tail -10
```

Expected: clean build. (Existing callers may show warnings for missing field because `Default` isn't derived, but those are in the next steps.)

- [ ] **Step 3: Fix all ChatMessage construction sites**

Every existing place that constructs `ChatMessage { ... }` needs `preview: None` added. Find them:

```bash
grep -n "ChatMessage {" crates/presenter-server/src/ai/ crates/presenter-server/src/router/ 2>&1
```

Expected callers (all in `agent.rs`):
- `agent.rs:171` — user message construction
- `agent.rs:231` — assistant message with tool calls
- `agent.rs:290` — tool result message
- `agent.rs:305` — assistant text-only response

For each, add `preview: None` (we populate only tool-result messages with actual previews in Task 5). Example for `agent.rs:171`:

```rust
conversation.push(ChatMessage {
    role: "user".to_string(),
    content: Some(user_message.to_string()),
    tool_calls: None,
    tool_call_id: None,
    name: None,
    preview: None,
});
```

Do the same for 231, 290, 305. The tool result at 290 will be updated in Task 5 to actually set preview.

- [ ] **Step 4: Build and test**

```bash
cargo check -p presenter-server 2>&1 | tail -10
cargo test -p presenter-server ai 2>&1 | tail -15
```

Expected: clean build, existing AI tests still pass (currently 10 tests).

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/presenter-server/src/ai/mod.rs crates/presenter-server/src/ai/agent.rs
git commit -m "feat(ai): add preview field to ChatMessage for tool-result display (#231 followup)

Task 1/6 of the AI mode cleanup plan. Adds an optional preview
field to ChatMessage that carries a human-readable summary of a
tool execution result. Only populated on role='tool' messages in
subsequent tasks. Not sent to the LLM: the wire format is built
manually in agent.rs by reading 5 specific fields, so this
internal-only field never leaks."
```

---

## Task 2: Add delete intent gate

**Files:**
- Modify: `crates/presenter-server/src/ai/agent.rs` (add helper + tests + gate integration)

- [ ] **Step 1: Add the `delete_intent_allowed` helper with tests (TDD)**

At the bottom of `crates/presenter-server/src/ai/agent.rs`, add a `#[cfg(test)]` module or extend the existing one if present. First write the failing tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delete_intent_allows_english_keywords() {
        assert!(delete_intent_allowed("delete the presentation"));
        assert!(delete_intent_allowed("please remove old slides"));
        assert!(delete_intent_allowed("discard this"));
        assert!(delete_intent_allowed("destroy the library"));
        assert!(delete_intent_allowed("erase everything"));
    }

    #[test]
    fn delete_intent_allows_slovak_keywords() {
        assert!(delete_intent_allowed("vymaž prezentáciu"));
        assert!(delete_intent_allowed("vymazat prezentaciu")); // no diacritics
        assert!(delete_intent_allowed("odstráň tento slajd"));
        assert!(delete_intent_allowed("odstranit tento slajd"));
        assert!(delete_intent_allowed("zmaž to"));
        assert!(delete_intent_allowed("zmaz to"));
    }

    #[test]
    fn delete_intent_allows_czech_keyword() {
        assert!(delete_intent_allowed("smazat to"));
    }

    #[test]
    fn delete_intent_case_insensitive() {
        assert!(delete_intent_allowed("DELETE THE PRESENTATION"));
        assert!(delete_intent_allowed("Vymazať"));
    }

    #[test]
    fn delete_intent_rejects_non_delete_messages() {
        assert!(!delete_intent_allowed("create a new presentation"));
        assert!(!delete_intent_allowed("clean up old stuff"));
        assert!(!delete_intent_allowed("make a fresh start"));
        assert!(!delete_intent_allowed("can you fix the layout"));
        assert!(!delete_intent_allowed(""));
    }

    #[test]
    fn delete_intent_matches_within_longer_sentence() {
        assert!(delete_intent_allowed("I want to delete the test and make a new one"));
        assert!(delete_intent_allowed("please vymaž everything from yesterday"));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
cargo test -p presenter-server ai::agent::tests::delete_intent 2>&1 | tail -15
```

Expected: compile error — `delete_intent_allowed` not defined yet.

- [ ] **Step 3: Implement `delete_intent_allowed`**

In `crates/presenter-server/src/ai/agent.rs`, add this function near the top (after the `MAX_ITERATIONS` constant and before `build_system_prompt`):

```rust
/// Returns `true` if the user's message contains an explicit intent to delete.
/// Used as a gate on all `delete_*` tool calls to prevent model hallucinations
/// from causing data loss. The model must see a keyword in the user's actual
/// message — it cannot invent the intent on its own.
fn delete_intent_allowed(user_message: &str) -> bool {
    const DELETE_KEYWORDS: &[&str] = &[
        // English
        "delete", "remove", "discard", "destroy", "erase",
        // Slovak (lowercase, matches both with and without diacritics)
        "vymazať",
        "vymazat",
        "vymaz",
        "odstrániť",
        "odstranit",
        "odstran",
        "zmazať",
        "zmazat",
        "zmaz",
        // Czech
        "smazat",
    ];
    let lower = user_message.to_lowercase();
    DELETE_KEYWORDS.iter().any(|kw| lower.contains(kw))
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test -p presenter-server ai::agent::tests::delete_intent 2>&1 | tail -15
```

Expected: all 6 tests pass.

- [ ] **Step 5: Capture original user message at agent-loop entry**

In `crates/presenter-server/src/ai/agent.rs`, modify `run_agent` to capture the original user message once at the top, for use inside the dispatch loop. Find the line near the start of `run_agent`:

```rust
// Add user message to conversation
conversation.push(ChatMessage {
    role: "user".to_string(),
    content: Some(user_message.to_string()),
    tool_calls: None,
    tool_call_id: None,
    name: None,
    preview: None,
});
```

No change to the push itself — but just above or below it, store a local for reuse:

```rust
// Capture the user's original message for the delete-intent gate.
// The gate runs on every delete_* tool call during this turn.
let original_user_message = user_message.to_string();
```

Place the `let original_user_message = ...` line immediately after the `conversation.push` call above.

- [ ] **Step 6: Add the delete gate in the dispatch loop**

In `agent.rs` around line 240 where `for tc in tool_calls` begins, add the gate as the FIRST thing inside the loop body — before the `info!` log and the `progress_tx.send(ToolStart...)`. Here's the complete new loop body prefix:

```rust
// Execute each tool call
for tc in tool_calls {
    // Delete-intent gate: block any delete_* tool unless the user's
    // original message contained an explicit delete keyword. Prevents
    // model hallucinations from causing data loss. See spec
    // docs/superpowers/specs/2026-04-11-ai-mode-cleanup-design.md
    if tc.function.name.starts_with("delete_")
        && !delete_intent_allowed(&original_user_message)
    {
        warn!(
            tool = %tc.function.name,
            "blocked delete tool call — user message did not contain delete intent"
        );
        let error_json = serde_json::json!({
            "error": "delete_blocked",
            "reason": "Delete operations require explicit user intent. The user's message did not contain any delete keywords (delete, remove, vymazať, odstrániť, zmazať, etc.). Ask the user to confirm the deletion explicitly before retrying."
        })
        .to_string();
        let preview = "BLOCKED: delete requires explicit user intent".to_string();

        if let Some(ref tx) = progress_tx {
            let _ = tx.send(ProgressEvent::ToolDone {
                tool: tc.function.name.clone(),
                preview: preview.clone(),
            });
        }
        actions.push(ToolAction {
            tool: tc.function.name.clone(),
            result_preview: preview.clone(),
        });
        conversation.push(ChatMessage {
            role: "tool".to_string(),
            content: Some(error_json),
            tool_calls: None,
            tool_call_id: Some(tc.id.clone()),
            name: Some(tc.function.name.clone()),
            preview: Some(preview),
        });
        continue;
    }

    info!(tool = %tc.function.name, "executing AI tool call");

    // ... rest of existing loop body (progress_tx ToolStart, execute_tool call, etc.)
}
```

The `continue` jumps to the next tool call without executing the delete. The next loop iteration re-enters the LLM with the error in its context, prompting it to ask for user confirmation.

- [ ] **Step 7: Add an integration-style test for the gate**

Add this test to the same `#[cfg(test)] mod tests` block:

```rust
#[test]
fn delete_intent_gate_produces_correct_tool_names() {
    // Verify the gate's prefix match covers all delete_* tools by name.
    // This is a cheap integration-style test that doesn't run the agent,
    // just sanity-checks the prefix logic.
    assert!("delete_presentation".starts_with("delete_"));
    assert!("delete_library".starts_with("delete_"));
    assert!("delete_slide".starts_with("delete_"));
    assert!("delete_bible_presentation".starts_with("delete_"));
    assert!("delete_bible_slide".starts_with("delete_"));

    // Non-delete tools must NOT match
    assert!(!"create_presentation".starts_with("delete_"));
    assert!(!"update_slide".starts_with("delete_"));
    assert!(!"trigger_slide".starts_with("delete_"));
}
```

- [ ] **Step 8: Build and run all AI tests**

```bash
cargo test -p presenter-server ai 2>&1 | tail -20
```

Expected: all tests pass including the 7 new delete gate tests.

- [ ] **Step 9: Commit**

```bash
cargo fmt --all
git add crates/presenter-server/src/ai/agent.rs
git commit -m "feat(ai): add delete intent gate to prevent hallucinated data loss (#231 followup)

Task 2/6. Before dispatching any delete_* tool call, the agent now
checks whether the user's original message contains an explicit
delete keyword (delete, remove, vymazať, odstrániť, zmazať, smazat,
etc.). If not, the call is blocked with a synthetic error that
tells the model to ask the user for confirmation.

The gate catches model hallucinations where a request to 'clean up'
or 'fix' gets interpreted as a delete. Users who actually want to
delete can always say so explicitly.

Covers delete_presentation, delete_library, delete_slide,
delete_bible_presentation, delete_bible_slide, and any future
delete_* tool via prefix match.

7 new unit tests covering English, Slovak, Czech keywords, accent
variants, case insensitivity, and the tool-name prefix match."
```

---

## Task 3: Turn-boundary conversation trimming

**Files:**
- Modify: `crates/presenter-server/src/ai/agent.rs` (rewrite `trim_conversation`, update caller, add tests)

- [ ] **Step 1: Write failing tests for the new trim logic**

Add this test module (or extend the existing `mod tests` block) in `crates/presenter-server/src/ai/agent.rs`:

```rust
#[cfg(test)]
mod trim_tests {
    use super::*;

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

    fn assistant_msg(content: &str) -> ChatMessage {
        ChatMessage {
            role: "assistant".to_string(),
            content: Some(content.to_string()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
            preview: None,
        }
    }

    fn assistant_tool_call(id: &str, name: &str) -> ChatMessage {
        ChatMessage {
            role: "assistant".to_string(),
            content: None,
            tool_calls: Some(vec![ToolCallMessage {
                id: id.to_string(),
                call_type: "function".to_string(),
                function: super::ToolCallFunction {
                    name: name.to_string(),
                    arguments: "{}".to_string(),
                },
            }]),
            tool_call_id: None,
            name: None,
            preview: None,
        }
    }

    fn tool_result(id: &str, name: &str, result: &str) -> ChatMessage {
        ChatMessage {
            role: "tool".to_string(),
            content: Some(result.to_string()),
            tool_calls: None,
            tool_call_id: Some(id.to_string()),
            name: Some(name.to_string()),
            preview: Some("done".to_string()),
        }
    }

    #[test]
    fn trim_preserves_all_turns_when_under_limit() {
        let mut conv = vec![
            user_msg("hi"),
            assistant_msg("hello"),
            user_msg("how are you"),
            assistant_msg("I am fine"),
        ];
        let before_len = conv.len();
        trim_conversation(&mut conv, 10);
        assert_eq!(conv.len(), before_len);
    }

    #[test]
    fn trim_drops_oldest_turns_when_over_limit() {
        // 15 user turns (each followed by an assistant message) = 30 messages
        let mut conv = Vec::new();
        for i in 0..15 {
            conv.push(user_msg(&format!("msg {}", i)));
            conv.push(assistant_msg(&format!("reply {}", i)));
        }
        trim_conversation(&mut conv, 5);
        // Should keep only the last 5 user turns = 10 messages
        assert_eq!(conv.len(), 10);
        // First remaining message should be user "msg 10"
        assert_eq!(conv[0].role, "user");
        assert_eq!(conv[0].content.as_deref(), Some("msg 10"));
        // Last message should be assistant "reply 14"
        assert_eq!(conv[conv.len() - 1].role, "assistant");
        assert_eq!(conv[conv.len() - 1].content.as_deref(), Some("reply 14"));
    }

    #[test]
    fn trim_never_orphans_tool_result() {
        // Build a conversation where each turn has a tool call + result
        // user1 → assistant_tool_call(call_1) → tool_result(call_1) → assistant_text
        // user2 → assistant_tool_call(call_2) → tool_result(call_2) → assistant_text
        // ... 12 turns total
        let mut conv = Vec::new();
        for i in 0..12 {
            conv.push(user_msg(&format!("user {}", i)));
            conv.push(assistant_tool_call(&format!("call_{}", i), "test_tool"));
            conv.push(tool_result(&format!("call_{}", i), "test_tool", "{}"));
            conv.push(assistant_msg(&format!("final {}", i)));
        }
        assert_eq!(conv.len(), 48); // 12 turns × 4 messages

        trim_conversation(&mut conv, 5);

        // Should keep exactly 5 turns = 20 messages
        assert_eq!(conv.len(), 20);
        // First remaining must be a user message (turn boundary)
        assert_eq!(conv[0].role, "user");
        // Every tool_call_id in the remaining conversation must have a matching
        // assistant tool_calls entry earlier in the slice
        for (idx, msg) in conv.iter().enumerate() {
            if let Some(ref tcid) = msg.tool_call_id {
                let earlier_has_call = conv[..idx].iter().any(|m| {
                    m.tool_calls
                        .as_ref()
                        .map(|tcs| tcs.iter().any(|t| &t.id == tcid))
                        .unwrap_or(false)
                });
                assert!(
                    earlier_has_call,
                    "tool result with id {tcid:?} has no matching tool_call earlier in conversation"
                );
            }
        }
    }

    #[test]
    fn trim_enforces_hard_ceiling_of_200_messages() {
        // Build a conversation with only 5 user turns but each turn has
        // 50 tool calls (= ~250 messages total)
        let mut conv = Vec::new();
        for turn in 0..5 {
            conv.push(user_msg(&format!("user {}", turn)));
            for call in 0..50 {
                let id = format!("turn_{}_call_{}", turn, call);
                conv.push(assistant_tool_call(&id, "test_tool"));
                conv.push(tool_result(&id, "test_tool", "{}"));
            }
            conv.push(assistant_msg(&format!("final {}", turn)));
        }
        assert!(conv.len() > 200);

        trim_conversation(&mut conv, 10); // max_turns won't trigger (only 5 turns)

        // Hard ceiling must reduce it to <= 200
        assert!(
            conv.len() <= 200,
            "expected <= 200 messages after trim, got {}",
            conv.len()
        );
        // First remaining must still be a user message (boundary preserved)
        assert_eq!(conv[0].role, "user");
    }

    #[test]
    fn trim_empty_conversation_is_noop() {
        let mut conv: Vec<ChatMessage> = Vec::new();
        trim_conversation(&mut conv, 10);
        assert_eq!(conv.len(), 0);
    }

    #[test]
    fn trim_single_user_message_is_noop() {
        let mut conv = vec![user_msg("first and only")];
        trim_conversation(&mut conv, 10);
        assert_eq!(conv.len(), 1);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test -p presenter-server ai::agent::trim_tests 2>&1 | tail -25
```

Expected: compile errors or failures because the current `trim_conversation` has a different signature (takes no `max_turns` parameter).

- [ ] **Step 3: Replace `trim_conversation` with the new version**

In `crates/presenter-server/src/ai/agent.rs`, find the current trim at the bottom of the file:

```rust
/// Keep conversation at a manageable size by trimming old messages.
fn trim_conversation(conversation: &mut Vec<ChatMessage>) {
    const MAX_MESSAGES: usize = 40;
    if conversation.len() > MAX_MESSAGES {
        let to_remove = conversation.len() - MAX_MESSAGES;
        conversation.drain(..to_remove);
    }
}
```

Replace with:

```rust
/// Trim the conversation to at most `max_turns` user turns, preserving
/// tool_call/result pairs. A "turn" is defined as a user message and all
/// subsequent assistant/tool messages up to the next user message. Trimming
/// at turn boundaries guarantees that no tool result is ever orphaned from
/// its originating tool_call, which would break the OpenAI API contract.
///
/// Also enforces a hard ceiling of 200 total messages to prevent unbounded
/// growth within a small number of turns (e.g., a session where the model
/// makes 50 tool calls per turn). When the ceiling is hit, the oldest user
/// turn is dropped repeatedly until the conversation fits.
fn trim_conversation(conversation: &mut Vec<ChatMessage>, max_turns: usize) {
    const HARD_CEILING: usize = 200;

    // Turn-based trimming: find the (max_turns)-th-from-the-end user message
    // and drop everything before it.
    let user_positions: Vec<usize> = conversation
        .iter()
        .enumerate()
        .filter(|(_, m)| m.role == "user")
        .map(|(i, _)| i)
        .collect();

    if user_positions.len() > max_turns {
        let drop_before = user_positions[user_positions.len() - max_turns];
        if drop_before > 0 {
            conversation.drain(0..drop_before);
        }
    }

    // Hard ceiling: even after turn trim, drop oldest user turns until
    // conversation fits under HARD_CEILING messages. We always drop
    // complete turns (to the next user message), never part of a turn.
    while conversation.len() > HARD_CEILING {
        let user_positions: Vec<usize> = conversation
            .iter()
            .enumerate()
            .filter(|(_, m)| m.role == "user")
            .map(|(i, _)| i)
            .collect();
        if user_positions.len() < 2 {
            // Only one user message left; can't drop more without losing it.
            break;
        }
        let next_user = user_positions[1];
        conversation.drain(0..next_user);
    }
}
```

- [ ] **Step 4: Update the caller in `run_agent`**

Find the single call site in `run_agent` (around line 314):

```rust
// Trim conversation to max ~20 user/assistant pairs (keep system working)
trim_conversation(conversation);
```

Replace with:

```rust
// Trim conversation to last 10 user turns, preserving tool_call/result
// pairs. A "turn" is user msg + subsequent assistant/tool messages.
trim_conversation(conversation, 10);
```

- [ ] **Step 5: Run all AI tests**

```bash
cargo test -p presenter-server ai 2>&1 | tail -20
```

Expected: all 7+ existing tests plus 6 new trim tests pass, 0 failures.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/presenter-server/src/ai/agent.rs
git commit -m "fix(ai): trim conversation at turn boundaries to preserve tool pairs (#231 followup)

Task 3/6. The old trim_conversation drained the oldest N messages
naively, which could split an assistant tool_calls message from
its matching tool result. The next LLM request would then reference
a tool_call_id that no longer existed in the conversation, breaking
the OpenAI API contract.

New version trims at user-message boundaries: it finds the Nth-
from-end user message and drops everything before it. Tool results
are always either fully included with their originating tool_call
or fully excluded together.

Also adds a hard ceiling of 200 total messages as a safety net
for sessions with tool-call-dense turns (e.g., 50 tool calls in
one turn). When hit, oldest user turns are dropped one at a time.

6 new unit tests: preserve-under-limit, drop-over-limit, no-orphan-
tool-results, hard-ceiling, empty-conversation, single-message."
```

---

## Task 4: Create style_guide.md + `get_style_guide` tool

**Files:**
- Create: `crates/presenter-server/src/ai/style_guide.md`
- Modify: `crates/presenter-server/src/ai/tools.rs` (tool definition + handler)

- [ ] **Step 1: Create style_guide.md**

Create `crates/presenter-server/src/ai/style_guide.md` with this exact content. It contains the detailed formatting rules that were pulled out of the system prompt:

```markdown
# AI Presentation Style Guide

This reference is for detailed formatting questions. The main system prompt
covers essentials; load this on demand when you need specifics.

## Slide field usage (Bible presentations)

- `main`: Verse text with leading verse number. Format: "1. Verse text here" or "27 Verse text here". NEVER include the reference in main.
- `main_reference`: Reference WITH translation code. Example: "Žalm 26:1 (ROH)". ALWAYS include the code in parentheses.
- `secondary`: Leave empty unless bilingual.
- `secondary_reference`: Secondary translation reference if bilingual.

## Reference format (mandatory — never omit the translation code)

- Single verse: "Žalm 26:1 (ROH)"
- Verse range: "Marek 3:14-15 (SEB)"
- Partial verse: "Žalm 26:3a (ROH)"

The code in parentheses is REQUIRED. Without it, Resolume cannot display the reference correctly.

## Multi-slide passages (critical)

When a Bible passage is split across multiple slides, ALL slides from that passage MUST use the SAME full reference — the complete verse range from start to end.

Example: Psalm 52:1-11 split into 4 slides:
- Slide 1 (vv 1-3): main_reference = "Žalm 52:1-11 (ROH)" ← FULL range, not "52:1-3"
- Slide 2 (vv 4-6): main_reference = "Žalm 52:1-11 (ROH)" ← same
- Slide 3 (vv 7-9): main_reference = "Žalm 52:1-11 (ROH)" ← same
- Slide 4 (vv 10-11): main_reference = "Žalm 52:1-11 (ROH)" ← same

WRONG: Using per-slide ranges like "Žalm 52:1-3", "Žalm 52:4-6" — this makes each slide look like a separate passage.

## Markdown markers (## for bold from email)

The pastor bolds text in emails. Bold text arrives wrapped in ## markers. Handle them by context:

1. **##reference## (e.g. ##Mt26:26-29##, ##Rim5:17##):** Bold section header — the pastor bolds references for readability. Do NOT create a slide for it. Use it to identify which Bible passage follows.
2. **##title## at the very start (e.g. ##Nová zmluva##):** Presentation title. Use as the presentation name.
3. **##word## inside a verse (e.g. "aby sme ##verili## menu"):** Emphasized word. Make that word UPPERCASE within the verse slide's main text. Do NOT create a separate emphasis slide.
4. **##phrase## as a standalone line (not a reference, not inside a verse):** Create a separate emphasis slide with main = phrase in UPPERCASE.

Do NOT create separate emphasis slides for bold references or bold words inside verses. Only standalone bold phrases that are not Bible references get their own emphasis slide.

## Slide size rules

Character limit per slide is provided in the live system prompt. Pack multiple verses onto one slide: keep adding verses until the next verse would exceed the limit, then start a new slide.

Example with limit 200:
- Verse 1: 70 chars → slide has 70, room for more
- Verse 2: 40 chars → slide has 110, room for more
- Verse 3: 80 chars → slide has 190, tight
- Verse 4: 50 chars → 240 total, start new slide

Result: slide 1 has verses 1-3, slide 2 has verse 4.

If a single verse exceeds the limit, split it at a natural sentence boundary.

## Slovak Bible book abbreviations

Common mappings:
- Ž / Žalm → Žalmy
- Žid → Židom
- 1Sa → 1. Samuelova
- 1Kra → 1. Kráľov
- 2Ti → 2. Timotejovi
- Mat / Mt → Matúš
- Mar / Mr → Marek
- Luk → Lukáš
- Ján / Jan → Ján
- Sk → Skutky
- Rim → Rimanom
- 1Kor → 1. Korinťanom
- 2Kor → 2. Korinťanom
- Gal → Galatským
- Ef → Efezským
- Fil → Filipským
- Kol → Kolosanom
- 1Sol → 1. Solúnčanom
- 1Tim → 1. Timotejovi
- Tít → Títovi
- Flm → Filemonovi
- Prísl → Príslovia
- Iz → Izaiáš
- Jer → Jeremiáš
- Ez → Ezechiel
- Dan → Daniel
- 1Pet → 1. Petra
- 2Pet → 2. Petra

The server's `find_bible_passage` and `resolve_bible_slides` tools accept both abbreviated and full forms.

## Translation code mapping

- SEB = Slovenský ekumenický preklad (slk-seb)
- ROH = Roháčkov preklad 1936 (slk-roh)
- SEVP / ECAV = Slovenský evanjelický preklad (slk-sevp)
- MIL = Milostný preklad (slk-mil)
- KJV = King James Version (eng-kjv)

## Other formatting rules

- Text written in ALL CAPS by the pastor → keep uppercase in `main`.
- "Nazov:" or "Názov:" → presentation title.
- "Vers na spamet:" → memory verse, use a group "Vers na zapamätanie".
```

- [ ] **Step 2: Add the `get_style_guide` tool definition**

In `crates/presenter-server/src/ai/tools.rs`, find the last `tool_def(...)` call inside `tool_definitions()` and add this new one before the closing `]`:

```rust
tool_def(
    "get_style_guide",
    "[REFERENCE] Get the detailed formatting guide for Bible references, Slovak book names, translation codes, multi-slide rules, and markdown conventions. The live system prompt only has essentials. Call this once at the start of a session if you need detailed rules.",
    json!({"type": "object", "properties": {}, "required": []}),
),
```

- [ ] **Step 3: Add the `get_style_guide` handler in `execute_tool`**

In `crates/presenter-server/src/ai/tools.rs`, find the `execute_tool` function and its match statement. Add a new arm. Place it near the other static/reference tools (e.g., near `list_bible_translations`):

```rust
"get_style_guide" => {
    let guide = include_str!("style_guide.md");
    Ok((
        guide.to_string(),
        "Style guide loaded".to_string(),
    ))
}
```

- [ ] **Step 4: Add a unit test**

In the existing `mod tests` block inside `tools.rs`, add:

```rust
#[tokio::test]
async fn get_style_guide_returns_expected_sections() {
    let state = AppState::in_memory().await.unwrap();
    let (result, preview) = execute_tool("get_style_guide", "{}", &state, 320)
        .await
        .unwrap();

    // Must contain the key section headers
    assert!(result.contains("# AI Presentation Style Guide"));
    assert!(result.contains("## Slide field usage"));
    assert!(result.contains("## Reference format"));
    assert!(result.contains("## Multi-slide passages"));
    assert!(result.contains("## Slovak Bible book abbreviations"));
    assert!(result.contains("## Translation code mapping"));

    // Must contain specific known content
    assert!(result.contains("Roháčkov preklad"));
    assert!(result.contains("Žalm 52:1-11"));

    // Preview should be short
    assert_eq!(preview, "Style guide loaded");
}
```

- [ ] **Step 5: Build and test**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
cargo test -p presenter-server ai::tools::tests::get_style_guide 2>&1 | tail -15
cargo test -p presenter-server ai 2>&1 | tail -15
```

Expected: the new test passes, all existing AI tests still pass.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/presenter-server/src/ai/style_guide.md crates/presenter-server/src/ai/tools.rs
git commit -m "feat(ai): add get_style_guide tool for on-demand formatting rules (#231 followup)

Task 4/6. Adds crates/presenter-server/src/ai/style_guide.md with
the detailed Slovak book abbreviations, translation codes, multi-
slide passage rules, and markdown conventions. A new get_style_guide
AI tool returns this markdown via include_str!, letting the model
load it on demand instead of carrying 80+ lines of rules in every
system prompt. The next task (prompt redesign) moves the rules out
of the hot path and relies on this tool for reference."
```

---

## Task 5: Redesign system prompt + populate tool-result previews

**Files:**
- Modify: `crates/presenter-server/src/ai/agent.rs` (rewrite `build_system_prompt`, populate preview on tool results)
- Modify: `crates/presenter-server/src/router/ai.rs` (use `msg.preview` instead of re-extraction)

- [ ] **Step 1: Rewrite `build_system_prompt`**

In `crates/presenter-server/src/ai/agent.rs`, replace the current `build_system_prompt` function (lines 29-153) with a much shorter version that adds a bible presentations block:

```rust
/// Build the system prompt with dynamic content from the database.
///
/// The prompt is intentionally short (~40 lines after interpolation).
/// Detailed formatting rules are NOT included here — the model can call
/// the `get_style_guide` tool if it needs them. See
/// `crates/presenter-server/src/ai/style_guide.md`.
async fn build_system_prompt(state: &AppState, extra: Option<&str>) -> (String, u32) {
    // Worship libraries (bible has its own separate storage after #231)
    let libraries = state.libraries().await.unwrap_or_default();
    let library_list: Vec<String> = libraries
        .iter()
        // Defensive filter: any library accidentally named "Bible" is NOT a
        // worship library. Bible content lives in bible_presentations.
        .filter(|l| !l.name.eq_ignore_ascii_case("Bible"))
        .map(|l| format!("- {} (id: {})", l.name, l.id))
        .collect();
    let libraries_str = if library_list.is_empty() {
        "(none)".to_string()
    } else {
        library_list.join("\n")
    };

    // Bible presentations (up to 20 most recent by repository order)
    let bible_presentations = state
        .list_bible_presentations()
        .await
        .unwrap_or_default();
    let bible_list: Vec<String> = bible_presentations
        .iter()
        .take(20)
        .map(|p| {
            format!(
                "- {} (id: {}, {} slides)",
                p.name, p.id, p.slide_count
            )
        })
        .collect();
    let bible_str = if bible_list.is_empty() {
        "(none yet)".to_string()
    } else {
        bible_list.join("\n")
    };

    // Bible translations — just the codes (model doesn't need the long names
    // on the hot path; get_style_guide has the human-readable mapping)
    let translations = state.list_bible_translations().await.unwrap_or_default();
    let translation_codes: Vec<String> =
        translations.iter().map(|t| t.code.clone()).collect();
    let translations_str = if translation_codes.is_empty() {
        "(none)".to_string()
    } else {
        translation_codes.join(", ")
    };

    let prefs = state.get_bible_preferences().await.unwrap_or_default();
    let char_limit = prefs.character_limit;

    let mut prompt = format!(
        r#"You are a presentation assistant for a church worship app.

## Live context

Worship libraries (for songs, hymns, band content):
{libraries}

Bible presentations (user-curated bible slide collections):
{bibles}

Bible translations available: {translations}
Slide character limit: {char_limit}

## Rules

1. For Bible content (verses, passages, sermon slides) use bible_* tools.
   Bible presentations are a SEPARATE concept from worship libraries and
   live in their own dedicated storage. Never create a worship library
   named "Bible".
2. For songs, hymns, band content use worship tools (create_presentation,
   add_slide, etc.) targeting a worship library from the list above.
3. Bible slide main_reference format: "Book Chapter:Verse TRANSLATION"
   (e.g. "Ján 3:16 SEB"). All slides in a multi-verse passage must carry
   the same full range.
4. If you need detailed formatting conventions (Slovak book names,
   translation code mapping, multi-verse rules, markdown syntax), call
   get_style_guide once — the rules live there, not in this prompt.
5. Destructive operations (delete_*) require explicit user intent. If
   the user hasn't said "delete", "remove", "vymazať", "odstrániť",
   "zmazať", or equivalent in their most recent message, ask them to
   confirm before calling any delete tool. The server will block delete
   calls that lack explicit user intent.

## Response format

Respond in the user's language (typically Slovak). Keep responses
concise. Summarize what you actually did based on tool results. Do not
claim success for tools that errored."#,
        libraries = libraries_str,
        bibles = bible_str,
        translations = translations_str,
        char_limit = char_limit,
    );

    if let Some(extra_prompt) = extra {
        if !extra_prompt.is_empty() {
            prompt.push_str("\n\n## Additional Instructions\n");
            prompt.push_str(extra_prompt);
        }
    }

    (prompt, char_limit)
}
```

- [ ] **Step 2: Populate preview on tool-result messages**

In `crates/presenter-server/src/ai/agent.rs`, find the tool dispatch loop where tool results are added to the conversation. The current code (around line 290) is:

```rust
// Add tool result to conversation
conversation.push(ChatMessage {
    role: "tool".to_string(),
    content: Some(result),
    tool_calls: None,
    tool_call_id: Some(tc.id.clone()),
    name: Some(tc.function.name.clone()),
});
```

Two things to fix here:

1. The `result` variable is a String (the JSON) — we need the preview too. Look at the `match super::tools::execute_tool(...)` block above: on `Ok((result, preview))` we save `preview` into `actions`. We need to also capture it for the `push`. Same for the `Err` case which uses `format!("Error: {err}")`.

2. Preview was already being pushed into `actions` — we just need to also attach it to the `ChatMessage`.

Refactor the block. Find the existing match starting around line 250:

```rust
let result = match super::tools::execute_tool(
    &tc.function.name,
    &tc.function.arguments,
    state,
    char_limit,
)
.await
{
    Ok((result, preview)) => {
        // Send progress: tool done
        if let Some(ref tx) = progress_tx {
            let _ = tx.send(ProgressEvent::ToolDone {
                tool: tc.function.name.clone(),
                preview: preview.clone(),
            });
        }
        actions.push(ToolAction {
            tool: tc.function.name.clone(),
            result_preview: preview,
        });
        result
    }
    Err(err) => {
        warn!(tool = %tc.function.name, ?err, "AI tool call failed");
        let preview = format!("Error: {err}");
        if let Some(ref tx) = progress_tx {
            let _ = tx.send(ProgressEvent::ToolDone {
                tool: tc.function.name.clone(),
                preview: preview.clone(),
            });
        }
        actions.push(ToolAction {
            tool: tc.function.name.clone(),
            result_preview: preview,
        });
        json!({"error": err.to_string()}).to_string()
    }
};

// Add tool result to conversation
conversation.push(ChatMessage {
    role: "tool".to_string(),
    content: Some(result),
    tool_calls: None,
    tool_call_id: Some(tc.id.clone()),
    name: Some(tc.function.name.clone()),
});
```

Replace with a version that captures preview in a separate binding:

```rust
let (result, preview) = match super::tools::execute_tool(
    &tc.function.name,
    &tc.function.arguments,
    state,
    char_limit,
)
.await
{
    Ok((result, preview)) => {
        if let Some(ref tx) = progress_tx {
            let _ = tx.send(ProgressEvent::ToolDone {
                tool: tc.function.name.clone(),
                preview: preview.clone(),
            });
        }
        actions.push(ToolAction {
            tool: tc.function.name.clone(),
            result_preview: preview.clone(),
        });
        (result, preview)
    }
    Err(err) => {
        warn!(tool = %tc.function.name, ?err, "AI tool call failed");
        let preview = format!("Error: {err}");
        if let Some(ref tx) = progress_tx {
            let _ = tx.send(ProgressEvent::ToolDone {
                tool: tc.function.name.clone(),
                preview: preview.clone(),
            });
        }
        actions.push(ToolAction {
            tool: tc.function.name.clone(),
            result_preview: preview.clone(),
        });
        (json!({"error": err.to_string()}).to_string(), preview)
    }
};

// Add tool result to conversation (preview is internal-only, not sent to LLM)
conversation.push(ChatMessage {
    role: "tool".to_string(),
    content: Some(result),
    tool_calls: None,
    tool_call_id: Some(tc.id.clone()),
    name: Some(tc.function.name.clone()),
    preview: Some(preview),
});
```

- [ ] **Step 3: Update `router/ai.rs` to use the stored preview**

In `crates/presenter-server/src/router/ai.rs`, find the `get_conversation` handler's `"tool"` branch (around line 146-170). The current code computes the preview from the message content JSON on every request:

```rust
"tool" => {
    // Accumulate tool results as actions for the next assistant text
    if let Some(ref name) = msg.name {
        let preview = msg
            .content
            .as_deref()
            .and_then(|c| {
                // Try to extract a short preview from the result
                serde_json::from_str::<serde_json::Value>(c)
                    .ok()
                    .and_then(|v| {
                        if let Some(arr) = v.as_array() {
                            Some(format!("{} results", arr.len()))
                        } else {
                            v.get("error").map(|err| format!("Error: {err}"))
                        }
                    })
            })
            .unwrap_or_else(|| "done".to_string());
        pending_actions.push(ToolAction {
            tool: name.clone(),
            result_preview: preview,
        });
    }
}
```

Replace with:

```rust
"tool" => {
    // Accumulate tool results as actions for the next assistant text.
    // Prefer the persisted preview field (populated in agent.rs at tool
    // execution time). Fall back to extracting from content for legacy
    // messages that were stored before the preview field existed.
    if let Some(ref name) = msg.name {
        let preview = msg.preview.clone().unwrap_or_else(|| {
            // Legacy fallback: best-effort extraction from tool result JSON.
            let Some(content) = msg.content.as_deref() else {
                return "done".to_string();
            };
            let Ok(json) = serde_json::from_str::<serde_json::Value>(content) else {
                return "done".to_string();
            };
            if let Some(err) = json.get("error").and_then(|v| v.as_str()) {
                return format!("Error: {err}");
            }
            if let Some(arr) = json.as_array() {
                return format!("{} results", arr.len());
            }
            "done".to_string()
        });
        pending_actions.push(ToolAction {
            tool: name.clone(),
            result_preview: preview,
        });
    }
}
```

- [ ] **Step 4: Build and run all AI tests**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
cargo test -p presenter-server ai 2>&1 | tail -20
cargo clippy -p presenter-server --all-targets -- -D warnings -W clippy::all 2>&1 | tail -10
```

Expected: all tests pass, clippy clean.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/presenter-server/src/ai/agent.rs crates/presenter-server/src/router/ai.rs
git commit -m "feat(ai): shrink system prompt + persist tool-result previews (#231 followup)

Task 5/6. Two coupled changes to finish the AI cleanup plan:

System prompt (agent.rs::build_system_prompt):
- Shrinks from 125 lines to ~40. Moves Slovak book abbreviations,
  translation code mapping, multi-verse rules, and markdown
  conventions OUT to a get_style_guide tool the model can call
  on demand (Task 4).
- Adds a new 'Bible presentations' section that lists up to 20
  recent BiblePresentations by name/id/slide_count. This is the
  structural fix for 'AI created bible content as a worship
  presentation' — the model now has a clear place to put bible
  slides that is separate from worship libraries.
- Adds defensive filter: any worship library accidentally named
  'Bible' is excluded from the list.
- Includes a note about the delete intent gate in rule #5 so the
  model knows to ask for confirmation rather than retry.

Preview persistence (agent.rs + router/ai.rs):
- Tool-result messages now carry the preview string in the new
  ChatMessage.preview field (added in Task 1).
- router/ai.rs::get_conversation uses msg.preview.clone() with a
  best-effort fallback for legacy messages that were stored before
  this field existed.
- UI tool action badges now survive page reloads and show the
  real summary ('Created bible presentation John 3:16 with 1
  slides') instead of the literal string 'done'."
```

---

## Task 6: Version bump, push, monitor CI, PR

- [ ] **Step 1: Check version state**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
grep '^version' Cargo.toml | head -1
git fetch origin
git show origin/main:Cargo.toml | grep '^version' | head -1
```

Expected: both show `0.4.16` since PR #235 was the last merge. Confirm dev is equal to or behind main, then bump.

- [ ] **Step 2: Bump version to 0.4.17**

In `Cargo.toml`, find `[workspace.package]` and change `version = "0.4.16"` to `version = "0.4.17"`.

```bash
cargo check -p presenter-server 2>&1 | tail -3
```

This refreshes `Cargo.lock` with the new version.

- [ ] **Step 3: Sync with main**

```bash
git fetch origin
git merge origin/main --no-edit
```

If no conflicts, continue. If conflicts appear, they'll be in files this PR doesn't touch — resolve and continue.

- [ ] **Step 4: Run all local checks**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all 2>&1 | tail -10
cargo test -p presenter-server 2>&1 | tail -15
./scripts/dev/quality-check.sh --strict --against origin/main 2>&1 | grep -E 'fail|FAIL' | head -5
```

Expected: zero failures across all checks. Fix anything that breaks before pushing.

- [ ] **Step 5: Commit version bump**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore: bump version to 0.4.17"
```

- [ ] **Step 6: Push and monitor CI**

```bash
git push origin dev
gh run list --branch dev --limit 3
```

Monitor the pipeline run until ALL jobs reach a terminal state. If anything fails, `gh run view <run-id> --log-failed`, fix ALL issues in ONE commit, push ONCE, monitor again.

- [ ] **Step 7: Verify dev deployment**

```bash
curl -s http://10.77.8.134:8080/healthz
```

Expected: `{"channel":"dev","status":"ok","version":"0.4.17"}`

- [ ] **Step 8: Open PR**

```bash
cat > /tmp/pr-ai-cleanup-body.md <<'EOF'
## Summary

Fixes 5 AI chat bugs identified in production after PR #235. Full design spec at `docs/superpowers/specs/2026-04-11-ai-mode-cleanup-design.md`.

### 1. Library type confusion → bible presentations block in prompt
The system prompt now lists bible presentations as a separate block (up to 20 recent). The model has a clear structural distinction between "worship libraries" and "bible presentations". Rule #1 in the prompt explicitly says bible content uses bible_* tools. Defensive filter also excludes any worship library accidentally named "Bible".

### 2. Preview loss on reload → `preview` field persists through conversation history
New `ChatMessage.preview` field stores the tool-result summary at execution time. `get_conversation` reads it back instead of re-extracting from JSON. UI tool action badges now survive page reloads showing "Created bible presentation 'John 3:16' with 1 slides" instead of literal "done".

### 3. Unprotected destructive ops → delete intent gate
Before dispatching any `delete_*` tool call, the agent checks the user's original message for delete keywords (English: delete/remove/discard/destroy/erase, Slovak: vymazať/odstrániť/zmazať, Czech: smazat — with accent variants). Blocked calls produce a synthetic error that tells the model to ask for confirmation. Hallucinated deletes can no longer cause data loss.

### 4. Trim corrupts tool call/result pairs → turn-boundary trimming
`trim_conversation` now trims at user-message boundaries, preserving tool_call/result pairs. Hard ceiling of 200 messages as a safety net for tool-call-dense sessions.

### 5. Bloated system prompt → shrink + `get_style_guide` tool
System prompt drops from 125 lines to ~40. Slovak book abbreviations, translation code mappings, multi-slide rules, and markdown conventions moved to a new `get_style_guide` AI tool that returns `crates/presenter-server/src/ai/style_guide.md` via `include_str!`. The model can load detailed rules on demand rather than carrying them in every request.

## Test coverage

- 6 unit tests for `delete_intent_allowed` (English, Slovak, Czech, accents, case, tool-name prefix match)
- 6 unit tests for `trim_conversation` (under-limit, over-limit, no-orphan-tool-results, hard-ceiling, empty, single-message)
- 1 unit test for `get_style_guide` tool (all sections present, expected content)
- All existing AI tests still pass

Total server tests: 119 (was 106).

## Verified on dev (v0.4.17)
- `curl /healthz` returns 0.4.17
- Cargo fmt/clippy/quality-check all clean
- Pipeline green

## Version
0.4.16 → 0.4.17

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
gh pr create --title "fix(ai): cleanup AI mode — delete gate, preview persistence, prompt shrink (#231 followup)" --body-file /tmp/pr-ai-cleanup-body.md
```

- [ ] **Step 9: After PR is mergeable, wait for explicit user merge instruction**

Per project policy, NEVER merge without explicit user instruction. Provide the PR URL and wait.

---

## Verification Checklist

After PR merges to main and deploys to production:

- [ ] `curl http://10.77.9.205/healthz` returns version 0.4.17
- [ ] Open `http://presenter.lan/ui/ai` — ask "Create a Bible presentation called Test with Ján 3:16 from SEB"; verify it shows up in the Bible tab at `/ui/bible`
- [ ] Same chat: ask "clean up old stuff" — AI should ask for confirmation, NOT delete anything
- [ ] Same chat: ask "delete the Test presentation" — AI should proceed with the delete
- [ ] After page reload, tool action badges show actual summaries, not literal "done"
- [ ] Long conversations (15+ turns with tool calls) don't break the OpenAI API with dangling tool_call_id references

---

## Risks Reminder

- **Old conversation entries from before this PR** will have `preview: None` and trigger the fallback path in `get_conversation`. The fallback is best-effort and may still show "done" for those messages — acceptable because they're historical.
- **System prompt shrinkage** might regress behavior if the model relied on inline Slovak abbreviations for edge cases. Mitigation: the `get_style_guide` tool is available, and server-side `find_bible_passage` already handles abbreviation parsing. If a regression is observed, the fix is to enlarge specific tool parameter descriptions, not to re-add prompt bloat.
- **Delete gate false positives** (e.g., user says "remove verse 3 from my slide" — contains "remove" — allowed through). Acceptable: that IS explicit delete intent.
- **Rollback:** all changes are in the same PR. Revert the PR if anything misbehaves. No database changes, no schema migration, no wire format breakage.
