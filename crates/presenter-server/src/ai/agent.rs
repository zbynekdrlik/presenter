use super::{AiSettings, ChatMessage, ToolAction, ToolCallFunction, ToolCallMessage};
use crate::state::AppState;
use serde_json::{json, Value};
use tracing::{info, warn};

const MAX_ITERATIONS: usize = 100;

/// Affirmative replies recognised by the cross-turn gate. Used when the
/// user types a short confirmation after the AI proposed a delete in the
/// preceding text response.
const AFFIRMATIVE_REPLIES: &[&str] = &[
    // English
    "yes",
    "yeah",
    "yep",
    "y",
    "ok",
    "okay",
    "sure",
    "confirm",
    "confirmed",
    "go ahead",
    "do it",
    "do it.",
    "please do",
    // Slovak with diacritics
    "áno",
    "súhlasím",
    "potvrdzujem",
    "iste",
    "určite",
    // Slovak ASCII-folded variants
    "ano",
    "suhlasim",
    "potvrdzujem.",
    "iste.",
    // Czech
    "souhlasim",
    "souhlasím",
    "ano.",
];

fn is_affirmative(msg: &str) -> bool {
    let trimmed = msg.trim().to_lowercase();
    if trimmed.is_empty() {
        return false;
    }
    // Exact match OR starts with an affirmative followed by space/comma/period.
    AFFIRMATIVE_REPLIES.iter().any(|a| {
        let a_lower = a.to_lowercase();
        trimmed == a_lower
            || trimmed.starts_with(&format!("{a_lower} "))
            || trimmed.starts_with(&format!("{a_lower},"))
            || trimmed.starts_with(&format!("{a_lower}."))
            || trimmed.starts_with(&format!("{a_lower}!"))
    })
}

/// Cross-turn delete-intent gate. Grants when:
/// - the current user message itself contains a delete keyword, OR
/// - the IMMEDIATELY PRECEDING user message contained a delete keyword AND
///   the current message is affirmative (covers the "user asks to delete,
///   AI defers for confirmation, user replies 'yes'" workflow).
///
/// The deferred path looks back EXACTLY one user turn — the previous one —
/// so an unrelated affirmative ("yes" to a different question turns later)
/// cannot unlock stale intent from far back in the conversation. This bounds
/// the gate window to the only pattern it needs to cover: a single defer.
fn delete_intent_for_turn(user_message: &str, conversation: &[ChatMessage]) -> bool {
    // Direct: current message has a delete keyword.
    if delete_intent_allowed(user_message) {
        return true;
    }

    // Deferred: current message must be affirmative AND the previous user
    // turn must have had explicit delete intent. Both signals required, and
    // the lookback is bounded to ONE prior user turn — not the whole
    // conversation — so stale intent decays naturally as the conversation
    // moves on.
    if !is_affirmative(user_message) {
        return false;
    }

    // The current user message is already at the END of conversation (the
    // run_agent loop pushes it before calling this gate). Skip that one and
    // grab the immediately preceding user message; if it had delete intent,
    // grant.
    let mut user_msgs = conversation
        .iter()
        .rev()
        .filter(|m| m.role == "user")
        .filter_map(|m| m.content.as_deref());
    let _current = user_msgs.next();
    let previous = user_msgs.next();
    previous.map(delete_intent_allowed).unwrap_or(false)
}

/// Returns `true` if the user's message contains an explicit intent to delete.
/// Used as a gate on all `delete_*` tool calls to prevent model hallucinations
/// from causing data loss. The model must see a keyword in the user's actual
/// message — it cannot invent the intent on its own.
fn delete_intent_allowed(user_message: &str) -> bool {
    const DELETE_KEYWORDS: &[&str] = &[
        // English
        "delete",
        "remove",
        "discard",
        "destroy",
        "erase",
        // Slovak with diacritics (lowercase forms)
        "vymazať",
        "vymaž",
        "odstrániť",
        "odstráň",
        "zmazať",
        "zmaž",
        // Slovak without diacritics
        "vymazat",
        "vymaz",
        "odstranit",
        "odstran",
        "zmazat",
        "zmaz",
        // Czech
        "smazat",
    ];
    let lower = user_message.to_lowercase();
    DELETE_KEYWORDS.iter().any(|kw| lower.contains(kw))
}

