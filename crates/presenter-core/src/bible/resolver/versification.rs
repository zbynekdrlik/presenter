//! The MAIN translation's chapter/verse structure (#850 spec §5.6): built from
//! `/bible/books` in the UI and from the chapter summaries on the server. It
//! binds a typed chapter/verse part to a book (the structure filter of §5.5)
//! and steps verse by verse across chapter and book bounds.

use std::collections::BTreeMap;

use super::fold::fold_book_text;
use super::hint::{chapters_hint, verses_hint};
use super::rank::{index_of, StoredName};
use super::types::{RawSpec, Segment};
use crate::bible::BibleBookChapterSummary;

/// Chapters beyond this in a summary row are malformed input and skipped.
const MAX_CHAPTER: u16 = 200;

/// One book of the main translation: its stored name and verses per chapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookStructure {
    pub code: String,
    pub number: u16,
    pub name: String,
    /// `verse_counts[c - 1]` = verses in chapter `c`; 0 = the chapter is missing.
    pub verse_counts: Vec<u16>,
}

impl BookStructure {
    pub fn chapter_count(&self) -> u16 {
        u16::try_from(self.verse_counts.len()).unwrap_or(u16::MAX)
    }

    /// Verses in `chapter`; `None` when that chapter does not exist.
    pub fn verse_count(&self, chapter: u16) -> Option<u16> {
        let index = usize::from(chapter).checked_sub(1)?;
        self.verse_counts
            .get(index)
            .copied()
            .filter(|count| *count > 0)
    }
}

/// Which way [`Versification::step`] moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Next,
    Prev,
}

/// The main translation's books in canonical order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Versification {
    books: Vec<BookStructure>,
    stored: Vec<StoredName>,
}

impl Versification {
    pub fn new(mut books: Vec<BookStructure>) -> Self {
        books.sort_by_key(|book| book.number);
        let stored = books.iter().filter_map(stored_name).collect();
        Self { books, stored }
    }

    /// From `bible_book_chapter_summaries` rows (one per book and chapter, in
    /// any order). Rows without a book code are skipped.
    pub fn from_chapter_summaries(rows: &[BibleBookChapterSummary]) -> Self {
        let mut books: BTreeMap<String, BookStructure> = BTreeMap::new();
        for row in rows {
            let Some(code) = row
                .book_code
                .as_deref()
                .map(str::trim)
                .filter(|c| !c.is_empty())
            else {
                continue;
            };
            if row.chapter == 0 || row.chapter > MAX_CHAPTER {
                continue;
            }
            let book = books
                .entry(code.to_string())
                .or_insert_with(|| BookStructure {
                    code: code.to_string(),
                    number: row
                        .book_number
                        .or_else(|| index_of(code).map(|index| u16::from(index) + 1))
                        .unwrap_or(0),
                    name: row.book.clone(),
                    verse_counts: Vec::new(),
                });
            let index = usize::from(row.chapter) - 1;
            if book.verse_counts.len() <= index {
                book.verse_counts.resize(index + 1, 0);
            }
            if let Some(slot) = book.verse_counts.get_mut(index) {
                *slot = row.verse_count;
            }
        }
        Self::new(books.into_values().collect())
    }

    pub fn books(&self) -> &[BookStructure] {
        &self.books
    }

    pub fn book(&self, code: &str) -> Option<&BookStructure> {
        self.books.iter().find(|book| book.code == code)
    }

    pub(crate) fn stored_names(&self) -> &[StoredName] {
        &self.stored
    }

    /// The next / previous verse, crossing chapter AND book bounds (the
    /// console decides whether to stop at the end of a book). `None` before
    /// the first verse, after the last one, or for an unknown book/chapter.
    pub fn step(
        &self,
        code: &str,
        chapter: u16,
        verse: u16,
        dir: Step,
    ) -> Option<(String, u16, u16)> {
        let position = self.books.iter().position(|book| book.code == code)?;
        let book = self.books.get(position)?;
        let count = book.verse_count(chapter)?;
        match dir {
            Step::Next if verse < count => Some((book.code.clone(), chapter, verse + 1)),
            Step::Prev if verse > 1 => Some((book.code.clone(), chapter, (verse - 1).min(count))),
            Step::Next => match (chapter.saturating_add(1)..=book.chapter_count())
                .find(|c| book.verse_count(*c).is_some())
            {
                Some(next) => Some((book.code.clone(), next, 1)),
                None => first_verse(self.books.get(position + 1)?),
            },
            Step::Prev => match (1..chapter)
                .rev()
                .find_map(|c| Some((c, book.verse_count(c)?)))
            {
                Some((previous, last)) => Some((book.code.clone(), previous, last)),
                None => last_verse(self.books.get(position.checked_sub(1)?)?),
            },
        }
    }
}

fn first_verse(book: &BookStructure) -> Option<(String, u16, u16)> {
    let chapter = (1..=book.chapter_count()).find(|c| book.verse_count(*c).is_some())?;
    Some((book.code.clone(), chapter, 1))
}

fn last_verse(book: &BookStructure) -> Option<(String, u16, u16)> {
    let (chapter, count) = (1..=book.chapter_count())
        .rev()
        .find_map(|c| Some((c, book.verse_count(c)?)))?;
    Some((book.code.clone(), chapter, count))
}

