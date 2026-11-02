//! #851 D4: the study API over HTTP (spec §11): status codes and the JSON
//! shape. What the reads answer is pinned in `state/bible_source/study/tests.rs`.

use crate::router::build_router;
use crate::state::AppState;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use presenter_core::bible::{
    BibleIngestionBatch, CrossReference, LexiconEntry, MarkupSegment, StudySource, VerseMarkup,
};
use presenter_core::{BiblePassage, BibleReference, BibleTranslation};
use serde_json::Value;
use tower::ServiceExt;

fn source(code: &str) -> StudySource {
    StudySource {
        code: code.to_string(),
        name: code.to_string(),
        license: "Public Domain".to_string(),
        source_digest: "digest".to_string(),
    }
}

/// KJV: Genesis 1:1 "love God" and Matthew 1:1 "love men", both tagged G26;
/// Matthew 1:2 "in the beginning was love" with G27.
async fn app() -> Router {
    let state = AppState::in_memory().await.expect("state");
    let translation = BibleTranslation::new("eng-kjv", "KJV", "en");
    let verse = |book: &str, code: &str, number: u16, verse: u16, text: &str| {
        let reference =
            BibleReference::new_with_code(book, code, number, 1, verse, verse).expect("reference");
        BiblePassage::new(reference, translation.clone(), text.to_string())
    };
    let passages = vec![
        verse("Genesis", "GEN", 1, 1, "love God"),
        verse("Matthew", "MAT", 40, 1, "love men"),
        verse("Matthew", "MAT", 40, 2, "in the beginning was love"),
    ];
    let markup = passages
        .iter()
        .map(|passage| {
            let strongs = if passage.text.starts_with("in") {
                "G27"
            } else {
                "G26"
            };
            Some(VerseMarkup(vec![MarkupSegment {
                text: passage.text.clone(),
                strongs: vec![strongs.to_string()],
                ..MarkupSegment::default()
            }]))
        })
        .collect();
    let batch = BibleIngestionBatch::new(translation.clone(), passages)
        .expect("batch")
        .with_markup(markup)
        .expect("markup");
    let repository = state.repository();
    repository
        .replace_bible_translation_passages(&batch)
        .await
        .expect("seed");
    let entry = LexiconEntry {
        strongs: "G26".to_string(),
        lemma: "ἀγάπη".to_string(),
        definition: "love".to_string(),
        kjv_renderings: "charity, love".to_string(),
        related: vec!["G25".to_string()],
        ..LexiconEntry::default()
    };
    repository
        .replace_bible_lexicon(&source("strongs-greek"), &[entry])
        .await
        .expect("lexicon");
    let row = CrossReference {
        from_book: 1,
        from_chapter: 1,
        from_verse: 1,
        to_book: 40,
        to_chapter: 1,
        to_verse: 1,
        to_book_end: 40,
        to_chapter_end: 1,
        to_verse_end: 2,
        votes: 7,
    };
    repository
        .replace_bible_cross_references(&source("openbible-xref"), &[row])
        .await
        .expect("references");
    build_router(state)
}

async fn get(app: &Router, uri: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .uri(uri)
        .body(Body::empty())
        .expect("request");
    let response = app.clone().oneshot(request).await.expect("response");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn chapter_answers_camel_case_and_404_for_the_unknown() {
    let app = app().await;
    let uri = "/bible/study/chapter?translation=eng-kjv&book_code=MAT&chapter=1";
    let (status, body) = get(&app, uri).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["translation"], "eng-kjv");
    assert_eq!(body["bookCode"], "MAT");
    assert_eq!(body["bookNumber"], 40);
    assert_eq!(body["book"], "Matthew");
    assert_eq!(body["verseCount"], 2);
    assert_eq!(body["prev"]["bookCode"], "GEN");
    assert_eq!(body["prev"]["chapter"], 1);
    assert_eq!(body["next"], Value::Null);
    assert_eq!(body["verses"][0]["verse"], 1);
    assert_eq!(body["verses"][0]["text"], "love men");
    assert_eq!(body["verses"][0]["markup"][0]["s"][0], "G26");

    for uri in [
        "/bible/study/chapter?translation=xxx&book_code=MAT&chapter=1",
        "/bible/study/chapter?translation=eng-kjv&book_code=XXX&chapter=1",
    ] {
        assert_eq!(get(&app, uri).await.0, StatusCode::NOT_FOUND, "{uri}");
    }
}

