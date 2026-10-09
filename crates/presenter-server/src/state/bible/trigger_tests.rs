//! #824 phase 2: the secondary side of a legacy `/bible/trigger` carries the
//! book name the SECONDARY translation uses, so `handle_bible_legacy` can name
//! it in `#bible-translate-reference` ("1 John …", not the main "1 Ján …").

use super::*;
use crate::bible_remote::{NltClient, NLT_CODE};
use presenter_core::bible::BibleIngestionBatch;
use std::time::Duration;
use wiremock::matchers::path;
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn seed(state: &AppState, code: &str, language: &str, book: &str, texts: &[&str]) {
    let translation = BibleTranslation::new(code, code, language);
    let passages = texts
        .iter()
        .enumerate()
        .map(|(index, text)| {
            let verse = index as u16 + 1;
            let reference =
                BibleReference::new_with_code(book, "1JN", 62, 1, verse, verse).expect("reference");
            presenter_core::BiblePassage::new(reference, translation.clone(), (*text).to_string())
        })
        .collect();
    let batch = BibleIngestionBatch::new(translation, passages).expect("batch");
    state
        .repository
        .replace_bible_translation_passages(&batch)
        .await
        .expect("seed");
}

/// SEB + KJV with 1 John 1:1-3, KJV saved as the secondary translation.
async fn seb_main_kjv_secondary() -> AppState {
    let state = AppState::in_memory().await.expect("state");
    seed(
        &state,
        "slk-seb",
        "sk",
        "1 Ján",
        &[
            "Čo bolo od počiatku.",
            "Lebo život sa zjavil.",
            "Čo sme videli.",
        ],
    )
    .await;
    seed(
        &state,
        "eng-kjv",
        "en",
        "1 John",
        &[
            "That which was from the beginning.",
            "For the life was manifested.",
            "That which we have seen.",
        ],
    )
    .await;
    state
        .set_bible_preferences(
            BiblePreferences::default().with_secondary_translation(Some("eng-kjv".to_string())),
        )
        .await
        .expect("preferences");
    state
}

#[tokio::test]
async fn the_secondary_side_names_the_secondary_translations_book() {
    let state = seb_main_kjv_secondary().await;
    let reference = BibleReference::new_with_code("1 Ján", "1JN", 62, 1, 1, 3).expect("reference");

    let secondary = state
        .trigger_secondary_text(&reference, None)
        .await
        .expect("secondary");

    assert_eq!(secondary.translation_code.as_deref(), Some("eng-kjv"));
    assert_eq!(secondary.book.as_deref(), Some("1 John"));
    assert!(secondary
        .text
        .as_deref()
        .is_some_and(|text| text.starts_with("1. That which was from the beginning.")));
}

#[tokio::test]
async fn a_reference_without_a_book_code_still_finds_the_secondary_verses() {
    // The AI tool and some callers send only the main book name ("1 Ján"),
    // which eng-kjv does not use — the canonical code must find its rows.
    let state = seb_main_kjv_secondary().await;
    let reference = BibleReference::new("1 Ján", 1, 1, 2).expect("reference");

    let secondary = state
        .trigger_secondary_text(&reference, None)
        .await
        .expect("secondary");

    assert_eq!(secondary.book.as_deref(), Some("1 John"));
    assert!(secondary
        .text
        .as_deref()
        .is_some_and(|text| text.contains("2. For the life was manifested.")));
}

#[tokio::test]
async fn an_edited_secondary_text_still_names_the_secondary_book() {
    let state = seb_main_kjv_secondary().await;
    let reference = BibleReference::new_with_code("1 Ján", "1JN", 62, 1, 1, 1).expect("reference");

    let secondary = state
        .trigger_secondary_text(&reference, Some("Edited English line.".to_string()))
        .await
        .expect("secondary");

    assert_eq!(secondary.text.as_deref(), Some("Edited English line."));
    assert_eq!(secondary.translation_code.as_deref(), Some("eng-kjv"));
    assert_eq!(secondary.book.as_deref(), Some("1 John"));
}

#[tokio::test]
async fn without_a_secondary_translation_there_is_no_secondary_book() {
    let state = AppState::in_memory().await.expect("state");
    let reference = BibleReference::new("1 Ján", 1, 1, 1).expect("reference");

    let secondary = state
        .trigger_secondary_text(&reference, None)
        .await
        .expect("secondary");

    assert_eq!(secondary.text, None);
    assert_eq!(secondary.translation_code, None);
    assert_eq!(secondary.book, None);
}

#[tokio::test]
async fn a_blank_book_code_still_finds_the_secondary_verses() {
    let state = seb_main_kjv_secondary().await;
    let reference = BibleReference {
        book: "1 Ján".to_string(),
        book_code: Some(String::new()),
        book_number: None,
        chapter: 1,
        verse_start: 1,
        verse_end: 1,
    };

    let secondary = state
        .trigger_secondary_text(&reference, None)
        .await
        .expect("secondary");

    assert_eq!(secondary.book.as_deref(), Some("1 John"));
    assert!(secondary.text.is_some());
}

#[tokio::test]
async fn an_edited_nlt_secondary_names_its_book_without_calling_the_api() {
    // The NLT's book names are eng-kjv's (#826): looking one up must not
    // cost an API request (up to a 10 s timeout) before the slide goes on air.
    let mut state = seb_main_kjv_secondary().await;
    let server = MockServer::start().await;
    Mock::given(path("/api/passages"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    state.set_test_nlt_client(NltClient::for_test(&server.uri(), Duration::from_secs(5)));
    state
        .set_bible_preferences(
            BiblePreferences::default().with_secondary_translation(Some(NLT_CODE.to_string())),
        )
        .await
        .expect("preferences");
    let reference = BibleReference::new_with_code("1 Ján", "1JN", 62, 1, 1, 1).expect("reference");

    let secondary = state
        .trigger_secondary_text(&reference, Some("Edited NLT line.".to_string()))
        .await
        .expect("secondary");

    assert_eq!(secondary.text.as_deref(), Some("Edited NLT line."));
    assert_eq!(secondary.book.as_deref(), Some("1 John"));
    server.verify().await;
}
