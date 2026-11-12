use serde_json::{json, Value};

/// Return OpenAI function-calling tool definitions.
pub fn tool_definitions() -> Vec<Value> {
    [
        library_tools(),
        slide_tools(),
        bible_tools(),
        stage_tools(),
        bible_presentation_tools(),
    ]
    .concat()
}

fn library_tools() -> Vec<Value> {
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
    ]
}

fn slide_tools() -> Vec<Value> {
    vec![
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
    ]
}

fn bible_tools() -> Vec<Value> {
    vec![
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
                    "book": {"type": "string", "description": "Any book form (Židom, Hebr, Heb, Hebrews)"},
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
            "load_bible_verses",
            "[BIBLE only] Load raw verse text from the database for a passage range. Returns an array of {number, text, reference} objects — NOT pre-split slides. Use this as the source of truth for verse text when building a bible presentation. Compare each returned verse to the sermon wording and override `text` where they differ.",
            json!({
                "type": "object",
                "properties": {
                    "translation": {"type": "string", "description": "Translation code (e.g. slk-seb)"},
                    "book": {"type": "string", "description": "Any book form (Židom, Hebr, Heb, Hebrews)"},
                    "chapter": {"type": "integer"},
                    "verse_start": {"type": "integer"},
                    "verse_end": {"type": "integer"}
                },
                "required": ["translation", "book", "chapter", "verse_start", "verse_end"]
            }),
        ),
        tool_def(
            "resolve_bible_slides",
            "Generate slides from a Bible passage, automatically split by character limit",
            json!({
                "type": "object",
                "properties": {
                    "translation": {"type": "string", "description": "Translation code"},
                    "book": {"type": "string", "description": "Any book form (Židom, Hebr, Heb, Hebrews)"},
                    "chapter": {"type": "integer"},
                    "verse_start": {"type": "integer"},
                    "verse_end": {"type": "integer"},
                    "character_limit": {"type": "integer", "description": "Max chars per slide (default 320)"}
                },
                "required": ["translation", "book", "chapter", "verse_start", "verse_end"]
            }),
        ),
    ]
}

fn stage_tools() -> Vec<Value> {
    vec![
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
                    "book": {"type": "string", "description": "Any book form (Židom, Hebr, Heb, Hebrews)"},
                    "chapter": {"type": "integer"},
                    "verse_start": {"type": "integer"},
                    "verse_end": {"type": "integer"}
                },
                "required": ["translation", "book", "chapter", "verse_start"]
            }),
        ),
    ]
}

/// One verse/emphasis item of `create_bible_presentation` and
/// `update_bible_presentation` (#939) — one schema for both tools.
fn bible_item_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "kind": {"type": "string", "enum": ["verse", "emphasis"]},
            "number": {"type": "integer", "description": "[verse] Verse number"},
            "text": {"type": "string", "description": "Verse text (with any uppercase ##word## transformations applied) or the emphasis phrase"},
            "book": {"type": "string", "description": "[verse] Full book name (e.g. Ján)"},
            "chapter": {"type": "integer", "description": "[verse] Chapter number"},
            "translation": {"type": "string", "description": "[verse] Short translation code (e.g. SEB, MIL, ROH)"}
        },
        "required": ["kind", "text"]
    })
}

fn bible_presentation_tools() -> Vec<Value> {
    vec![
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
            "[BIBLE only] Create a NEW Bible presentation from a stream of typed items. The server composes slides from your items — you do NOT decide where slide breaks happen. Emphasis items and a new passage force slide breaks: another translation/book/chapter, or a gap in the verse numbers (1Sa 4:1-11 then 1Sa 4:20-22 are two passages with two references, 4:1-11 and 4:20-22). Otherwise consecutive verse items pack together until the character limit. Always call load_bible_verses first to get DB verse text, edit the text to match the sermon where needed, then assemble items[] in sermon order. Call it ONCE per request: to fix or change a presentation you already made, use update_bible_presentation on it — never create a second, test or experiment presentation.",
            json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "Presentation name (e.g. 'Sunday Sermon 2026-04-14')"},
                    "items": {
                        "type": "array",
                        "description": "Ordered stream of verse and emphasis items. The server composes slides respecting the character limit.",
                        "items": bible_item_schema()
                    }
                },
                "required": ["name", "items"]
            }),
        ),
        tool_def(
            "update_bible_presentation",
            "[BIBLE only] Correct an EXISTING Bible presentation: replaces ALL its slides with slides composed from items[] (same items and rules as create_bible_presentation), optionally renaming it. Use this whenever the user asks to fix, change or redo a presentation you created — the presentation keeps its id, no new presentation appears.",
            json!({
                "type": "object",
                "properties": {
                    "presentation_id": {"type": "string", "description": "UUID of the Bible presentation to correct"},
                    "name": {"type": "string", "description": "Optional new name"},
                    "items": {
                        "type": "array",
                        "description": "The COMPLETE ordered item stream of the corrected presentation (verse and emphasis items, as for create_bible_presentation).",
                        "items": bible_item_schema()
                    }
                },
                "required": ["presentation_id", "items"]
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
