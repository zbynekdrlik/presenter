//! Tests for the bible repository. The production code lives in the sibling
//! `query`, `import`, and `presentations` submodules of `bible`. These imports
//! reproduce the names the test modules below pull in via `use super::*;`,
//! exactly as they were resolved when all of this lived in `bible.rs`.
pub(super) use presenter_core::{BiblePresentationSlide, BibleSlideId};

#[cfg(test)]
mod presentation_tests {
    use super::*;
    use crate::repository::Repository;
    use presenter_core::SlideText;

    async fn fresh_repo() -> Repository {
        Repository::connect_in_memory()
            .await
            .expect("in-memory repo")
    }

    fn sample_slide(main: &str, reference: &str) -> BiblePresentationSlide {
        BiblePresentationSlide {
            id: BibleSlideId::new(),
            order: 0,
            main: SlideText::new(main).unwrap(),
            main_reference: reference.to_string(),
            secondary: SlideText::new("").unwrap(),
            secondary_reference: String::new(),
            metadata: None,
        }
    }

    #[tokio::test]
    async fn create_and_fetch_bible_presentation() {
        let repo = fresh_repo().await;
        let created = repo.create_bible_presentation("My Sermon").await.unwrap();
        assert_eq!(created.name, "My Sermon");
        assert!(created.slides.is_empty());

        let fetched = repo
            .fetch_bible_presentation(created.id)
            .await
            .unwrap()
            .expect("should exist");
        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.name, "My Sermon");
    }

    #[tokio::test]
    async fn list_bible_presentation_summaries_returns_all_with_correct_counts() {
        let repo = fresh_repo().await;
        repo.create_bible_presentation("Bravo").await.unwrap();
        let alpha = repo.create_bible_presentation("Alpha").await.unwrap();

        let slide_a = sample_slide("First", "Gen 1:1");
        let slide_b = BiblePresentationSlide {
            id: BibleSlideId::new(),
            order: 1,
            main: SlideText::new("Second").unwrap(),
            main_reference: "Gen 1:2".to_string(),
            secondary: SlideText::new("").unwrap(),
            secondary_reference: String::new(),
            metadata: None,
        };
        repo.replace_bible_presentation_slides(alpha.id, &[slide_a, slide_b])
            .await
            .unwrap();

        let list = repo.list_bible_presentation_summaries().await.unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "Alpha");
        assert_eq!(list[0].slide_count, 2);
        assert_eq!(list[1].name, "Bravo");
        assert_eq!(list[1].slide_count, 0);
    }

    #[tokio::test]
    async fn rename_bible_presentation_updates_name() {
        let repo = fresh_repo().await;
        let p = repo.create_bible_presentation("Old").await.unwrap();
        repo.rename_bible_presentation(p.id, "New").await.unwrap();
        let fetched = repo.fetch_bible_presentation(p.id).await.unwrap().unwrap();
        assert_eq!(fetched.name, "New");
    }

    #[tokio::test]
    async fn delete_bible_presentation_removes_it_and_cascades_slides() {
        use crate::entities::bible_slide;
        use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};

        let repo = fresh_repo().await;
        let p = repo.create_bible_presentation("Doomed").await.unwrap();
        let slide = sample_slide("text", "Ref");
        repo.replace_bible_presentation_slides(p.id, &[slide])
            .await
            .unwrap();

        let count_before = bible_slide::Entity::find()
            .filter(bible_slide::Column::PresentationId.eq(p.id.to_string()))
            .count(&repo.db)
            .await
            .unwrap();
        assert_eq!(count_before, 1);

        repo.delete_bible_presentation(p.id).await.unwrap();

        assert!(repo.fetch_bible_presentation(p.id).await.unwrap().is_none());

        let count_after = bible_slide::Entity::find()
            .filter(bible_slide::Column::PresentationId.eq(p.id.to_string()))
            .count(&repo.db)
            .await
            .unwrap();
        assert_eq!(count_after, 0, "FK cascade should have removed the slide");
    }

    #[tokio::test]
    async fn replace_bible_slides_overwrites_existing() {
        let repo = fresh_repo().await;
        let p = repo.create_bible_presentation("Test").await.unwrap();
        let slide = sample_slide("For God so loved the world", "John 3:16");
        repo.replace_bible_presentation_slides(p.id, &[slide])
            .await
            .unwrap();
        let fetched = repo.fetch_bible_presentation(p.id).await.unwrap().unwrap();
        assert_eq!(fetched.slides.len(), 1);
        assert_eq!(fetched.slides[0].main_reference, "John 3:16");

        repo.replace_bible_presentation_slides(p.id, &[])
            .await
            .unwrap();
        let fetched = repo.fetch_bible_presentation(p.id).await.unwrap().unwrap();
        assert!(fetched.slides.is_empty());
    }

    #[tokio::test]
    async fn bible_slide_metadata_round_trips_through_replace_and_fetch() {
        use presenter_core::slide::{BibleSlideMetadata, BibleSlideVerseRef};

        let repo = fresh_repo().await;
        let p = repo
            .create_bible_presentation("Metadata Test")
            .await
            .unwrap();

        let metadata = BibleSlideMetadata {
            translation_code: "en-kjv".to_string(),
            secondary_translation_code: Some("sk-sevp".to_string()),
            book: "John".to_string(),
            book_code: Some("JHN".to_string()),
            book_number: Some(43),
            chapter: 3,
            verses: vec![
                BibleSlideVerseRef::new(16, 16),
                BibleSlideVerseRef::new(17, 17),
            ],
            main_reference_label: Some("John 3:16-17".to_string()),
            translation_reference_label: Some("Ján 3:16-17".to_string()),
        };

        let slide = BiblePresentationSlide {
            id: BibleSlideId::new(),
            order: 0,
            main: SlideText::new("For God so loved the world").unwrap(),
            main_reference: "John 3:16-17".to_string(),
            secondary: SlideText::new("Lebo tak Boh miloval svet").unwrap(),
            secondary_reference: "Ján 3:16-17".to_string(),
            metadata: Some(metadata.clone()),
        };

        repo.replace_bible_presentation_slides(p.id, std::slice::from_ref(&slide))
            .await
            .unwrap();

        let fetched = repo.fetch_bible_presentation(p.id).await.unwrap().unwrap();
        assert_eq!(fetched.slides.len(), 1);
        let fetched_slide = &fetched.slides[0];
        assert_eq!(fetched_slide.metadata, Some(metadata));
        assert_eq!(fetched_slide.main.value(), "For God so loved the world");
        assert_eq!(fetched_slide.main_reference, "John 3:16-17");
        assert_eq!(fetched_slide.secondary_reference, "Ján 3:16-17");
    }

    #[tokio::test]
    async fn append_bible_slides_preserves_order() {
        let repo = fresh_repo().await;
        let p = repo.create_bible_presentation("Test").await.unwrap();
        let slide_a = sample_slide("First", "Gen 1:1");
        let slide_b = sample_slide("Second", "Gen 1:2");
        repo.append_bible_presentation_slides(p.id, &[slide_a])
            .await
            .unwrap();
        let result = repo
            .append_bible_presentation_slides(p.id, &[slide_b])
            .await
            .unwrap();
        assert_eq!(result.slides.len(), 2);
        assert_eq!(result.slides[0].order, 0);
        assert_eq!(result.slides[1].order, 1);
        assert_eq!(result.slides[0].main_reference, "Gen 1:1");
        assert_eq!(result.slides[1].main_reference, "Gen 1:2");
    }
}

