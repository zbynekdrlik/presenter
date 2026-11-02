//! #851 D4: the study reads through a real in-memory database (spec §6.7, §11).

use super::*;
use crate::bible_remote::{NltClient, NLT_CODE};
use presenter_core::bible::{BibleIngestionBatch, MarkupSegment, StudySource};
use presenter_core::{BiblePassage, BibleReference, BibleTranslation};
use std::time::Duration;
use wiremock::{Mock, MockServer, ResponseTemplate};

const KJV: &str = "eng-kjv";
const SEB: &str = "slk-seb";

struct Book {
    name: &'static str,
    code: &'static str,
    number: u16,
    chapter: u16,
    verses: u16,
}

const fn book(
    name: &'static str,
    code: &'static str,
    number: u16,
    chapter: u16,
    verses: u16,
) -> Book {
    Book {
        name,
        code,
        number,
        chapter,
        verses,
    }
}

/// Every verse reads `"<translation> <code> <chapter>:<verse>"`.
async fn seed(state: &AppState, code: &str, books: &[Book]) {
    let translation = BibleTranslation::new(code, code, "sk");
    let mut passages = Vec::new();
    for book in books {
        for verse in 1..=book.verses {
            let reference = BibleReference::new_with_code(
                book.name,
                book.code,
                book.number,
                book.chapter,
                verse,
                verse,
            )
            .expect("reference");
            let text = format!("{code} {} {}:{verse}", book.code, book.chapter);
            passages.push(BiblePassage::new(reference, translation.clone(), text));
        }
    }
    let mut markup: Vec<_> = passages
        .iter()
        .map(|passage| {
            let tagged = passage.text.starts_with(KJV);
            tagged.then(|| {
                VerseMarkup(vec![MarkupSegment {
                    text: passage.text.clone(),
                    strongs: vec!["G26".to_string()],
                    ..MarkupSegment::default()
                }])
            })
        })
        .collect();
    markup.truncate(passages.len());
    let batch = BibleIngestionBatch::new(translation, passages)
        .expect("batch")
        .with_markup(markup)
        .expect("markup");
    state
        .repository()
        .replace_bible_translation_passages(&batch)
        .await
        .expect("seed");
}

fn kjv_books() -> [Book; 9] {
    [
        book("Genesis", "GEN", 1, 1, 3),
        book("Genesis", "GEN", 1, 2, 5),
        book("Malachi", "MAL", 39, 3, 2),
        book("Malachi", "MAL", 39, 4, 6),
        book("Matthew", "MAT", 40, 1, 3),
        book("2 John", "2JN", 63, 1, 13),
        book("3 John", "3JN", 64, 1, 14),
        book("Revelation", "REV", 66, 22, 2),
        book("Hebrews", "HEB", 58, 11, 4),
    ]
}

fn seb_books() -> [Book; 5] {
    [
        book("Genezis", "GEN", 1, 1, 3),
        book("Genezis", "GEN", 1, 2, 2),
        book("Malachiáš", "MAL", 39, 3, 2),
        book("Hebrejom", "HEB", 58, 11, 4),
        book("Matúš", "MAT", 40, 1, 3),
    ]
}

async fn state() -> AppState {
    let state = AppState::in_memory().await.expect("state");
    seed(&state, KJV, &kjv_books()).await;
    seed(&state, SEB, &seb_books()).await;
    state
}

fn reference(to: (i32, i32, i32), end: (i32, i32, i32), votes: i32) -> CrossReference {
    CrossReference {
        from_book: 1,
        from_chapter: 1,
        from_verse: 1,
        to_book: to.0,
        to_chapter: to.1,
        to_verse: to.2,
        to_book_end: end.0,
        to_chapter_end: end.1,
        to_verse_end: end.2,
        votes,
    }
}

async fn seed_references(state: &AppState, rows: &[CrossReference]) {
    let source = StudySource {
        code: "openbible-xref".to_string(),
        name: "OpenBible.info".to_string(),
        license: "CC BY".to_string(),
        source_digest: "digest".to_string(),
    };
    state
        .repository()
        .replace_bible_cross_references(&source, rows)
        .await
        .expect("seed references");
}

fn place(book_code: &str, chapter: u16) -> Option<ChapterPlace> {
    Some(ChapterPlace {
        book_code: book_code.to_string(),
        chapter,
    })
}

