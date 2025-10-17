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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub book_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub book_number: Option<u16>,
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
        Self::validate(chapter, verse_start, verse_end)?;
        Ok(Self {
            book: book.into(),
            book_code: None,
            book_number: None,
            chapter,
            verse_start,
            verse_end,
        })
    }

    pub fn new_with_code<T: Into<String>, U: Into<String>>(
        book: T,
        book_code: U,
        book_number: u16,
        chapter: u16,
        verse_start: u16,
        verse_end: u16,
    ) -> Result<Self, BibleReferenceError> {
        Self::validate(chapter, verse_start, verse_end)?;
        Ok(Self {
            book: book.into(),
            book_code: Some(book_code.into()),
            book_number: Some(book_number),
            chapter,
            verse_start,
            verse_end,
        })
    }

    fn validate(chapter: u16, verse_start: u16, verse_end: u16) -> Result<(), BibleReferenceError> {
        if chapter == 0 {
            return Err(BibleReferenceError::InvalidChapter);
        }
        if verse_start == 0 || verse_end == 0 || verse_start > verse_end {
            return Err(BibleReferenceError::InvalidVerseRange);
        }
        Ok(())
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
