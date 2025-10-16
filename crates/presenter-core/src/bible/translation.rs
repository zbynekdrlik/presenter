use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use thiserror::Error;

use super::reference::BibleReference;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BibleTranslation {
    pub code: String,
    pub name: String,
    pub language: String,
    #[serde(default = "BibleTranslation::default_show_in_dashboard")]
    pub show_in_dashboard: bool,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

impl BibleTranslation {
    const fn default_show_in_dashboard() -> bool {
        true
    }

    pub fn new(
        code: impl Into<String>,
        name: impl Into<String>,
        language: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            name: name.into(),
            language: language.into(),
            show_in_dashboard: Self::default_show_in_dashboard(),
            source: None,
        }
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    pub fn with_show_in_dashboard(mut self, show: bool) -> Self {
        self.show_in_dashboard = show;
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
pub struct BiblePreferences {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub main_translation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secondary_translation: Option<String>,
    pub character_limit: u32,
}

impl Default for BiblePreferences {
    fn default() -> Self {
        Self {
            main_translation: None,
            secondary_translation: None,
            character_limit: 320,
        }
    }
}

impl BiblePreferences {
    pub fn with_main_translation(mut self, code: Option<String>) -> Self {
        self.main_translation = code;
        self
    }

    pub fn with_secondary_translation(mut self, code: Option<String>) -> Self {
        self.secondary_translation = code;
        self
    }

    pub fn with_character_limit(mut self, limit: u32) -> Self {
        self.character_limit = limit;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BiblePreferencesDraft {
    pub main_translation: Option<String>,
    pub secondary_translation: Option<String>,
    pub character_limit: Option<u32>,
}

impl BiblePreferencesDraft {
    pub fn apply(self, mut base: BiblePreferences) -> BiblePreferences {
        if let Some(main) = self.main_translation {
            base.main_translation = Some(main);
        }
        if let Some(secondary) = self.secondary_translation {
            base.secondary_translation = Some(secondary);
        }
        if let Some(limit) = self.character_limit {
            base.character_limit = limit;
        }
        base
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BibleBookChapterSummary {
    pub book: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub book_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub book_number: Option<u16>,
    pub chapter: u16,
    pub verse_count: u16,
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
        let mut seen = HashSet::new();
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
