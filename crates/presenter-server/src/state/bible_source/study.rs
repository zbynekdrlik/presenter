//! The study page's reads (#851 D4, spec §11): a chapter with its markup, a
//! Strong's entry, the verses of a number, the cross-references of a verse.
//!
//! Cross-references never call a remote API: a list of ten would spend ten of
//! the NLT's 500 daily requests and fail whole with it, so the NLT reads the
//! KJV's text and says so. A place the translation does not have stays inside
//! its chapter, and its text comes from the KJV (`fallback_translation`).

use std::collections::HashMap;

use presenter_core::bible::{
    canonical_book_by_code, canonical_book_by_number, BibleBookCanonical, BibleBookChapterSummary,
    BookScope, CrossReference, LexiconEntry, VerseMarkup,
};
use presenter_persistence::StrongsVerse;

use super::super::AppState;
use crate::bible_remote::{self, STRUCTURE_TRANSLATION_CODE};

/// Strong's numbers live in the KJV only.
const STRONGS_TRANSLATION: &str = STRUCTURE_TRANSLATION_CODE;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChapterPlace {
    pub(crate) book_code: String,
    pub(crate) chapter: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StudyVerse {
    pub(crate) verse: u16,
    pub(crate) text: String,
    pub(crate) markup: Option<VerseMarkup>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StudyChapter {
    pub(crate) book_code: String,
    pub(crate) book_number: u16,
    pub(crate) book: String,
    pub(crate) chapter: u16,
    pub(crate) verse_count: u16,
    pub(crate) prev: Option<ChapterPlace>,
    pub(crate) next: Option<ChapterPlace>,
    pub(crate) verses: Vec<StudyVerse>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StrongsPage {
    pub(crate) total: u64,
    pub(crate) per_book: Vec<(String, u64)>,
    pub(crate) items: Vec<StrongsVerse>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StudyCrossReference {
    pub(crate) label: String,
    pub(crate) book_code: String,
    pub(crate) chapter: u16,
    pub(crate) verse: u16,
    pub(crate) chapter_end: u16,
    pub(crate) verse_end: u16,
    pub(crate) votes: i32,
    pub(crate) text: String,
    pub(crate) fallback_translation: Option<String>,
}

/// The chapters of one translation, by book number.
struct Structure {
    books: HashMap<u16, (String, Vec<(u16, u16)>)>,
}

impl Structure {
    fn new(summaries: &[BibleBookChapterSummary]) -> Self {
        let mut books: HashMap<u16, (String, Vec<(u16, u16)>)> = HashMap::new();
        for summary in summaries {
            let Some(number) = summary.book_number else {
                continue;
            };
            books
                .entry(number)
                .or_insert_with(|| (summary.book.clone(), Vec::new()))
                .1
                .push((summary.chapter, summary.verse_count));
        }
        Self { books }
    }

    fn name(&self, book: u16) -> Option<&str> {
        self.books.get(&book).map(|(name, _)| name.as_str())
    }

    fn verse_count(&self, book: u16, chapter: u16) -> Option<u16> {
        let (_, chapters) = self.books.get(&book)?;
        let found = chapters.iter().find(|(number, _)| *number == chapter)?;
        Some(found.1)
    }

    fn last_place(&self, book: u16) -> Option<(u16, u16)> {
        let (_, chapters) = self.books.get(&book)?;
        chapters.iter().copied().max()
    }

    /// The chapters before and after `(book, chapter)`, which need not exist.
    fn neighbours(&self, book: u16, chapter: u16) -> (Option<ChapterPlace>, Option<ChapterPlace>) {
        let mut places: Vec<(u16, u16)> = self
            .books
            .iter()
            .flat_map(|(number, (_, chapters))| chapters.iter().map(|c| (*number, c.0)))
            .collect();
        places.sort_unstable();
        let split = places.partition_point(|place| *place < (book, chapter));
        let prev = split.checked_sub(1).map(|at| places[at]);
        let next = places[split..]
            .iter()
            .copied()
            .find(|place| *place > (book, chapter));
        (place_of(prev), place_of(next))
    }
}

fn place_of(place: Option<(u16, u16)>) -> Option<ChapterPlace> {
    let (book, chapter) = place?;
    Some(ChapterPlace {
        book_code: canonical_book_by_number(book)?.code.to_string(),
        chapter,
    })
}

/// `Heb 11:1`, `Heb 11:1-3`, `Heb 11:39-12:2`, `2 Jn 1:13 - 3 Jn 1:2`.
fn range_label(book: &str, end_book: Option<&str>, start: (u16, u16), end: (u16, u16)) -> String {
    let (chapter, verse) = start;
    let (chapter_end, verse_end) = end;
    if let Some(end_book) = end_book {
        return format!("{book} {chapter}:{verse} - {end_book} {chapter_end}:{verse_end}");
    }
    if start == end {
        format!("{book} {chapter}:{verse}")
    } else if chapter == chapter_end {
        format!("{book} {chapter}:{verse}-{verse_end}")
    } else {
        format!("{book} {chapter}:{verse}-{chapter_end}:{verse_end}")
    }
}

/// A reference placed in one translation's chapters.
struct Placed {
    start: (u16, u16),
    end: (u16, u16),
    /// What the label shows: the row's own end when the range leaves the book.
    label_end: (u16, u16),
    end_book: Option<u16>,
}

/// Keep a row's range inside the structure's chapters; the end of a range that
/// leaves the book is the book's last verse. `None`: the structure has no such
/// chapter.
fn place_reference(structure: &Structure, row: &CrossReference) -> Option<Placed> {
    let number = |value: i32| u16::try_from(value).ok();
    let book = number(row.to_book)?;
    let chapter = number(row.to_chapter)?;
    let verse = number(row.to_verse)?.min(structure.verse_count(book, chapter)?);
    let start = (chapter, verse);
    let row_end = (number(row.to_chapter_end)?, number(row.to_verse_end)?);
    if row.to_book_end != row.to_book {
        return Some(Placed {
            start,
            end: structure.last_place(book)?,
            label_end: row_end,
            end_book: number(row.to_book_end),
        });
    }
    let last = structure.last_place(book)?;
    let chapter_end = row_end.0.min(last.0);
    let verse_end = match structure.verse_count(book, chapter_end) {
        Some(count) => row_end.1.min(count),
        None => row_end.1,
    };
    let end = (chapter_end, verse_end).max(start);
    Some(Placed {
        start,
        end,
        label_end: end,
        end_book: None,
    })
}

impl AppState {
    async fn knows_translation(&self, translation: &str) -> anyhow::Result<bool> {
        let translations = self.list_bible_translations().await?;
        Ok(translations.iter().any(|known| known.code == translation))
    }

    /// `None`: no such translation, book or chapter. A chapter only the KJV
    /// has (a partial translation, Malachi 4 in the SEB) is a chapter with no
    /// verses, so the reader can say why and move on.
    pub(crate) async fn study_chapter(
        &self,
        translation: &str,
        book_code: &str,
        chapter: u16,
    ) -> anyhow::Result<Option<StudyChapter>> {
        let Some(canon) = canonical_book_by_code(book_code) else {
            return Ok(None);
        };
        if !self.knows_translation(translation).await? {
            return Ok(None);
        }
        let summaries = self.bible_book_chapter_summaries(translation).await?;
        let structure = Structure::new(&summaries);
        let (prev, next) = structure.neighbours(canon.number, chapter);
        let Some(verse_count) = structure.verse_count(canon.number, chapter) else {
            let reference = self
                .bible_book_chapter_summaries(STRUCTURE_TRANSLATION_CODE)
                .await?;
            if Structure::new(&reference)
                .verse_count(canon.number, chapter)
                .is_none()
            {
                return Ok(None);
            }
            return Ok(Some(StudyChapter {
                book_code: canon.code.to_string(),
                book_number: canon.number,
                book: canon.english_name.to_string(),
                chapter,
                verse_count: 0,
                prev,
                next,
                verses: Vec::new(),
            }));
        };
        let book = structure
            .name(canon.number)
            .unwrap_or(canon.english_name)
            .to_string();
        let verses = self
            .study_verses(translation, canon, &book, chapter, verse_count)
            .await?;
        Ok(Some(StudyChapter {
            book_code: canon.code.to_string(),
            book_number: canon.number,
            book,
            chapter,
            verse_count,
            prev,
            next,
            verses,
        }))
    }

    async fn study_verses(
        &self,
        translation: &str,
        canon: BibleBookCanonical,
        book: &str,
        chapter: u16,
        verse_count: u16,
    ) -> anyhow::Result<Vec<StudyVerse>> {
        let mut markup: HashMap<u16, VerseMarkup> =
            if bible_remote::is_remote_translation(translation) {
                HashMap::new()
            } else {
                self.repository
                    .bible_chapter_markup(translation, canon.code, chapter)
                    .await?
                    .into_iter()
                    .collect()
            };
        let passages = self
            .bible_passage_range(translation, book, Some(canon.code), chapter, 1, verse_count)
            .await?;
        Ok(passages
            .into_iter()
            .map(|passage| {
                let verse = passage.reference.verse_start;
                StudyVerse {
                    verse,
                    text: passage.text,
                    markup: markup.remove(&verse),
                }
            })
            .collect())
    }

    /// The entry and the number of KJV verses holding it.
    pub(crate) async fn study_lexicon(
        &self,
        strongs: &str,
    ) -> anyhow::Result<Option<(LexiconEntry, u64)>> {
        let Some(entry) = self.repository.bible_lexicon_entry(strongs).await? else {
            return Ok(None);
        };
        let count = self
            .repository
            .bible_strongs_verse_count(STRONGS_TRANSLATION, strongs, BookScope::All)
            .await?;
        Ok(Some((entry, count)))
    }

    /// One page of a scope. `per_book` always counts the whole Bible: the
    /// scope chips show it whichever scope is chosen.
    pub(crate) async fn study_strongs(
        &self,
        strongs: &str,
        scope: BookScope,
        offset: u32,
        limit: u32,
    ) -> anyhow::Result<StrongsPage> {
        let repository = &self.repository;
        Ok(StrongsPage {
            total: repository
                .bible_strongs_verse_count(STRONGS_TRANSLATION, strongs, scope)
                .await?,
            per_book: repository
                .bible_strongs_book_counts(STRONGS_TRANSLATION, strongs)
                .await?,
            items: repository
                .bible_strongs_verses(STRONGS_TRANSLATION, strongs, scope, offset, limit)
                .await?,
        })
    }

    /// The total and the `limit` most voted references; `None`: no such
    /// translation or book. A verse without references is an empty list.
    pub(crate) async fn study_crossrefs(
        &self,
        translation: &str,
        book_code: &str,
        chapter: u16,
        verse: u16,
        limit: usize,
    ) -> anyhow::Result<Option<(usize, Vec<StudyCrossReference>)>> {
        let Some(canon) = canonical_book_by_code(book_code) else {
            return Ok(None);
        };
        if !self.knows_translation(translation).await? {
            return Ok(None);
        }
        let rows = self
            .repository
            .bible_cross_references_from(
                i32::from(canon.number),
                i32::from(chapter),
                i32::from(verse),
            )
            .await?;
        let total = rows.len();
        let reader = bible_remote::structure_source(translation);
        let own = Structure::new(&self.repository.bible_book_chapter_summaries(reader).await?);
        let kjv = Structure::new(
            &self
                .repository
                .bible_book_chapter_summaries(STRUCTURE_TRANSLATION_CODE)
                .await?,
        );
        let mut items = Vec::with_capacity(limit.min(total));
        for row in rows.iter().take(limit) {
            match self
                .study_crossref(translation, reader, &own, &kjv, row)
                .await?
            {
                Some(item) => items.push(item),
                None => tracing::warn!(?row, "cross-reference outside the KJV — left out"),
            }
        }
        Ok(Some((total, items)))
    }

    async fn study_crossref(
        &self,
        translation: &str,
        reader: &str,
        own: &Structure,
        kjv: &Structure,
        row: &CrossReference,
    ) -> anyhow::Result<Option<StudyCrossReference>> {
        let Some(book) = u16::try_from(row.to_book)
            .ok()
            .and_then(canonical_book_by_number)
        else {
            return Ok(None);
        };
        let own_place = place_reference(own, row);
        let own_text = match &own_place {
            Some(place) => self.range_text(reader, book.code, place).await?,
            None => String::new(),
        };
        let (place, names, text, read_from) = match own_place {
            Some(place) if !own_text.is_empty() => (place, own, own_text, reader),
            _ => {
                let Some(place) = place_reference(kjv, row) else {
                    return Ok(None);
                };
                let text = self
                    .range_text(STRUCTURE_TRANSLATION_CODE, book.code, &place)
                    .await?;
                (place, kjv, text, STRUCTURE_TRANSLATION_CODE)
            }
        };
        let name = names.name(book.number).unwrap_or(book.english_name);
        let end_name = place.end_book.map(|number| {
            names
                .name(number)
                .or(canonical_book_by_number(number).map(|end| end.english_name))
                .unwrap_or_default()
        });
        Ok(Some(StudyCrossReference {
            label: range_label(name, end_name, place.start, place.label_end),
            book_code: book.code.to_string(),
            chapter: place.start.0,
            verse: place.start.1,
            chapter_end: place.end.0,
            verse_end: place.end.1,
            votes: row.votes,
            text,
            fallback_translation: (read_from != translation).then(|| read_from.to_string()),
        }))
    }

    /// The range's verses joined, across its chapters.
    async fn range_text(
        &self,
        reader: &str,
        book_code: &str,
        place: &Placed,
    ) -> anyhow::Result<String> {
        let mut parts = Vec::new();
        for chapter in place.start.0..=place.end.0 {
            let from = if chapter == place.start.0 {
                place.start.1
            } else {
                1
            };
            let to = if chapter == place.end.0 {
                place.end.1
            } else {
                u16::MAX
            };
            let passages = self
                .repository
                .bible_passage_range(reader, "", Some(book_code), chapter, from, to)
                .await?;
            parts.extend(passages.into_iter().map(|passage| passage.text));
        }
        Ok(parts.join(" "))
    }
}

#[cfg(test)]
mod tests;
