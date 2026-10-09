//! #826: the translation-source dispatch. SEB and KJV rows are seeded into
//! the in-memory repository; the NLT comes from a mock API (an external
//! network service — the one thing tests may mock).

use crate::bible_remote::{NltClient, RemoteBibleError, NLT_CODE};
use crate::state::AppState;
use presenter_core::bible::BibleIngestionBatch;
use presenter_core::slide::BibleSlideMetadata;
use presenter_core::{BiblePassage, BiblePreferences, BibleReference, BibleTranslation, Slide};
use std::time::Duration;
use wiremock::matchers::{path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const FIXTURE_1JN: &str = include_str!("../../bible_remote/fixtures/nlt_1jn_1_1-3.html");
const FIXTURE_PS3: &str = include_str!("../../bible_remote/fixtures/nlt_ps_3_1-3.html");

/// One book of one translation: `(name, code, number, chapter, verse texts)`.
struct SeedBook<'a> {
    name: &'a str,
    code: &'a str,
    number: u16,
    chapter: u16,
    verses: &'a [&'a str],
}

async fn seed(state: &AppState, translation: BibleTranslation, books: &[SeedBook<'_>]) {
    let mut passages = Vec::new();
    for book in books {
        for (index, text) in book.verses.iter().enumerate() {
            let verse = index as u16 + 1;
            let reference = BibleReference::new_with_code(
                book.name,
                book.code,
                book.number,
                book.chapter,
                verse,
                verse,
            )
            .expect("reference");
            passages.push(BiblePassage::new(
                reference,
                translation.clone(),
                (*text).to_string(),
            ));
        }
    }
    let batch = BibleIngestionBatch::new(translation, passages).expect("batch");
    state
        .repository()
        .replace_bible_translation_passages(&batch)
        .await
        .expect("seed translation");
}

async fn seed_seb(state: &AppState) {
    let book = SeedBook {
        name: "1 Ján",
        code: "1JN",
        number: 62,
        chapter: 1,
        verses: &[
            "Čo bolo od počiatku.",
            "Lebo život sa zjavil.",
            "Čo sme videli a počuli.",
        ],
    };
    let translation = BibleTranslation::new("slk-seb", "Slovenský ekumenický preklad", "sk");
    seed(state, translation, &[book]).await;
}

async fn seed_kjv(state: &AppState) {
    let first_john = SeedBook {
        name: "1 John",
        code: "1JN",
        number: 62,
        chapter: 1,
        verses: &[
            "That which was from the beginning.",
            "For the life was manifested.",
            "That which we have seen and heard.",
        ],
    };
    // eng-kjv names this book "Psalms"; the canonical English name is "Psalm".
    let psalms = SeedBook {
        name: "Psalms",
        code: "PSA",
        number: 19,
        chapter: 1,
        verses: &[
            "Blessed is the man.",
            "But his delight.",
            "And he shall be.",
        ],
    };
    let translation = BibleTranslation::new("eng-kjv", "King James Version", "en");
    seed(state, translation, &[first_john, psalms]).await;
}

async fn seb_and_kjv() -> AppState {
    let state = AppState::in_memory().await.expect("state");
    seed_seb(&state).await;
    seed_kjv(&state).await;
    state
}

/// A mock NLT API answering `ref=<reference>` with `response`, expected
/// exactly `expect` times, wired into `state`.
async fn mock_nlt(
    state: &mut AppState,
    reference: &str,
    response: ResponseTemplate,
    expect: u64,
) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(path("/api/passages"))
        .and(query_param("ref", reference))
        .respond_with(response)
        .expect(expect)
        .mount(&server)
        .await;
    state.set_test_nlt_client(NltClient::for_test(&server.uri(), Duration::from_secs(5)));
    server
}

fn page(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_string(body)
}

fn bible_metadata(slide: &Slide) -> &BibleSlideMetadata {
    slide
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.bible.as_ref())
        .expect("bible metadata")
}

