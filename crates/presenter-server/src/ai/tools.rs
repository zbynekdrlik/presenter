use super::bible_validator::{validate_bible_slide, ValidationError};
use crate::state::bible::BibleTriggerOverrides;
use crate::state::AppState;
use presenter_core::slide::{SlideContent, SlideText};
use presenter_core::{
    BiblePresentationId, BiblePresentationSlide, BibleReference, BibleSlideId, LibraryId,
    PresentationId, Slide, SlideId,
};
use serde_json::{json, Value};
use uuid::Uuid;

/// Return OpenAI function-calling tool definitions.
pub fn tool_definitions() -> Vec<Value> {
    vec![
        tool_def(
            "list_libraries",
            "[WORSHIP only] List all worship presentation libraries (songs, lyrics). For Bible content use the bible_* tools instead.",
            json!({"type": "object", "properties": {}, "required": []}),
        ),
        tool_def(
            "create_library",
            "[WORSHIP only] Create a new worship presentation library (songs, lyrics). For Bible content use the bible_* tools instead.",
            json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "Library name"}
                },
                "required": ["name"]
            }),
        ),
        tool_def(
            "list_presentations",
            "[WORSHIP only] List worship presentations in a library (songs, lyrics). For Bible content use the bible_* tools instead.",
            json!({
                "type": "object",
                "properties": {
                    "library_id": {"type": "string", "description": "Library UUID"}
                },
                "required": ["library_id"]
            }),
        ),
        tool_def(
            "get_presentation",
            "[WORSHIP only] Get full worship presentation detail including slides. For Bible content use the bible_* tools instead.",
            json!({
                "type": "object",
                "properties": {
                    "presentation_id": {"type": "string", "description": "Presentation UUID"}
                },
                "required": ["presentation_id"]
            }),
        ),
        tool_def(
            "create_presentation",
            "[WORSHIP only] Create a new worship presentation (songs, lyrics) with slides in a library. For Bible content use create_bible_presentation instead.",
            json!({
                "type": "object",
                "properties": {
                    "library_id": {"type": "string", "description": "Library UUID"},
                    "name": {"type": "string", "description": "Presentation name"},
                    "slides": {
                        "type": "array",
                        "description": "Slides to create",
                        "items": {
                            "type": "object",
                            "properties": {
                                "main": {"type": "string", "description": "Main text displayed on screen"},
                                "translation": {"type": "string", "description": "Secondary language text"},
                                "stage": {"type": "string", "description": "Confidence monitor text"},
                                "group": {"type": "string", "description": "Section label (e.g. Verse 1, Chorus)"}
                            },
                            "required": ["main"]
                        }
                    }
                },
                "required": ["library_id", "name", "slides"]
            }),
        ),
        tool_def(
            "rename_presentation",
            "[WORSHIP only] Rename an existing worship presentation. For Bible content use rename_bible_presentation instead.",
            json!({
                "type": "object",
                "properties": {
                    "presentation_id": {"type": "string"},
                    "name": {"type": "string"}
                },
                "required": ["presentation_id", "name"]
            }),
        ),
        tool_def(
            "delete_presentation",
            "[WORSHIP only] Delete a worship presentation. For Bible content use delete_bible_presentation instead.",
            json!({
                "type": "object",
                "properties": {
                    "presentation_id": {"type": "string"}
                },
                "required": ["presentation_id"]
            }),
        ),
        tool_def(
            "add_slide",
            "[WORSHIP only] Add a slide to an existing worship presentation. For Bible content use add_bible_slide instead.",
            json!({
                "type": "object",
                "properties": {
                    "presentation_id": {"type": "string"},
                    "position": {"type": "integer", "description": "0-based insert position (end if omitted)"},
                    "main": {"type": "string"},
                    "translation": {"type": "string"},
                    "stage": {"type": "string"},
                    "group": {"type": "string"}
                },
                "required": ["presentation_id", "main"]
            }),
        ),
        tool_def(
            "update_slide",
            "[WORSHIP only] Update the content of an existing worship slide. For Bible content use update_bible_slide instead.",
            json!({
                "type": "object",
                "properties": {
                    "presentation_id": {"type": "string"},
                    "slide_id": {"type": "string"},
                    "main": {"type": "string"},
                    "translation": {"type": "string"},
                    "stage": {"type": "string"},
                    "group": {"type": "string"}
                },
                "required": ["presentation_id", "slide_id", "main"]
            }),
        ),
        tool_def(
            "delete_slide",
            "[WORSHIP only] Delete a slide from a worship presentation. For Bible content use delete_bible_slide instead.",
            json!({
                "type": "object",
                "properties": {
                    "presentation_id": {"type": "string"},
                    "slide_id": {"type": "string"}
                },
                "required": ["presentation_id", "slide_id"]
            }),
        ),
        tool_def(
            "reorder_slides",
            "[WORSHIP only] Reorder slides in a worship presentation. For Bible content use the bible_* tools instead.",
            json!({
                "type": "object",
                "properties": {
                    "presentation_id": {"type": "string"},
                    "slide_ids": {"type": "array", "items": {"type": "string"}}
                },
                "required": ["presentation_id", "slide_ids"]
            }),
        ),
        tool_def(
            "search_bible",
            "Search Bible passages by text query",
            json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Search text"},
                    "translation": {"type": "string", "description": "Translation code (e.g. slk-seb)"}
                },
                "required": ["query"]
            }),
        ),
        tool_def(
            "get_bible_passage",
            "Get the text of a specific Bible passage",
            json!({
                "type": "object",
                "properties": {
                    "translation": {"type": "string", "description": "Translation code (e.g. slk-seb)"},
                    "book": {"type": "string", "description": "Full book name (e.g. Židom)"},
                    "chapter": {"type": "integer"},
                    "verse_start": {"type": "integer"},
                    "verse_end": {"type": "integer"}
                },
                "required": ["translation", "book", "chapter", "verse_start"]
            }),
        ),
        tool_def(
            "get_style_guide",
            "[REFERENCE] Get the detailed formatting guide for Bible references, Slovak book names, translation codes, multi-slide rules, and markdown conventions. The live system prompt only has essentials. Call this once at the start of a session if you need detailed rules.",
            json!({"type": "object", "properties": {}, "required": []}),
        ),
        tool_def(
            "list_bible_translations",
            "List all available Bible translations",
            json!({"type": "object", "properties": {}, "required": []}),
        ),
        tool_def(
            "resolve_bible_slides",
            "Generate slides from a Bible passage, automatically split by character limit",
            json!({
                "type": "object",
                "properties": {
                    "translation": {"type": "string", "description": "Translation code"},
                    "book": {"type": "string", "description": "Full book name"},
                    "chapter": {"type": "integer"},
                    "verse_start": {"type": "integer"},
                    "verse_end": {"type": "integer"},
                    "character_limit": {"type": "integer", "description": "Max chars per slide (default 320)"}
                },
                "required": ["translation", "book", "chapter", "verse_start", "verse_end"]
            }),
        ),
        tool_def(
            "trigger_slide",
            "Display a specific slide on the stage",
            json!({
                "type": "object",
                "properties": {
                    "presentation_id": {"type": "string"},
                    "slide_id": {"type": "string"}
                },
                "required": ["presentation_id", "slide_id"]
            }),
        ),
        tool_def(
            "clear_stage",
            "Clear the stage display",
            json!({"type": "object", "properties": {}, "required": []}),
        ),
        tool_def(
            "trigger_bible_verse",
            "Display a Bible verse directly on the stage",
            json!({
                "type": "object",
                "properties": {
                    "translation": {"type": "string"},
                    "book": {"type": "string"},
                    "chapter": {"type": "integer"},
                    "verse_start": {"type": "integer"},
                    "verse_end": {"type": "integer"}
                },
                "required": ["translation", "book", "chapter", "verse_start"]
            }),
        ),
        tool_def(
            "list_bible_presentations",
            "[BIBLE only] List all Bible presentations (user-curated collections of Bible slides). Use this when the user asks about Bible passages, verses, or collections.",
            json!({"type": "object", "properties": {}, "required": []}),
        ),
        tool_def(
            "get_bible_presentation",
            "[BIBLE only] Get a Bible presentation with all its slides (main text, references, metadata).",
            json!({
                "type": "object",
                "properties": {
                    "presentation_id": {"type": "string", "description": "Bible presentation UUID"}
                },
                "required": ["presentation_id"]
            }),
        ),
        tool_def(
            "create_bible_presentation",
            "[BIBLE only] Create a new Bible presentation (a named collection of Bible slides, e.g. a sermon series or topical study). Optionally include initial slides. Use this when the user asks to create a Bible presentation, sermon, or verse collection.",
            json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "Presentation name (e.g. 'Sunday Sermon 2026-04-14')"},
                    "slides": {
                        "type": "array",
                        "description": "Optional initial slides. Each slide represents one bible verse or passage.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "main": {"type": "string", "description": "Main verse text (e.g. 'For God so loved the world...')"},
                                "main_reference": {"type": "string", "description": "Reference label (e.g. 'John 3:16')"},
                                "secondary": {"type": "string", "description": "Secondary translation text (optional)"},
                                "secondary_reference": {"type": "string", "description": "Secondary reference label (optional)"}
                            },
                            "required": ["main", "main_reference"]
                        }
                    }
                },
                "required": ["name"]
            }),
        ),
        tool_def(
            "rename_bible_presentation",
            "[BIBLE only] Rename an existing Bible presentation.",
            json!({
                "type": "object",
                "properties": {
                    "presentation_id": {"type": "string", "description": "Bible presentation UUID"},
                    "name": {"type": "string", "description": "New name"}
                },
                "required": ["presentation_id", "name"]
            }),
        ),
        tool_def(
            "delete_bible_presentation",
            "[BIBLE only] Delete a Bible presentation and all its slides.",
            json!({
                "type": "object",
                "properties": {
                    "presentation_id": {"type": "string", "description": "Bible presentation UUID"}
                },
                "required": ["presentation_id"]
            }),
        ),
        tool_def(
            "add_bible_slide",
            "[BIBLE only] Append a single slide to an existing Bible presentation. For adding multiple slides at once, prefer create_bible_presentation with the slides array.",
            json!({
                "type": "object",
                "properties": {
                    "presentation_id": {"type": "string", "description": "Bible presentation UUID"},
                    "main": {"type": "string", "description": "Main verse text"},
                    "main_reference": {"type": "string", "description": "Reference label (e.g. 'John 3:16')"},
                    "secondary": {"type": "string", "description": "Secondary translation text (optional)"},
                    "secondary_reference": {"type": "string", "description": "Secondary reference label (optional)"}
                },
                "required": ["presentation_id", "main", "main_reference"]
            }),
        ),
        tool_def(
            "update_bible_slide",
            "[BIBLE only] Update the text and references on a single Bible slide.",
            json!({
                "type": "object",
                "properties": {
                    "presentation_id": {"type": "string", "description": "Bible presentation UUID"},
                    "slide_id": {"type": "string", "description": "Bible slide UUID"},
                    "main": {"type": "string"},
                    "main_reference": {"type": "string"},
                    "secondary": {"type": "string"},
                    "secondary_reference": {"type": "string"}
                },
                "required": ["presentation_id", "slide_id", "main", "main_reference"]
            }),
        ),
        tool_def(
            "delete_bible_slide",
            "[BIBLE only] Delete a single slide from a Bible presentation.",
            json!({
                "type": "object",
                "properties": {
                    "presentation_id": {"type": "string", "description": "Bible presentation UUID"},
                    "slide_id": {"type": "string", "description": "Bible slide UUID"}
                },
                "required": ["presentation_id", "slide_id"]
            }),
        ),
    ]
}

