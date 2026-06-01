//! Bible-presentation CRUD tools (server-side slide composition).

use super::{str_field, uuid_field, validation_error_response};
use crate::ai::bible_validator::validate_bible_slide;
use crate::state::slides::{compose_bible_items_into_slides, BibleItem, ComposedBibleSlide};
use crate::state::AppState;
use presenter_core::slide::SlideText;
use presenter_core::{BiblePresentationId, BiblePresentationSlide, BibleSlideId};
use serde_json::{json, Value};

pub(super) async fn list_bible_presentations(state: &AppState) -> anyhow::Result<(String, String)> {
    let summaries = state.list_bible_presentations().await?;
    let list: Vec<Value> = summaries
        .iter()
        .map(|s| {
            json!({
                "id": s.id.to_string(),
                "name": s.name,
                "slide_count": s.slide_count,
            })
        })
        .collect();
    let preview = format!("Found {} bible presentations", summaries.len());
    Ok((serde_json::to_string(&list)?, preview))
}

pub(super) async fn get_bible_presentation(
    args: &Value,
    state: &AppState,
) -> anyhow::Result<(String, String)> {
    let pres_id = BiblePresentationId::from_uuid(uuid_field(args, "presentation_id")?);
    match state.bible_presentation_detail(pres_id).await? {
        Some(p) => {
            let slides: Vec<Value> = p
                .slides
                .iter()
                .map(|s| {
                    json!({
                        "id": s.id.to_string(),
                        "order": s.order,
                        "main": s.main.value(),
                        "main_reference": s.main_reference,
                        "secondary": s.secondary.value(),
                        "secondary_reference": s.secondary_reference,
                    })
                })
                .collect();
            let preview = format!("'{}' - {} slides", p.name, p.slides.len());
            Ok((
                json!({
                    "id": p.id.to_string(),
                    "name": p.name,
                    "slides": slides,
                })
                .to_string(),
                preview,
            ))
        }
        None => Ok((
            json!({"error": "bible presentation not found"}).to_string(),
            "Not found".to_string(),
        )),
    }
}