#[tokio::test]
async fn nlt_is_listed_once_and_last_when_kjv_is_installed() {
    let state = seb_and_kjv().await;

    let codes: Vec<String> = state
        .list_bible_translations()
        .await
        .expect("translations")
        .into_iter()
        .map(|translation| translation.code)
        .collect();

    assert_eq!(
        codes.last().map(String::as_str),
        Some(NLT_CODE),
        "{codes:?}"
    );
    assert_eq!(codes.iter().filter(|code| *code == NLT_CODE).count(), 1);
    assert!(codes.contains(&"slk-seb".to_string()));
}

#[tokio::test]
async fn nlt_is_not_listed_without_kjv_its_book_structure() {
    let state = AppState::in_memory().await.expect("state");
    seed_seb(&state).await;

    let codes: Vec<String> = state
        .list_bible_translations()
        .await
        .expect("translations")
        .into_iter()
        .map(|translation| translation.code)
        .collect();

    assert_eq!(codes, vec!["slk-seb".to_string()]);
}

#[tokio::test]
async fn seb_main_with_nlt_secondary_names_the_book_in_english_with_nlt() {
    let mut state = seb_and_kjv().await;
    let server = mock_nlt(&mut state, "1Jn.1.1-3", page(FIXTURE_1JN), 1).await;

    let (main, secondary, slides) = state
        .generate_bible_slides(
            "slk-seb",
            Some(NLT_CODE),
            "1 Ján",
            Some("1JN"),
            1,
            1,
            3,
            2000,
        )
        .await
        .expect("slides");

    assert_eq!(main.code, "slk-seb");
    assert_eq!(secondary.map(|t| t.code).as_deref(), Some(NLT_CODE));
    assert_eq!(slides.len(), 1);
    let bible = bible_metadata(&slides[0]);
    assert_eq!(
        bible.main_reference_label.as_deref(),
        Some("1 Ján 1:1-3 (SEB)")
    );
    assert_eq!(
        bible.translation_reference_label.as_deref(),
        Some("1 John 1:1-3 (NLT)")
    );
    let translation_text = slides[0].content.translation.value();
    assert!(
        translation_text.starts_with(
            "1. We proclaim to you the one who existed from the beginning, whom we have heard"
        ),
        "{translation_text}"
    );
    assert!(
        translation_text.contains("\n3. We proclaim to you what we ourselves"),
        "{translation_text}"
    );
    server.verify().await;
}

#[tokio::test]
async fn nlt_passages_take_the_kjv_book_name() {
    let mut state = seb_and_kjv().await;
    let server = mock_nlt(&mut state, "Ps.3.1-3", page(FIXTURE_PS3), 1).await;

    let passages = state
        .bible_passage_range(NLT_CODE, "Psalm", Some("PSA"), 3, 1, 3)
        .await
        .expect("passages");

    assert_eq!(passages.len(), 3);
    assert!(passages.iter().all(|p| p.reference.book == "Psalms"));
    assert!(passages.iter().all(|p| p.translation.code == NLT_CODE));
    assert_eq!(
        passages[0].text,
        "O LORD, I have so many enemies; so many are against me."
    );
    server.verify().await;
}

#[tokio::test]
async fn nlt_book_and_chapter_structure_is_the_kjv_one_without_a_request() {
    let mut state = seb_and_kjv().await;
    let server = mock_nlt(&mut state, "never", page(""), 0).await;

    let nlt = state
        .bible_book_chapter_summaries(NLT_CODE)
        .await
        .expect("nlt");
    let kjv = state
        .bible_book_chapter_summaries("eng-kjv")
        .await
        .expect("kjv");
    let books = state.list_bible_books(NLT_CODE).await.expect("books");

    assert!(!nlt.is_empty());
    assert_eq!(nlt, kjv);
    assert_eq!(books, kjv);
    server.verify().await;
}