#[cfg(test)]
mod digest_tests {
    use crate::repository::Repository;
    use presenter_core::bible::BibleIngestionBatch;
    use presenter_core::BibleTranslation;

    async fn fresh_repo() -> Repository {
        Repository::connect_in_memory()
            .await
            .expect("in-memory repo")
    }

    async fn seed_translation(repo: &Repository, code: &str) {
        let translation = BibleTranslation::new(code, code, "xx").with_source("test");
        let batch =
            BibleIngestionBatch::new(translation, Vec::new()).expect("empty batch is valid");
        repo.replace_bible_translation_passages(&batch)
            .await
            .expect("seed translation");
    }

    #[tokio::test]
    async fn set_and_get_source_digest_round_trip() {
        let repo = fresh_repo().await;
        seed_translation(&repo, "eng-test").await;

        assert_eq!(
            repo.get_bible_source_digest("eng-test").await.unwrap(),
            None,
            "fresh translation has no digest",
        );

        repo.set_bible_source_digest("eng-test", "abc123")
            .await
            .expect("set digest");

        assert_eq!(
            repo.get_bible_source_digest("eng-test").await.unwrap(),
            Some("abc123".to_string()),
        );
    }

    #[tokio::test]
    async fn get_digest_for_unknown_code_is_none() {
        let repo = fresh_repo().await;
        assert_eq!(
            repo.get_bible_source_digest("nope-none").await.unwrap(),
            None,
        );
    }
}

