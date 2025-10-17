use chrono::{TimeZone, Utc};

use super::super::reference::BibleReference;
use super::super::translation::{
    BibleBroadcast, BibleIngestionBatch, BibleIngestionError, BiblePassage, BibleTranslation,
};

#[test]
fn ingestion_batch_rejects_translation_mismatch() {
    let translation = BibleTranslation::new("sk-seb", "Slovak", "sk");
    let reference = BibleReference::new("John", 3, 16, 16).unwrap();
    let passage = BiblePassage::new(
        reference,
        BibleTranslation::new("en-kjv", "KJV", "en"),
        "For God so loved".to_string(),
    );
    let err = BibleIngestionBatch::new(translation, vec![passage]).unwrap_err();
    assert!(matches!(
        err,
        BibleIngestionError::TranslationMismatch { expected, found }
        if expected == "sk-seb" && found == "en-kjv"
    ));
}

#[test]
fn ingestion_batch_rejects_duplicate_references() {
    let translation = BibleTranslation::new("en-kjv", "KJV", "en");
    let reference = BibleReference::new("John", 3, 16, 16).unwrap();
    let passage = BiblePassage::new(
        reference.clone(),
        translation.clone(),
        "For God so loved".to_string(),
    );
    let err = BibleIngestionBatch::new(translation, vec![passage.clone(), passage]).unwrap_err();
    assert!(matches!(
        err,
        BibleIngestionError::DuplicatePassage { book, chapter, start, end }
        if book == "John" && chapter == 3 && start == 16 && end == 16
    ));
}

#[test]
fn ingestion_batch_accepts_unique_passages() {
    let translation = BibleTranslation::new("en-kjv", "KJV", "en");
    let reference = BibleReference::new("John", 3, 16, 17).unwrap();
    let next = BibleReference::new("John", 3, 18, 18).unwrap();
    let passages = vec![
        BiblePassage::new(
            reference,
            translation.clone(),
            "For God so loved".to_string(),
        ),
        BiblePassage::new(next, translation.clone(), "He that believeth".to_string()),
    ];
    let batch = BibleIngestionBatch::new(translation.clone(), passages.clone()).unwrap();
    assert_eq!(batch.translation(), &translation);
    assert_eq!(batch.passages(), passages.as_slice());
}

#[test]
fn bible_broadcast_constructor_wraps_passage() {
    let reference = BibleReference::new("John", 3, 16, 16).unwrap();
    let translation = BibleTranslation::new("en-kjv", "KJV", "en");
    let passage = BiblePassage::new(reference, translation.clone(), "For God so loved".into());
    let triggered_at = Utc.timestamp_opt(5, 0).unwrap();
    let broadcast = BibleBroadcast::new(passage.clone(), triggered_at);
    assert_eq!(broadcast.passage, passage);
    assert_eq!(broadcast.triggered_at, triggered_at);
}