#[tokio::test]
async fn nlt_down_is_a_typed_error_and_other_translations_keep_working() {
    let mut state = seb_and_kjv().await;
    let _server = mock_nlt(&mut state, "1Jn.1.1-3", ResponseTemplate::new(500), 1).await;

    let err = state
        .generate_bible_slides(
            "slk-seb",
            Some(NLT_CODE),
            "1 Ján",
            Some("1JN"),
            1,
            1,
            3,
            2000,
        )
        .await
        .expect_err("NLT down must fail the load");
    assert!(err.downcast_ref::<RemoteBibleError>().is_some(), "{err:#}");

    let (_, _, slides) = state
        .generate_bible_slides(
            "slk-seb",
            Some("eng-kjv"),
            "1 Ján",
            Some("1JN"),
            1,
            1,
            3,
            2000,
        )
        .await
        .expect("KJV still works");
    assert_eq!(
        bible_metadata(&slides[0])
            .translation_reference_label
            .as_deref(),
        Some("1 John 1:1-3 (KJV)")
    );
}

#[tokio::test]
async fn searching_the_nlt_is_empty_and_makes_no_request() {
    let mut state = seb_and_kjv().await;
    let server = mock_nlt(&mut state, "never", page(""), 0).await;

    let nlt_hits = state
        .search_bible_passages_cross(Some(NLT_CODE), "beginning", 10)
        .await
        .expect("no error");
    let all_hits = state
        .search_bible_passages_cross(None, "beginning", 10)
        .await
        .expect("cross search");

    assert!(nlt_hits.is_empty());
    assert!(all_hits.iter().any(|p| p.translation.code == "eng-kjv"));
    assert!(all_hits.iter().all(|p| p.translation.code != NLT_CODE));
    server.verify().await;
}

#[tokio::test]
async fn find_by_english_book_name_reads_one_nlt_verse() {
    let mut state = seb_and_kjv().await;
    let server = mock_nlt(&mut state, "1Jn.1.2", page(FIXTURE_1JN), 1).await;

    let single = BibleReference::new("1 John", 1, 2, 2).expect("reference");
    let passage = state
        .find_bible_passage(NLT_CODE, &single)
        .await
        .expect("no error")
        .expect("verse 2");
    // A multi-verse span matches no single NLT verse — and costs no request.
    let span = BibleReference::new("1 John", 1, 1, 3).expect("reference");
    let none = state
        .find_bible_passage(NLT_CODE, &span)
        .await
        .expect("no error");

    assert_eq!(passage.reference.book, "1 John");
    assert_eq!(passage.reference.book_code.as_deref(), Some("1JN"));
    assert_eq!(passage.reference.verse_start, 2);
    assert!(passage.text.starts_with("This one who is life itself"));
    assert!(none.is_none());
    server.verify().await;
}

#[tokio::test]
async fn legacy_trigger_reads_the_nlt_as_main_translation() {
    let mut state = seb_and_kjv().await;
    let server = mock_nlt(&mut state, "1Jn.1.1-3", page(FIXTURE_1JN), 1).await;
    let reference = BibleReference::new_with_code("1 John", "1JN", 62, 1, 1, 3).expect("reference");

    let broadcast = state
        .trigger_bible_passage(NLT_CODE, &reference, Default::default())
        .await
        .expect("trigger");

    assert_eq!(broadcast.passage.translation.code, NLT_CODE);
    assert!(
        broadcast.passage.text.starts_with("1. We proclaim to you"),
        "{}",
        broadcast.passage.text
    );
    assert!(broadcast
        .passage
        .text
        .contains("\n\n2. This one who is life itself"));
    server.verify().await;
}

#[tokio::test]
async fn legacy_trigger_goes_on_air_without_an_unreachable_nlt_secondary() {
    let mut state = seb_and_kjv().await;
    let server = mock_nlt(&mut state, "1Jn.1.1-3", ResponseTemplate::new(503), 1).await;
    state
        .set_bible_preferences(
            BiblePreferences::default().with_secondary_translation(Some(NLT_CODE.to_string())),
        )
        .await
        .expect("preferences");
    let reference = BibleReference::new_with_code("1 Ján", "1JN", 62, 1, 1, 3).expect("reference");

    let broadcast = state
        .trigger_bible_passage("slk-seb", &reference, Default::default())
        .await
        .expect("the SEB passage still triggers");

    assert_eq!(broadcast.passage.translation.code, "slk-seb");
    assert!(broadcast
        .passage
        .text
        .starts_with("1. Čo bolo od počiatku."));
    server.verify().await;
}