/// Progress events sent during agent execution for real-time UI updates.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
#[allow(dead_code)] // Variants constructed via serialization patterns
pub enum ProgressEvent {
    ToolStart {
        tool: String,
    },
    ToolDone {
        tool: String,
        preview: String,
    },
    Response {
        response: String,
        actions: Vec<ToolAction>,
    },
    Error {
        message: String,
    },
}

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
    let bible_presentations = state.list_bible_presentations().await.unwrap_or_default();
    let bible_list: Vec<String> = bible_presentations
        .iter()
        .take(20)
        .map(|p| format!("- {} (id: {}, {} slides)", p.name, p.id, p.slide_count))
        .collect();
    let bible_str = if bible_list.is_empty() {
        "(none yet)".to_string()
    } else {
        bible_list.join("\n")
    };

    // Bible translations — just the codes (model doesn't need the long names
    // on the hot path; get_style_guide has the human-readable mapping)
    let translations = state.list_bible_translations().await.unwrap_or_default();
    let translation_codes: Vec<String> = translations.iter().map(|t| t.code.clone()).collect();
    let translations_str = if translation_codes.is_empty() {
        "(none)".to_string()
    } else {
        translation_codes.join(", ")
    };

    let prefs = state.get_bible_preferences().await.unwrap_or_default();
    let char_limit = prefs.character_limit;

    let mut prompt =
        format_system_prompt(&libraries_str, &bible_str, &translations_str, char_limit);

    if let Some(extra_prompt) = extra {
        if !extra_prompt.is_empty() {
            prompt.push_str("\n\n## Additional Instructions\n");
            prompt.push_str(extra_prompt);
        }
    }

    (prompt, char_limit)
}

