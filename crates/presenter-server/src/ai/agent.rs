use super::{AiSettings, ChatMessage, ToolAction, ToolCallFunction, ToolCallMessage};
use crate::state::AppState;
use serde_json::{json, Value};
use tracing::{info, warn};

const MAX_ITERATIONS: usize = 100;

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
    });

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
                });

                // Execute each tool call
                for tc in tool_calls {
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
        });

        // Trim conversation to max ~20 user/assistant pairs (keep system working)
        trim_conversation(conversation);

        return Ok((response_text, actions));
    }

    Ok((
        "I reached the maximum number of processing steps. Please try a simpler request."
            .to_string(),
        actions,
    ))
}

/// Keep conversation at a manageable size by trimming old messages.
fn trim_conversation(conversation: &mut Vec<ChatMessage>) {
    const MAX_MESSAGES: usize = 40;
    if conversation.len() > MAX_MESSAGES {
        let to_remove = conversation.len() - MAX_MESSAGES;
        conversation.drain(..to_remove);
    }
}
