//! The study page's reads (#851). [`load_chapter`] is the ONE place a chapter
//! is fetched, so the chapter endpoint of unit D4 replaces it here.
//!
//! Until then the chapter comes from `POST /bible/resolve`: `/bible/passage`
//! finds only a row whose verse span equals the request, and rows are single
//! verses, so it can never answer a chapter. Resolve composes through the
//! server's one verse dispatch (`bible_source`, NLT included), stores nothing,
//! and with `characterLimit: 1` every verse is a slide of its own (a lone verse
//! is never split, #434), written `"{verse}. {text}"`.

use std::collections::VecDeque;

use super::bible::{resolve_slides, BibleSlideDto, ResolveRequest};
use super::ApiError;
use crate::state::study::StudyVerse;

/// The last chapters kept in memory (spec §6.3).
const CACHED_CHAPTERS: usize = 30;

/// One chapter of one translation, as the reader shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    let answer = resolve_slides(&chapter_request(translation, book_code, chapter)).await?;
    Ok(chapter_text(&answer.slides))
}

fn chapter_request(translation: &str, book_code: &str, chapter: u16) -> ResolveRequest {
    ResolveRequest {
        main_translation: translation.to_string(),
        secondary_translation: None,
        book: book_code.to_string(),
        book_code: Some(book_code.to_string()),
        chapter,
        verse_start: 1,
        verse_end: None,
        character_limit: Some(1),
    }
}

fn chapter_text(slides: &[BibleSlideDto]) -> ChapterText {
    let book = slides
        .first()
        .and_then(|slide| slide.metadata.as_ref()?.bible.as_ref()?.book.clone())
        .unwrap_or_default();
    ChapterText {
        book,
        verses: slides
            .iter()
            .filter_map(|slide| verse_of(&slide.bible_main))
            .collect(),
    }
}

/// A composer line back into its verse number and text.
fn verse_of(line: &str) -> Option<StudyVerse> {
    let (number, text) = line.split_once(". ")?;
    Some(StudyVerse {
        verse: number.parse().ok()?,
        text: text.to_string(),
    })
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

    fn slide(main: &str) -> BibleSlideDto {
        serde_json::from_value(serde_json::json!({
            "id": "s", "order": 0, "bibleMain": main, "bibleTranslation": "",
            "bibleMainReference": "", "bibleTranslationReference": "",
            "metadata": { "bible": { "book": "Hebrejom", "verses": [] } }
        }))
        .expect("a resolve slide")
    }

    #[test]
    fn the_request_asks_for_the_whole_chapter_one_verse_per_slide() {
        let request = chapter_request("slk-seb", "HEB", 11);
        assert_eq!(request.verse_start, 1);
        assert_eq!(request.verse_end, None, "None = to the chapter's end");
        assert_eq!(request.character_limit, Some(1));
        assert_eq!(request.secondary_translation, None);
        assert_eq!(request.book_code.as_deref(), Some("HEB"));
    }

    #[test]
    fn slides_become_numbered_verses_and_the_translations_book_name() {
        let chapter = chapter_text(&[
            slide("1. Viera je zárukou."),
            slide("2. Pre ňu. Druhá veta."),
        ]);
        assert_eq!(chapter.book, "Hebrejom");
        assert_eq!(
            chapter.verses,
            [
                StudyVerse {
                    verse: 1,
                    text: "Viera je zárukou.".to_string()
                },
                StudyVerse {
                    verse: 2,
                    text: "Pre ňu. Druhá veta.".to_string()
                },
            ]
        );
    }

    #[test]
    fn no_slides_is_a_chapter_this_translation_lacks() {
        assert_eq!(
            chapter_text(&[]),
            ChapterText {
                book: String::new(),
                verses: Vec::new()
            }
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