#[tokio::test]
async fn a_chapter_has_its_verses_markup_and_neighbours_across_books() {
    let state = state().await;
    let chapter = state
        .study_chapter(KJV, "MAT", 1)
        .await
        .unwrap()
        .expect("chapter");
    assert_eq!(chapter.book, "Matthew");
    assert_eq!((chapter.book_number, chapter.verse_count), (40, 3));
    assert_eq!(chapter.prev, place("MAL", 4));
    assert_eq!(chapter.next, place("HEB", 11));
    let verses: Vec<u16> = chapter.verses.iter().map(|verse| verse.verse).collect();
    assert_eq!(verses, vec![1, 2, 3]);
    assert_eq!(chapter.verses[1].text, "eng-kjv MAT 1:2");
    let markup = chapter.verses[1].markup.as_ref().expect("markup");
    assert_eq!(markup.0[0].strongs, vec!["G26".to_string()]);
}

#[tokio::test]
async fn the_reader_stops_at_genesis_1_and_revelation_22() {
    let state = state().await;
    let first = state.study_chapter(KJV, "GEN", 1).await.unwrap().unwrap();
    assert_eq!(first.prev, None);
    assert_eq!(first.next, place("GEN", 2));
    let last = state.study_chapter(KJV, "REV", 22).await.unwrap().unwrap();
    assert_eq!(last.next, None);
}

#[tokio::test]
async fn a_translation_without_markup_answers_plain_verses() {
    let state = state().await;
    let chapter = state.study_chapter(SEB, "GEN", 1).await.unwrap().unwrap();
    assert_eq!(chapter.book, "Genezis");
    assert!(chapter.verses.iter().all(|verse| verse.markup.is_none()));
}

#[tokio::test]
async fn a_chapter_the_translation_lacks_is_empty_and_still_moves_on() {
    let state = state().await;
    let chapter = state.study_chapter(SEB, "MAL", 4).await.unwrap().unwrap();
    assert!(chapter.verses.is_empty());
    assert_eq!(chapter.verse_count, 0);
    assert_eq!(chapter.book, "Malachiáš");
    assert_eq!(chapter.prev, place("MAL", 3));
    assert_eq!(chapter.next, place("MAT", 1));
}

#[tokio::test]
async fn unknown_places_are_none() {
    let state = state().await;
    assert!(state
        .study_chapter("xxx", "GEN", 1)
        .await
        .unwrap()
        .is_none());
    assert!(state.study_chapter(KJV, "XXX", 1).await.unwrap().is_none());
    let references = state.study_crossrefs("xxx", "GEN", 1, 1, 10);
    assert!(references.await.unwrap().is_none());
    let references = state.study_crossrefs(KJV, "XXX", 1, 1, 10);
    assert!(references.await.unwrap().is_none());
}

#[tokio::test]
async fn an_nlt_chapter_has_no_markup_and_a_failure_is_the_typed_error() {
    let mut state = state().await;
    let server = MockServer::start().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    state.set_test_nlt_client(NltClient::for_test(&server.uri(), Duration::from_secs(2)));
    let err = state
        .study_chapter(NLT_CODE, "GEN", 1)
        .await
        .expect_err("the API failed");
    assert!(
        err.downcast_ref::<bible_remote::RemoteBibleError>()
            .is_some(),
        "{err:#}"
    );
}

#[tokio::test]
async fn references_keep_the_vote_order_and_the_total_passes_the_limit() {
    let state = state().await;
    seed_references(
        &state,
        &[
            reference((58, 11, 1), (58, 11, 1), 10),
            reference((58, 11, 2), (58, 11, 3), 40),
            reference((40, 1, 1), (40, 1, 1), 5),
        ],
    )
    .await;
    let (total, items) = state
        .study_crossrefs(SEB, "GEN", 1, 1, 2)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(total, 3);
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].label, "Hebrejom 11:2-3");
    assert_eq!(items[0].text, "slk-seb HEB 11:2 slk-seb HEB 11:3");
    assert_eq!(items[0].fallback_translation, None);
    assert_eq!((items[0].votes, items[1].votes), (40, 10));
    assert_eq!(items[1].label, "Hebrejom 11:1");

    let (total, none) = state
        .study_crossrefs(SEB, "GEN", 2, 2, 10)
        .await
        .unwrap()
        .unwrap();
    assert_eq!((total, none.len()), (0, 0));
}