fn tool_def(name: &str, description: &str, parameters: Value) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": parameters
        }
    })
}

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

            // Pre-validate every slide in the batch BEFORE touching the DB.
            // All-or-nothing: if any slide fails, the presentation is not
            // created and the LLM sees the rule-keyed error so it can fix
            // the specific slide and retry with the full batch.
            if let Some(arr) = args["slides"].as_array() {
                for (idx, s) in arr.iter().enumerate() {
                    let main_text = s["main"].as_str().unwrap_or("");
                    let main_reference = s["main_reference"].as_str().unwrap_or("");
                    if let Err(mut err) = validate_bible_slide(main_text, main_reference) {
                        // Annotate the `got` field with the slide index so the
                        // LLM knows which slide in the batch to fix.
                        err.got = format!("slide[{idx}]: {}", err.got);
                        return Ok(validation_error_response(err));
                    }
                }
            }

            let presentation = state.create_bible_presentation(&name).await?;

            // If slides were provided, append them.
            let slides_arr = args["slides"].as_array();
            let final_presentation = if let Some(arr) = slides_arr {
                let mut new_slides: Vec<BiblePresentationSlide> = Vec::with_capacity(arr.len());
                for s in arr {
                    let main_text = s["main"].as_str().unwrap_or("").to_string();
                    let main_reference = s["main_reference"].as_str().unwrap_or("").to_string();
                    let secondary_text = s["secondary"].as_str().unwrap_or("").to_string();
                    let secondary_reference =
                        s["secondary_reference"].as_str().unwrap_or("").to_string();
                    new_slides.push(BiblePresentationSlide {
                        id: BibleSlideId::new(),
                        order: 0,
                        main: SlideText::new(&main_text)
                            .unwrap_or_else(|_| SlideText::new("").unwrap()),
                        main_reference,
                        secondary: SlideText::new(&secondary_text)
                            .unwrap_or_else(|_| SlideText::new("").unwrap()),
                        secondary_reference,
                        metadata: None,
                    });
                }
                if !new_slides.is_empty() {
                    state
                        .append_bible_presentation_slides(presentation.id, new_slides)
                        .await?
                } else {
                    presentation
                }
            } else {
                presentation
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

        "add_bible_slide" => {
            let pres_id = BiblePresentationId::from_uuid(uuid_field(&args, "presentation_id")?);
            let main_text = str_field(&args, "main")?;
            let main_reference = str_field(&args, "main_reference")?;
            let secondary_text = args["secondary"].as_str().unwrap_or("").to_string();
            let secondary_reference = args["secondary_reference"]
                .as_str()
                .unwrap_or("")
                .to_string();

            // Validate before touching the DB.
            if let Err(err) = validate_bible_slide(&main_text, &main_reference) {
                return Ok(validation_error_response(err));
            }

            let slide = BiblePresentationSlide {
                id: BibleSlideId::new(),
                order: 0,
                main: SlideText::new(&main_text).unwrap_or_else(|_| SlideText::new("").unwrap()),
                main_reference,
                secondary: SlideText::new(&secondary_text)
                    .unwrap_or_else(|_| SlideText::new("").unwrap()),
                secondary_reference,
                metadata: None,
            };
            let updated = state
                .append_bible_presentation_slides(pres_id, vec![slide])
                .await?;
            let preview = format!(
                "Added bible slide to '{}' (now {} total)",
                updated.name,
                updated.slides.len()
            );
            Ok((
                json!({"ok": true, "slide_count": updated.slides.len()}).to_string(),
                preview,
            ))
        }

        "update_bible_slide" => {
            let pres_id = BiblePresentationId::from_uuid(uuid_field(&args, "presentation_id")?);
            let slide_id = BibleSlideId::from_uuid(uuid_field(&args, "slide_id")?);
            let main_text = str_field(&args, "main")?;
            let main_reference = str_field(&args, "main_reference")?;
            let secondary_text = args["secondary"].as_str().unwrap_or("").to_string();
            let secondary_reference = args["secondary_reference"]
                .as_str()
                .unwrap_or("")
                .to_string();

            // Validate before touching the DB.
            if let Err(err) = validate_bible_slide(&main_text, &main_reference) {
                return Ok(validation_error_response(err));
            }

            // Preserve existing metadata if present.
            let existing_metadata = match state.bible_presentation_detail(pres_id).await? {
                Some(p) => p
                    .slides
                    .iter()
                    .find(|s| s.id == slide_id)
                    .and_then(|s| s.metadata.clone()),
                None => None,
            };

            state
                .update_bible_slide(
                    pres_id,
                    slide_id,
                    main_text,
                    main_reference,
                    secondary_text,
                    secondary_reference,
                    existing_metadata,
                )
                .await?;
            Ok((
                json!({"ok": true}).to_string(),
                "Updated bible slide".to_string(),
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
    async fn create_bible_presentation_with_slides() {
        let state = AppState::in_memory().await.unwrap();

        let args = json!({
            "name": "Sunday Sermon",
            "slides": [
                {
                    "main": "16. For God so loved the world...",
                    "main_reference": "John 3:16",
                    "secondary": "Lebo tak Boh miloval svet...",
                    "secondary_reference": "Ján 3:16"
                },
                {
                    "main": "1. The Lord is my shepherd...",
                    "main_reference": "Psalm 23:1"
                }
            ]
        });
        let (result, preview) =
            execute_tool("create_bible_presentation", &args.to_string(), &state, 320)
                .await
                .unwrap();

        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["name"].as_str().unwrap(), "Sunday Sermon");
        assert_eq!(parsed["slide_count"].as_u64().unwrap(), 2);
        assert!(preview.contains("Sunday Sermon"));

        // Verify from state
        let pres_id_str = parsed["id"].as_str().unwrap();
        let pres_id = BiblePresentationId::from_uuid(Uuid::parse_str(pres_id_str).unwrap());
        let presentation = state
            .bible_presentation_detail(pres_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(presentation.slides.len(), 2);
        assert_eq!(presentation.slides[0].main_reference, "John 3:16");
        assert_eq!(presentation.slides[1].main_reference, "Psalm 23:1");
    }

    #[tokio::test]
    async fn list_bible_presentations_returns_summaries() {
        let state = AppState::in_memory().await.unwrap();

        execute_tool(
            "create_bible_presentation",
            &json!({"name": "First"}).to_string(),
            &state,
            320,
        )
        .await
        .unwrap();
        execute_tool(
            "create_bible_presentation",
            &json!({"name": "Second"}).to_string(),
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
    async fn add_bible_slide_appends() {
        let state = AppState::in_memory().await.unwrap();

        let (result, _) = execute_tool(
            "create_bible_presentation",
            &json!({"name": "My Study"}).to_string(),
            &state,
            320,
        )
        .await
        .unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        let pres_id = parsed["id"].as_str().unwrap();

        let args = json!({
            "presentation_id": pres_id,
            "main": "1. In the beginning was the Word",
            "main_reference": "John 1:1"
        });
        let (result, preview) = execute_tool("add_bible_slide", &args.to_string(), &state, 320)
            .await
            .unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["slide_count"].as_u64().unwrap(), 1);
        assert!(preview.contains("1 total"));

        // Add another
        let args = json!({
            "presentation_id": pres_id,
            "main": "1. And the Word was with God",
            "main_reference": "John 1:1b"
        });
        let (result, _) = execute_tool("add_bible_slide", &args.to_string(), &state, 320)
            .await
            .unwrap();
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["slide_count"].as_u64().unwrap(), 2);
    }

    #[tokio::test]
    async fn delete_bible_slide_removes_it() {
        let state = AppState::in_memory().await.unwrap();

        let args = json!({
            "name": "Deletable",
            "slides": [
                {"main": "1. Verse one", "main_reference": "Ref 1:1"},
                {"main": "2. Verse two", "main_reference": "Ref 1:2"}
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
        assert_eq!(presentation.slides.len(), 2);
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
        assert_eq!(after.slides.len(), 1);
        assert_eq!(after.slides[0].main_reference, "Ref 1:2");
    }

    #[tokio::test]
    async fn get_bible_presentation_returns_slides() {
        let state = AppState::in_memory().await.unwrap();

        let args = json!({
            "name": "Get Test",
            "slides": [{"main": "1. text one", "main_reference": "Gen 1:1"}]
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
        assert_eq!(slides[0]["main"], "1. text one");
        assert_eq!(slides[0]["main_reference"], "Gen 1:1");
        assert!(preview.contains("Get Test"));
    }

    #[tokio::test]
    async fn rename_bible_presentation_updates_name() {
        let state = AppState::in_memory().await.unwrap();

        let (create_result, _) = execute_tool(
            "create_bible_presentation",
            &json!({"name": "Old Name"}).to_string(),
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
            &json!({"name": "Doomed"}).to_string(),
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
    async fn update_bible_slide_changes_text_and_references() {
        let state = AppState::in_memory().await.unwrap();

        // Create a presentation with one slide
        let args = json!({
            "name": "Update Test",
            "slides": [{"main": "1. original", "main_reference": "Ref 1:1"}]
        });
        let (create_result, _) =
            execute_tool("create_bible_presentation", &args.to_string(), &state, 320)
                .await
                .unwrap();
        let created: Value = serde_json::from_str(&create_result).unwrap();
        let pres_id = created["id"].as_str().unwrap();

        // Fetch to get the slide ID
        let get_args = json!({"presentation_id": pres_id});
        let (get_result, _) =
            execute_tool("get_bible_presentation", &get_args.to_string(), &state, 320)
                .await
                .unwrap();
        let fetched: Value = serde_json::from_str(&get_result).unwrap();
        let slide_id = fetched["slides"][0]["id"].as_str().unwrap();

        // Update the slide
        let update_args = json!({
            "presentation_id": pres_id,
            "slide_id": slide_id,
            "main": "2. updated",
            "main_reference": "Ref 2:2",
            "secondary": "trans",
            "secondary_reference": "Ref 2:2 trans"
        });
        let (update_result, _) =
            execute_tool("update_bible_slide", &update_args.to_string(), &state, 320)
                .await
                .unwrap();
        let parsed: Value = serde_json::from_str(&update_result).unwrap();
        assert!(parsed["ok"].as_bool().unwrap());

        // Verify by fetching again
        let (get_result2, _) =
            execute_tool("get_bible_presentation", &get_args.to_string(), &state, 320)
                .await
                .unwrap();
        let fetched2: Value = serde_json::from_str(&get_result2).unwrap();
        let slide = &fetched2["slides"][0];
        assert_eq!(slide["main"], "2. updated");
        assert_eq!(slide["main_reference"], "Ref 2:2");
        assert_eq!(slide["secondary"], "trans");
        assert_eq!(slide["secondary_reference"], "Ref 2:2 trans");
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
    async fn create_bible_rejects_reference_without_parens() {
        // Exact production-bug input — AI wrote "Židom 4:13 SEB" without parens.
        let state = AppState::in_memory().await.unwrap();
        let args = json!({
            "name": "Test Sermon",
            "slides": [{
                "main": "13. A nieto tvora, čo by bol preň neviditeľný",
                "main_reference": "Židom 4:13 SEB"
            }]
        });
        let (result, preview) =
            execute_tool("create_bible_presentation", &args.to_string(), &state, 320)
                .await
                .unwrap();

        let json: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(json["error"], "slide_validation");
        assert_eq!(json["rule"], "reference_format_requires_parens");
        assert!(json["got"].as_str().unwrap().contains("Židom 4:13 SEB"));
        assert!(preview.starts_with("Validation failed:"));

        // No presentation should have been created.
        let list = state.list_bible_presentations().await.unwrap();
        assert!(
            list.is_empty(),
            "presentation must not be created on rejection"
        );
    }

    #[tokio::test]
    async fn create_bible_rejects_main_without_verse_numbers() {
        let state = AppState::in_memory().await.unwrap();
        let args = json!({
            "name": "Test",
            "slides": [{
                "main": "Na počiatku bolo Slovo, to Slovo bolo u Boha",
                "main_reference": "Ján 1:1 (MIL)"
            }]
        });
        let (result, _) = execute_tool("create_bible_presentation", &args.to_string(), &state, 320)
            .await
            .unwrap();

        let json: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(json["rule"], "missing_verse_number_prefix");
        assert!(state.list_bible_presentations().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn create_bible_rejects_main_with_hash_markers() {
        let state = AppState::in_memory().await.unwrap();
        let args = json!({
            "name": "Test",
            "slides": [{
                "main": "1. aby sme ##verili## menu jeho Syna",
                "main_reference": "Ján 1:12 (MIL)"
            }]
        });
        let (result, _) = execute_tool("create_bible_presentation", &args.to_string(), &state, 320)
            .await
            .unwrap();

        let json: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(json["rule"], "unprocessed_bold_markers");
    }

    #[tokio::test]
    async fn create_bible_accepts_correctly_formatted_slides() {
        let state = AppState::in_memory().await.unwrap();
        let args = json!({
            "name": "Test Sermon",
            "slides": [
                {
                    "main": "1. Na počiatku bolo Slovo.\n2. Ono bolo na počiatku u Boha.\n3. Všetko vzniklo skrze neho.",
                    "main_reference": "Ján 1:1-51 (MIL)"
                },
                {
                    "main": "13. A nieto tvora, čo by bol preň neviditeľný",
                    "main_reference": "Židom 4:13 (SEB)"
                }
            ]
        });
        let (result, _) = execute_tool("create_bible_presentation", &args.to_string(), &state, 320)
            .await
            .unwrap();

        let json: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(json["slide_count"], 2);
    }

    #[tokio::test]
    async fn create_bible_accepts_emphasis_slide_without_reference() {
        let state = AppState::in_memory().await.unwrap();
        let args = json!({
            "name": "Test Sermon",
            "slides": [
                {
                    "main": "1. Na počiatku bolo Slovo",
                    "main_reference": "Ján 1:1 (MIL)"
                },
                {
                    "main": "NOVÁ ZMLUVA",
                    "main_reference": ""
                }
            ]
        });
        let (result, _) = execute_tool("create_bible_presentation", &args.to_string(), &state, 320)
            .await
            .unwrap();

        let json: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(json["slide_count"], 2);
    }

    #[tokio::test]
    async fn create_bible_rejects_entire_batch_on_first_invalid_slide() {
        // Slide 0 valid, slide 1 invalid, slide 2 valid — whole batch rejected,
        // zero slides and zero presentation created.
        let state = AppState::in_memory().await.unwrap();
        let args = json!({
            "name": "Partial Batch Test",
            "slides": [
                {"main": "1. OK verse", "main_reference": "Ján 1:1 (MIL)"},
                {"main": "bad text no prefix", "main_reference": "Ján 1:2 (MIL)"},
                {"main": "3. OK verse", "main_reference": "Ján 1:3 (MIL)"}
            ]
        });
        let (result, _) = execute_tool("create_bible_presentation", &args.to_string(), &state, 320)
            .await
            .unwrap();

        let json: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(json["rule"], "missing_verse_number_prefix");
        assert!(json["got"].as_str().unwrap().contains("slide[1]"));
        assert!(state.list_bible_presentations().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn add_bible_slide_runs_validator() {
        let state = AppState::in_memory().await.unwrap();
        // Create a valid presentation first.
        let create_args = json!({
            "name": "Base",
            "slides": [{"main": "1. test", "main_reference": "Ján 1:1 (MIL)"}]
        });
        let (create_result, _) = execute_tool(
            "create_bible_presentation",
            &create_args.to_string(),
            &state,
            320,
        )
        .await
        .unwrap();
        let created: Value = serde_json::from_str(&create_result).unwrap();
        let pres_id = created["id"].as_str().unwrap().to_string();
        let slide_count_before = created["slide_count"].as_u64().unwrap();

        // Now try to add a malformed slide.
        let add_args = json!({
            "presentation_id": pres_id,
            "main": "no verse number",
            "main_reference": "Ján 1:2 (MIL)"
        });
        let (add_result, _) = execute_tool("add_bible_slide", &add_args.to_string(), &state, 320)
            .await
            .unwrap();

        let json: Value = serde_json::from_str(&add_result).unwrap();
        assert_eq!(json["rule"], "missing_verse_number_prefix");

        // Slide count must not have changed.
        let get_args = json!({"presentation_id": pres_id});
        let (get_result, _) =
            execute_tool("get_bible_presentation", &get_args.to_string(), &state, 320)
                .await
                .unwrap();
        let fetched: Value = serde_json::from_str(&get_result).unwrap();
        assert_eq!(
            fetched["slides"].as_array().unwrap().len() as u64,
            slide_count_before
        );
    }

    #[tokio::test]
    async fn update_bible_slide_runs_validator() {
        let state = AppState::in_memory().await.unwrap();
        // Create a valid presentation with one slide.
        let create_args = json!({
            "name": "Base",
            "slides": [{"main": "1. original text", "main_reference": "Ján 1:1 (MIL)"}]
        });
        let (create_result, _) = execute_tool(
            "create_bible_presentation",
            &create_args.to_string(),
            &state,
            320,
        )
        .await
        .unwrap();
        let created: Value = serde_json::from_str(&create_result).unwrap();
        let pres_id = created["id"].as_str().unwrap().to_string();

        // Fetch to get slide id.
        let get_args = json!({"presentation_id": pres_id});
        let (get_result, _) =
            execute_tool("get_bible_presentation", &get_args.to_string(), &state, 320)
                .await
                .unwrap();
        let fetched: Value = serde_json::from_str(&get_result).unwrap();
        let slide_id = fetched["slides"][0]["id"].as_str().unwrap().to_string();

        // Try to update with raw ## markers.
        let update_args = json!({
            "presentation_id": pres_id,
            "slide_id": slide_id,
            "main": "1. aby sme ##verili##",
            "main_reference": "Ján 1:12 (MIL)"
        });
        let (update_result, _) =
            execute_tool("update_bible_slide", &update_args.to_string(), &state, 320)
                .await
                .unwrap();

        let json: Value = serde_json::from_str(&update_result).unwrap();
        assert_eq!(json["rule"], "unprocessed_bold_markers");

        // Verify the original text is unchanged.
        let (get_after, _) =
            execute_tool("get_bible_presentation", &get_args.to_string(), &state, 320)
                .await
                .unwrap();
        let after: Value = serde_json::from_str(&get_after).unwrap();
        assert_eq!(after["slides"][0]["main"], "1. original text");
    }
}
