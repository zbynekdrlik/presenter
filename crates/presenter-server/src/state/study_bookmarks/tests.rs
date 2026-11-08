//! #915: the bookmark rules over a seeded in-memory state (SEB + KJV Hebrews
//! 11:1-3; the NLT reads the KJV's structure, so no request is made).

use super::*;
use presenter_core::bible::BibleIngestionBatch;
use presenter_core::{BiblePassage, BibleReference, BibleTranslation};

async fn seed(state: &AppState, code: &str, book: &str) {
    let translation = BibleTranslation::new(code, code, "sk");
    let passages = (1..=3)
        .map(|verse| {
            let reference = BibleReference::new_with_code(book, "HEB", 58, 11, verse, verse)
                .expect("reference");
            BiblePassage::new(reference, translation.clone(), format!("verš {verse}"))
        })
        .collect();
    let batch = BibleIngestionBatch::new(translation, passages).expect("batch");
    state
        .repository()
        .replace_bible_translation_passages(&batch)
        .await
        .expect("seed");
}

async fn hebrews() -> AppState {
    let state = AppState::in_memory().await.expect("state");
    seed(&state, "slk-seb", "Hebrejom").await;
    seed(&state, "eng-kjv", "Hebrews").await;
    state
}

fn place(translation: &str, book: &str, chapter: u16, verse: u16) -> PlaceRequest {
    PlaceRequest {
        translation: translation.to_string(),
        book_code: book.to_string(),
        chapter,
        verse,
    }
}

fn refusal(err: &anyhow::Error) -> String {
    err.downcast_ref::<BookmarkRefusal>()
        .map(|refusal| refusal.0.clone())
        .unwrap_or_else(|| format!("not a refusal: {err}"))
}

#[tokio::test]
async fn a_bookmark_keeps_its_trimmed_name_and_the_translations_book_name() {
    let state = hebrews().await;
    let owner = Some("preacher@example.org");
    let made = state
        .create_study_bookmark(owner, "  Kázeň  ", &place("SLK-SEB", "heb", 11, 3), None)
        .await
        .expect("create");
    assert_eq!(made.name, "Kázeň");
    assert_eq!(made.translation, "slk-seb");
    assert_eq!(made.book_code, "HEB");
    assert_eq!(made.book, "Hebrejom");
    assert_eq!((made.chapter, made.verse), (11, 3));
    let list = state.list_study_bookmarks(owner).await.expect("list");
    assert_eq!(list, vec![made]);
    assert!(state
        .list_study_bookmarks(None)
        .await
        .expect("lan")
        .is_empty());
}

#[tokio::test]
async fn a_place_the_translation_lacks_is_refused_in_slovak() {
    let state = hebrews().await;
    let cases = [
        (place("slk-seb", "XYZ", 11, 1), "Neznáma kniha XYZ"),
        (
            place("slk-roh", "HEB", 11, 1),
            "Preklad slk-roh nie je nainštalovaný",
        ),
        (
            place("slk-seb", "HEB", 11, 4),
            "HEB 11:4 v preklade SEB nie je",
        ),
        (
            place("slk-seb", "HEB", 11, 0),
            "HEB 11:0 v preklade SEB nie je",
        ),
        (
            place("slk-seb", "HEB", 12, 1),
            "HEB 12:1 v preklade SEB nie je",
        ),
        (
            place("slk-seb", "JHN", 1, 1),
            "JHN 1:1 v preklade SEB nie je",
        ),
    ];
    for (asked, expected) in cases {
        let err = state
            .create_study_bookmark(None, "Kázeň", &asked, None)
            .await
            .expect_err("refused");
        assert_eq!(refusal(&err), expected, "{asked:?}");
    }
    assert!(state
        .list_study_bookmarks(None)
        .await
        .expect("list")
        .is_empty());
}

#[tokio::test]
async fn the_nlt_is_checked_against_its_structure_without_a_request() {
    let state = hebrews().await;
    let made = state
        .create_study_bookmark(None, "NLT", &place("eng-nlt", "HEB", 11, 2), None)
        .await
        .expect("create");
    assert_eq!((made.translation.as_str(), made.verse), ("eng-nlt", 2));
    assert_eq!(made.book, "Hebrews");
}

