//! The study page's reads (#851). [`load_chapter`] is the ONE place a chapter
//! is fetched; every verse read of the server goes through `bible_source`, so
//! the NLT arrives here like any other translation.

use std::collections::VecDeque;

use serde::Deserialize;

use super::{get_json_detail, ApiError};
use crate::state::study::StudyVerse;

/// The last chapters kept in memory (spec §6.3).
const CACHED_CHAPTERS: usize = 30;
/// The most cross-references the server answers for a verse.
const MAX_CROSS_REFERENCES: u32 = 500;

/// One chapter of one translation, as the reader shows it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ChapterText {
    /// The name this translation gives the book.
    pub book: String,
    pub verses: Vec<StudyVerse>,
}

/// Every verse of a chapter. An empty `verses` = this translation does not
/// have the chapter. A failure carries the server's message ("NLT nedostupné…").
pub async fn load_chapter(
    translation: &str,
    book_code: &str,
    chapter: u16,
) -> Result<ChapterText, ApiError> {
    get_json_detail(&chapter_path(translation, book_code, chapter)).await
}

fn chapter_path(translation: &str, book_code: &str, chapter: u16) -> String {
    format!(
        "/bible/study/chapter?translation={}&book_code={}&chapter={chapter}",
        urlencoding::encode(translation),
        urlencoding::encode(book_code),
    )
}

/// A passage that shares a theme with a verse (OpenBible.info).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CrossReference {
    pub label: String,
    pub book_code: String,
    pub chapter: u16,
    pub verse: u16,
    pub chapter_end: u16,
    pub verse_end: u16,
    pub votes: i32,
    pub text: String,
    /// Whose text this is, when the asked translation has none here.
    #[serde(default)]
    pub fallback_translation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CrossReferences {
    pub total: usize,
    pub items: Vec<CrossReference>,
}

/// All the cross-references of a verse, the most voted first.
pub async fn load_cross_references(
    translation: &str,
    book_code: &str,
    (chapter, verse): (u16, u16),
) -> Result<CrossReferences, ApiError> {
    get_json_detail(&crossrefs_path(translation, book_code, chapter, verse)).await
}

fn crossrefs_path(translation: &str, book_code: &str, chapter: u16, verse: u16) -> String {
    format!(
        "/bible/study/crossrefs?translation={}&book_code={}&chapter={chapter}&verse={verse}&limit={MAX_CROSS_REFERENCES}",
        urlencoding::encode(translation),
        urlencoding::encode(book_code),
    )
}

/// The most recently read chapters by [`crate::state::study::chapter_key`].
#[derive(Debug, Default)]
pub struct ChapterCache {
    chapters: VecDeque<(String, ChapterText)>,
}

impl ChapterCache {
    /// A chapter read again counts as the newest.
    pub fn get(&mut self, key: &str) -> Option<ChapterText> {
        let index = self.chapters.iter().position(|(held, _)| held == key)?;
        let entry = self.chapters.remove(index)?;
        let text = entry.1.clone();
        self.chapters.push_back(entry);
        Some(text)
    }

    pub fn put(&mut self, key: String, text: ChapterText) {
        self.chapters.retain(|(held, _)| *held != key);
        self.chapters.push_back((key, text));
        if self.chapters.len() > CACHED_CHAPTERS {
            self.chapters.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chapter_is_read_by_book_code() {
        assert_eq!(
            chapter_path("slk-seb", "HEB", 11),
            "/bible/study/chapter?translation=slk-seb&book_code=HEB&chapter=11"
        );
    }

    #[test]
    fn the_server_chapter_becomes_the_reader_text() {
        let chapter: ChapterText = serde_json::from_value(serde_json::json!({
            "translation": "slk-seb", "bookCode": "HEB", "bookNumber": 58,
            "book": "Hebrejom", "chapter": 11, "verseCount": 40,
            "prev": null, "next": null,
            "verses": [{ "verse": 1, "text": "Viera", "markup": {} }]
        }))
        .expect("a chapter");
        assert_eq!(chapter.book, "Hebrejom");
        assert_eq!(
            chapter.verses,
            [StudyVerse {
                verse: 1,
                text: "Viera".to_string()
            }]
        );
    }

    #[test]
    fn cross_references_ask_for_every_row_of_the_verse() {
        assert_eq!(
            crossrefs_path("slk-seb", "JHN", 3, 16),
            "/bible/study/crossrefs?translation=slk-seb&book_code=JHN&chapter=3&verse=16&limit=500"
        );
    }

    #[test]
    fn a_cross_reference_knows_whose_text_it_carries() {
        let answer: CrossReferences = serde_json::from_value(serde_json::json!({
            "total": 1,
            "items": [{
                "label": "Joel 2:28", "bookCode": "JOL", "chapter": 2, "verse": 28,
                "chapterEnd": 2, "verseEnd": 28, "votes": 9, "text": "And it shall",
                "fallbackTranslation": "eng-kjv"
            }]
        }))
        .expect("cross references");
        assert_eq!(
            answer.items[0].fallback_translation.as_deref(),
            Some("eng-kjv")
        );
    }

    #[test]
    fn the_cache_keeps_the_30_newest_and_a_read_renews_a_chapter() {
        let mut cache = ChapterCache::default();
        let text = ChapterText {
            book: "Ján".to_string(),
            verses: Vec::new(),
        };
        for chapter in 0..30 {
            cache.put(format!("k{chapter}"), text.clone());
        }
        assert!(cache.get("k0").is_some());
        cache.put("k30".to_string(), text.clone());
        assert!(cache.get("k0").is_some(), "renewed by the read");
        assert!(cache.get("k1").is_none(), "the oldest left");
    }

    #[test]
    fn the_same_chapter_put_twice_is_one_entry() {
        let mut cache = ChapterCache::default();
        let text = |book: &str| ChapterText {
            book: book.to_string(),
            verses: Vec::new(),
        };
        cache.put("k".to_string(), text("a"));
        cache.put("k".to_string(), text("b"));
        assert_eq!(cache.get("k").expect("cached").book, "b");
        assert_eq!(cache.chapters.len(), 1);
    }
}
