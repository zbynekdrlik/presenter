//! AI tool dispatch.
//!
//! [`execute_tool`] is a thin dispatcher that routes a tool name to a per-domain
//! handler in one of the submodules (`library`, `slides`, `bible`,
//! `bible_presentation`, `misc`). Argument/serialization helpers shared across
//! those handlers live here so each submodule stays focused on its tools.

use super::bible_validator::ValidationError;
use crate::state::AppState;
use presenter_core::slide::{SlideContent, SlideText};
use presenter_core::Slide;
use serde_json::{json, Value};
use uuid::Uuid;

pub use super::tool_defs::tool_definitions;

mod bible;
mod bible_presentation;
mod library;
mod misc;
mod slides;

#[cfg(test)]
mod tests;

fn slide_to_json(s: &Slide) -> Value {
    json!({
        "id": s.id.to_string(),
        "order": s.order,
        "main": s.content.main.value(),
        "translation": s.content.translation.value(),
        "stage": s.content.stage.value(),
        "group": s.content.group.as_ref().map(|g| g.name())
    })
}

fn make_slide(i: usize, s: &Value) -> Slide {
    let main_text = SlideText::new(s["main"].as_str().unwrap_or(""))
        .unwrap_or_else(|_| SlideText::new("").unwrap());
    let translation_text = SlideText::new(s["translation"].as_str().unwrap_or(""))
        .unwrap_or_else(|_| SlideText::new("").unwrap());
    let stage_text = SlideText::new(s["stage"].as_str().unwrap_or(""))
        .unwrap_or_else(|_| SlideText::new("").unwrap());
    let group = s["group"]
        .as_str()
        .map(presenter_core::slide::SlideGroup::new);

    let content = SlideContent::new(main_text, translation_text, stage_text, group);
    Slide::new(i as u32, content)
}

/// Convert a slide validation error into the tool-result tuple
/// `(result_json_string, preview_string)` used by the tool dispatch path.
/// The `preview` is short for UI badges; the full error JSON is sent back
/// to the LLM as the tool result content so it can self-correct on retry.
fn validation_error_response(err: ValidationError) -> (String, String) {
    // Truncate the offending string so the AI conversation preview stays
    // readable. The full string still goes back to the LLM via the JSON
    // body and to the server log via tracing::warn!.
    let truncated_got: String = err.got.chars().take(80).collect();
    let preview = if err.got.chars().count() > 80 {
        format!(
            "Validation failed: {} (got: '{}...')",
            err.rule.as_str(),
            truncated_got
        )
    } else {
        format!(
            "Validation failed: {} (got: '{}')",
            err.rule.as_str(),
            truncated_got
        )
    };
    tracing::warn!(
        rule = %err.rule.as_str(),
        got = %err.got,
        "bible slide validation rejected AI output"
    );
    (err.to_json().to_string(), preview)
}

/// Execute a tool call against AppState and return (result_json, preview).
pub async fn execute_tool(
    name: &str,
    args: &str,
    state: &AppState,
    default_char_limit: u32,
) -> anyhow::Result<(String, String)> {
    let args: Value = serde_json::from_str(args).unwrap_or(json!({}));

    match name {
        "list_libraries" => library::list_libraries(state).await,
        "create_library" => library::create_library(&args, state).await,
        "list_presentations" => library::list_presentations(&args, state).await,
        "get_presentation" => library::get_presentation(&args, state).await,
        "create_presentation" => library::create_presentation(&args, state).await,
        "rename_presentation" => library::rename_presentation(&args, state).await,
        "delete_presentation" => library::delete_presentation(&args, state).await,

        "add_slide" => slides::add_slide(&args, state).await,
        "update_slide" => slides::update_slide(&args, state).await,
        "delete_slide" => slides::delete_slide(&args, state).await,
        "reorder_slides" => slides::reorder_slides(&args, state).await,
        "trigger_slide" => slides::trigger_slide(&args, state).await,
        "clear_stage" => slides::clear_stage(state).await,

        "search_bible" => bible::search_bible(&args, state).await,
        "get_bible_passage" => bible::get_bible_passage(&args, state).await,
        "list_bible_translations" => bible::list_bible_translations(state).await,
        "load_bible_verses" => bible::load_bible_verses(&args, state).await,
        "resolve_bible_slides" => {
            bible::resolve_bible_slides(&args, state, default_char_limit).await
        }
        "trigger_bible_verse" => bible::trigger_bible_verse(&args, state).await,

        "get_style_guide" => misc::get_style_guide().await,

        "list_bible_presentations" => bible_presentation::list_bible_presentations(state).await,
        "get_bible_presentation" => bible_presentation::get_bible_presentation(&args, state).await,
        "create_bible_presentation" => {
            bible_presentation::create_bible_presentation(&args, state, default_char_limit).await
        }
        "rename_bible_presentation" => {
            bible_presentation::rename_bible_presentation(&args, state).await
        }
        "delete_bible_presentation" => {
            bible_presentation::delete_bible_presentation(&args, state).await
        }
        "delete_bible_slide" => bible_presentation::delete_bible_slide(&args, state).await,

        _ => Ok((
            json!({"error": format!("unknown tool: {name}")}).to_string(),
            format!("Unknown tool: {name}"),
        )),
    }
}

fn str_field(args: &Value, field: &str) -> anyhow::Result<String> {
    args[field]
        .as_str()
        .map(String::from)
        .ok_or_else(|| anyhow::anyhow!("missing required field: {field}"))
}

fn uuid_field(args: &Value, field: &str) -> anyhow::Result<Uuid> {
    let s = str_field(args, field)?;
    Uuid::parse_str(&s).map_err(|_| anyhow::anyhow!("{field} must be a valid UUID"))
}

/// Extract a required integer field. Errors (rather than silently defaulting)
/// when the field is absent or not an unsigned integer, so the model gets
/// explicit feedback and can self-correct on retry.
fn u64_field(args: &Value, field: &str) -> anyhow::Result<u64> {
    args[field]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("missing or invalid required integer field: {field}"))
}