#[tokio::test]
async fn an_empty_or_too_long_name_is_refused() {
    let state = hebrews().await;
    let heb = place("slk-seb", "HEB", 11, 1);
    let empty = state
        .create_study_bookmark(None, "   ", &heb, None)
        .await
        .expect_err("empty");
    assert_eq!(refusal(&empty), "Záložka potrebuje názov");
    let long = "a".repeat(MAX_NAME_CHARS + 1);
    let too_long = state
        .create_study_bookmark(None, &long, &heb, None)
        .await
        .expect_err("long");
    assert_eq!(
        refusal(&too_long),
        "Názov záložky môže mať najviac 80 znakov"
    );
    let fits = "ž".repeat(MAX_NAME_CHARS);
    assert!(state
        .create_study_bookmark(None, &fits, &heb, None)
        .await
        .is_ok());
}

#[tokio::test]
async fn a_full_list_refuses_one_more() {
    let state = hebrews().await;
    let heb = place("slk-seb", "HEB", 11, 1);
    for index in 0..MAX_BOOKMARKS {
        state
            .create_study_bookmark(None, &format!("Z {index}"), &heb, None)
            .await
            .expect("fits");
    }
    let err = state
        .create_study_bookmark(None, "Navyše", &heb, None)
        .await
        .expect_err("full");
    assert_eq!(
        refusal(&err),
        "Záložiek môže byť najviac 100 — niektorú zmaž"
    );
    // Another list is not full.
    assert!(state
        .create_study_bookmark(Some("preacher@example.org"), "Moja", &heb, None)
        .await
        .is_ok());
}

#[tokio::test]
async fn an_update_renames_moves_and_refuses_an_empty_change_or_a_bad_place() {
    let state = hebrews().await;
    let made = state
        .create_study_bookmark(None, "Kázeň", &place("slk-seb", "HEB", 11, 1), None)
        .await
        .expect("create");
    let renamed = state
        .update_study_bookmark(None, made.id, Some(" Nedeľa "), None)
        .await
        .expect("rename");
    assert_eq!((renamed.name.as_str(), renamed.verse), ("Nedeľa", 1));
    let moved = state
        .update_study_bookmark(None, made.id, None, Some(&place("eng-kjv", "HEB", 11, 3)))
        .await
        .expect("move");
    assert_eq!(
        (
            moved.name.as_str(),
            moved.translation.as_str(),
            moved.book.as_str(),
            moved.verse
        ),
        ("Nedeľa", "eng-kjv", "Hebrews", 3)
    );
    let nothing = state
        .update_study_bookmark(None, made.id, None, None)
        .await
        .expect_err("nothing");
    assert_eq!(refusal(&nothing), "Nie je čo zmeniť");
    let bad = state
        .update_study_bookmark(None, made.id, None, Some(&place("eng-kjv", "HEB", 11, 9)))
        .await
        .expect_err("bad place");
    assert_eq!(refusal(&bad), "HEB 11:9 v preklade KJV nie je");
    let still = state.list_study_bookmarks(None).await.expect("list");
    assert_eq!(still, vec![moved]);
}

#[tokio::test]
async fn a_delete_answers_the_index_and_spat_puts_it_back_there() {
    let state = hebrews().await;
    let heb = place("slk-seb", "HEB", 11, 1);
    let mut ids = Vec::new();
    for name in ["A", "B", "C"] {
        let made = state
            .create_study_bookmark(None, name, &heb, None)
            .await
            .expect("create");
        ids.push(made.id);
    }
    let (deleted, index) = state
        .delete_study_bookmark(None, ids[1])
        .await
        .expect("delete");
    assert_eq!((deleted.name.as_str(), index), ("B", 1));
    state
        .create_study_bookmark(None, &deleted.name, &heb, Some(index))
        .await
        .expect("back");
    let names: Vec<String> = state
        .list_study_bookmarks(None)
        .await
        .expect("list")
        .into_iter()
        .map(|bookmark| bookmark.name)
        .collect();
    assert_eq!(names, ["A", "B", "C"]);
}
