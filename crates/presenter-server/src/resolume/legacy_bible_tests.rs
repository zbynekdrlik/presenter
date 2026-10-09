//! #824 phase 2: the legacy `/bible/trigger` path (Companion, the AI
//! `trigger_bible` tool) must name the SECONDARY translation's book in
//! `#bible-translate-reference` — "1 John 1:1-3 (KJV)", not the main
//! "1 Ján 1:1-3 (KJV)". Own file: `tests.rs` carries the #487 length debt.

use super::legacy_reference::legacy_translation_reference;
use super::tests::setup_bible_driver;
use super::BibleUpdate;
use chrono::Utc;
use presenter_core::{BibleBroadcast, BiblePassage, BibleReference, BibleTranslation};

fn first_jan(verse_start: u16, verse_end: u16) -> BibleReference {
    BibleReference::new_with_code("1 Ján", "1JN", 62, 1, verse_start, verse_end).expect("reference")
}

#[test]
fn the_translation_reference_names_the_secondary_book() {
    assert_eq!(
        legacy_translation_reference(&first_jan(1, 3), Some("1 John"), Some("eng-kjv")),
        "1 John 1:1-3 (KJV)"
    );
}

#[test]
fn without_a_secondary_book_the_main_book_name_stays() {
    assert_eq!(
        legacy_translation_reference(&first_jan(1, 1), None, Some("eng-kjv")),
        "1 Ján 1:1 (KJV)"
    );
    assert_eq!(
        legacy_translation_reference(&first_jan(1, 1), Some("  "), Some("eng-kjv")),
        "1 Ján 1:1 (KJV)"
    );
}

#[test]
fn without_a_secondary_translation_there_is_no_translation_reference() {
    assert_eq!(
        legacy_translation_reference(&first_jan(1, 3), Some("1 John"), None),
        ""
    );
}

/// Bodies of every `PUT /parameter/by-id/{param_id}` the mock received.
fn put_bodies(requests: &[wiremock::Request], param_id: i64) -> Vec<String> {
    let wanted = format!("/api/v1/parameter/by-id/{param_id}");
    requests
        .iter()
        .filter(|req| req.method.as_str() == "PUT" && req.url.path() == wanted)
        .map(|req| String::from_utf8_lossy(&req.body).into_owned())
        .collect()
}

#[tokio::test]
async fn a_legacy_push_sends_the_secondary_book_to_the_translate_reference_clip() {
    // The shared 16-clip Arena carries every Bible lane clip (#bible-a/b,
    // #bible-reference-a/b, #bible-translate-a/b, #bible-translate-reference-a/b).
    let (server, mut driver, status) = setup_bible_driver().await;
    let passage = BiblePassage::new(
        first_jan(1, 3),
        BibleTranslation::new("slk-seb", "Slovenský ekumenický preklad", "sk"),
        "1. Čo bolo od počiatku.".to_string(),
    );
    let update = BibleUpdate {
        passage: Some(BibleBroadcast::new(passage, Utc::now())),
        secondary_text: Some("1. That which was from the beginning.".to_string()),
        secondary_translation_code: Some("eng-kjv".to_string()),
        secondary_book: Some("1 John".to_string()),
        slide_output: None,
    };

    driver.handle_bible(update, &status).await.expect("bible");

    let requests = server.received_requests().await.expect("requests");
    let translate_reference = put_bodies(&requests, 45);
    assert!(
        translate_reference
            .iter()
            .any(|body| body.contains("1 John 1:1-3 (KJV)")),
        "{translate_reference:?}"
    );
    assert!(
        translate_reference
            .iter()
            .all(|body| !body.contains("1 Ján")),
        "{translate_reference:?}"
    );
    // The main reference keeps the main translation's book name.
    let main_reference = put_bodies(&requests, 35);
    assert!(
        main_reference
            .iter()
            .any(|body| body.contains("1 Ján 1:1-3 (SEB)")),
        "{main_reference:?}"
    );
}
