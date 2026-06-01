//! Bible lookup, verse-loading, slide-resolution and trigger tools.

use super::{slide_to_json, str_field, u64_field};
use crate::state::bible::BibleTriggerOverrides;
use crate::state::AppState;
use presenter_core::BibleReference;
use serde_json::{json, Value};

pub(super) async fn search_bible(
    args: &Value,
    state: &AppState,
) -> anyhow::Result<(String, String)> {
    let query = str_field(args, "query")?;
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

pub(super) async fn get_bible_passage(
    args: &Value,
    state: &AppState,
) -> anyhow::Result<(String, String)> {
    let translation = str_field(args, "translation")?;
    let book = str_field(args, "book")?;
    let chapter = u64_field(args, "chapter")? as u16;
    let verse_start = u64_field(args, "verse_start")? as u16;
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

pub(super) async fn list_bible_translations(state: &AppState) -> anyhow::Result<(String, String)> {
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

pub(super) async fn load_bible_verses(
    args: &Value,
    state: &AppState,
) -> anyhow::Result<(String, String)> {
    let translation = str_field(args, "translation")?;
    let book = str_field(args, "book")?;
    let chapter = u64_field(args, "chapter")? as u16;
    let verse_start = u64_field(args, "verse_start")? as u16;
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
                json!({"error": "translation not found", "translation": translation}).to_string(),
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

    // Resolve the input book name to its canonical code so the
    // query matches regardless of which Slovak naming tradition the
    // AI used. Without this step the Roháček "1. Mojžišova" would
    // miss SEB rows stored as "Genezis" and return 0 verses (#310).
    // Falls back to raw book-name filter when the alias map has no
    // entry — preserves behavior for languages without canonical
    // coverage.
    let resolved_code = presenter_core::bible::canonical_book_by_name(&book).map(|m| m.code);
    // Single range query instead of per-verse round trips. The
    // repository returns only the verses that exist, so we walk
    // the requested range and fill gaps with explicit not-found
    // entries — the LLM sees exactly which verses are missing
    // and can decide whether to use the sermon's text as-is,
    // shorten the range, or report back to the user.
    let loaded = state
        .bible_passage_range(
            &main_trans.code,
            &book,
            resolved_code,
            chapter,
            verse_start,
            verse_end,
        )
        .await?;
    let loaded_by_number: std::collections::HashMap<u16, &presenter_core::BiblePassage> = loaded
        .iter()
        .map(|p| (p.reference.verse_start, p))
        .collect();

    let mut verses: Vec<Value> = Vec::with_capacity((verse_end - verse_start + 1) as usize);
    let mut found_count: usize = 0;
    for v in verse_start..=verse_end {
        match loaded_by_number.get(&v) {
            Some(p) => {
                found_count += 1;
                verses.push(json!({
                    "number": p.reference.verse_start,
                    "text": p.text,
                    "reference": format!(
                        "{} {}:{} ({})",
                        p.reference.book,
                        p.reference.chapter,
                        p.reference.verse_start,
                        short_code,
                    ),
                }));
            }
            None => {
                verses.push(json!({
                    "number": v,
                    "error": "not_found",
                    "reference": format!(
                        "{} {}:{} ({})",
                        book, chapter, v, short_code
                    ),
                }));
            }
        }
    }

    let preview = format!(
        "{} {}:{}-{} ({}) - {}/{} verses",
        book,
        chapter,
        verse_start,
        verse_end,
        short_code,
        found_count,
        verses.len()
    );
    Ok((serde_json::to_string(&verses)?, preview))
}

pub(super) async fn resolve_bible_slides(
    args: &Value,
    state: &AppState,
    default_char_limit: u32,
) -> anyhow::Result<(String, String)> {
    let translation = str_field(args, "translation")?;
    let book = str_field(args, "book")?;
    let chapter = u64_field(args, "chapter")? as u16;
    let verse_start = u64_field(args, "verse_start")? as u16;
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

pub(super) async fn trigger_bible_verse(
    args: &Value,
    state: &AppState,
) -> anyhow::Result<(String, String)> {
    let translation = str_field(args, "translation")?;
    let book = str_field(args, "book")?;
    let chapter = u64_field(args, "chapter")? as u16;
    let verse_start = u64_field(args, "verse_start")? as u16;
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