/// Format the static system-prompt template, interpolating the live-context
/// strings and the character limit. Pure (no DB / async) so it is unit-tested
/// directly. Extracted from `build_system_prompt` to keep that function under
/// the 120-line cap and to give the prompt's wording a direct test target.
/// Named `format_*` (not `render_*`) deliberately so the function-length gate
/// still applies — `render_*` is the Leptos-UI exemption prefix.
fn format_system_prompt(
    libraries: &str,
    bibles: &str,
    translations: &str,
    char_limit: u32,
) -> String {
    format!(
        r#"You are a presentation assistant for a church worship app.

## Live context

Worship libraries (for songs, hymns, band content):
{libraries}

Bible presentations (user-curated bible slide collections):
{bibles}

Bible translations available: {translations}
Slide character limit: {char_limit}

## Creating Bible slides

You do NOT decide where slides break. The server composes slides from a
typed stream of items you submit. You pick the items; the server decides
how many slides they become.

1. Parse the sermon text yourself: find passage references (##Book Ch:V##
   or ##Book Ch:V-V##), ##bold## markers inside verses, and any ##title##
   at the very start (use as presentation name).

2. For each passage: call load_bible_verses(book, chapter, verse_start,
   verse_end, translation) to get the raw DB verses as an array of
   {{number, text, reference}} objects. This is the source of truth for
   verse text. Never invent verses from memory.

3. For each loaded verse, compare its text to the sermon's wording.
   The sermon is authoritative for text content. If they differ, REPLACE
   the text field with the sermon's wording. If the pastor quotes a
   verse number that does not match the DB (e.g. says Ján 3:16 but quotes
   Ján 3:17 text), keep the sermon's text and the sermon's verse number.

4. Apply ##word## markers: inside a verse, replace the word with WORD
   (uppercase) inline. The result stays as a single verse item — do NOT
   create a separate slide for in-verse emphasis.

5. Extract ##phrase## markers that appear as standalone emphasis (not a
   reference, not inside a verse): emit a separate
   {{"kind": "emphasis", "text": "PHRASE"}} item at the position where
   the phrase appears in the sermon. Phrase text goes uppercase.

6. Assemble an items[] array in sermon order:

       [
         {{"kind": "verse", "number": 1, "text": "Na počiatku bolo Slovo.",
          "book": "Ján", "chapter": 1, "translation": "SEB"}},
         {{"kind": "verse", "number": 2, "text": "Ono bolo na počiatku.",
          "book": "Ján", "chapter": 1, "translation": "SEB"}},
         {{"kind": "emphasis", "text": "NOVÁ ZMLUVA"}},
         {{"kind": "verse", "number": 3, "text": "Všetko vzniklo.",
          "book": "Ján", "chapter": 1, "translation": "SEB"}}
       ]

   Verse items MUST include number, text, book, chapter, and translation
   (short code like SEB, MIL, ROH). Emphasis items need only kind and text.

7. Call create_bible_presentation(name, items). The server greedy-packs
   consecutive verse items into slides until the character limit ({char_limit}
   chars) would overflow, then flushes. Emphasis items and translation,
   book, or chapter changes force slide breaks. The server auto-computes
   reference labels like "Ján 1:1-2 (SEB)".

8. If a single verse is longer than the character limit on its own
   (rare), DO NOTHING SPECIAL — submit it as one normal verse item. A lone
   whole verse over the limit is accepted whole on its own slide and shrunk
   to fit by display autofit; it is NOT an error and you must NOT split it
   across items. If you do supply several continuation items that share the
   same verse number, the server merges them back into ONE whole verse on a
   single slide (a verse is never split mid-text). So
   main_exceeds_character_limit only ever means a slide over-packed MULTIPLE
   distinct verses, or an emphasis/title slide is too long — keep verses as
   separate items (let the server pack them) or shorten the emphasis text.

9. The server validates composed slides and returns a rule-keyed JSON
   error on failure (rules: main_exceeds_character_limit,
   unprocessed_bold_markers, empty_main_on_emphasis_slide,
   reference_format_requires_parens, missing_verse_number_prefix).
   Read the rule and expected fields, fix the item, and retry.

## Rules

1. For Bible content (verses, passages, sermon slides) use bible_* tools.
   Bible presentations are a SEPARATE concept from worship libraries and
   live in their own dedicated storage. Never create a worship library
   named "Bible".
2. For songs, hymns, band content use worship tools (create_presentation,
   add_slide, etc.) targeting a worship library from the list above.
3. If you need detailed secondary reference material (Slovak book name
   abbreviations, translation code mapping table), call get_style_guide
   once — the bible slide creation rules above are authoritative, this
   is just a lookup aid.
4. Destructive operations (delete_*) require explicit user intent. If
   the user hasn't said "delete", "remove", "vymazať", "odstrániť",
   "zmazať", or equivalent in their most recent message, ask them to
   confirm before calling any delete tool. The server will block delete
   calls that lack explicit user intent.

## Response format

Respond in the user's language (typically Slovak). Keep responses
concise. Summarize what you actually did based on tool results. Do not
claim success for tools that errored."#,
        libraries = libraries,
        bibles = bibles,
        translations = translations,
        char_limit = char_limit,
    )
}

/// Build the OpenAI-style messages array for one API call: the system prompt
/// followed by every conversation message (content / tool_calls / tool_call_id
/// / name copied through). Extracted from `run_agent` to keep it under the
/// 120-line cap; pure transformation, no behavior change.
fn build_api_messages(
    system_prompt: &str,
    conversation: &[ChatMessage],
) -> anyhow::Result<Vec<Value>> {
    let mut messages: Vec<Value> = vec![json!({
        "role": "system",
        "content": system_prompt
    })];

    for msg in conversation.iter() {
        let mut m = json!({"role": msg.role});
        if let Some(ref content) = msg.content {
            m["content"] = json!(content);
        }
        if let Some(ref tool_calls) = msg.tool_calls {
            m["tool_calls"] = serde_json::to_value(tool_calls)?;
        }
        if let Some(ref tool_call_id) = msg.tool_call_id {
            m["tool_call_id"] = json!(tool_call_id);
        }
        if let Some(ref name) = msg.name {
            m["name"] = json!(name);
        }
        messages.push(m);
    }

    Ok(messages)
}

