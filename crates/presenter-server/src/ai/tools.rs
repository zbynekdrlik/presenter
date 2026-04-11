use super::bible_validator::{validate_bible_slide, ValidationError};
use crate::state::bible::BibleTriggerOverrides;
use crate::state::slides::{compose_bible_items_into_slides, BibleItem, ComposedBibleSlide};
use crate::state::AppState;
use presenter_core::slide::{SlideContent, SlideText};
use presenter_core::{
    BiblePresentationId, BiblePresentationSlide, BibleReference, BibleSlideId, LibraryId,
    PresentationId, Slide, SlideId,
};
use serde_json::{json, Value};
use uuid::Uuid;

pub use super::tool_defs::tool_definitions;

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
    let preview = format!("Validation failed: {}", err.rule.as_str());
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
        "list_libraries" => {
            let libs = state.libraries().await?;
            let summary: Vec<Value> = libs
                .iter()
                .map(|l| json!({"id": l.id.to_string(), "name": l.name}))
                .collect();
            let preview = format!("Found {} libraries", summary.len());
            Ok((serde_json::to_string(&summary)?, preview))
        }

        "create_library" => {
            let name_val = str_field(&args, "name")?;
            let lib = state.create_library(&name_val).await?;
            let preview = format!("Created library '{}'", lib.name);
            Ok((
                json!({"id": lib.id.to_string(), "name": lib.name}).to_string(),
                preview,
            ))
        }

        "list_presentations" => {
            let lib_id = LibraryId::from_uuid(uuid_field(&args, "library_id")?);
            let summaries = state.library_summaries(None).await?;
            let lib_summary = summaries.iter().find(|s| s.id == lib_id);
            let presentations: Vec<Value> = lib_summary
                .map(|ls| {
                    ls.presentations
                        .iter()
                        .map(|p| json!({"id": p.id.to_string(), "name": p.name}))
                        .collect()
                })
                .unwrap_or_default();
            let preview = format!("Found {} presentations", presentations.len());
            Ok((serde_json::to_string(&presentations)?, preview))
        }

        "get_presentation" => {
            let pres_id = PresentationId::from_uuid(uuid_field(&args, "presentation_id")?);
            let detail = state.presentation_detail(pres_id).await?;
            match detail {
                Some((lib_id, lib_name, pres)) => {
                    let slides: Vec<Value> = pres.slides.iter().map(|s| slide_to_json(s)).collect();
                    let preview = format!("'{}' - {} slides", pres.name, pres.slides.len());
                    Ok((
                        json!({
                            "id": pres.id.to_string(),
                            "name": pres.name,
                            "library_id": lib_id.to_string(),
                            "library_name": lib_name,
                            "slides": slides
                        })
                        .to_string(),
                        preview,
                    ))
                }
                None => Ok((
                    json!({"error": "presentation not found"}).to_string(),
                    "Not found".to_string(),
                )),
            }
        }

        "create_presentation" => {
            let lib_id = LibraryId::from_uuid(uuid_field(&args, "library_id")?);
            let name_val = str_field(&args, "name")?;
            let slides_arr = args["slides"].as_array();

            let slides: Option<Vec<Slide>> = slides_arr.map(|arr| {
                arr.iter()
                    .enumerate()
                    .map(|(i, s)| make_slide(i, s))
                    .collect()
            });

            let (_, _, pres, _) = state
                .create_presentation(lib_id, &name_val, slides.as_deref())
                .await?;
            let preview = format!("Created '{}' with {} slides", pres.name, pres.slides.len());
            Ok((
                json!({
                    "id": pres.id.to_string(),
                    "name": pres.name,
                    "slide_count": pres.slides.len()
                })
                .to_string(),
                preview,
            ))
        }

        "rename_presentation" => {
            let pres_id = PresentationId::from_uuid(uuid_field(&args, "presentation_id")?);
            let name_val = str_field(&args, "name")?;
            state.rename_presentation(pres_id, &name_val).await?;
            let preview = format!("Renamed to '{name_val}'");
            Ok((json!({"ok": true}).to_string(), preview))
        }

        "delete_presentation" => {
            let pres_id = PresentationId::from_uuid(uuid_field(&args, "presentation_id")?);
            state.delete_presentation(pres_id).await?;
            Ok((
                json!({"ok": true}).to_string(),
                "Deleted presentation".to_string(),
            ))
        }

        "add_slide" => {
            let pres_id = PresentationId::from_uuid(uuid_field(&args, "presentation_id")?);
            let position = args["position"].as_u64().map(|p| p as u32);
            let slides = state.insert_blank_slide(pres_id, position).await?;
            // Update the last inserted slide with content
            if let Some(slide) = slides.last() {
                let main = args["main"].as_str().unwrap_or("").to_string();
                let translation = args["translation"].as_str().unwrap_or("").to_string();
                let stage = args["stage"].as_str().unwrap_or("").to_string();
                let group = args["group"].as_str().map(String::from);
                state
                    .update_slide_content(pres_id, slide.id, main, translation, stage, group, None)
                    .await?;
            }
            let preview = format!("Added slide (now {} total)", slides.len());
            Ok((
                json!({"ok": true, "slide_count": slides.len()}).to_string(),
                preview,
            ))
        }

        "update_slide" => {
            let pres_id = PresentationId::from_uuid(uuid_field(&args, "presentation_id")?);
            let slide_id = SlideId::from_uuid(uuid_field(&args, "slide_id")?);
            let main = args["main"].as_str().unwrap_or("").to_string();
            let translation = args["translation"].as_str().unwrap_or("").to_string();
            let stage = args["stage"].as_str().unwrap_or("").to_string();
            let group = args["group"].as_str().map(String::from);
            state
                .update_slide_content(pres_id, slide_id, main, translation, stage, group, None)
                .await?;
            Ok((json!({"ok": true}).to_string(), "Updated slide".to_string()))
        }

        "delete_slide" => {
            let pres_id = PresentationId::from_uuid(uuid_field(&args, "presentation_id")?);
            let slide_id = SlideId::from_uuid(uuid_field(&args, "slide_id")?);
            let slides = state.delete_slide(pres_id, slide_id).await?;
            let preview = format!("Deleted slide ({} remaining)", slides.len());
            Ok((
                json!({"ok": true, "remaining": slides.len()}).to_string(),
                preview,
            ))
        }

        "reorder_slides" => {
            let pres_id = PresentationId::from_uuid(uuid_field(&args, "presentation_id")?);
            let ids: Vec<SlideId> = args["slide_ids"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .filter_map(|s| Uuid::parse_str(s).ok())
                        .map(SlideId::from_uuid)
                        .collect()
                })
                .unwrap_or_default();
            state.reorder_slides(pres_id, ids).await?;
            Ok((
                json!({"ok": true}).to_string(),
                "Reordered slides".to_string(),
            ))
        }

        "search_bible" => {
            let query = str_field(&args, "query")?;
            let translation = args["translation"].as_str();
            let passages = state
                .search_bible_passages_cross(translation, &query, 10)
                .await?;
            let results: Vec<Value> = passages
                .iter()
                .map(|p| {
                    json!({
                        "reference": format!("{} {}:{}", p.reference.book, p.reference.chapter, p.reference.verse_start),
                        "translation": p.translation.code,
                        "text": p.text
                    })
                })
                .collect();
            let preview = format!("Found {} passages", results.len());
            Ok((serde_json::to_string(&results)?, preview))
        }

        "get_bible_passage" => {
            let translation = str_field(&args, "translation")?;
            let book = str_field(&args, "book")?;
            let chapter = args["chapter"].as_u64().unwrap_or(1) as u16;
            let verse_start = args["verse_start"].as_u64().unwrap_or(1) as u16;
            let verse_end = args["verse_end"]
                .as_u64()
                .map(|v| v as u16)
                .unwrap_or(verse_start);

            let reference = BibleReference {
                book: book.clone(),
                book_code: None,
                book_number: None,
                chapter,
                verse_start,
                verse_end,
            };

            let passage = state.find_bible_passage(&translation, &reference).await?;
            match passage {
                Some(p) => {
                    let preview = format!("{} {}:{}", book, chapter, verse_start);
                    Ok((
                        json!({
                            "reference": format!("{} {}:{}", p.reference.book, p.reference.chapter, p.reference.verse_start),
                            "translation": p.translation.code,
                            "text": p.text
                        })
                        .to_string(),
                        preview,
                    ))
                }
                None => Ok((
                    json!({"error": "passage not found"}).to_string(),
                    "Passage not found".to_string(),
                )),
            }
        }

        "get_style_guide" => {
            let guide = include_str!("style_guide.md");
            Ok((guide.to_string(), "Style guide loaded".to_string()))
        }

        "list_bible_translations" => {
            let translations = state.list_bible_translations().await?;
            let results: Vec<Value> = translations
                .iter()
                .map(|t| {
                    json!({
                        "code": t.code,
                        "name": t.name,
                        "language": t.language
                    })
                })
                .collect();
            let preview = format!("Found {} translations", results.len());
            Ok((serde_json::to_string(&results)?, preview))
        }

        "load_bible_verses" => {
            let translation = str_field(&args, "translation")?;
            let book = str_field(&args, "book")?;
            let chapter = args["chapter"].as_u64().unwrap_or(1) as u16;
            let verse_start = args["verse_start"].as_u64().unwrap_or(1) as u16;
            let verse_end = args["verse_end"].as_u64().unwrap_or(verse_start as u64) as u16;

            // Resolve the translation to get its short code for reference labels.
            let translations = state.list_bible_translations().await?;
            let main_trans = match translations
                .iter()
                .find(|t| t.code.eq_ignore_ascii_case(&translation))
            {
                Some(t) => t.clone(),
                None => {
                    return Ok((
                        json!({"error": "translation not found", "translation": translation})
                            .to_string(),
                        format!("Translation '{translation}' not found"),
                    ));
                }
            };
            let short_code = main_trans
                .code
                .rsplit('-')
                .next()
                .unwrap_or(&main_trans.code)
                .to_uppercase();

            // Load the passage range one verse at a time to build the
            // per-verse reference labels. We reuse find_bible_passage so
            // we do not depend on repository-level range APIs here.
            let mut verses: Vec<Value> = Vec::new();
            for v in verse_start..=verse_end {
                let reference = BibleReference {
                    book: book.clone(),
                    book_code: None,
                    book_number: None,
                    chapter,
                    verse_start: v,
                    verse_end: v,
                };
                if let Some(p) = state
                    .find_bible_passage(&main_trans.code, &reference)
                    .await?
                {
                    verses.push(json!({
                        "number": p.reference.verse_start,
                        "text": p.text,
                        "reference": format!(
                            "{} {}:{} ({})",
                            p.reference.book, p.reference.chapter, p.reference.verse_start, short_code
                        ),
                    }));
                }
            }

            let preview = format!(
                "{} {}:{}-{} ({}) - {} verses",
                book,
                chapter,
                verse_start,
                verse_end,
                short_code,
                verses.len()
            );
            Ok((serde_json::to_string(&verses)?, preview))
        }

        "resolve_bible_slides" => {
            let translation = str_field(&args, "translation")?;
            let book = str_field(&args, "book")?;
            let chapter = args["chapter"].as_u64().unwrap_or(1) as u16;
            let verse_start = args["verse_start"].as_u64().unwrap_or(1) as u16;
            let verse_end = args["verse_end"].as_u64().unwrap_or(verse_start as u64) as u16;
            let char_limit = args["character_limit"]
                .as_u64()
                .map(|v| v as u32)
                .unwrap_or(default_char_limit);

            let (main_trans, _, slides) = state
                .generate_bible_slides(
                    &translation,
                    None,
                    &book,
                    None,
                    chapter,
                    verse_start,
                    verse_end,
                    char_limit,
                )
                .await?;

            let slide_data: Vec<Value> = slides.iter().map(|s| slide_to_json(s)).collect();

            let preview = format!(
                "{} {}:{}-{} ({}) - {} slides",
                book,
                chapter,
                verse_start,
                verse_end,
                main_trans.code,
                slides.len()
            );
            Ok((serde_json::to_string(&slide_data)?, preview))
        }

        "trigger_slide" => {
            let pres_id = PresentationId::from_uuid(uuid_field(&args, "presentation_id")?);
            let slide_id = SlideId::from_uuid(uuid_field(&args, "slide_id")?);

            // Find the next slide
            let detail = state.presentation_detail(pres_id).await?;
            let next_slide_id = detail.and_then(|(_, _, pres)| {
                let pos = pres.slides.iter().position(|s| s.id == slide_id);
                pos.and_then(|i| pres.slides.get(i + 1)).map(|s| s.id)
            });

            state
                .update_stage_state(pres_id, slide_id, next_slide_id, None)
                .await?;
            Ok((
                json!({"ok": true}).to_string(),
                "Triggered slide on stage".to_string(),
            ))
        }

        "clear_stage" => {
            state.clear_stage().await?;
            Ok((json!({"ok": true}).to_string(), "Stage cleared".to_string()))
        }

        "trigger_bible_verse" => {
            let translation = str_field(&args, "translation")?;
            let book = str_field(&args, "book")?;
            let chapter = args["chapter"].as_u64().unwrap_or(1) as u16;
            let verse_start = args["verse_start"].as_u64().unwrap_or(1) as u16;
            let verse_end = args["verse_end"]
                .as_u64()
                .map(|v| v as u16)
                .unwrap_or(verse_start);

            let reference = BibleReference {
                book: book.clone(),
                book_code: None,
                book_number: None,
                chapter,
                verse_start,
                verse_end,
            };

            let overrides = BibleTriggerOverrides::default();
            state
                .trigger_bible_passage(&translation, &reference, overrides)
                .await?;
            let preview = format!("Triggered {} {}:{}", book, chapter, verse_start);
            Ok((json!({"ok": true}).to_string(), preview))
        }

        "list_bible_presentations" => {
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

        "get_bible_presentation" => {
            let pres_id = BiblePresentationId::from_uuid(uuid_field(&args, "presentation_id")?);
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

        "create_bible_presentation" => {
            let name = str_field(&args, "name")?;
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
                        let number =
                            u32::try_from(raw["number"].as_u64().unwrap_or(0)).unwrap_or(0);
                        let text = raw["text"].as_str().unwrap_or("").to_string();
                        let book = raw["book"].as_str().unwrap_or("").to_string();
                        let chapter =
                            u32::try_from(raw["chapter"].as_u64().unwrap_or(0)).unwrap_or(0);
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
                let mut new_slides: Vec<BiblePresentationSlide> =
                    Vec::with_capacity(composed.len());
                for c in composed {
                    let main = SlideText::new(&c.main).map_err(|e| {
                        anyhow::anyhow!("composed slide main SlideText failed: {e}")
                    })?;
                    let secondary = SlideText::new("")
                        .map_err(|e| anyhow::anyhow!("empty SlideText failed: {e}"))?;
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

        "rename_bible_presentation" => {
            let pres_id = BiblePresentationId::from_uuid(uuid_field(&args, "presentation_id")?);
            let name = str_field(&args, "name")?;
            state.rename_bible_presentation(pres_id, &name).await?;
            let preview = format!("Renamed bible presentation to '{name}'");
            Ok((json!({"ok": true}).to_string(), preview))
        }

        "delete_bible_presentation" => {
            let pres_id = BiblePresentationId::from_uuid(uuid_field(&args, "presentation_id")?);
            state.delete_bible_presentation(pres_id).await?;
            Ok((
                json!({"ok": true}).to_string(),
                "Deleted bible presentation".to_string(),
            ))
        }

        "delete_bible_slide" => {
            let pres_id = BiblePresentationId::from_uuid(uuid_field(&args, "presentation_id")?);
            let slide_id = BibleSlideId::from_uuid(uuid_field(&args, "slide_id")?);
            state.delete_bible_slide(pres_id, slide_id).await?;
            Ok((
                json!({"ok": true}).to_string(),
                "Deleted bible slide".to_string(),
            ))
        }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;

    #[tokio::test]
    async fn create_presentation_in_non_bible_library_has_no_metadata() {
        let state = AppState::in_memory().await.unwrap();

        let (result, _) = execute_tool("create_library", r#"{"name":"Worship"}"#, &state, 320)
            .await
            .unwrap();
        let lib: Value = serde_json::from_str(&result).unwrap();
        let lib_id = lib["id"].as_str().unwrap();

        let args = json!({
            "library_id": lib_id,
            "name": "Amazing Grace",
            "slides": [{"main": "Amazing grace", "stage": "Verse 1"}]
        });
        let (result, _) = execute_tool("create_presentation", &args.to_string(), &state, 320)
            .await
            .unwrap();
        let pres: Value = serde_json::from_str(&result).unwrap();
        let pres_id_str = pres["id"].as_str().unwrap();

        let pres_id = PresentationId::from_uuid(Uuid::parse_str(pres_id_str).unwrap());
        let detail = state.presentation_detail(pres_id).await.unwrap().unwrap();
        let (_, _, presentation) = detail;

        for slide in &presentation.slides {
            assert!(
                slide.metadata.is_none(),
                "non-Bible slide should not have metadata"
            );
        }
    }

    #[tokio::test]
    async fn create_bible_presentation_rejects_oversized_single_verse() {
        let state = AppState::in_memory().await.unwrap();
        let long_text = "a".repeat(400);
        let args = json!({
            "name": "Length Test",
            "items": [
                {
                    "kind": "verse",
                    "number": 1,
                    "text": long_text,
                    "book": "Ján",
                    "chapter": 1,
                    "translation": "SEB"
                }
            ]
        });
        let (body, _preview) =
            execute_tool("create_bible_presentation", &args.to_string(), &state, 320)
                .await
                .unwrap();
        let parsed: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["error"], "slide_validation");
        assert_eq!(parsed["rule"], "main_exceeds_character_limit");
        assert_eq!(parsed["limit"], 320);
    }

    #[tokio::test]
    async fn create_bible_presentation_with_items_composes_server_side() {
        let state = AppState::in_memory().await.unwrap();
        let args = json!({
            "name": "Server-side Composition",
            "items": [
                {"kind": "verse", "number": 1, "text": "Na počiatku bolo Slovo.",
                 "book": "Ján", "chapter": 1, "translation": "SEB"},
                {"kind": "verse", "number": 2, "text": "Ono bolo na počiatku u Boha.",
                 "book": "Ján", "chapter": 1, "translation": "SEB"},
                {"kind": "emphasis", "text": "NOVÁ ZMLUVA"},
                {"kind": "verse", "number": 3, "text": "Všetko vzniklo skrze neho.",
                 "book": "Ján", "chapter": 1, "translation": "SEB"}
            ]
        });
        let (body, _preview) =
            execute_tool("create_bible_presentation", &args.to_string(), &state, 320)
                .await
                .unwrap();
        let parsed: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["name"].as_str().unwrap(), "Server-side Composition");
        // 2 verses batched into 1 slide + 1 emphasis slide + 1 verse slide = 3 slides
        assert_eq!(parsed["slide_count"].as_u64().unwrap(), 3);

        // Verify actual persisted slides
        let pres_id_str = parsed["id"].as_str().unwrap();
        let pres_id = BiblePresentationId::from_uuid(Uuid::parse_str(pres_id_str).unwrap());
        let pres = state
            .bible_presentation_detail(pres_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(pres.slides.len(), 3);

        // Slide 0: verses 1-2 with range reference
        assert_eq!(pres.slides[0].main_reference, "Ján 1:1-2 (SEB)");
        assert!(pres.slides[0].main.value().contains("1. Na počiatku"));
        assert!(pres.slides[0].main.value().contains("2. Ono bolo"));

        // Slide 1: emphasis
        assert_eq!(pres.slides[1].main_reference, "");
        assert_eq!(pres.slides[1].main.value(), "NOVÁ ZMLUVA");

        // Slide 2: verse 3
        assert_eq!(pres.slides[2].main_reference, "Ján 1:3 (SEB)");
    }

    #[tokio::test]
    async fn create_bible_presentation_rejects_missing_items_array() {
        let state = AppState::in_memory().await.unwrap();
        let args = json!({"name": "No Items"});
        let (body, _preview) =
            execute_tool("create_bible_presentation", &args.to_string(), &state, 320)
                .await
                .unwrap();
        let parsed: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["error"], "missing_items");
    }

    #[tokio::test]
    async fn create_bible_presentation_with_empty_items_creates_empty_presentation() {
        // Locks in the documented behavior: an explicit items: [] is allowed
        // and creates a zero-slide presentation. This is used by the operator
        // UI and by other tests as a scaffolding shortcut. A future change
        // that rejects empty items would need to update this test and the
        // inline comment in the handler.
        let state = AppState::in_memory().await.unwrap();
        let args = json!({"name": "Empty Scaffold", "items": []});
        let (body, _preview) =
            execute_tool("create_bible_presentation", &args.to_string(), &state, 320)
                .await
                .unwrap();
        let parsed: Value = serde_json::from_str(&body).unwrap();
        assert!(parsed["error"].is_null(), "expected success, got: {body}");
        assert_eq!(parsed["name"], "Empty Scaffold");
        assert_eq!(parsed["slide_count"], 0);
    }

    #[tokio::test]
    async fn create_bible_presentation_rejects_huge_verse_number() {
        // Regression guard for the u64->u32 cast: a u64 that overflows u32
        // must be rejected cleanly, not silently truncated. Picks a value
        // that used to produce a non-zero u32 after `as u32` truncation
        // (2^32 + 7 → 7) and would have slipped through.
        let state = AppState::in_memory().await.unwrap();
        let args = json!({
            "name": "Overflow Test",
            "items": [
                {"kind": "verse", "number": 4_294_967_303u64, "text": "hi",
                 "book": "Ján", "chapter": 1, "translation": "SEB"}
            ]
        });
        let (body, _preview) =
            execute_tool("create_bible_presentation", &args.to_string(), &state, 320)
                .await
                .unwrap();
        let parsed: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["error"], "invalid_verse_item");
    }

    #[tokio::test]
    async fn list_bible_presentations_returns_summaries() {
        let state = AppState::in_memory().await.unwrap();

        execute_tool(
            "create_bible_presentation",
            &json!({"name": "First", "items": []}).to_string(),
            &state,
            320,
        )
        .await
        .unwrap();
        execute_tool(
            "create_bible_presentation",
            &json!({"name": "Second", "items": []}).to_string(),
            &state,
            320,
        )
        .await
        .unwrap();

        let (result, preview) = execute_tool("list_bible_presentations", "{}", &state, 320)
            .await
            .unwrap();
        let list: Vec<Value> = serde_json::from_str(&result).unwrap();
        assert_eq!(list.len(), 2);
        let names: Vec<&str> = list.iter().filter_map(|v| v["name"].as_str()).collect();
        assert!(names.contains(&"First"));
        assert!(names.contains(&"Second"));
        assert!(preview.contains("2"));
    }

    #[tokio::test]
    async fn delete_bible_slide_removes_it() {
        let state = AppState::in_memory().await.unwrap();

        let args = json!({
            "name": "Deletable",
            "items": [
                {"kind": "verse", "number": 1, "text": "Verse one", "book": "Ref", "chapter": 1, "translation": "SEB"},
                {"kind": "verse", "number": 2, "text": "Verse two", "book": "Ref", "chapter": 1, "translation": "SEB"}
            ]
        });
        let (result, _) = execute_tool("create_bible_presentation", &args.to_string(), &state, 320)
            .await
            .unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        let pres_id_str = parsed["id"].as_str().unwrap();
        let pres_id = BiblePresentationId::from_uuid(Uuid::parse_str(pres_id_str).unwrap());

        let presentation = state
            .bible_presentation_detail(pres_id)
            .await
            .unwrap()
            .unwrap();
        // Composer packs both verses into one slide (same book/chapter/translation)
        assert!(!presentation.slides.is_empty());
        let first_slide_id = presentation.slides[0].id.to_string();

        let args = json!({
            "presentation_id": pres_id_str,
            "slide_id": first_slide_id
        });
        let (result, _) = execute_tool("delete_bible_slide", &args.to_string(), &state, 320)
            .await
            .unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert!(parsed["ok"].as_bool().unwrap());

        let after = state
            .bible_presentation_detail(pres_id)
            .await
            .unwrap()
            .unwrap();
        assert!(after.slides.len() < presentation.slides.len());
    }

    #[tokio::test]
    async fn get_bible_presentation_returns_slides() {
        let state = AppState::in_memory().await.unwrap();

        let args = json!({
            "name": "Get Test",
            "items": [
                {"kind": "verse", "number": 1, "text": "text one", "book": "Gen", "chapter": 1, "translation": "SEB"}
            ]
        });
        let (create_result, _) =
            execute_tool("create_bible_presentation", &args.to_string(), &state, 320)
                .await
                .unwrap();
        let created: Value = serde_json::from_str(&create_result).unwrap();
        let pres_id = created["id"].as_str().unwrap();

        let get_args = json!({"presentation_id": pres_id});
        let (get_result, preview) =
            execute_tool("get_bible_presentation", &get_args.to_string(), &state, 320)
                .await
                .unwrap();
        let fetched: Value = serde_json::from_str(&get_result).unwrap();
        assert_eq!(fetched["name"], "Get Test");
        let slides = fetched["slides"].as_array().unwrap();
        assert_eq!(slides.len(), 1);
        assert!(slides[0]["main"].as_str().unwrap().contains("1. text one"));
        assert_eq!(slides[0]["main_reference"], "Gen 1:1 (SEB)");
        assert!(preview.contains("Get Test"));
    }

    #[tokio::test]
    async fn rename_bible_presentation_updates_name() {
        let state = AppState::in_memory().await.unwrap();

        let (create_result, _) = execute_tool(
            "create_bible_presentation",
            &json!({"name": "Old Name", "items": []}).to_string(),
            &state,
            320,
        )
        .await
        .unwrap();
        let created: Value = serde_json::from_str(&create_result).unwrap();
        let pres_id = created["id"].as_str().unwrap();

        let rename_args = json!({"presentation_id": pres_id, "name": "New Name"});
        let (rename_result, preview) = execute_tool(
            "rename_bible_presentation",
            &rename_args.to_string(),
            &state,
            320,
        )
        .await
        .unwrap();
        let parsed: Value = serde_json::from_str(&rename_result).unwrap();
        assert!(parsed["ok"].as_bool().unwrap());
        assert!(preview.contains("New Name"));

        // Verify via list
        let (list_result, _) = execute_tool("list_bible_presentations", "{}", &state, 320)
            .await
            .unwrap();
        let list: Vec<Value> = serde_json::from_str(&list_result).unwrap();
        let found = list
            .iter()
            .find(|p| p["id"].as_str() == Some(pres_id))
            .expect("presentation should exist");
        assert_eq!(found["name"], "New Name");
    }

    #[tokio::test]
    async fn delete_bible_presentation_removes_it() {
        let state = AppState::in_memory().await.unwrap();

        let (create_result, _) = execute_tool(
            "create_bible_presentation",
            &json!({"name": "Doomed", "items": []}).to_string(),
            &state,
            320,
        )
        .await
        .unwrap();
        let created: Value = serde_json::from_str(&create_result).unwrap();
        let pres_id = created["id"].as_str().unwrap();

        // Verify exists first
        let (list_before, _) = execute_tool("list_bible_presentations", "{}", &state, 320)
            .await
            .unwrap();
        let list_before: Vec<Value> = serde_json::from_str(&list_before).unwrap();
        assert!(list_before
            .iter()
            .any(|p| p["id"].as_str() == Some(pres_id)));

        // Delete
        let delete_args = json!({"presentation_id": pres_id});
        let (delete_result, _) = execute_tool(
            "delete_bible_presentation",
            &delete_args.to_string(),
            &state,
            320,
        )
        .await
        .unwrap();
        let parsed: Value = serde_json::from_str(&delete_result).unwrap();
        assert!(parsed["ok"].as_bool().unwrap());

        // Verify gone
        let (list_after, _) = execute_tool("list_bible_presentations", "{}", &state, 320)
            .await
            .unwrap();
        let list_after: Vec<Value> = serde_json::from_str(&list_after).unwrap();
        assert!(!list_after.iter().any(|p| p["id"].as_str() == Some(pres_id)));
    }

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

    #[tokio::test]
    async fn create_bible_accepts_verse_items() {
        let state = AppState::in_memory().await.unwrap();
        let args = json!({
            "name": "Test Sermon",
            "items": [
                {"kind": "verse", "number": 1, "text": "Na počiatku bolo Slovo.", "book": "Ján", "chapter": 1, "translation": "MIL"},
                {"kind": "verse", "number": 13, "text": "A nieto tvora, čo by bol preň neviditeľný", "book": "Židom", "chapter": 4, "translation": "SEB"}
            ]
        });
        let (result, _) = execute_tool("create_bible_presentation", &args.to_string(), &state, 320)
            .await
            .unwrap();

        let json: Value = serde_json::from_str(&result).unwrap();
        // Two different books/translations → two slides
        assert_eq!(json["slide_count"], 2);
    }

    #[tokio::test]
    async fn create_bible_accepts_emphasis_item() {
        let state = AppState::in_memory().await.unwrap();
        let args = json!({
            "name": "Test Sermon",
            "items": [
                {"kind": "verse", "number": 1, "text": "Na počiatku bolo Slovo", "book": "Ján", "chapter": 1, "translation": "MIL"},
                {"kind": "emphasis", "text": "NOVÁ ZMLUVA"}
            ]
        });
        let (result, _) = execute_tool("create_bible_presentation", &args.to_string(), &state, 320)
            .await
            .unwrap();

        let json: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(json["slide_count"], 2);
    }

    #[tokio::test]
    async fn create_bible_rejects_invalid_verse_item_missing_fields() {
        // Item 0 valid, item 1 missing required fields — whole call rejected,
        // zero presentation created.
        let state = AppState::in_memory().await.unwrap();
        let args = json!({
            "name": "Partial Batch Test",
            "items": [
                {"kind": "verse", "number": 1, "text": "OK verse", "book": "Ján", "chapter": 1, "translation": "MIL"},
                {"kind": "verse", "number": 2, "text": "bad", "book": "", "chapter": 1, "translation": "MIL"},
                {"kind": "verse", "number": 3, "text": "OK verse", "book": "Ján", "chapter": 1, "translation": "MIL"}
            ]
        });
        let (result, _) = execute_tool("create_bible_presentation", &args.to_string(), &state, 320)
            .await
            .unwrap();

        let json: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(json["error"], "invalid_verse_item");
        assert!(json["got"].as_str().unwrap().contains("item[1]"));
        assert!(state.list_bible_presentations().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn create_bible_presentation_with_long_passage_composes_many_slides() {
        // Simulates the realistic AI path: a full sermon passage with many
        // verses of varying lengths. The server must split them into multiple
        // slides and every slide must fit under the character limit.
        //
        // This is the end-to-end proof that moving slide-break decisions from
        // the LLM to the server actually enforces the limit. The /ai/chat
        // endpoint itself requires a live LLM we can't test in CI — this
        // Rust test exercises the exact same execute_tool() dispatch path.
        let state = AppState::in_memory().await.unwrap();
        let char_limit: u32 = 200;

        // 12 verses of real-ish Slovak bible text with varied lengths. The
        // total is far above the 200-char limit, so the server MUST produce
        // multiple slides. Mix of short and long verses tests the packing
        // logic: short verses should cluster, long verses should split.
        let items: Vec<serde_json::Value> = vec![
            json!({"kind": "verse", "number": 1, "text": "Na počiatku bolo Slovo a to Slovo bolo u Boha a to Slovo bolo Boh.", "book": "Ján", "chapter": 1, "translation": "SEB"}),
            json!({"kind": "verse", "number": 2, "text": "Ono bolo na počiatku u Boha.", "book": "Ján", "chapter": 1, "translation": "SEB"}),
            json!({"kind": "verse", "number": 3, "text": "Všetko povstalo skrze neho a bez neho nepovstalo nič, čo povstalo.", "book": "Ján", "chapter": 1, "translation": "SEB"}),
            json!({"kind": "verse", "number": 4, "text": "V ňom bol život a život bol svetlom ľudí.", "book": "Ján", "chapter": 1, "translation": "SEB"}),
            json!({"kind": "verse", "number": 5, "text": "A svetlo svieti v tme, ale tma ho nepohltila.", "book": "Ján", "chapter": 1, "translation": "SEB"}),
            json!({"kind": "emphasis", "text": "NOVÁ ZMLUVA"}),
            json!({"kind": "verse", "number": 6, "text": "Bol človek, poslaný od Boha, ktorý sa volal Ján.", "book": "Ján", "chapter": 1, "translation": "SEB"}),
            json!({"kind": "verse", "number": 7, "text": "Ten prišiel na svedectvo, aby svedčil o svetle, aby skrze neho všetci uverili.", "book": "Ján", "chapter": 1, "translation": "SEB"}),
            json!({"kind": "verse", "number": 8, "text": "On sám nebol svetlo, ale prišiel svedčiť o svetle.", "book": "Ján", "chapter": 1, "translation": "SEB"}),
            json!({"kind": "verse", "number": 9, "text": "Pravé svetlo, ktoré osvecuje každého človeka, prichádzalo na svet.", "book": "Ján", "chapter": 1, "translation": "SEB"}),
            json!({"kind": "verse", "number": 10, "text": "Bol na svete a svet povstal skrze neho, ale svet ho nepoznal.", "book": "Ján", "chapter": 1, "translation": "SEB"}),
            json!({"kind": "verse", "number": 11, "text": "Prišiel do svojho vlastného, ale vlastní ho neprijali.", "book": "Ján", "chapter": 1, "translation": "SEB"}),
        ];

        let args = json!({
            "name": "Ján 1 — End-to-End Composition Proof",
            "items": items,
        });
        let (body, _preview) = execute_tool(
            "create_bible_presentation",
            &args.to_string(),
            &state,
            char_limit,
        )
        .await
        .unwrap();

        // Parse response — should be the created presentation, NOT an error.
        let parsed: Value = serde_json::from_str(&body).unwrap();
        assert!(
            parsed["error"].is_null(),
            "expected successful creation, got error: {body}"
        );
        assert_eq!(
            parsed["name"].as_str().unwrap(),
            "Ján 1 — End-to-End Composition Proof"
        );
        let slide_count = parsed["slide_count"].as_u64().unwrap();

        // With 12 items and a 200-char limit, the server MUST split into
        // multiple slides. Exact count depends on packing, but must be > 1
        // (proves the server is actually splitting) and reasonably bounded
        // (proves the packer is not producing one-slide-per-verse when they
        // could group).
        assert!(
            slide_count > 1,
            "expected multiple slides from a long passage, got {slide_count}"
        );
        assert!(
            slide_count < 12,
            "expected server to pack verses into fewer slides than 1-per-verse, got {slide_count} for 12 items"
        );

        // Fetch the persisted presentation and verify every slide's main
        // text fits under the character limit. THIS IS THE CORE ASSERTION —
        // it proves the fix works end-to-end.
        let pres_id_str = parsed["id"].as_str().unwrap();
        let pres_id = BiblePresentationId::from_uuid(Uuid::parse_str(pres_id_str).unwrap());
        let pres = state
            .bible_presentation_detail(pres_id)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(pres.slides.len() as u64, slide_count);

        for (idx, slide) in pres.slides.iter().enumerate() {
            let main = slide.main.value();
            assert!(
                main.len() <= char_limit as usize,
                "slide[{idx}] main.len()={} exceeds limit {}: {:?}",
                main.len(),
                char_limit,
                main
            );
            // Sanity: no raw ## markers survived.
            assert!(
                !main.contains("##"),
                "slide[{idx}] main should not contain ## markers: {main}"
            );
        }

        // At least one slide should be the emphasis slide with empty reference.
        let emphasis_slides: Vec<&_> = pres
            .slides
            .iter()
            .filter(|s| s.main_reference.is_empty() && s.main.value() == "NOVÁ ZMLUVA")
            .collect();
        assert_eq!(
            emphasis_slides.len(),
            1,
            "expected exactly 1 emphasis slide with main='NOVÁ ZMLUVA' and empty reference"
        );
    }

    #[tokio::test]
    async fn load_bible_verses_handler_is_registered() {
        // The in-memory state seeds no bible translations, so we expect
        // "translation not found" error. This proves the tool is registered
        // and the handler exists (not the "unknown tool" fallthrough).
        let state = AppState::in_memory().await.unwrap();
        let args = json!({
            "translation": "slk-seb",
            "book": "Ján",
            "chapter": 1,
            "verse_start": 1,
            "verse_end": 3
        });
        let result = execute_tool("load_bible_verses", &args.to_string(), &state, 320).await;
        match result {
            Ok((body, _preview)) => {
                assert!(
                    !body.contains("unknown tool"),
                    "tool must be registered, got body: {body}"
                );
                // Expected: "translation not found" error JSON
                assert!(
                    body.contains("translation not found") || body.contains("not found"),
                    "expected translation-not-found error, got: {body}"
                );
            }
            Err(_) => {
                // Also acceptable — the handler exists but errored looking up translations.
            }
        }
    }
}