#[tokio::test]
async fn lexicon_counts_the_verses_and_refuses_a_bad_id() {
    let app = app().await;
    let (status, body) = get(&app, "/bible/study/lexicon/g026").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["strongs"], "G26");
    assert_eq!(body["lemma"], "ἀγάπη");
    assert_eq!(body["kjvRenderings"], "charity, love");
    assert_eq!(body["related"][0], "G25");
    assert_eq!(body["occurrenceCount"], 2);

    let (status, _) = get(&app, "/bible/study/lexicon/G26a").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = get(&app, "/bible/study/lexicon/G27").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn strongs_pages_a_scope_and_counts_every_book() {
    let app = app().await;
    let (status, body) = get(&app, "/bible/study/strongs/G26?scope=nt&limit=1").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["strongs"], "G26");
    assert_eq!(body["total"], 1);
    assert_eq!(body["perBook"].as_array().map(Vec::len), Some(2));
    assert_eq!(body["perBook"][0]["bookCode"], "GEN");
    assert_eq!(body["perBook"][1]["count"], 1);
    assert_eq!(body["items"][0]["bookCode"], "MAT");
    assert_eq!(body["items"][0]["book"], "Matthew");
    assert_eq!(body["items"][0]["text"], "love men");
    assert_eq!(body["items"][0]["markup"][0]["s"][0], "G26");

    let (_, all) = get(&app, "/bible/study/strongs/G26?offset=1").await;
    assert_eq!(all["total"], 2);
    assert_eq!(all["items"].as_array().map(Vec::len), Some(1));

    let (status, _) = get(&app, "/bible/study/strongs/G26?scope=nowhere").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = get(&app, "/bible/study/strongs/26").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn crossrefs_answer_the_list_and_404_for_the_unknown() {
    let app = app().await;
    let uri = "/bible/study/crossrefs?book_code=GEN&chapter=1&verse=1&translation=eng-kjv";
    let (status, body) = get(&app, uri).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], 1);
    let item = &body["items"][0];
    assert_eq!(item["label"], "Matthew 1:1-2");
    assert_eq!(item["bookCode"], "MAT");
    assert_eq!(item["chapterEnd"], 1);
    assert_eq!(item["verseEnd"], 2);
    assert_eq!(item["votes"], 7);
    assert_eq!(item["text"], "love men in the beginning was love");
    assert!(item.get("fallbackTranslation").is_none());

    let uri = "/bible/study/crossrefs?book_code=XXX&chapter=1&verse=1&translation=eng-kjv";
    assert_eq!(get(&app, uri).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn search_takes_a_scope_and_a_quoted_phrase() {
    let app = app().await;
    let (status, ot) = get(
        &app,
        "/bible/search?translation=eng-kjv&query=love&scope=ot",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ot.as_array().map(Vec::len), Some(1));
    assert_eq!(ot[0]["reference"]["book"], "Genesis");

    let (_, book) = get(
        &app,
        "/bible/search?translation=eng-kjv&query=love&scope=MAT",
    )
    .await;
    assert_eq!(book.as_array().map(Vec::len), Some(2));

    let (_, all) = get(&app, "/bible/search?translation=eng-kjv&query=love").await;
    assert_eq!(all.as_array().map(Vec::len), Some(3));

    let (_, phrase) = get(
        &app,
        "/bible/search?translation=eng-kjv&query=%22beginning%20was%20love%22",
    )
    .await;
    assert_eq!(phrase.as_array().map(Vec::len), Some(1));
    let (_, title) = get(
        &app,
        "/bible/search?translation=eng-kjv&query=%22matthew%22",
    )
    .await;
    assert_eq!(title.as_array().map(Vec::len), Some(0));
    let (_, empty) = get(&app, "/bible/search?translation=eng-kjv&query=%22%22").await;
    assert_eq!(empty.as_array().map(Vec::len), Some(0));
    let (_, marks) = get(&app, "/bible/search?translation=eng-kjv&query=%22-%22").await;
    assert_eq!(marks.as_array().map(Vec::len), Some(0));
    let (_, broken) = get(
        &app,
        "/bible/search?translation=eng-kjv&query=%22love%20beginning%22",
    )
    .await;
    assert_eq!(broken.as_array().map(Vec::len), Some(0));

    let (status, _) = get(&app, "/bible/search?query=love&scope=nowhere").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
