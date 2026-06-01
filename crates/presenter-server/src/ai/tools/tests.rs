//! Integration-style tests for the AI tool dispatch (`execute_tool`).

use super::execute_tool;
use crate::state::AppState;
use presenter_core::{BiblePresentationId, PresentationId};
use serde_json::{json, Value};
use uuid::Uuid;

#[tokio::test]
async fn add_slide_rejects_missing_required_main() {
    let state = AppState::in_memory().await.unwrap();
    let args = json!({ "presentation_id": Uuid::new_v4().to_string() }).to_string();
    let result = execute_tool("add_slide", &args, &state, 320).await;
    assert!(
        result.is_err(),
        "add_slide without required 'main' must error, not silently default"
    );
}

#[tokio::test]
async fn get_bible_passage_rejects_missing_required_chapter() {
    let state = AppState::in_memory().await.unwrap();
    let args = json!({ "translation": "slk-seb", "book": "Ján", "verse_start": 1 }).to_string();
    let result = execute_tool("get_bible_passage", &args, &state, 320).await;
    assert!(
        result.is_err(),
        "get_bible_passage without required 'chapter' must error, not default to 1"
    );
}

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

    // Slide 0: verses 1-2, ref shows full passage range across all verse items
    assert_eq!(pres.slides[0].main_reference, "Ján 1:1-3 (SEB)");
    assert!(pres.slides[0].main.value().contains("1. Na počiatku"));
    assert!(pres.slides[0].main.value().contains("2. Ono bolo"));

    // Slide 1: emphasis
    assert_eq!(pres.slides[1].main_reference, "");
    assert_eq!(pres.slides[1].main.value(), "NOVÁ ZMLUVA");

    // Slide 2: verse 3, same full passage range as slide 0
    assert_eq!(pres.slides[2].main_reference, "Ján 1:1-3 (SEB)");
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
async fn load_bible_verses_resolves_rohacek_book_name_against_ecumenical_translation() {
    // Regression #310: AI submitted load_bible_verses("1. Mojžišova", ...)
    // against the SEB (ecumenical) translation, which stores the book as
    // "Genezis". The lookup matched book.eq("1. Mojžišova") and returned
    // 0/N verses. The fix: resolve the input book name to its canonical
    // code via canonical_book_by_name() and query by book_code instead.
    use presenter_core::bible::BibleIngestionBatch;
    use presenter_core::{BiblePassage, BibleReference, BibleTranslation};

    let state = AppState::in_memory().await.unwrap();
    let translation = BibleTranslation::new("slk-seb", "Slovenský ekumenický", "sk");
    let reference = BibleReference::new("Genezis", 20, 2, 2).unwrap();
    let passage = BiblePassage::new(
        reference,
        translation.clone(),
        "Abrahám vtedy o svojej manželke...".to_string(),
    );
    let batch = BibleIngestionBatch::new(translation, vec![passage]).unwrap();
    state
        .repository()
        .replace_bible_translation_passages(&batch)
        .await
        .unwrap();

    let args = json!({
        "translation": "slk-seb",
        "book": "1. Mojžišova",
        "chapter": 20,
        "verse_start": 2,
        "verse_end": 2,
    });
    let (body, _preview) = execute_tool("load_bible_verses", &args.to_string(), &state, 320)
        .await
        .unwrap();
    let verses: Vec<Value> =
        serde_json::from_str(&body).expect("response body must be a verses array");
    assert_eq!(verses.len(), 1, "expected 1 verse, got body: {body}");
    assert!(
        verses[0].get("text").is_some(),
        "verse should have text (not error), got: {body}"
    );
    assert!(
        !verses[0]["text"].as_str().unwrap_or("").is_empty(),
        "verse text should be non-empty, got: {body}"
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