/// Execute each tool call from one assistant turn: apply the delete-intent
/// gate, run the tool, push progress events + the tool-result message into the
/// conversation, and record each `ToolAction`. Extracted from `run_agent` to
/// keep it under the 120-line cap; the loop body is unchanged.
async fn execute_tool_calls(
    tool_calls: &[super::client::ResponseToolCall],
    conversation: &mut Vec<ChatMessage>,
    actions: &mut Vec<ToolAction>,
    state: &AppState,
    char_limit: u32,
    original_user_message: &str,
    progress_tx: Option<&tokio::sync::mpsc::UnboundedSender<ProgressEvent>>,
) {
    for tc in tool_calls {
        // Delete-intent gate: block any delete_* tool unless either the
        // current user message OR a deferred-intent pattern across prior
        // user messages signals an explicit delete request. Prevents
        // model hallucinations from causing data loss while still
        // allowing the "user asks to delete → AI defers for confirmation
        // → user replies 'yes'" workflow that the single-message gate
        // mistakenly blocked (#310). See spec
        // docs/superpowers/specs/2026-04-11-ai-mode-cleanup-design.md
        if tc.function.name.starts_with("delete_")
            && !delete_intent_for_turn(original_user_message, conversation)
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

            if let Some(tx) = progress_tx {
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

        // Send progress: tool starting
        if let Some(tx) = progress_tx {
            let _ = tx.send(ProgressEvent::ToolStart {
                tool: tc.function.name.clone(),
            });
        }

        let (result, preview) = match super::tools::execute_tool(
            &tc.function.name,
            &tc.function.arguments,
            state,
            char_limit,
        )
        .await
        {
            Ok((result, preview)) => {
                // Send progress: tool done
                if let Some(tx) = progress_tx {
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
                if let Some(tx) = progress_tx {
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

        // Add tool result to conversation (preview is internal-only,
        // not sent to the LLM — it's used for UI badges after reload).
        conversation.push(ChatMessage {
            role: "tool".to_string(),
            content: Some(result),
            tool_calls: None,
            tool_call_id: Some(tc.id.clone()),
            name: Some(tc.function.name.clone()),
            preview: Some(preview),
        });
    }
}

/// Run the agentic loop: send to LLM, execute tools, repeat until text response.
///
/// If `progress_tx` is provided, sends real-time progress events for each tool execution.
pub async fn run_agent(
    user_message: &str,
    conversation: &mut Vec<ChatMessage>,
    state: &AppState,
    settings: &AiSettings,
    progress_tx: Option<tokio::sync::mpsc::UnboundedSender<ProgressEvent>>,
) -> anyhow::Result<(String, Vec<ToolAction>)> {
    let (system_prompt, char_limit) =
        build_system_prompt(state, settings.system_prompt_extra.as_deref()).await;
    let tools = super::tools::tool_definitions();
    let mut actions = Vec::new();

    // Add user message to conversation
    conversation.push(ChatMessage {
        role: "user".to_string(),
        content: Some(user_message.to_string()),
        tool_calls: None,
        tool_call_id: None,
        name: None,
        preview: None,
    });

    // Capture the user's original message for the delete-intent gate.
    // The gate runs on every delete_* tool call during this turn.
    let original_user_message = user_message.to_string();

    for iteration in 0..MAX_ITERATIONS {
        let messages = build_api_messages(&system_prompt, conversation)?;

        info!(iteration, "AI agent loop iteration");
        let response =
            super::client::call_chat_completions(&messages, Some(&tools), settings).await?;

        let choice = response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("no choices in AI response"))?;

        let msg = choice.message;

        // Check for tool calls
        if let Some(ref tool_calls) = msg.tool_calls {
            if !tool_calls.is_empty() {
                // Add assistant message with tool calls to conversation
                let tc_messages: Vec<ToolCallMessage> = tool_calls
                    .iter()
                    .map(|tc| ToolCallMessage {
                        id: tc.id.clone(),
                        call_type: tc.call_type.clone(),
                        function: ToolCallFunction {
                            name: tc.function.name.clone(),
                            arguments: tc.function.arguments.clone(),
                        },
                    })
                    .collect();

                conversation.push(ChatMessage {
                    role: "assistant".to_string(),
                    content: msg.content.clone(),
                    tool_calls: Some(tc_messages),
                    tool_call_id: None,
                    name: None,
                    preview: None,
                });

                execute_tool_calls(
                    tool_calls,
                    conversation,
                    &mut actions,
                    state,
                    char_limit,
                    &original_user_message,
                    progress_tx.as_ref(),
                )
                .await;

                continue; // Call LLM again with tool results
            }
        }

        // Text response — we're done
        let response_text = msg.content.unwrap_or_default();
        conversation.push(ChatMessage {
            role: "assistant".to_string(),
            content: Some(response_text.clone()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
            preview: None,
        });

        // Trim conversation to last 10 user turns, preserving tool_call/result
        // pairs. A "turn" is user msg + subsequent assistant/tool messages.
        trim_conversation(conversation, 10);

        return Ok((response_text, actions));
    }

    Ok((
        "I reached the maximum number of processing steps. Please try a simpler request."
            .to_string(),
        actions,
    ))
}

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

#[cfg(test)]
mod tests {
    use super::*;

    // --- helpers for trim tests ---

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

    fn assistant_tool_call_msg(id: &str, name: &str) -> ChatMessage {
        ChatMessage {
            role: "assistant".to_string(),
            content: None,
            tool_calls: Some(vec![ToolCallMessage {
                id: id.to_string(),
                call_type: "function".to_string(),
                function: ToolCallFunction {
                    name: name.to_string(),
                    arguments: "{}".to_string(),
                },
            }]),
            tool_call_id: None,
            name: None,
            preview: None,
        }
    }

    fn tool_result_msg(id: &str, name: &str, result: &str) -> ChatMessage {
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
        let mut conv = Vec::new();
        for i in 0..15 {
            conv.push(user_msg(&format!("msg {}", i)));
            conv.push(assistant_msg(&format!("reply {}", i)));
        }
        trim_conversation(&mut conv, 5);
        // Should keep only the last 5 user turns = 10 messages
        assert_eq!(conv.len(), 10);
        assert_eq!(conv[0].role, "user");
        assert_eq!(conv[0].content.as_deref(), Some("msg 10"));
        assert_eq!(conv[conv.len() - 1].role, "assistant");
        assert_eq!(conv[conv.len() - 1].content.as_deref(), Some("reply 14"));
    }

    #[test]
    fn trim_never_orphans_tool_result() {
        // Each turn: user → assistant_tool_call → tool_result → assistant_text
        let mut conv = Vec::new();
        for i in 0..12 {
            conv.push(user_msg(&format!("user {}", i)));
            conv.push(assistant_tool_call_msg(&format!("call_{}", i), "test_tool"));
            conv.push(tool_result_msg(&format!("call_{}", i), "test_tool", "{}"));
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
        // 5 user turns but each has 50 tool calls = ~252 messages total
        let mut conv = Vec::new();
        for turn in 0..5 {
            conv.push(user_msg(&format!("user {}", turn)));
            for call in 0..50 {
                let id = format!("turn_{}_call_{}", turn, call);
                conv.push(assistant_tool_call_msg(&id, "test_tool"));
                conv.push(tool_result_msg(&id, "test_tool", "{}"));
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

    // --- delete_intent tests ---

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
        assert!(delete_intent_allowed(
            "I want to delete the test and make a new one"
        ));
        assert!(delete_intent_allowed(
            "please vymaž everything from yesterday"
        ));
    }

    #[test]
    fn turn_intent_grants_when_prior_user_message_asked_to_delete() {
        // Regression #310: user said "vymaž to" in turn N, AI asked
        // "potvrdzujete?" in its text response, user replied "ano" in turn
        // N+1. Without conversation context the gate sees only "ano" which
        // has no delete keyword and blocks. The cross-turn gate must walk
        // back through prior user messages and grant intent when a recent
        // one contained an explicit delete keyword AND the current message
        // is affirmative.
        let convo = vec![
            user_msg("vymaž tie dve prezentácie"),
            assistant_msg("Potvrdzujete, že chcete vymazať tie dve prezentácie?"),
            user_msg("ano"),
        ];
        assert!(
            delete_intent_for_turn("ano", &convo),
            "deferred delete intent + affirmative reply must grant the gate"
        );
    }

    #[test]
    fn turn_intent_grants_when_current_message_has_keyword() {
        // Direct path: gate must still grant when the current message itself
        // contains a delete keyword, regardless of conversation history.
        let convo = vec![user_msg("delete the slide")];
        assert!(delete_intent_for_turn("delete the slide", &convo));
    }

    #[test]
    fn turn_intent_blocks_when_no_user_message_ever_asked_to_delete() {
        // No prior message had delete intent — affirmative alone is not
        // enough. Prevents the AI from inventing a delete then asking
        // "should I?" then proceeding on "yes" without the user ever
        // having actually asked.
        let convo = vec![
            user_msg("create a new presentation"),
            assistant_msg("Should I delete the old one first?"),
            user_msg("yes"),
        ];
        assert!(
            !delete_intent_for_turn("yes", &convo),
            "affirmative without prior user delete intent must NOT grant the gate"
        );
    }

    #[test]
    fn turn_intent_does_not_grant_on_stale_intent_far_back_in_history() {
        // Reviewer concern: unbounded lookback means a "delete X" from 30
        // turns ago can be unlocked by an affirmative reply to a totally
        // unrelated current question. The deferred-intent window must be
        // bounded — only the most recent few user turns count.
        let mut convo = vec![user_msg("vymaž tie staré slajdy")];
        // 5 unrelated turns after the original delete request.
        for i in 0..5 {
            convo.push(assistant_msg(&format!("Hotovo {i}")));
            convo.push(user_msg(&format!("now make a new song presentation {i}")));
        }
        // AI now asks "delete the cover slide?" and user replies yes —
        // the user's CURRENT yes refers to the AI's question, NOT the
        // long-ago delete. Gate must block.
        convo.push(assistant_msg("Should I delete the cover slide?"));
        convo.push(user_msg("ano"));
        assert!(
            !delete_intent_for_turn("ano", &convo),
            "stale delete intent from >3 turns ago must NOT grant on current affirmative"
        );
    }

    #[test]
    fn turn_intent_blocks_when_neither_signal_present() {
        let convo = vec![user_msg("make a song presentation about hope")];
        assert!(!delete_intent_for_turn(
            "make a song presentation about hope",
            &convo
        ));
    }

    #[test]
    fn delete_intent_gate_produces_correct_tool_names() {
        // Verify the gate's prefix match covers all delete_* tools by name.
        assert!("delete_presentation".starts_with("delete_"));
        assert!("delete_library".starts_with("delete_"));
        assert!("delete_slide".starts_with("delete_"));
        assert!("delete_bible_presentation".starts_with("delete_"));
        assert!("delete_bible_slide".starts_with("delete_"));
        assert!(!"create_presentation".starts_with("delete_"));
        assert!(!"update_slide".starts_with("delete_"));
        assert!(!"trigger_slide".starts_with("delete_"));
    }

    // --- #434: system prompt reflects post-#394 whole-verse behavior ---

    #[test]
    fn system_prompt_describes_post_394_whole_verse_behavior() {
        // The prompt's bible-slide guidance must match the shipped #394 composer
        // behavior: a lone oversized verse is ACCEPTED WHOLE automatically (the
        // model does not split it), and same-number continuation items are MERGED
        // into one whole verse. It must NOT carry the pre-#394 "split that verse
        // into multiple items, emitted as separate slides" recovery — that
        // instruction now misleads the model.
        let prompt = format_system_prompt("(none)", "(none yet)", "SEB", 320);

        // Stale wording must be gone.
        assert!(
            !prompt.contains("split that verse into multiple verse items"),
            "system prompt still tells the model to split a verse into multiple items (pre-#394)"
        );
        assert!(
            !prompt.contains("emit them as separate slides"),
            "system prompt still says same-number items are emitted as separate slides (pre-#394)"
        );

        // New behavior must be described: lone oversized verse accepted whole.
        let lower = prompt.to_lowercase();
        assert!(
            lower.contains("accepted whole") || lower.contains("kept whole"),
            "system prompt must state a lone oversized verse is accepted/kept whole (post-#394)"
        );
        // And same-number continuation items merge into one whole verse.
        assert!(
            lower.contains("merged") || lower.contains("merge"),
            "system prompt must state same-number continuation items are merged (post-#394)"
        );
    }
}
