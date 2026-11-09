//! #850 §5.8, S2: every form of a book and a translation works through the
//! server's entry points; S10: the lines a lookup leaves.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use presenter_core::bible::BibleIngestionBatch;
use presenter_core::{BiblePassage, BiblePreferences, BibleReference, BibleTranslation};
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::router::build_router;
use crate::state::AppState;
use crate::tracing_capture::{quoted, Capture};

const VIERA: &str = "Viera je podstatou toho, v čo dúfame";

async fn seed_book(state: &AppState, code: &str, name: &str, chapters: &[u16]) {
    let translation = BibleTranslation::new(code, code, "sk");
    let passages = chapters
        .iter()
        .flat_map(|chapter| (1..=3u16).map(move |verse| (*chapter, verse)))
        .map(|(chapter, verse)| {
            let reference =
                BibleReference::new_with_code(name, "HEB", 58, chapter, verse, verse).unwrap();
            let text = format!("{VIERA} {chapter}:{verse}");
            BiblePassage::new(reference, translation.clone(), text)
        })
        .collect();
    let batch = BibleIngestionBatch::new(translation.clone(), passages).unwrap();
    state
        .repository()
        .replace_bible_translation_passages(&batch)
        .await
        .unwrap();
}

async fn seeded() -> AppState {
    let state = AppState::in_memory().await.unwrap();
    seed_book(&state, "slk-seb", "Hebrejom", &[1, 11, 13]).await;
    seed_book(&state, "eng-kjv", "Hebrews", &[1, 11, 13]).await;
    state
        .set_bible_preferences(BiblePreferences {
            main_translation: Some("slk-seb".to_string()),
            secondary_translation: None,
            character_limit: 320,
            ..BiblePreferences::default()
        })
        .await
        .unwrap();
    state
}

async fn get(state: &AppState, uri: &str) -> (StatusCode, Value) {
    let response = build_router(state.clone())
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn lookup_answers_a_typed_reference_with_the_stored_name() {
    let state = seeded().await;
    let (status, body) = get(&state, "/bible/lookup?q=zid%2011%201").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["query"], "zid 11 1");
    assert_eq!(body["translation"], "slk-seb");
    assert_eq!(body["kind"], "passage");
    assert_eq!(body["default"]["bookCode"], "HEB");
    assert_eq!(body["default"]["book"], "Hebrejom");
    assert_eq!(body["default"]["label"], "Hebrejom 11:1");
    assert_eq!(
        body["default"]["segments"],
        json!([{ "chapter": 11, "verseStart": 1, "verseEnd": 1 }])
    );
    assert_eq!(body["hint"], Value::Null);
}

#[tokio::test]
async fn lookup_past_the_book_end_gives_a_hint_and_a_fix() {
    let state = seeded().await;
    let (status, body) = get(&state, "/bible/lookup?q=zid%2014").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["kind"], "outOfRange");
    assert_eq!(body["hint"], "Kniha má len 13 kapitol");
    assert_eq!(body["fix"]["label"], "Hebrejom 13");
}

#[tokio::test]
async fn lookup_refuses_an_empty_query_and_an_unknown_translation() {
    let state = seeded().await;
    assert_eq!(
        get(&state, "/bible/lookup?q=%20").await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        get(&state, "/bible/lookup?q=zid&translation=nope").await.0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn lookup_takes_a_translation_by_its_short_code() {
    let state = seeded().await;
    let (_, body) = get(&state, "/bible/lookup?q=Heb%2011:1&translation=KJV").await;
    assert_eq!(body["translation"], "eng-kjv");
    assert_eq!(body["default"]["label"], "Hebrews 11:1");
}

#[tokio::test]
async fn lookup_leaves_one_line_with_what_was_typed() {
    let state = seeded().await;
    let capture = Capture::install();
    get(&state, "/bible/lookup?q=zid%2011%201").await;
    let lines = capture.events("presenter::bible::lookup");
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0].field("q"), Some(quoted("zid 11 1").as_str()));
    assert_eq!(lines[0].field("kind"), Some(quoted("passage").as_str()));
    assert_eq!(lines[0].field("book"), Some(quoted("HEB").as_str()));
    assert_eq!(lines[0].field("chapter"), Some("11"));
    assert_eq!(lines[0].field("verses"), Some(quoted("1-1").as_str()));
}

#[tokio::test]
async fn passage_endpoint_reads_every_form_of_the_name_on_seb() {
    let state = seeded().await;
    for form in ["Hebrejom", "%C5%BDidom", "zid", "Hebr", "Hebrews", "HEB"] {
        let uri = format!("/bible/passage?translation=SEB&book={form}&chapter=11&verse_start=1");
        let (status, body) = get(&state, &uri).await;
        assert_eq!(status, StatusCode::OK, "{form}");
        assert_eq!(body["reference"]["book"], "Hebrejom", "{form}: {body}");
    }
}

#[tokio::test]
async fn passage_endpoint_gives_404_for_an_unknown_translation_and_none_for_an_unknown_book() {
    let state = seeded().await;
    let unknown = "/bible/passage?translation=nope&book=zid&chapter=11&verse_start=1";
    assert_eq!(get(&state, unknown).await.0, StatusCode::NOT_FOUND);
    let no_book = "/bible/passage?translation=SEB&book=qqqqqqqq&chapter=11&verse_start=1";
    let (status, body) = get(&state, no_book).await;
    assert_eq!((status, body), (StatusCode::OK, Value::Null));
}

#[tokio::test]
async fn a_typed_name_logs_what_it_meant_and_the_stored_name_logs_nothing() {
    let state = seeded().await;
    let capture = Capture::install();
    let stored = "/bible/passage?translation=slk-seb&book=Hebrejom&chapter=11&verse_start=1";
    get(&state, stored).await;
    assert!(capture.events("presenter::bible::name").is_empty());

    let typed = "/bible/passage?translation=slk-seb&book=zid&chapter=11&verse_start=1";
    get(&state, typed).await;
    let lines = capture.events("presenter::bible::name");
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0].level, tracing::Level::INFO);
    assert_eq!(lines[0].field("name"), Some(quoted("zid").as_str()));
    assert_eq!(lines[0].field("code"), Some(quoted("HEB").as_str()));
    assert!(lines[0].field("match").is_some());
}

#[tokio::test]
async fn legacy_trigger_takes_a_typed_name_and_a_short_code_and_shows_the_stored_name() {
    let state = seeded().await;
    let body = json!({ "translation": "KJV", "book": "zid", "chapter": 11, "verseStart": 1 });
    let response = build_router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/bible/trigger")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let (_, active) = get(&state, "/bible/active-slide").await;
    let reference = active["mainReference"].as_str().unwrap_or_default();
    assert!(reference.starts_with("Hebrews 11:1"), "{active}");
    let (_, broadcast) = get(&state, "/bible/active").await;
    assert_eq!(
        broadcast["passage"]["reference"]["book"], "Hebrews",
        "{broadcast}"
    );
}

#[tokio::test]
async fn slides_resolve_from_a_typed_name_with_the_stored_label() {
    let state = seeded().await;
    let generated = state
        .generate_bible_slides("slk-seb", None, "Hebr", None, 11, 1, 2, 320)
        .await
        .unwrap();
    assert_eq!(generated.slides.len(), 1);
    assert!(generated.slides[0].content.main.value().contains(VIERA));
}
