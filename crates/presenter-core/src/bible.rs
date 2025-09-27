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
        }
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
}
