use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use thiserror::Error;

use super::markup::VerseMarkup;
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

/// The console's main row shows these translations when nothing else was
/// chosen in Biblia → Nastavenie (#921, owner 2026-11-09).
pub const DEFAULT_PREFERRED_MAIN: [&str; 4] = ["slk-seb", "slk-roh", "slk-sevp", "eng-kjv"];
/// The console's second row ("Druhý preklad") by default (#921).
pub const DEFAULT_PREFERRED_SECONDARY: [&str; 2] = ["eng-nlt", "eng-kjv"];
/// A preferred list keeps at most this many codes: the bar shows 3–4 of
/// them, every translation is in its ▾ list anyway.
pub const MAX_PREFERRED_TRANSLATIONS: usize = 12;
/// A translation code longer than this is no code (`slk-sevp` is 8).
const MAX_TRANSLATION_CODE_LEN: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BiblePreferences {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub main_translation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secondary_translation: Option<String>,
    pub character_limit: u32,
    /// The translations with a chip in the console's main row (#921); `None`
    /// = never set, [`DEFAULT_PREFERRED_MAIN`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_main: Option<Vec<String>>,
    /// The same for the second row; `None` = [`DEFAULT_PREFERRED_SECONDARY`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_secondary: Option<Vec<String>>,
}

impl Default for BiblePreferences {
    fn default() -> Self {
        Self {
            main_translation: None,
            secondary_translation: None,
            character_limit: 320,
            preferred_main: None,
            preferred_secondary: None,
        }
    }
}

/// A preferred list as stored: trimmed codes, no empty or overlong one, each
/// once (the first place wins), at most [`MAX_PREFERRED_TRANSLATIONS`].
pub fn normalize_preferred_translations(codes: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    codes
        .into_iter()
        .map(|code| code.trim().to_string())
        .filter(|code| !code.is_empty() && code.len() <= MAX_TRANSLATION_CODE_LEN)
        .filter(|code| seen.insert(code.clone()))
        .take(MAX_PREFERRED_TRANSLATIONS)
        .collect()
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

    /// The main row's preferred translations, the default when never set.
    pub fn preferred_main_or_default(&self) -> Vec<String> {
        self.preferred_main
            .clone()
            .unwrap_or_else(|| DEFAULT_PREFERRED_MAIN.map(str::to_string).to_vec())
    }

    /// The second row's preferred translations, the default when never set.
    pub fn preferred_secondary_or_default(&self) -> Vec<String> {
        self.preferred_secondary
            .clone()
            .unwrap_or_else(|| DEFAULT_PREFERRED_SECONDARY.map(str::to_string).to_vec())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BiblePreferencesDraft {
    pub main_translation: Option<String>,
    pub secondary_translation: Option<String>,
    pub character_limit: Option<u32>,
    /// `Some` replaces the stored list (#921), `None` keeps it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_main: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_secondary: Option<Vec<String>>,
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
        if let Some(list) = self.preferred_main {
            base.preferred_main = Some(normalize_preferred_translations(list));
        }
        if let Some(list) = self.preferred_secondary {
            base.preferred_secondary = Some(normalize_preferred_translations(list));
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
    /// Optional reference label from slide (overrides passage.reference.to_human_readable())
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_label: Option<String>,
}

impl BibleBroadcast {
    pub fn new(passage: BiblePassage, triggered_at: DateTime<Utc>) -> Self {
        Self {
            passage,
            triggered_at,
            reference_label: None,
        }
    }

    pub fn with_reference_label(mut self, label: String) -> Self {
        self.reference_label = Some(label);
        self
    }
}

/// Single source of truth for all Bible slide outputs.
/// What you see in the slide IS what goes to all outputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BibleSlideOutput {
    /// Main verse text (displayed on main output)
    pub main_text: String,

    /// Main reference label (e.g., "John 3:16 (NIV)")
    pub main_reference: String,

    /// Secondary verse text (may be empty)
    pub secondary_text: String,

    /// Secondary reference label (e.g., "John 3:16 (ESV)")
    pub secondary_reference: String,

    /// Timestamp when triggered
    pub triggered_at: DateTime<Utc>,
}

impl BibleSlideOutput {
    pub fn new(
        main_text: impl Into<String>,
        main_reference: impl Into<String>,
        secondary_text: impl Into<String>,
        secondary_reference: impl Into<String>,
        triggered_at: DateTime<Utc>,
    ) -> Self {
        Self {
            main_text: main_text.into(),
            main_reference: main_reference.into(),
            secondary_text: secondary_text.into(),
            secondary_reference: secondary_reference.into(),
            triggered_at,
        }
    }

    /// Returns true if there's any content to display
    pub fn has_content(&self) -> bool {
        !self.main_text.is_empty() || !self.secondary_text.is_empty()
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
    #[error("{found} markup entries for {expected} passages")]
    MarkupCount { expected: usize, found: usize },
    #[error("markup of {book} {chapter}:{start} does not spell the passage text")]
    MarkupText {
        book: String,
        chapter: u16,
        start: u16,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BibleIngestionBatch {
    translation: BibleTranslation,
    passages: Vec<BiblePassage>,
    /// One entry per passage; `None` for a verse with nothing to tell.
    markup: Vec<Option<VerseMarkup>>,
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
        let markup = vec![None; passages.len()];
        Ok(Self {
            translation,
            passages,
            markup,
        })
    }

    /// The batch with one markup entry per passage (#851 D2a). Each must spell
    /// its passage's text exactly: the wall text and the study text are one.
    pub fn with_markup(
        self,
        markup: Vec<Option<VerseMarkup>>,
    ) -> Result<Self, BibleIngestionError> {
        if markup.len() != self.passages.len() {
            return Err(BibleIngestionError::MarkupCount {
                expected: self.passages.len(),
                found: markup.len(),
            });
        }
        for (passage, entry) in self.passages.iter().zip(&markup) {
            if entry
                .as_ref()
                .is_some_and(|m| m.plain_text() != passage.text)
            {
                return Err(BibleIngestionError::MarkupText {
                    book: passage.reference.book.clone(),
                    chapter: passage.reference.chapter,
                    start: passage.reference.verse_start,
                });
            }
        }
        Ok(Self { markup, ..self })
    }

    pub fn translation(&self) -> &BibleTranslation {
        &self.translation
    }

    pub fn passages(&self) -> &[BiblePassage] {
        &self.passages
    }

    pub fn markup(&self) -> &[Option<VerseMarkup>] {
        &self.markup
    }

    pub fn into_parts(self) -> (BibleTranslation, Vec<BiblePassage>) {
        (self.translation, self.passages)
    }

    pub fn into_parts_with_markup(
        self,
    ) -> (
        BibleTranslation,
        Vec<BiblePassage>,
        Vec<Option<VerseMarkup>>,
    ) {
        (self.translation, self.passages, self.markup)
    }
}