pub(super) async fn create_bible_presentation(
    args: &Value,
    state: &AppState,
    default_char_limit: u32,
) -> anyhow::Result<(String, String)> {
    let name = str_field(args, "name")?;
    let items_arr = match args["items"].as_array() {
        Some(arr) => arr,
        None => {
            return Ok((
                json!({
                    "error": "missing_items",
                    "expected": "items must be an array of verse/emphasis objects",
                })
                .to_string(),
                "Missing items array".to_string(),
            ));
        }
    };

    // Parse items into typed BibleItem values. Fail fast on any
    // malformed item — the LLM sees the error and retries.
    let mut items: Vec<BibleItem> = Vec::with_capacity(items_arr.len());
    for (idx, raw) in items_arr.iter().enumerate() {
        let kind = raw["kind"].as_str().unwrap_or("");
        match kind {
            "verse" => {
                // try_from(u64->u32): any u64 that does not fit in u32
                // collapses to 0 and is rejected by the number>=1 /
                // chapter>=1 checks below. Avoids silent truncation
                // of e.g. 2^33+5 → 5 that a raw `as u32` cast would do.
                let number = u32::try_from(raw["number"].as_u64().unwrap_or(0)).unwrap_or(0);
                let text = raw["text"].as_str().unwrap_or("").to_string();
                let book = raw["book"].as_str().unwrap_or("").to_string();
                let chapter = u32::try_from(raw["chapter"].as_u64().unwrap_or(0)).unwrap_or(0);
                let translation = raw["translation"].as_str().unwrap_or("").to_string();
                if number == 0
                    || text.is_empty()
                    || book.is_empty()
                    || chapter == 0
                    || translation.is_empty()
                {
                    return Ok((
                        json!({
                            "error": "invalid_verse_item",
                            "expected": "verse items require number>=1, non-empty text, book, chapter>=1, translation",
                            "got": format!("item[{idx}]"),
                        })
                        .to_string(),
                        format!("Invalid verse item at index {idx}"),
                    ));
                }
                items.push(BibleItem::Verse {
                    number,
                    text,
                    book,
                    chapter,
                    translation,
                });
            }
            "emphasis" => {
                let text = raw["text"].as_str().unwrap_or("").to_string();
                if text.trim().is_empty() {
                    return Ok((
                        json!({
                            "error": "invalid_emphasis_item",
                            "expected": "emphasis items require non-empty text",
                            "got": format!("item[{idx}]"),
                        })
                        .to_string(),
                        format!("Invalid emphasis item at index {idx}"),
                    ));
                }
                items.push(BibleItem::Emphasis { text });
            }
            other => {
                return Ok((
                    json!({
                        "error": "invalid_item_kind",
                        "expected": "kind must be 'verse' or 'emphasis'",
                        "got": format!("item[{idx}] kind={other}"),
                    })
                    .to_string(),
                    format!("Invalid kind '{other}' at index {idx}"),
                ));
            }
        }
    }

    // Compose slides server-side using the configured character limit.
    let composed: Vec<ComposedBibleSlide> =
        compose_bible_items_into_slides(&items, default_char_limit);

    // Validate each composed slide. With a correct composer only
    // the oversized-single-verse case should ever trip this.
    for (idx, slide) in composed.iter().enumerate() {
        if let Err(mut err) =
            validate_bible_slide(&slide.main, &slide.main_reference, default_char_limit)
        {
            err.got = format!("composed_slide[{idx}]: {}", err.got);
            return Ok(validation_error_response(err));
        }
    }

    // Persist. Empty items[] produces an empty presentation, which
    // is used intentionally by some tests (and by the operator UI
    // when a user wants a blank bible presentation to populate by
    // hand). The LLM should submit non-empty items in practice;
    // there is no explicit rejection because the prompt guides it
    // and an empty presentation is harmless.
    let presentation = state.create_bible_presentation(&name).await?;
    let final_presentation = if composed.is_empty() {
        presentation
    } else {
        // SlideText::new only fails at ~4000 chars; the validator
        // has already guaranteed slide.main.len() <= default_char_limit
        // (typically 320) above, so this is statically unreachable.
        // Propagate the error anyway so a future limit change does
        // not silently drop slide content through an unwrap_or fallback.
        let mut new_slides: Vec<BiblePresentationSlide> = Vec::with_capacity(composed.len());
        for c in composed {
            let main = SlideText::new(&c.main)
                .map_err(|e| anyhow::anyhow!("composed slide main SlideText failed: {e}"))?;
            let secondary =
                SlideText::new("").map_err(|e| anyhow::anyhow!("empty SlideText failed: {e}"))?;
            new_slides.push(BiblePresentationSlide {
                id: BibleSlideId::new(),
                order: 0,
                main,
                main_reference: c.main_reference,
                secondary,
                secondary_reference: String::new(),
                metadata: None,
            });
        }
        state
            .append_bible_presentation_slides(presentation.id, new_slides)
            .await?
    };

    let preview = format!(
        "Created bible presentation '{}' with {} slides",
        final_presentation.name,
        final_presentation.slides.len()
    );
    Ok((
        json!({
            "id": final_presentation.id.to_string(),
            "name": final_presentation.name,
            "slide_count": final_presentation.slides.len(),
        })
        .to_string(),
        preview,
    ))
}

pub(super) async fn rename_bible_presentation(
    args: &Value,
    state: &AppState,
) -> anyhow::Result<(String, String)> {
    let pres_id = BiblePresentationId::from_uuid(uuid_field(args, "presentation_id")?);
    let name = str_field(args, "name")?;
    state.rename_bible_presentation(pres_id, &name).await?;
    let preview = format!("Renamed bible presentation to '{name}'");
    Ok((json!({"ok": true}).to_string(), preview))
}

pub(super) async fn delete_bible_presentation(
    args: &Value,
    state: &AppState,
) -> anyhow::Result<(String, String)> {
    let pres_id = BiblePresentationId::from_uuid(uuid_field(args, "presentation_id")?);
    state.delete_bible_presentation(pres_id).await?;
    Ok((
        json!({"ok": true}).to_string(),
        "Deleted bible presentation".to_string(),
    ))
}

pub(super) async fn delete_bible_slide(
    args: &Value,
    state: &AppState,
) -> anyhow::Result<(String, String)> {
    let pres_id = BiblePresentationId::from_uuid(uuid_field(args, "presentation_id")?);
    let slide_id = BibleSlideId::from_uuid(uuid_field(args, "slide_id")?);
    state.delete_bible_slide(pres_id, slide_id).await?;
    Ok((
        json!({"ok": true}).to_string(),
        "Deleted bible slide".to_string(),
    ))
}
