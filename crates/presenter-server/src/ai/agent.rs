use super::{AiSettings, ChatMessage, ToolAction, ToolCallFunction, ToolCallMessage};
use crate::state::AppState;
use serde_json::{json, Value};
use tracing::{info, warn};

const MAX_ITERATIONS: usize = 100;

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
async fn build_system_prompt(state: &AppState, extra: Option<&str>) -> (String, u32) {
    let translations = state.list_bible_translations().await.unwrap_or_default();
    let translation_list: Vec<String> = translations
        .iter()
        .map(|t| format!("{} ({})", t.code, t.name))
        .collect();

    let libraries = state.libraries().await.unwrap_or_default();
    let library_list: Vec<String> = libraries
        .iter()
        .map(|l| format!("- {} (id: {})", l.name, l.id))
        .collect();

    let prefs = state.get_bible_preferences().await.unwrap_or_default();
    let char_limit = prefs.character_limit;

    let mut prompt = format!(
        r#"You are an AI assistant for Presenter, a church worship presentation system for a Slovak church.
You have full access to manage presentations, slides, Bible passages, and stage display.

## Available Bible Translations
{translations}

## Available Libraries
{libraries}

When creating presentations, choose the most appropriate library:
- For Bible verse presentations, prefer a library with "Bible" or "Biblia" in its name
- For worship songs, use the appropriate worship/song library
- If no matching library exists, create one with an appropriate name

## Slide Field Usage (CRITICAL — follow exactly)
- `main`: Verse text prefixed with the verse number. Format: "1. Verse text here" or "27 Verse text here". NEVER include the reference in main.
- `translation`: Leave empty unless bilingual.
- `stage`: Reference WITH translation code. Example: "Žalm 26:1 (ROH)". ALWAYS include the code in parentheses.
- `group`: Same as stage — reference with translation code. Example: "Žalm 26:1 (ROH)".

## Reference Format (MANDATORY — never omit the translation code)
- Single verse: "Žalm 26:1 (ROH)"
- Verse range: "Marek 3:14-15 (SEB)"
- Partial verse: "Žalm 26:3a (ROH)"
- The code in parentheses is REQUIRED. Without it, Resolume cannot display the reference correctly.

## Multi-Slide Passages (CRITICAL)
When a Bible passage is split across multiple slides, ALL slides from that passage MUST use the SAME full reference in `stage` and `group` — the complete verse range from start to end.

Example: Psalm 52:1-11 split into 4 slides:
- Slide 1 (vv 1-3): stage = "Žalm 52:1-11 (ROH)" ← FULL range, not "52:1-3"
- Slide 2 (vv 4-6): stage = "Žalm 52:1-11 (ROH)" ← same
- Slide 3 (vv 7-9): stage = "Žalm 52:1-11 (ROH)" ← same
- Slide 4 (vv 10-11): stage = "Žalm 52:1-11 (ROH)" ← same

WRONG: Using per-slide ranges like "Žalm 52:1-3", "Žalm 52:4-6" — this makes each slide look like a separate passage.

## Formatting Rules — ## markers (bold text from email)
The pastor bolds text in emails. Bold text arrives wrapped in ## markers. Handle them by context:

1. **##reference## (e.g. ##Mt26:26-29##, ##Rim5:17##):** This is a bold section header — the pastor bolds references for readability. Do NOT create a slide for it. Just use it to identify which Bible passage follows.
2. **##title## at the very start (e.g. ##Nová zmluva##):** This is the presentation title. Use it as the presentation name.
3. **##word## inside a verse (e.g. "aby sme ##verili## menu"):** The pastor emphasizes a word. Make that word UPPERCASE *within* the verse slide's main text. Do NOT create a separate emphasis slide.
4. **##phrase## as a standalone line (not a reference, not inside a verse):** Create a separate emphasis slide with main = phrase in UPPERCASE, group = "Zvýraznenie".

CRITICAL: Do NOT create separate "Zvýraznenie" slides for bold references or bold words inside verses. Only standalone bold phrases that are not Bible references get their own emphasis slide.

## Slide Size Rules (CRITICAL — follow exactly)
Character limit per slide: {char_limit} characters in `main`.

**ALWAYS pack multiple verses onto one slide.** One verse per slide is WRONG. Keep adding verses to the current slide until the next verse would exceed {char_limit}. Only then start a new slide.

Example with limit 200:
- Verse 1 is 70 chars → slide has 70 chars, room for more
- Verse 2 is 40 chars → slide has 110 chars (70+40), room for more
- Verse 3 is 80 chars → slide has 190 chars (110+80), room is tight
- Verse 4 is 50 chars → 190+50=240 > 200, so start NEW slide with verse 4

Result: slide 1 = "1. ...\n2. ...\n3. ...", slide 2 = "4. ..."

If a single verse exceeds {char_limit}, split that verse at a natural sentence boundary.

## Other Formatting Rules
- Text written in ALL CAPS by the pastor = keep it uppercase in `main`.
- "Nazov:" or "Názov:" = presentation title.
- "Vers na spamet:" = memory verse, use group "Vers na zapamätanie".

## Common Slovak Bible Abbreviations
Ž/Žalm=Žalmy, Žid=Židom, 1Sa=1. Samuelova, 1Kra=1. Kráľov, 2Ti=2. Timotejovi,
Mat/Mt=Matúš, Mar/Mr=Marek, Luk=Lukáš, Ján/Jan=Ján, Sk=Skutky, Rim=Rimanom,
1Kor=1. Korinťanom, 2Kor=2. Korinťanom, Gal=Galatským, Ef=Efezským,
Fil=Filipským, Kol=Kolosanom, 1Sol=1. Solúnčanom, 1Tim=1. Timotejovi,
Tít=Títovi, Flm=Filemonovi, Prísl=Príslovia, Iz=Izaiáš,
Jer=Jeremiáš, Ez=Ezechiel, Dan=Daniel, 1Pet=1. Petra, 2Pet=2. Petra

## Translation Code Mapping
SEB=slk-seb, ROH=slk-roh, SEVP=slk-sevp, MIL=slk-mil, KJV=eng-kjv, ECAV=slk-sevp

## Workflow for Pastor's Messages
1. Parse input for Bible references, plain text, titles, emphasis markers.
2. Detect the translation:
   - If a translation label appears after a reference (e.g. "Luk6:45 ECAV") → use that translation.
   - If the pastor provides verse text, use search_bible with a short snippet from the first verse to identify which translation matches. Compare the result codes.
   - If you cannot determine the translation, ask the user before creating slides.
3. For simple Bible references (single verse or contiguous range): use resolve_bible_slides — it returns slides with correct stage/group fields including the translation code. Use these values directly.
4. For complex references (partial verses like "3a", non-contiguous ranges, or when the pastor provides their own text): create slides manually BUT always include the translation code in stage and group (e.g. "Žalm 26:1 (ROH)").
5. Create ALL slides in one create_presentation call in the Bible library.
6. Confirm what was created with a brief summary.

## Important
- Do NOT call list_bible_translations or list_libraries — they are listed above.
- Use the user's provided text when available — the pastor may use a specific emphasis or paraphrase.
- Never duplicate the reference in the main text field.
- Respond in the same language the user writes in."#,
        translations = translation_list.join(", "),
        libraries = library_list.join("\n"),
    );

    if let Some(extra_prompt) = extra {
        if !extra_prompt.is_empty() {
            prompt.push_str("\n\n## Additional Instructions\n");
            prompt.push_str(extra_prompt);
        }
    }

    (prompt, char_limit)
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
        // Build messages array for API call
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

                    // Send progress: tool starting
                    if let Some(ref tx) = progress_tx {
                        let _ = tx.send(ProgressEvent::ToolStart {
                            tool: tc.function.name.clone(),
                        });
                    }

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
                        preview: None,
                    });
                }

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
}