#[cfg(test)]
mod fast_import_tests {
    use crate::repository::Repository;
    use presenter_core::bible::BibleIngestionBatch;
    use presenter_core::{BiblePassage, BibleReference, BibleTranslation};

    fn make_passage(
        translation: &BibleTranslation,
        book: &str,
        chapter: u16,
        verse: u16,
        content: &str,
    ) -> BiblePassage {
        let reference =
            BibleReference::new(book.to_string(), chapter, verse, verse).expect("valid reference");
        BiblePassage::new(reference, translation.clone(), content.to_string())
    }

    async fn fresh_repo() -> Repository {
        Repository::connect_in_memory()
            .await
            .expect("in-memory repo")
    }

    #[tokio::test]
    async fn fast_import_preserves_fts_search() {
        let repo = fresh_repo().await;
        let translation = BibleTranslation::new("eng-fast", "Fast Test", "en").with_source("test");
        let passages = vec![
            make_passage(&translation, "John", 3, 16, "For God so loved the world"),
            make_passage(
                &translation,
                "Genesis",
                1,
                1,
                "In the beginning God created",
            ),
        ];
        let batch = BibleIngestionBatch::new(translation, passages).expect("valid batch");

        repo.replace_bible_translation_passages(&batch)
            .await
            .expect("import");

        let hits = repo
            .search_bible_passages("eng-fast", "beginning", 10)
            .await
            .expect("search");
        assert!(
            hits.iter().any(|p| p.text.contains("In the beginning")),
            "FTS search should find 'beginning' after fast import, got {:?}",
            hits,
        );
    }

    #[tokio::test]
    async fn fast_import_idempotent_row_counts() {
        let repo = fresh_repo().await;
        let translation = BibleTranslation::new("eng-idem", "Idem Test", "en").with_source("test");
        let passages = vec![
            make_passage(&translation, "John", 1, 1, "In the beginning was the Word"),
            make_passage(
                &translation,
                "John",
                1,
                2,
                "He was with God in the beginning",
            ),
        ];
        let batch = BibleIngestionBatch::new(translation, passages).expect("valid batch");

        repo.replace_bible_translation_passages(&batch)
            .await
            .unwrap();
        let first_hits = repo
            .search_bible_passages("eng-idem", "beginning", 10)
            .await
            .unwrap();

        // Second import with identical data
        repo.replace_bible_translation_passages(&batch)
            .await
            .unwrap();
        let second_hits = repo
            .search_bible_passages("eng-idem", "beginning", 10)
            .await
            .unwrap();

        assert_eq!(
            first_hits.len(),
            second_hits.len(),
            "re-import should produce identical hit count (not accumulating FTS rows)",
        );
        assert_eq!(first_hits.len(), 2);
    }

    #[tokio::test]
    async fn fast_import_preserves_existing_digest() {
        let repo = fresh_repo().await;
        let translation = BibleTranslation::new("eng-dig", "Dig", "en").with_source("test");
        let batch = BibleIngestionBatch::new(translation.clone(), Vec::new()).expect("valid batch");

        // Initial import — no digest yet
        repo.replace_bible_translation_passages(&batch)
            .await
            .unwrap();
        assert_eq!(
            repo.get_bible_source_digest("eng-dig").await.unwrap(),
            None,
            "fresh translation has no digest",
        );

        // Stamp a digest
        repo.set_bible_source_digest("eng-dig", "keepme")
            .await
            .unwrap();

        // Re-import — digest must survive
        repo.replace_bible_translation_passages(&batch)
            .await
            .unwrap();
        assert_eq!(
            repo.get_bible_source_digest("eng-dig").await.unwrap(),
            Some("keepme".to_string()),
            "re-import must NOT nuke an existing digest",
        );
    }
}