fn stored_name(book: &BookStructure) -> Option<StoredName> {
    let folded = fold_book_text(&book.name);
    if folded.key.is_empty() {
        return None;
    }
    Some(StoredName {
        index: index_of(&book.code)?,
        ordinal: folded.ordinal.unwrap_or(0),
        key: folded.key,
        form: book.name.clone(),
    })
}

/// Why a typed chapter/verse part does not fit a book.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Failure {
    pub(crate) requested: Segment,
    pub(crate) hint: String,
    /// The nearest passage that exists (last chapter / last verse); empty = none.
    pub(crate) fix: Vec<Segment>,
}

/// On a single-chapter book (OBA, PHM, 2JN, 3JN, JUD) a lone `n > 1` is verse
/// `n` and `n-m` are verses; `1` stays the whole book.
fn adapt(spec: &RawSpec, book: &BookStructure) -> RawSpec {
    if book.chapter_count() != 1 {
        return spec.clone();
    }
    match *spec {
        RawSpec::Chapter(verse) if verse > 1 => RawSpec::Verses {
            chapter: 1,
            ranges: vec![(verse, verse)],
        },
        RawSpec::ChapterRange { start, end } => RawSpec::Verses {
            chapter: 1,
            ranges: vec![(start, end)],
        },
        _ => spec.clone(),
    }
}

/// The segments `spec` names in `book`, or `None` when a chapter or verse does
/// not exist (spec §5.5: the structure filter runs last).
pub(crate) fn bind(spec: &RawSpec, book: &BookStructure) -> Option<Vec<Segment>> {
    let existing = |c: &u16| book.verse_count(*c).is_some();
    match adapt(spec, book) {
        RawSpec::Chapter(chapter) => book
            .verse_count(chapter)
            .map(|_| vec![Segment::whole(chapter)]),
        RawSpec::ChapterRange { start, end } => {
            book.verse_count(start)?;
            book.verse_count(end)?;
            Some((start..=end).filter(existing).map(Segment::whole).collect())
        }
        RawSpec::Verses { chapter, ranges } => {
            let count = book.verse_count(chapter)?;
            ranges.iter().all(|&(_, end)| end <= count).then(|| {
                ranges
                    .iter()
                    .map(|&(start, end)| Segment::verses(chapter, start, end))
                    .collect()
            })
        }
        RawSpec::CrossChapter {
            chapter,
            verse_start,
            end_chapter,
            verse_end,
        } => {
            let first = book.verse_count(chapter)?;
            let last = book.verse_count(end_chapter)?;
            if verse_start > first || verse_end > last {
                return None;
            }
            let mut segments = vec![Segment::verses(chapter, verse_start, first)];
            let middle = chapter.saturating_add(1)..end_chapter;
            segments.extend(middle.filter(existing).map(Segment::whole));
            segments.push(Segment::verses(end_chapter, 1, verse_end));
            Some(segments)
        }
    }
}

/// The hint and the nearest fix for a `spec` that [`bind`] refused on `book`.
pub(crate) fn out_of_range(spec: &RawSpec, book: &BookStructure) -> Failure {
    let chapters = book.chapter_count();
    let chapter_failure = |requested: Segment| Failure {
        requested,
        hint: chapters_hint(chapters),
        fix: if chapters == 0 {
            Vec::new()
        } else {
            vec![Segment::whole(chapters)]
        },
    };
    match adapt(spec, book) {
        RawSpec::Chapter(chapter) => chapter_failure(Segment::whole(chapter)),
        RawSpec::ChapterRange { start, end } => {
            let bad = if book.verse_count(start).is_none() {
                start
            } else {
                end
            };
            chapter_failure(Segment::whole(bad))
        }
        RawSpec::Verses { chapter, ranges } => {
            let (first_start, first_end) = ranges.first().copied().unwrap_or((1, 1));
            let Some(count) = book.verse_count(chapter) else {
                return chapter_failure(Segment::verses(chapter, first_start, first_end));
            };
            let (start, end) = ranges
                .iter()
                .copied()
                .find(|&(_, end)| end > count)
                .unwrap_or((first_start, first_end));
            Failure {
                requested: Segment::verses(chapter, start, end),
                hint: verses_hint(count),
                fix: vec![Segment::verses(chapter, start.min(count), count)],
            }
        }
        RawSpec::CrossChapter {
            chapter,
            verse_start,
            end_chapter,
            verse_end,
        } => {
            let requested = Segment {
                chapter,
                verse_start: Some(verse_start),
                verse_end: None,
            };
            let (Some(first), Some(last)) =
                (book.verse_count(chapter), book.verse_count(end_chapter))
            else {
                return chapter_failure(requested);
            };
            let clamped = RawSpec::CrossChapter {
                chapter,
                verse_start: verse_start.min(first),
                end_chapter,
                verse_end: verse_end.min(last),
            };
            Failure {
                requested,
                hint: verses_hint(if verse_start > first { first } else { last }),
                fix: bind(&clamped, book).unwrap_or_default(),
            }
        }
    }
}
