use super::{AiSettings, ChatMessage, ToolAction, ToolCallFunction, ToolCallMessage};
use crate::state::AppState;
use serde_json::{json, Value};
use tracing::{info, warn};

const MAX_ITERATIONS: usize = 100;

/// Build the system prompt with dynamic content from the database.
async fn build_system_prompt(state: &AppState, extra: Option<&str>) -> String {
    let translations = state.list_bible_translations().await.unwrap_or_default();
    let translation_list: Vec<String> = translations
        .iter()
        .map(|t| format!("{} ({})", t.code, t.name))
        .collect();

    let mut prompt = format!(
        r#"You are an AI assistant for Presenter, a church worship presentation system for a Slovak church.
You have full access to manage presentations, slides, Bible passages, and stage display.

## Available Bible Translations
{translations}

## Slide Rules
- Each slide has: main text (displayed), translation text (secondary language), stage text (confidence monitor), group (section label like "Verse 1", "Chorus")
- Maximum 4000 characters per field, but aim for ~320 characters per slide for readability
- When creating Bible presentations, use resolve_bible_slides to automatically split by character limit
- For non-Bible text slides, split content so each slide has ~320 characters max

## Pastor's Message Conventions
- "Nazov:" or "Názov:" = presentation title
- "Vers na spamet:" = memory verse (use group "Vers na zapamätanie")
- ##text## = emphasized/big text (put on its own slide, mark group as "Zvýraznenie")
- Bible references like "Žid 4:13 SEB" mean: book=Židom, chapter=4, verse=13, translation=SEB

## Common Slovak Bible Abbreviations
Žid=Židom (Hebrews), 1Sa=1. Samuelova, 1Kra=1. Kráľov, 2Ti=2. Timotejovi,
Mat=Matúš, Mar=Marek, Luk=Lukáš, Ján=Ján, Sk=Skutky, Rim=Rimanom,
1Kor=1. Korinťanom, 2Kor=2. Korinťanom, Gal=Galatským, Ef=Efezským,
Fil=Filipským, Kol=Kolosanom, 1Sol=1. Solúnčanom, 1Tim=1. Timotejovi,
Tít=Títovi, Flm=Filemonovi, Žalm=Žalmy, Prísl=Príslovia, Iz=Izaiáš,
Jer=Jeremiáš, Ez=Ezechiel, Dan=Daniel

## Translation Code Mapping
SEB=slk-seb, ROH=slk-roh, SEVP=slk-sevp, MIL=slk-mil, KJV=eng-kjv

## Workflow
1. Parse the user's message to identify Bible references, titles, and text
2. Look up Bible translations to verify which are available
3. For Bible references: use get_bible_passage or resolve_bible_slides to fetch actual verse text, then create slides
4. For plain text: create slides directly, splitting by ~320 char limit
5. Create the presentation with all slides in order
6. Confirm what was created

Always use the actual Bible text from the database, never reproduce from memory.
Respond in the same language the user writes in."#,
        translations = translation_list.join(", ")
    );

    if let Some(extra_prompt) = extra {
        if !extra_prompt.is_empty() {
            prompt.push_str("\n\n## Additional Instructions\n");
            prompt.push_str(extra_prompt);
        }
    }

    prompt
}

/// Run the agentic loop: send to LLM, execute tools, repeat until text response.
pub async fn run_agent(
    user_message: &str,
    conversation: &mut Vec<ChatMessage>,
    state: &AppState,
    settings: &AiSettings,
) -> anyhow::Result<(String, Vec<ToolAction>)> {
    let system_prompt = build_system_prompt(state, settings.system_prompt_extra.as_deref()).await;
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
                    let result = match super::tools::execute_tool(
                        &tc.function.name,
                        &tc.function.arguments,
                        state,
                    )
                    .await
                    {
                        Ok((result, preview)) => {
                            actions.push(ToolAction {
                                tool: tc.function.name.clone(),
                                result_preview: preview,
                            });
                            result
                        }
                        Err(err) => {
                            warn!(tool = %tc.function.name, ?err, "AI tool call failed");
                            actions.push(ToolAction {
                                tool: tc.function.name.clone(),
                                result_preview: format!("Error: {err}"),
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
