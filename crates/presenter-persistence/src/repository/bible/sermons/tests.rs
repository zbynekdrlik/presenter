//! #851 P2a/P2b: sermon persistence (spec §7.1, §7.5, §5.7 / T6).

use super::*;
use presenter_core::bible::sermon::VerseEdits;
use presenter_core::bible::BibleIngestionBatch;
use presenter_core::{BiblePassage, BibleReference, BibleSlideId, BibleTranslation, SlideText};

async fn repo() -> Repository {
    Repository::connect_in_memory()
        .await
        .expect("in-memory repo")
}

fn date(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").expect("date")
}

fn new_sermon(title: &str, service_date: &str) -> NewSermon {
    NewSermon {
        title: title.to_string(),
        service_date: date(service_date),
        default_translation_code: "slk-seb".to_string(),
        secondary_translation_code: Some("eng-kjv".to_string()),
        author_email: Some("preacher@example.org".to_string()),
    }
}

#[tokio::test]
async fn a_new_sermon_is_a_presentation_with_a_date() {
    let repo = repo().await;
    let created = repo
        .create_sermon(&new_sermon("Kázeň 18. 10. 2026", "2026-10-18"))
        .await
        .expect("create");

    let snapshot = repo
        .fetch_sermon(created.id)
        .await
        .expect("fetch")
        .expect("sermon exists");
    assert_eq!(snapshot.sermon.title, "Kázeň 18. 10. 2026");
    assert_eq!(snapshot.sermon.service_date, date("2026-10-18"));
    assert_eq!(snapshot.sermon.revision, 0);
    assert_eq!(
        snapshot.sermon.author_email.as_deref(),
        Some("preacher@example.org")
    );
    assert!(snapshot.items.is_empty());
    assert!(repo.is_sermon(created.id).await.expect("is_sermon"));

    let plain = repo
        .create_bible_presentation("Ad hoc")
        .await
        .expect("plain");
    assert!(!repo.is_sermon(plain.id).await.expect("is_sermon"));
    let summaries = repo
        .list_bible_presentation_summaries()
        .await
        .expect("list");
    let dates: Vec<(String, Option<NaiveDate>)> = summaries
        .into_iter()
        .map(|summary| (summary.name, summary.sermon_date))
        .collect();
    assert_eq!(
        dates,
        [
            ("Ad hoc".to_string(), None),
            ("Kázeň 18. 10. 2026".to_string(), Some(date("2026-10-18"))),
        ]
    );
    let detail = repo
        .fetch_bible_presentation(created.id)
        .await
        .expect("fetch presentation")
        .expect("presentation exists");
    assert_eq!(detail.sermon_date, Some(date("2026-10-18")));
}

#[tokio::test]
async fn deleting_the_presentation_deletes_its_sermon() {
    let repo = repo().await;
    let sermon = repo
        .create_sermon(&new_sermon("Kázeň", "2026-10-18"))
        .await
        .expect("create");

    repo.delete_bible_presentation(sermon.id)
        .await
        .expect("delete");

    assert!(repo.fetch_sermon(sermon.id).await.expect("fetch").is_none());
    assert!(!repo.is_sermon(sermon.id).await.expect("is_sermon"));
}

// ---- #851 P2b: sermon writes ------------------------------------------------

