# AI Mode Cleanup & Redesign — Design Spec

**Date:** 2026-04-11
**Scope:** 5-fix bundle addressing known AI chat bugs (hallucinated deletes, preview loss on reload, trim corruption, library type confusion, prompt bloat)
**Approach:** Keep the current agent loop + tool architecture. Surgical fixes, not a rewrite.

## Problem

The AI chat mode has five concrete bugs observed in production:

1. **Library type confusion** — The AI claims to create a Bible presentation but actually creates a worship presentation in a random worship library. Root cause: the system prompt lists libraries as `- Name (id: uuid)` with no type info. The model guesses "bible" by name-matching. After PR #234 (bible/worship separation), bible content lives in its own `bible_presentations` table and has NO library wrapper — but the system prompt still only shows worship libraries, giving the model no structural context for bible content.

2. **Preview loss on conversation reload** — After page reload, tool action badges show `"create_bible_presentation: done"` instead of the real summary. Root cause: `router/ai.rs:149-164` recomputes the preview text from the tool result JSON on every `GET /ai/conversation`, and the fallback for non-array objects is the literal string `"done"`. Most tool results are objects.

3. **No protection on destructive operations** — `delete_*` tools execute immediately when the model decides. Model hallucinations cause data loss. A user reported a delete-then-create loop that left duplicate presentations on production.

4. **Conversation trim drops tool call/result pairs** — `agent.rs:327-333` naively drains the oldest N messages when conversation exceeds 40. This can split an assistant tool-call message from its tool-result message, violating the OpenAI API contract (`tool_call_id` references a nonexistent call) and causing the next LLM request to fail.

5. **System prompt is 125 lines of formatting rules** — Slovak book abbreviations, translation code mappings, multi-slide rules. Most of it is dead weight on the hot path. The model ignores details.

## Goals

- Structural fix for bible/worship library confusion so the AI never creates bible content in a worship library again
- Preview text survives page reload
- Destructive ops require explicit user intent (no hallucinated deletes)
- Trim preserves tool call/result pairs
- System prompt shrinks to ~40 lines; detailed rules move to on-demand `get_style_guide` tool

## Non-goals

- No structured JSON response format (stays free text)
- No undo stack for destructive ops
- No multi-step user-approved plans
- No transactional semantics for delete+create sequences
- No conversation persistence to disk (stays in-memory)
- No rate limiting

## Architecture

Unchanged. Current flow:

```
User → /ai/chat (SSE) → agent.rs::run_agent → LLM (function-calling) → tool dispatch → state mutation → next LLM iteration → text response
```

Trimming, conversation history, system prompt construction all stay in `agent.rs`. Tool definitions stay in `tools.rs`. Wire API unchanged.

## Changes

### 1. System prompt redesign (`crates/presenter-server/src/ai/agent.rs:29-153`)

Replace the current `build_system_prompt` with a much shorter version that adds a bible presentations block.

**New prompt body** (~40 lines including dynamic data):

```text
You are a presentation assistant for a church worship app.

## Live context

Worship libraries (for songs, hymns, band content):
{for each worship library: "- {name} (id: {id})"}

Bible presentations (user-curated bible slide collections):
{for each bible presentation (up to 20 most recent): "- {name} (id: {id}, {slide_count} slides)"}

Bible translations: {comma-separated translation codes}
Slide character limit: {char_limit}

## Rules

1. For Bible content (verses, passages, sermon slides) use bible_* tools. Bible
   presentations are a SEPARATE concept from worship libraries and live in their
   own storage.
2. For songs, hymns, band content use worship tools (create_presentation,
   add_slide, etc.) targeting a worship library.
3. Never create a worship library named "Bible". Bible content has dedicated
   storage; use create_bible_presentation instead.
4. Bible slide main_reference format: "Book Chapter:Verse TRANSLATION"
   (e.g. "Ján 3:16 SEB"). All slides in a multi-verse passage must carry
   the same full range.
5. If you need detailed formatting conventions (Slovak book names, translation
   codes, multi-verse rules, markdown syntax), call get_style_guide once at
   the start of a session.
6. Destructive operations (delete_*) require explicit user intent. If the user
   hasn't said "delete", "remove", "vymazať", or equivalent in their most
   recent message, ask them to confirm before calling any delete tool.

## Response format

Respond in the user's language (typically Slovak). Keep responses concise.
Summarize what you actually did based on tool results. Do not claim success
for tools that errored.
```

