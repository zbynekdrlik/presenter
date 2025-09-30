use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BibleReferenceError {
    #[error("chapter must be positive")]
    InvalidChapter,
    #[error("verse numbers must be positive and ordered")]
    InvalidVerseRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BibleReference {
    pub book: String,
    pub chapter: u16,
    pub verse_start: u16,
    pub verse_end: u16,
}

impl BibleReference {
    pub fn new<T: Into<String>>(
        book: T,
        chapter: u16,
        verse_start: u16,
        verse_end: u16,
    ) -> Result<Self, BibleReferenceError> {
        if chapter == 0 {
            return Err(BibleReferenceError::InvalidChapter);
        }
        if verse_start == 0 || verse_end == 0 || verse_start > verse_end {
            return Err(BibleReferenceError::InvalidVerseRange);
        }
        Ok(Self {
            book: book.into(),
            chapter,
            verse_start,
            verse_end,
        })
    }

    pub fn to_human_readable(&self) -> String {
        if self.verse_start == self.verse_end {
            format!("{} {}:{}", self.book, self.chapter, self.verse_start)
        } else {
            format!(
                "{} {}:{}-{}",
                self.book, self.chapter, self.verse_start, self.verse_end
            )
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BibleTranslation {
    pub code: String,
    pub name: String,
    pub language: String,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

impl BibleTranslation {
    pub fn new(
        code: impl Into<String>,
        name: impl Into<String>,
        language: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            name: name.into(),
            language: language.into(),
            source: None,
        }
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BiblePassage {
    pub reference: BibleReference,
    pub translation: BibleTranslation,
    pub text: String,
}

impl BiblePassage {
    pub fn new(reference: BibleReference, translation: BibleTranslation, text: String) -> Self {
        Self {
            reference,
            translation,
            text,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BibleBroadcast {
    pub passage: BiblePassage,
    pub triggered_at: DateTime<Utc>,
}

impl BibleBroadcast {
    pub fn new(passage: BiblePassage, triggered_at: DateTime<Utc>) -> Self {
        Self {
            passage,
            triggered_at,
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BibleIngestionError {
    #[error("passage translation '{found}' does not match batch translation '{expected}'")]
    TranslationMismatch { expected: String, found: String },
    #[error("duplicate passage for {book} {chapter}:{start}-{end}")]
    DuplicatePassage {
        book: String,
        chapter: u16,
        start: u16,
        end: u16,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BibleIngestionBatch {
    translation: BibleTranslation,
    passages: Vec<BiblePassage>,
}

impl BibleIngestionBatch {
    pub fn new(
        translation: BibleTranslation,
        passages: Vec<BiblePassage>,
    ) -> Result<Self, BibleIngestionError> {
        let expected = translation.code.clone();
        let mut seen = std::collections::HashSet::new();
        for passage in &passages {
            let found = &passage.translation.code;
            if found != &expected {
                return Err(BibleIngestionError::TranslationMismatch {
                    expected: expected.clone(),
                    found: found.clone(),
                });
            }
            let key = (
                passage.reference.book.clone(),
                passage.reference.chapter,
                passage.reference.verse_start,
                passage.reference.verse_end,
            );
            if !seen.insert(key.clone()) {
                return Err(BibleIngestionError::DuplicatePassage {
                    book: key.0,
                    chapter: key.1,
                    start: key.2,
                    end: key.3,
                });
            }
        }
        Ok(Self {
            translation,
            passages,
        })
    }

    pub fn translation(&self) -> &BibleTranslation {
        &self.translation
    }

    pub fn passages(&self) -> &[BiblePassage] {
        &self.passages
    }

    pub fn into_parts(self) -> (BibleTranslation, Vec<BiblePassage>) {
        (self.translation, self.passages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_reference_ranges() {
        assert_eq!(
            BibleReference::new("John", 1, 5, 3).unwrap_err(),
            BibleReferenceError::InvalidVerseRange
        );
    }

    #[test]
    fn formats_reference() {
        let reference = BibleReference::new("John", 3, 16, 16).unwrap();
        assert_eq!(reference.to_human_readable(), "John 3:16");
        let range = BibleReference::new("Psalm", 23, 1, 3).unwrap();
        assert_eq!(range.to_human_readable(), "Psalm 23:1-3");
    }

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
        let err =
            BibleIngestionBatch::new(translation, vec![passage.clone(), passage]).unwrap_err();
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
}
