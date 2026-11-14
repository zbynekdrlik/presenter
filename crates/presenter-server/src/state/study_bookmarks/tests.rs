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
    // The list adds the verse's text (#951); a write answers none.
    assert_eq!(made.text, None);
    let listed = StudyBookmark {
        text: Some("verš 3".to_string()),
        ..made
    };
    assert_eq!(list, vec![listed]);
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
            "Hebrejom 11:4 v preklade SEB nie je",
        ),
        (
            place("slk-seb", "HEB", 11, 0),
            "Hebrejom 11:0 v preklade SEB nie je",
        ),
        (
            place("slk-seb", "HEB", 12, 1),
            "Hebrejom 12:1 v preklade SEB nie je",
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
async fn the_list_is_newest_first_with_each_verses_text() {
    let state = hebrews().await;
    for verse in [1, 2, 3] {
        state
            .create_study_bookmark(None, "x", &place("slk-seb", "HEB", 11, verse), None)
            .await
            .expect("create");
    }
    // The NLT is remote: no text of its own, and no main translation set → none.
    state
        .create_study_bookmark(None, "nlt", &place("eng-nlt", "HEB", 11, 2), Some(3))
        .await
        .expect("nlt");
    let list = state.list_study_bookmarks(None).await.expect("list");
    let shown: Vec<(u16, Option<&str>)> = list
        .iter()
        .map(|row| (row.verse, row.text.as_deref()))
        .collect();
    assert_eq!(
        shown,
        vec![
            (3, Some("verš 3")),
            (2, Some("verš 2")),
            (1, Some("verš 1")),
            (2, None),
        ]
    );
    // With the KJV as the main translation, the NLT row reads its text.
    let mut prefs = state.get_bible_preferences().await.expect("prefs");
    prefs.main_translation = Some("eng-kjv".to_string());
    state.set_bible_preferences(prefs).await.expect("set prefs");
    let list = state.list_study_bookmarks(None).await.expect("list");
    assert_eq!(list[3].text.as_deref(), Some("verš 2"));
    assert_eq!(list[0].text.as_deref(), Some("verš 3"));
}

#[test]
fn a_long_verse_is_cut_at_the_excerpt_length_on_a_character() {
    assert_eq!(excerpt("  Viera je istota.  "), "Viera je istota.");
    let long = "ž".repeat(EXCERPT_CHARS + 5);
    let cut = excerpt(&long);
    assert_eq!(cut.chars().count(), EXCERPT_CHARS + 1);
    assert!(cut.ends_with('…'));
    let exact = "a".repeat(EXCERPT_CHARS);
    assert_eq!(excerpt(&exact), exact);
    // A cut right after a space leaves no space before the "…".
    let spaced = format!("{} b", "a".repeat(EXCERPT_CHARS - 1));
    assert_eq!(
        excerpt(&spaced),
        format!("{}…", "a".repeat(EXCERPT_CHARS - 1))
    );
}

#[tokio::test]
async fn a_reorder_and_a_bulk_delete_with_spat_keep_every_place() {
    let state = hebrews().await;
    let heb = place("slk-seb", "HEB", 11, 1);
    let mut ids = Vec::new();
    for name in ["C", "B", "A"] {
        let made = state
            .create_study_bookmark(None, name, &heb, None)
            .await
            .expect("create");
        ids.push(made.id);
    }
    let names = |list: Vec<StudyBookmark>| -> Vec<String> {
        list.into_iter().map(|bookmark| bookmark.name).collect()
    };
    assert_eq!(
        names(state.list_study_bookmarks(None).await.expect("list")),
        ["A", "B", "C"]
    );
    state
        .reorder_study_bookmarks(None, &[ids[0], ids[2], ids[1]])
        .await
        .expect("reorder");
    assert_eq!(
        names(state.list_study_bookmarks(None).await.expect("list")),
        ["C", "A", "B"]
    );
    let deleted = state
        .delete_study_bookmarks(None, &[ids[1], ids[0]])
        .await
        .expect("delete");
    let answered: Vec<(&str, usize)> = deleted
        .iter()
        .map(|(bookmark, index)| (bookmark.name.as_str(), *index))
        .collect();
    assert_eq!(answered, [("C", 0), ("B", 2)]);
    for (bookmark, index) in &deleted {
        state
            .create_study_bookmark(None, &bookmark.name, &heb, Some(*index))
            .await
            .expect("back");
    }
    assert_eq!(
        names(state.list_study_bookmarks(None).await.expect("list")),
        ["C", "A", "B"]
    );
    let nothing = state
        .delete_study_bookmarks(None, &[])
        .await
        .expect_err("nothing");
    assert_eq!(refusal(&nothing), "Nie je čo zmazať");
}