**Dynamic data population:**
- Worship libraries: `state.libraries().await` filtered to remove any trailing "Bible" library row (there shouldn't be one after migration #234, defensive check)
- Bible presentations: `state.list_bible_presentations().await`, up to 20 most recent (sorted by `created_at` desc on server side or name asc — use whatever the existing repository returns; cap at 20)
- Translation codes: `state.list_bible_translations().await`, comma-joined codes
- Char limit: `state.get_bible_preferences().await.character_limit`

**Removed content (moved to `get_style_guide` tool):**
- Slovak book abbreviation table
- Translation code → human name mapping (e.g. "SEB = Slovenský ekumenický preklad")
- Multi-slide passage rules
- Markdown formatting examples
- Library selection heuristics (because the structural separation makes them unnecessary)

### 2. Delete intent gate (`crates/presenter-server/src/ai/agent.rs`)

Add a pre-dispatch check in `run_agent`. Before calling `execute_tool` for any tool whose name starts with `delete_`, verify the user's original message contains a delete keyword. If not, return a synthetic tool error to the model instead of executing.

```rust
/// Check whether the user's message carries explicit intent to delete.
/// Used as a gate on all delete_* tool calls to prevent model hallucinations
/// from causing data loss.
fn delete_intent_allowed(user_message: &str) -> bool {
    const DELETE_KEYWORDS: &[&str] = &[
        // English
        "delete", "remove", "discard", "destroy", "erase",
        // Slovak (lowercase, matches both with and without accents)
        "vymazať", "vymazat", "vymaz",
        "odstrániť", "odstranit", "odstran",
        "zmazať", "zmazat", "zmaz",
        // Czech
        "smazat",
    ];
    let lower = user_message.to_lowercase();
    DELETE_KEYWORDS.iter().any(|kw| lower.contains(kw))
}
```

**Integration in the dispatch loop:**

In `run_agent`, capture the original user message once at the top of the function. Inside the tool dispatch loop, before calling `execute_tool`:

```rust
if tool_name.starts_with("delete_") && !delete_intent_allowed(&original_user_message) {
    let error_json = serde_json::json!({
        "error": "delete_blocked",
        "reason": "Delete operations require explicit user intent. The user's message did not contain any delete keywords (delete, remove, vymazať, odstrániť, zmazať, etc.). Ask the user to confirm the deletion explicitly before retrying."
    }).to_string();
    let preview = "BLOCKED: delete requires explicit user intent".to_string();

    // Send SSE progress event so the UI shows the blocked call
    let _ = progress_tx.send(ProgressEvent::ToolDone {
        tool: tool_name.clone(),
        preview: preview.clone(),
    });

    // Record the blocked action
    actions.push(ToolAction {
        tool: tool_name.clone(),
        result_preview: preview.clone(),
    });

    // Add the synthetic error to the conversation so the model sees it
    conversation.push(ChatMessage {
        role: "tool".to_string(),
        content: Some(error_json),
        tool_call_id: Some(tool_call.id.clone()),
        name: Some(tool_name.clone()),
        preview: Some(preview),
        tool_calls: None,
    });

    continue; // Skip to next iteration — model will see error and respond
}
```

**Effect:**
- User says "clean up old stuff" → model decides to call `delete_presentation` → intent gate blocks → model sees `"delete_blocked"` error → model responds "Do you want me to delete the old presentations? Please confirm with 'delete'."
- User says "delete the Psalm 23 presentation" → intent gate allows → delete proceeds normally.
- User says "vymaž prezentáciu" (Slovak for "delete the presentation") → intent gate allows.

**Coverage:** the gate blocks `delete_presentation`, `delete_library`, `delete_slide`, `delete_bible_presentation`, `delete_bible_slide`, and any future `delete_*` tool by prefix match.

**False positive rate:** low. A user who writes "I want to remove verse 3 from my slide" contains "remove" and is allowed through (correct — that IS an explicit delete intent).

### 3. Preview persistence fix (`crates/presenter-server/src/ai/mod.rs`, `agent.rs`, `router/ai.rs`, `client.rs`)

Add a `preview: Option<String>` field to `ChatMessage`. Populate it on tool-result messages at execution time. Read it back on `get_conversation`. Critically: the field MUST NOT be serialized when building the OpenAI API request (wire-safety).

**Type change** in `crates/presenter-server/src/ai/mod.rs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCallMessage>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Human-readable summary of the tool result. Only set on role="tool"
    /// messages. NEVER sent to the LLM — stripped by the wire serializer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
}
```

**Population in `agent.rs`** — where tool results are added to the conversation:

```rust
conversation.push(ChatMessage {
    role: "tool".to_string(),
    content: Some(result_json),
    tool_call_id: Some(tool_call.id.clone()),
    name: Some(tool_name.clone()),
    preview: Some(preview.clone()),
    tool_calls: None,
});
```

**Read-back in `router/ai.rs::get_conversation`** — replace the 149-164 extraction block:

```rust
let preview = msg
    .preview
    .clone()
    .or_else(|| {
        // Legacy fallback for conversation entries saved before this PR.
        // Attempt content extraction; if nothing useful, use "done".
        let content = msg.content.as_ref()?;
        let json = serde_json::from_str::<serde_json::Value>(content).ok()?;
        if let Some(err) = json.get("error").and_then(|v| v.as_str()) {
            return Some(format!("Error: {err}"));
        }
        None
    })
    .unwrap_or_else(|| "done".to_string());
```

**Wire-safety** — in `crates/presenter-server/src/ai/client.rs`, the struct used to serialize outgoing messages to the OpenAI API must NOT include `preview`. The cleanest approach is a wire-only struct:

```rust
// client.rs — wire struct used only for outgoing API calls
#[derive(Serialize)]
struct WireMessage<'a> {
    role: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<&'a [ToolCallMessage]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
}

impl<'a> From<&'a ChatMessage> for WireMessage<'a> {
    fn from(msg: &'a ChatMessage) -> Self {
        Self {
            role: &msg.role,
            content: msg.content.as_deref(),
            tool_calls: msg.tool_calls.as_deref(),
            tool_call_id: msg.tool_call_id.as_deref(),
            name: msg.name.as_deref(),
        }
    }
}
```

The existing `call_chat_completions` builds the request body using `WireMessage::from` for each conversation message. `preview` cannot leak.

### 4. Turn-boundary conversation trimming (`crates/presenter-server/src/ai/agent.rs`)

Replace the current `trim_conversation` that drains the oldest N messages:

```rust
/// Trim conversation to at most `max_turns` user turns. A "turn" is defined
/// as a user message and all subsequent assistant/tool messages up to the
/// next user message. Trimming at turn boundaries preserves tool_call/result
/// pairs (never orphans a tool result, never leaves a dangling tool_call_id).
///
/// Also enforces a hard ceiling of 200 messages total to prevent runaway
/// conversations from unbounded growth even within max_turns.
fn trim_conversation(conversation: &mut Vec<ChatMessage>, max_turns: usize) {
    // Find positions of all user messages
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

    // Hard ceiling: if we're still above 200 messages even after turn trim,
    // drop the oldest user turn. Repeat until we're under the cap.
    while conversation.len() > 200 {
        let user_positions: Vec<usize> = conversation
            .iter()
            .enumerate()
            .filter(|(_, m)| m.role == "user")
            .map(|(i, _)| i)
            .collect();
        if user_positions.len() < 2 {
            break;
        }
        let next_user = user_positions[1];
        conversation.drain(0..next_user);
    }
}
```

Call with `max_turns = 10` at the end of `run_agent`.

**Invariants this preserves:**
- A tool-result message always has its matching tool-call message earlier in the conversation.
- `tool_call_id` references are never dangling.
- The last N user turns are always intact.

### 5. `get_style_guide` tool (`crates/presenter-server/src/ai/tools.rs`)

New tool that returns the detailed formatting rules the model occasionally needs but shouldn't carry in every prompt. Called on-demand by the model when it needs guidance.

**Tool definition:**

```rust
tool_def(
    "get_style_guide",
    "[REFERENCE] Get the detailed formatting guide for Bible references, Slovak book names, translation codes, multi-slide rules, and markdown conventions. Call this once at the start of a session if you need detailed rules — the live system prompt only has essentials.",
    json!({"type": "object", "properties": {}, "required": []}),
),
```

**Handler in `execute_tool`:**

```rust
"get_style_guide" => {
    let guide = include_str!("style_guide.md");
    Ok((
        guide.to_string(),
        "Style guide loaded".to_string(),
    ))
}
```

**New file:** `crates/presenter-server/src/ai/style_guide.md` — contains the content that was stripped from the system prompt:

```markdown
# AI Presentation Style Guide

## Slovak Bible book abbreviations

- Genesis → Genezis / Gen / 1M
- Exodus → Exodus / Ex / 2M
- Žalmy → Ž / Ps
- Príslovia → Prís / Pr
- Evanjelium podľa Jána → Ján / Jn
- ... (full list)

## Translation codes

- SEB = Slovenský ekumenický preklad
- ROH = Roháčkov preklad (1936)
- SEVP = Slovenský evanjelický preklad
- MIL = Milostný preklad
- KJV = King James Version

## Multi-verse passage rules

When creating slides for a Bible passage spanning multiple verses:
- Every slide's main_reference MUST carry the full range (e.g., "Ján 3:16-17 SEB")
- Split verses by the character limit ({char_limit})
- If a single verse exceeds the limit, split at sentence boundaries

## Markdown conventions

- Use ## as emphasis markers around key phrases: "##God so loved the world##"
- These render bold in stage output
- Do NOT use **bold** or _italic_ — the stage renderer does not parse them
```

**Note:** the character limit in the markdown is a static placeholder, not live. The model already sees the live char limit in the system prompt.

## Tests

### Unit tests (in `ai/tests.rs` or inline in each module)

1. **`delete_intent_allowed` positive cases** — each keyword must match:
   - "delete the presentation", "remove old slides", "discard this", "destroy the library"
   - "vymaž prezentáciu", "odstráň tento slajd", "zmaž to"
   - "smazat to" (Czech)

2. **`delete_intent_allowed` negative cases** — must not match:
   - "clean up old stuff" (no keyword)
   - "make a fresh start" (no keyword)
   - "can you fix the layout"

3. **`delete_intent_allowed` accent variants**:
   - "Vymazať" (capitalized), "VYMAZAT" (uppercase, no diacritics) both match

4. **`trim_conversation` preserves tool_call/result pairs** — build a 15-turn conversation where most turns have tool calls and results, call `trim_conversation(&mut conv, 5)`, verify:
   - Length shrinks appropriately
   - Every `tool_call_id` in the remaining conversation has a matching tool_call earlier
   - The trim point is exactly at a user-message boundary

5. **`trim_conversation` hard ceiling** — build a conversation with 5 turns but 300 messages (heavy tool-call density), call `trim_conversation(&mut conv, 10)`, verify result is ≤200 messages and still preserves tool_call/result pairs.

6. **`ChatMessage` serde round-trip with preview field** — serialize a tool-role message with `preview: Some("foo")`, deserialize, verify preview survives.

7. **`WireMessage` excludes preview field** — serialize a `ChatMessage` with `preview: Some("foo")` through `WireMessage::from`, deserialize the resulting JSON, verify `preview` is NOT present.

8. **`get_style_guide` returns expected content** — call `execute_tool("get_style_guide", "{}", &state, 320)`, verify the result contains "Translation codes" and "Multi-verse passage rules".

### Integration tests

9. **End-to-end delete gate blocks hallucinated delete** — start an in-memory server, mock the LLM to return a `delete_presentation` tool call with `user_message = "create a slide"`, verify no state mutation occurred and the conversation has a `delete_blocked` error message.

10. **End-to-end delete gate allows explicit delete** — same setup but `user_message = "delete the test presentation"`, verify the delete proceeds and state mutates.

Not required: Playwright E2E. These are server-internal behaviors and the existing UI E2E tests cover the chat flow at a higher level.

## Risks & rollback

| Risk | Severity | Mitigation |
|------|----------|------------|
| Delete gate blocks legitimate deletes (false positive) | LOW | Keyword list is broad (English + Slovak + Czech + variants). Users can always explicitly say "delete". Blocked calls produce a clear error that tells the model to ask for confirmation. |
| Trim ceiling of 200 messages too tight | LOW | Conservative estimate; Claude 200K context easily holds 200 messages. Can be raised if users hit it. |
| `preview` field leaks to LLM via serde | HIGH | Mitigated by `WireMessage` wire-only struct. Unit test #7 verifies the exclusion. |
| Model doesn't find `get_style_guide` when needed | MEDIUM | System prompt rule 5 explicitly mentions it. If the model ignores the hint, the fallback is that it just uses general knowledge, which is usually sufficient for English Bible content. |
| Shrinking system prompt breaks model behavior on edge cases (e.g. Slovak markdown formatting) | MEDIUM | Tool parameter descriptions compensate. If a regression is observed, the fix is to enlarge specific tool description text rather than re-adding prompt bloat. |

**Rollback:** all changes are in the same PR. Revert the PR if anything misbehaves. No database changes, no schema migration, no wire breakage (new `preview` field is optional on the wire).

## Version bump

0.4.16 → 0.4.17

## File impact summary

| File | Change |
|------|--------|
| `crates/presenter-server/src/ai/mod.rs` | Add `preview: Option<String>` to `ChatMessage` |
| `crates/presenter-server/src/ai/agent.rs` | Rewrite `build_system_prompt`, add `delete_intent_allowed`, add delete gate to dispatch loop, rewrite `trim_conversation`, capture `original_user_message`, populate `preview` on tool results |
| `crates/presenter-server/src/ai/client.rs` | Add `WireMessage` wire-only struct, use it when building OpenAI request body |
| `crates/presenter-server/src/ai/tools.rs` | Add `get_style_guide` tool definition + handler |
| `crates/presenter-server/src/ai/style_guide.md` | NEW file — detailed formatting rules content |
| `crates/presenter-server/src/router/ai.rs` | Use `msg.preview` instead of re-computing on `get_conversation` |
| `Cargo.toml` | Version bump 0.4.16 → 0.4.17 |

~+300 / -120 LOC, 7 files.