#[tokio::test]
async fn a_range_reads_across_its_chapters() {
    let state = state().await;
    seed_references(&state, &[reference((1, 1, 3), (1, 2, 1), 9)]).await;
    let (_, items) = state
        .study_crossrefs(KJV, "GEN", 1, 1, 10)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(items[0].label, "Genesis 1:3-2:1");
    assert_eq!(items[0].text, "eng-kjv GEN 1:3 eng-kjv GEN 2:1");
    assert_eq!((items[0].chapter_end, items[0].verse_end), (2, 1));
}

#[tokio::test]
async fn a_range_leaving_the_book_stays_in_it_and_the_label_shows_both() {
    let state = state().await;
    seed_references(&state, &[reference((63, 1, 12), (64, 1, 2), 9)]).await;
    let (_, items) = state
        .study_crossrefs(KJV, "GEN", 1, 1, 10)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(items[0].label, "2 John 1:12 - 3 John 1:2");
    assert_eq!(items[0].book_code, "2JN");
    assert_eq!((items[0].chapter_end, items[0].verse_end), (1, 13));
    assert_eq!(items[0].text, "eng-kjv 2JN 1:12 eng-kjv 2JN 1:13");
}

#[tokio::test]
async fn third_john_15_stays_inside_the_kjv_chapter() {
    let state = state().await;
    seed_references(&state, &[reference((64, 1, 15), (64, 1, 15), 9)]).await;
    let (_, items) = state
        .study_crossrefs(KJV, "GEN", 1, 1, 10)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(items[0].label, "3 John 1:14");
    assert_eq!((items[0].verse, items[0].verse_end), (14, 14));
    assert_eq!(items[0].text, "eng-kjv 3JN 1:14");
}

#[tokio::test]
async fn a_place_the_translation_lacks_comes_from_the_kjv_and_says_so() {
    let state = state().await;
    seed_references(&state, &[reference((39, 4, 5), (39, 4, 5), 9)]).await;
    let (_, items) = state
        .study_crossrefs(SEB, "GEN", 1, 1, 10)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(items[0].label, "Malachi 4:5");
    assert_eq!(items[0].text, "eng-kjv MAL 4:5");
    assert_eq!(items[0].fallback_translation.as_deref(), Some(KJV));
}

#[tokio::test]
async fn a_verse_past_the_translations_chapter_is_never_another_verse() {
    let state = state().await;
    seed_references(&state, &[reference((1, 2, 4), (1, 2, 5), 9)]).await;
    let (_, items) = state
        .study_crossrefs(SEB, "GEN", 1, 1, 10)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(items[0].label, "Genesis 2:4-5");
    assert_eq!(items[0].text, "eng-kjv GEN 2:4 eng-kjv GEN 2:5");
    assert_eq!(items[0].fallback_translation.as_deref(), Some(KJV));
}

#[tokio::test]
async fn a_range_past_the_translations_last_chapter_ends_with_its_last_verse() {
    let state = state().await;
    seed_references(&state, &[reference((39, 3, 1), (39, 4, 1), 9)]).await;
    let (_, items) = state
        .study_crossrefs(SEB, "GEN", 1, 1, 10)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(items[0].label, "Malachiáš 3:1-2");
    assert_eq!(items[0].text, "slk-seb MAL 3:1 slk-seb MAL 3:2");
    assert_eq!(items[0].fallback_translation, None);
}

#[tokio::test]
async fn nlt_references_read_the_kjv_without_a_single_request() {
    let mut state = state().await;
    let server = MockServer::start().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    state.set_test_nlt_client(NltClient::for_test(&server.uri(), Duration::from_secs(2)));
    seed_references(&state, &[reference((58, 11, 1), (58, 11, 1), 9)]).await;
    let (_, items) = state
        .study_crossrefs(NLT_CODE, "GEN", 1, 1, 10)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(items[0].label, "Hebrews 11:1");
    assert_eq!(items[0].text, "eng-kjv HEB 11:1");
    assert_eq!(items[0].fallback_translation.as_deref(), Some(KJV));
}
