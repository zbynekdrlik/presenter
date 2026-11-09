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

// #921: the console's preferred translations live with the Bible preferences.
mod preferred {
    use super::super::super::translation::{
        normalize_preferred_translations, BiblePreferences, BiblePreferencesDraft,
        MAX_PREFERRED_TRANSLATIONS,
    };

    fn codes(list: &[&str]) -> Vec<String> {
        list.iter().map(|c| c.to_string()).collect()
    }

    #[test]
    fn a_stored_row_without_the_lists_reads_the_defaults() {
        let stored: BiblePreferences =
            serde_json::from_str(r#"{"mainTranslation":"slk-seb","characterLimit":320}"#)
                .expect("old row parses");
        assert_eq!(stored.preferred_main, None);
        assert_eq!(
            stored.preferred_main_or_default(),
            codes(&["slk-seb", "slk-roh", "slk-sevp", "eng-kjv"])
        );
        assert_eq!(
            stored.preferred_secondary_or_default(),
            codes(&["eng-nlt", "eng-kjv"])
        );
        // Never set = not written, so an old reader sees the row it knew.
        let json = serde_json::to_string(&stored).expect("serializes");
        assert!(!json.contains("preferred"));
    }

    #[test]
    fn a_draft_replaces_only_the_lists_it_carries() {
        let base = BiblePreferences::default().with_main_translation(Some("slk-roh".into()));
        let draft: BiblePreferencesDraft =
            serde_json::from_str(r#"{"preferredMain":["slk-mil"," eng-ampc ","slk-mil",""]}"#)
                .expect("draft parses");
        let updated = draft.apply(base);
        assert_eq!(
            updated.preferred_main,
            Some(codes(&["slk-mil", "eng-ampc"]))
        );
        assert_eq!(updated.preferred_secondary, None);
        assert_eq!(updated.main_translation.as_deref(), Some("slk-roh"));

        // An empty list is a choice too: no chip, only ▾.
        let cleared = BiblePreferencesDraft {
            preferred_secondary: Some(Vec::new()),
            ..Default::default()
        }
        .apply(updated);
        assert_eq!(cleared.preferred_secondary, Some(Vec::new()));
        assert_eq!(
            cleared.preferred_main,
            Some(codes(&["slk-mil", "eng-ampc"]))
        );
    }

    #[test]
    fn a_list_is_capped_and_overlong_codes_dropped() {
        let many: Vec<String> = (0..40).map(|i| format!("x-{i}")).collect();
        assert_eq!(
            normalize_preferred_translations(many).len(),
            MAX_PREFERRED_TRANSLATIONS
        );
        assert!(normalize_preferred_translations(vec!["a".repeat(65)]).is_empty());
    }
}