fn item(verse: u16, text: &str, slide_count: u32) -> SermonItemRecord {
    let mut edits = ItemEdits::default();
    edits.0.insert(
        verse,
        VerseEdits {
            deleted: vec![1],
            emphasized: vec![0],
        },
    );
    SermonItemRecord {
        id: Uuid::new_v4(),
        translation_code: "slk-seb".to_string(),
        book_code: "HEB".to_string(),
        book_number: 58,
        book_name: "Hebrejom".to_string(),
        chapter: 11,
        verse_start: verse,
        verse_end: verse,
        source: vec![SourceVerse {
            verse,
            text: text.to_string(),
        }],
        edits,
        slide_count,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn slide(text: &str) -> BiblePresentationSlide {
    BiblePresentationSlide {
        id: BibleSlideId::new(),
        order: 0,
        main: SlideText::new(text).expect("slide text"),
        main_reference: "Hebrejom 11:1a (SEB)".to_string(),
        secondary: SlideText::new("").expect("slide text"),
        secondary_reference: String::new(),
        metadata: None,
    }
}

fn write(
    expected_revision: i64,
    title: &str,
    items: Vec<SermonItemRecord>,
    slides: Vec<BiblePresentationSlide>,
) -> SermonWrite {
    SermonWrite {
        expected_revision,
        title: title.to_string(),
        service_date: date("2026-10-18"),
        secondary_translation_code: None,
        items,
        slides,
    }
}

fn refusal(err: &anyhow::Error) -> Option<&RepositoryError> {
    err.downcast_ref::<RepositoryError>()
}

#[tokio::test]
async fn a_write_replaces_items_and_slides_and_bumps_the_revision() {
    let repo = repo().await;
    let sermon = repo
        .create_sermon(&new_sermon("Kázeň", "2026-10-18"))
        .await
        .expect("create");
    let items = vec![item(1, "Viera je zárukou", 2), item(3, "Vierou chápeme", 1)];
    let slides = vec![
        slide("1. VIERA zárukou"),
        slide("1. pokračovanie"),
        slide("3. Vierou"),
    ];

    let revision = repo
        .save_sermon_change(sermon.id, &write(0, "Viera", items.clone(), slides.clone()))
        .await
        .expect("save");

    assert_eq!(revision, 1);
    let snapshot = repo
        .fetch_sermon(sermon.id)
        .await
        .expect("fetch")
        .expect("exists");
    assert_eq!(snapshot.sermon.revision, 1);
    assert_eq!(snapshot.sermon.title, "Viera");
    assert_eq!(snapshot.sermon.secondary_translation_code, None);
    let stored: Vec<(Uuid, Vec<SourceVerse>, ItemEdits, u32)> = snapshot
        .items
        .into_iter()
        .map(|item| (item.id, item.source, item.edits, item.slide_count))
        .collect();
    let expected: Vec<(Uuid, Vec<SourceVerse>, ItemEdits, u32)> = items
        .into_iter()
        .map(|item| (item.id, item.source, item.edits, item.slide_count))
        .collect();
    assert_eq!(stored, expected);
    let presentation = repo
        .fetch_bible_presentation(sermon.id)
        .await
        .expect("fetch presentation")
        .expect("exists");
    let stored_slides: Vec<(BibleSlideId, u32)> = presentation
        .slides
        .iter()
        .map(|slide| (slide.id, slide.order))
        .collect();
    let expected_slides: Vec<(BibleSlideId, u32)> = slides
        .iter()
        .zip(0..)
        .map(|(slide, order)| (slide.id, order))
        .collect();
    assert_eq!(stored_slides, expected_slides);
    assert_eq!(presentation.name, "Viera");
}

#[tokio::test]
async fn a_stale_revision_is_a_conflict_and_changes_nothing() {
    let repo = repo().await;
    let sermon = repo
        .create_sermon(&new_sermon("Kázeň", "2026-10-18"))
        .await
        .expect("create");
    let first = item(1, "Prvý zápis", 1);
    repo.save_sermon_change(
        sermon.id,
        &write(0, "Kázeň", vec![first.clone()], vec![slide("1. Prvý")]),
    )
    .await
    .expect("first write");

    let err = repo
        .save_sermon_change(
            sermon.id,
            &write(0, "Iný", vec![item(2, "Druhý", 1)], Vec::new()),
        )
        .await
        .expect_err("stale revision");

    assert!(matches!(
        refusal(&err),
        Some(RepositoryError::Conflict(SERMON_CONFLICT))
    ));
    let snapshot = repo
        .fetch_sermon(sermon.id)
        .await
        .expect("fetch")
        .expect("exists");
    assert_eq!(snapshot.sermon.revision, 1);
    assert_eq!(snapshot.sermon.title, "Kázeň");
    assert_eq!(
        snapshot
            .items
            .iter()
            .map(|item| item.id)
            .collect::<Vec<_>>(),
        [first.id]
    );
    let presentation = repo
        .fetch_bible_presentation(sermon.id)
        .await
        .expect("fetch presentation")
        .expect("exists");
    assert_eq!(presentation.slides.len(), 1);
}

#[tokio::test]
async fn writing_a_missing_sermon_is_not_found() {
    let repo = repo().await;
    let err = repo
        .save_sermon_change(
            BiblePresentationId::new(),
            &write(0, "Nič", Vec::new(), Vec::new()),
        )
        .await
        .expect_err("missing sermon");
    assert!(matches!(
        refusal(&err),
        Some(RepositoryError::NotFound(SERMON_NOT_FOUND))
    ));
}

#[tokio::test]
async fn deleting_the_presentation_cascades_to_the_sermon_and_its_items() {
    let repo = repo().await;
    let sermon = repo
        .create_sermon(&new_sermon("Kázeň", "2026-10-18"))
        .await
        .expect("create");
    repo.save_sermon_change(
        sermon.id,
        &write(0, "Kázeň", vec![item(1, "Text", 0)], Vec::new()),
    )
    .await
    .expect("write");

    repo.delete_bible_presentation(sermon.id)
        .await
        .expect("delete");

    assert!(repo.fetch_sermon(sermon.id).await.expect("fetch").is_none());
    let items = bible_sermon_item::Entity::find()
        .all(repo.connection())
        .await
        .expect("items");
    assert!(items.is_empty(), "{items:?}");
}

#[tokio::test]
async fn summaries_and_date_lookups() {
    let repo = repo().await;
    let later = repo
        .create_sermon(&new_sermon("Neskôr", "2026-10-25"))
        .await
        .expect("create");
    let today = repo
        .create_sermon(&new_sermon("Dnes", "2026-10-18"))
        .await
        .expect("create");
    let past = repo
        .create_sermon(&new_sermon("Minule", "2026-10-11"))
        .await
        .expect("create");
    let items = vec![item(1, "a", 2), item(2, "b", 3)];
    repo.save_sermon_change(today.id, &write(0, "Dnes", items, Vec::new()))
        .await
        .expect("write");

    let summaries = repo.list_sermon_summaries().await.expect("summaries");
    let rows: Vec<(&str, usize, usize, i64)> = summaries
        .iter()
        .map(|s| (s.title.as_str(), s.item_count, s.slide_count, s.revision))
        .collect();
    assert_eq!(
        rows,
        [("Minule", 0, 0, 0), ("Dnes", 2, 5, 1), ("Neskôr", 0, 0, 0)]
    );
    assert_eq!(
        repo.sermon_ids_on(date("2026-10-18")).await.expect("on"),
        [today.id]
    );
    assert_eq!(
        repo.sermon_ids_from(date("2026-10-18"))
            .await
            .expect("from"),
        [today.id, later.id]
    );
    assert!(!repo
        .sermon_ids_from(date("2026-10-18"))
        .await
        .expect("from")
        .contains(&past.id));
}

/// T6 / spec §5.7: a re-import (PARSER_VERSION bump) deletes and re-inserts
/// the translation; saved presentations and sermons must come out identical.
#[tokio::test]
async fn a_reimport_leaves_slides_sermons_and_items_byte_identical() {
    let repo = repo().await;
    let seb = BibleTranslation::new("slk-seb", "Slovenský ekumenický preklad", "sk");
    let passage = |text: &str| {
        let reference =
            BibleReference::new_with_code("Hebrejom", "HEB", 58, 11, 1, 1).expect("reference");
        BiblePassage::new(reference, seb.clone(), text.to_string())
    };
    let first =
        BibleIngestionBatch::new(seb.clone(), vec![passage("Viera je zárukou")]).expect("batch");
    repo.replace_bible_translation_passages(&first)
        .await
        .expect("first import");
    let plain = repo
        .create_bible_presentation("Ad hoc")
        .await
        .expect("plain");
    repo.replace_bible_presentation_slides(plain.id, &[slide("1. Viera")])
        .await
        .expect("plain slides");
    let sermon = repo
        .create_sermon(&new_sermon("Kázeň", "2026-10-18"))
        .await
        .expect("create");
    repo.save_sermon_change(
        sermon.id,
        &write(
            0,
            "Kázeň",
            vec![item(1, "Viera je zárukou", 1)],
            vec![slide("1. VIERA")],
        ),
    )
    .await
    .expect("sermon write");
    let db = repo.connection();
    let slides_before = bible_slide::Entity::find().all(db).await.expect("slides");
    let sermons_before = bible_sermon::Entity::find().all(db).await.expect("sermons");
    let items_before = bible_sermon_item::Entity::find()
        .all(db)
        .await
        .expect("items");

    let second =
        BibleIngestionBatch::new(seb.clone(), vec![passage("Viera je istota")]).expect("batch");
    repo.replace_bible_translation_passages(&second)
        .await
        .expect("re-import");

    assert_eq!(
        bible_slide::Entity::find().all(db).await.expect("slides"),
        slides_before
    );
    assert_eq!(
        bible_sermon::Entity::find().all(db).await.expect("sermons"),
        sermons_before
    );
    assert_eq!(
        bible_sermon_item::Entity::find()
            .all(db)
            .await
            .expect("items"),
        items_before
    );
    assert_eq!(slides_before.len(), 2);
    assert_eq!(items_before.len(), 1);
}

#[tokio::test]
async fn a_rename_makes_a_write_loaded_before_it_stale() {
    let repo = repo().await;
    let sermon = repo
        .create_sermon(&new_sermon("Kázeň", "2026-10-18"))
        .await
        .expect("create");

    repo.rename_bible_presentation(sermon.id, "Viera")
        .await
        .expect("rename");

    let err = repo
        .save_sermon_change(sermon.id, &write(0, "Kázeň", Vec::new(), Vec::new()))
        .await
        .expect_err("loaded before the rename");
    assert!(matches!(
        refusal(&err),
        Some(RepositoryError::Conflict(SERMON_CONFLICT))
    ));
    let snapshot = repo
        .fetch_sermon(sermon.id)
        .await
        .expect("fetch")
        .expect("exists");
    assert_eq!(snapshot.sermon.title, "Viera");
    assert_eq!(snapshot.sermon.revision, 1);
}
