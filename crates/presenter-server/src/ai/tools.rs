use crate::state::bible::BibleTriggerOverrides;
use crate::state::AppState;
use presenter_core::slide::{SlideContent, SlideText};
use presenter_core::{BibleReference, LibraryId, PresentationId, Slide, SlideId};
use serde_json::{json, Value};
use uuid::Uuid;

/// Return OpenAI function-calling tool definitions.
pub fn tool_definitions() -> Vec<Value> {
    vec![
        tool_def(
            "list_libraries",
            "List all presentation libraries",
            json!({"type": "object", "properties": {}, "required": []}),
        ),
        tool_def(
            "create_library",
            "Create a new presentation library",
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
            "List presentations in a library",
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
            "Get full presentation detail including slides",
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
            "Create a new presentation with slides in a library",
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
            "Rename an existing presentation",
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
            "Delete a presentation",
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
            "Add a slide to an existing presentation",
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
            "Update the content of an existing slide",
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
            "Delete a slide from a presentation",
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
            "Reorder slides in a presentation",
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
