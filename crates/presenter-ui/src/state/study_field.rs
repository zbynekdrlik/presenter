//! What the study page's one field offers for an input (#851 spec §6.4): a
//! Strong's number, the resolver's reading of a reference, a book's chapters and
//! the text search. Pure, host-tested in `state::study`.

use presenter_core::bible::resolver::{lookup, parse, Resolution, Versification};

use crate::state::bible_console::{console_view, ConsoleView};
use crate::state::study::MIN_SEARCH_CHARS;
use crate::state::study_word::{strongs_offer, StrongsOffer};

/// Where a pick from the field goes in the reader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StudyTarget {
    pub book: String,
    pub chapter: u16,
    pub verses: Option<(u16, u16)>,
}

/// A book of the chapter grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChapterGrid {
    pub code: String,
    pub name: String,
    pub chapters: Vec<u16>,
}

/// What the field offers under it for one input.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StudySuggest {
    pub view: ConsoleView,
    pub grid: Option<ChapterGrid>,
    /// The text to search for, when the input can be words of a verse.
    pub text_search: Option<String>,
    /// The input read as a Strong's number.
    pub strongs: Option<StrongsOffer>,
}

/// What Enter does in the field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldEnter {
    Open(StudyTarget),
    Strongs(String),
    Search(String),
}

impl StudySuggest {
    /// `G26` is the number, `g 26` the passage; words search the text.
    pub fn enter(&self) -> Option<FieldEnter> {
        let number = self.strongs.as_ref();
        if let Some(first) = number.filter(|offer| offer.first) {
            return Some(FieldEnter::Strongs(first.number.clone()));
        }
        self.enter_target()
            .map(FieldEnter::Open)
            .or_else(|| number.map(|offer| FieldEnter::Strongs(offer.number.clone())))
            .or_else(|| self.text_search.clone().map(FieldEnter::Search))
    }

    /// The first passage, or a book's first chapter.
    pub fn enter_target(&self) -> Option<StudyTarget> {
        if let Some(target) = self.view.default_target() {
            return Some(preview_target(target));
        }
        let grid = self.grid.as_ref()?;
        Some(StudyTarget {
            book: grid.code.clone(),
            chapter: *grid.chapters.first()?,
            verses: None,
        })
    }
}

/// A resolver target as a reader position.
pub fn preview_target(target: &crate::state::bible_console::PreviewTarget) -> StudyTarget {
    let verses = target.segment.verse_start.map(|start| {
        let end = target.segment.verse_end.unwrap_or(start).max(start);
        (start, end)
    });
    StudyTarget {
        book: target.book.code.clone(),
        chapter: target.segment.chapter,
        verses,
    }
}

/// The resolver's reading of `input` against the reader's translation (spec
/// §6.4), through the verse console's row builder. Words that are no reference
/// offer the text search, and so does a word that happens to name a book
/// ("Mojžiš", "kráľ"), under its chapters; Enter still opens the book. A hint
/// about numbers ("Napíš aj knihu") or a chapter past the end searches nothing.
pub fn suggest(input: &str, structure: &Versification, translation: &str) -> StudySuggest {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return StudySuggest::default();
    }
    let resolution = lookup(input, structure);
    let view = console_view(&resolution, translation);
    let grid = match &resolution {
        Resolution::BookOnly { book, .. } => chapter_grid(structure, &book.code),
        _ => None,
    };
    let wordy = trimmed.chars().count() >= MIN_SEARCH_CHARS
        && matches!(
            resolution,
            Resolution::NoBook { hint: None, .. } | Resolution::BookOnly { .. }
        );
    StudySuggest {
        view,
        grid,
        text_search: wordy.then(|| trimmed.to_string()),
        strongs: strongs_offer(trimmed),
    }
}

/// The field without the translation's structure: nothing can be resolved. Words
/// that name no book can still be searched, but a reference must not fall into
/// the text search and come back as a false "Nič sa nenašlo".
pub fn suggest_unresolved(input: &str) -> StudySuggest {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return StudySuggest::default();
    }
    let parsed = parse(input);
    if !parsed.candidates.is_empty() || parsed.spec.is_some() || parsed.bare_ordinal.is_some() {
        return StudySuggest {
            view: ConsoleView {
                hint: Some("Zoznam kníh nie je načítaný, odkaz sa nedá otvoriť".to_string()),
                ..ConsoleView::default()
            },
            strongs: strongs_offer(trimmed),
            ..StudySuggest::default()
        };
    }
    StudySuggest {
        text_search: (trimmed.chars().count() >= MIN_SEARCH_CHARS).then(|| trimmed.to_string()),
        ..StudySuggest::default()
    }
}

/// The chapters of a book in this translation.
pub fn chapter_grid(structure: &Versification, code: &str) -> Option<ChapterGrid> {
    let book = structure.book(code)?;
    Some(ChapterGrid {
        code: book.code.clone(),
        name: book.name.clone(),
        chapters: (1..=book.chapter_count())
            .filter(|chapter| book.verse_count(*chapter).is_some())
            .collect(),
    })
}
