//! The `/ui/study` page's pure half (#851 spec §6): the URL of every view,
//! chapter stepping across books, the favourite translations, the swipe rule,
//! the verse selection and what the search field offers for an input. No
//! `web_sys` here, host-tested with `cargo test --lib`. The back / forward list
//! is in `state::study_history`.

use presenter_core::bible::canonical_book_by_code;
use presenter_core::bible::resolver::{lookup, parse, Resolution, Step, Versification};
use presenter_core::bible::sermon::translation_short_code;
use serde::{Deserialize, Serialize};

use crate::state::bible_console::{console_view, ConsoleView};

/// localStorage / sessionStorage keys (spec §6.1), after `presenter:`.
pub const KEY_LAST: &str = "study.last";
pub const KEY_LAST_VERSE: &str = "study.lastVerse";
pub const KEY_FAVOURITES: &str = "study.favourites";
pub const KEY_TEXT_SIZE: &str = "study.textSize";
pub const KEY_HISTORY: &str = "study.history";

const PATH: &str = "/ui/study";
/// Where a first visit opens: the Gospel of John, in the first favourite.
pub const START_BOOK: &str = "JHN";
pub const START_CHAPTER: u16 = 1;
pub const DEFAULT_FAVOURITES: [&str; 5] = ["slk-seb", "slk-roh", "slk-sevp", "eng-kjv", "eng-nlt"];

/// The server's minimum for `/bible/search`.
const MIN_SEARCH_CHARS: usize = 2;
/// A translation with this few books holds only the gospels (`slk-mil`).
const GOSPELS_ONLY_BOOKS: usize = 4;

/// A swipe is at least this many px sideways (spec §6.3)…
const SWIPE_MIN_PX: f64 = 60.0;
/// …and less than 30° off the horizontal (tan 30°).
const SWIPE_MAX_SLOPE: f64 = 0.577_350_269;

/// What shows over the reader. Each sheet has its own URL, so the system Back
/// gesture closes it (spec §6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StudySheet {
    /// The selected verses (`v`) and what can be done with them.
    Actions,
    /// The back list (long press on ◀).
    History,
    TextSize,
    /// Which translation sits in this favourite slot (long press on a chip).
    Favourite(usize),
}

impl StudySheet {
    fn to_param(self) -> String {
        match self {
            Self::Actions => "akcie".to_string(),
            Self::History => "historia".to_string(),
            Self::TextSize => "pismo".to_string(),
            Self::Favourite(slot) => format!("preklad{slot}"),
        }
    }

    fn parse(param: &str) -> Option<Self> {
        match param {
            "akcie" => Some(Self::Actions),
            "historia" => Some(Self::History),
            "pismo" => Some(Self::TextSize),
            other => other
                .strip_prefix("preklad")?
                .parse()
                .ok()
                .filter(|slot| *slot < DEFAULT_FAVOURITES.len())
                .map(Self::Favourite),
        }
    }
}

/// The reader or the text search. New views (sermons, E1) add a variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StudyView {
    Reader { book: String, chapter: u16 },
    Search { query: String },
}

/// Everything a study URL says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StudyRoute {
    /// `None` until the page picks one (a hand-typed URL without `t`).
    pub translation: Option<String>,
    pub view: StudyView,
    /// The highlighted verses of the reader, in reading order.
    pub verses: Option<(u16, u16)>,
    pub sheet: Option<StudySheet>,
}

impl StudyRoute {
    pub fn reader(translation: &str, book: &str, chapter: u16) -> Self {
        Self {
            translation: Some(translation.to_string()),
            view: StudyView::Reader {
                book: book.to_string(),
                chapter,
            },
            verses: None,
            sheet: None,
        }
    }

    pub fn search(translation: &str, query: &str) -> Self {
        Self {
            translation: Some(translation.to_string()),
            view: StudyView::Search {
                query: query.to_string(),
            },
            verses: None,
            sheet: None,
        }
    }

    pub fn with_verses(mut self, verses: Option<(u16, u16)>) -> Self {
        self.verses = verses;
        self
    }

    pub fn with_sheet(mut self, sheet: Option<StudySheet>) -> Self {
        self.sheet = sheet;
        self
    }

    /// The route for a location's query string (with or without `?`). `None`
    /// for anything that names no view; the page then opens the last position.
    pub fn parse(search: &str) -> Option<Self> {
        let mut translation = None;
        let mut passage = None;
        let mut view = None;
        let mut query = None;
        let mut verses = None;
        let mut sheet = None;
        for pair in search.trim_start_matches('?').split('&') {
            let Some((name, value)) = pair.split_once('=') else {
                continue;
            };
            let value = decode(value);
            match name {
                "t" => translation = Some(value).filter(|code| !code.is_empty()),
                "p" => passage = parse_passage(&value),
                "view" => view = Some(value),
                "q" => query = Some(value),
                "v" => verses = parse_verses(&value),
                "s" => sheet = StudySheet::parse(&value),
                _ => {}
            }
        }
        let view = match view.as_deref() {
            Some("search") => StudyView::Search {
                query: query.filter(|text| !text.trim().is_empty())?,
            },
            None => {
                let (book, chapter) = passage?;
                StudyView::Reader { book, chapter }
            }
            Some(_) => return None,
        };
        let reading = matches!(view, StudyView::Reader { .. });
        Some(Self {
            translation,
            view,
            verses: verses.filter(|_| reading),
            sheet,
        })
    }

    pub fn to_url(&self) -> String {
        let mut params = Vec::new();
        if matches!(self.view, StudyView::Search { .. }) {
            params.push("view=search".to_string());
        }
        if let Some(code) = &self.translation {
            params.push(format!("t={}", urlencoding::encode(code)));
        }
        match &self.view {
            StudyView::Reader { book, chapter } => {
                params.push(format!("p={}.{chapter}", urlencoding::encode(book)));
                if let Some((start, end)) = self.verses {
                    params.push(if start == end {
                        format!("v={start}")
                    } else {
                        format!("v={start}-{end}")
                    });
                }
            }
            StudyView::Search { query } => {
                params.push(format!(
                    "q={}",
                    urlencoding::encode(query).replace("%20", "+")
                ));
            }
        }
        if let Some(sheet) = self.sheet {
            params.push(format!("s={}", sheet.to_param()));
        }
        format!("{PATH}?{}", params.join("&"))
    }

    /// The same position with a sheet closed: what the history list shows.
    pub fn without_sheet(&self) -> Self {
        self.clone().with_sheet(None)
    }
}

fn decode(value: &str) -> String {
    let spaced = value.replace('+', " ");
    urlencoding::decode(&spaced)
        .map(|text| text.into_owned())
        .unwrap_or(spaced)
}

/// `HEB.11`: a three-character book code and a chapter.
fn parse_passage(value: &str) -> Option<(String, u16)> {
    let (book, chapter) = value.split_once('.')?;
    let book = book.to_ascii_uppercase();
    let chapter = chapter.parse().ok().filter(|chapter| *chapter > 0)?;
    (book.len() == 3 && book.chars().all(|c| c.is_ascii_alphanumeric())).then_some((book, chapter))
}

/// `1` or `3-12`.
fn parse_verses(value: &str) -> Option<(u16, u16)> {
    let (start, end) = value.split_once('-').unwrap_or((value, value));
    let (start, end) = (start.parse().ok()?, end.parse().ok()?);
    (start > 0 && end >= start).then_some((start, end))
}

// ---------------------------------------------------------------------------
// Labels
// ---------------------------------------------------------------------------

/// `Hebrejom 11 · SEB`: the history list's row.
pub fn reader_label(book: &str, chapter: u16, translation: &str) -> String {
    format!("{book} {chapter} · {}", translation_short_code(translation))
}

/// `Hľadanie: viera · SEB`.
pub fn search_label(query: &str, translation: &str) -> String {
    format!(
        "Hľadanie: {query} · {}",
        translation_short_code(translation)
    )
}

/// `Hebrejom 11:1-3 (SEB)`, the form every quotation carries (NLT attribution).
pub fn verse_reference(book: &str, chapter: u16, verses: (u16, u16), translation: &str) -> String {
    let (start, end) = verses;
    let short = translation_short_code(translation);
    if start == end {
        format!("{book} {chapter}:{start} ({short})")
    } else {
        format!("{book} {chapter}:{start}-{end} ({short})")
    }
}

/// What Kopírovať puts on the clipboard: the text, numbered when it is more
/// than one verse, then the reference.
pub fn copy_text<'a>(verses: impl Iterator<Item = (u16, &'a str)>, reference: &str) -> String {
    let verses: Vec<_> = verses.collect();
    let body = match verses.as_slice() {
        [(_, text)] => (*text).to_string(),
        many => many
            .iter()
            .map(|(verse, text)| format!("{verse} {text}"))
            .collect::<Vec<_>>()
            .join("\n"),
    };
    format!("{body}\n{reference}")
}

// ---------------------------------------------------------------------------
// Chapter stepping
// ---------------------------------------------------------------------------

/// The chapter before / after `chapter` of `book` in this translation: over a
/// book boundary, and `None` at Genesis 1 and Revelation 22. A position the
/// translation does not have (a partial one) steps to its nearest chapter, so
/// the reader never gets stuck there.
pub fn step_chapter(
    structure: &Versification,
    book: &str,
    chapter: u16,
    dir: Step,
) -> Option<(String, u16)> {
    let known = structure
        .book(book)
        .and_then(|found| found.verse_count(chapter));
    if let Some(verses) = known {
        let from = if dir == Step::Next { verses } else { 1 };
        let (code, chapter, _) = structure.step(book, chapter, from, dir)?;
        return Some((code, chapter));
    }
    if dir == Step::Prev {
        if let Some(found) = structure.book(book) {
            let last = (1..=found.chapter_count())
                .rev()
                .find(|c| found.verse_count(*c).is_some());
            if let Some(last) = last.filter(|last| *last < chapter) {
                return Some((book.to_string(), last));
            }
        }
    }
    nearest_book(structure, book, dir)
}

fn nearest_book(structure: &Versification, book: &str, dir: Step) -> Option<(String, u16)> {
    let number = canonical_book_by_code(book)?.number;
    let books = structure.books().iter();
    let found = match dir {
        Step::Next => books.clone().find(|other| other.number > number),
        Step::Prev => books.rev().find(|other| other.number < number),
    }?;
    let mut chapters = (1..=found.chapter_count()).filter(|c| found.verse_count(*c).is_some());
    let chapter = match dir {
        Step::Next => chapters.next(),
        Step::Prev => chapters.next_back(),
    }?;
    Some((found.code.clone(), chapter))
}

/// The bottom bar's text for a chapter step: `kap. 10`, or the book too when
/// the step leaves this book (`Malachiáš 4`).
pub fn step_label(structure: &Versification, book: &str, target: (&str, u16)) -> String {
    let (code, chapter) = target;
    if code == book {
        return format!("kap. {chapter}");
    }
    match structure.book(code) {
        Some(found) => format!("{} {chapter}", found.name),
        None => format!("{code} {chapter}"),
    }
}

/// Why a translation shows no text at this position.
pub fn missing_text(structure: Option<&Versification>) -> &'static str {
    match structure {
        Some(found) if found.books().len() == GOSPELS_ONLY_BOOKS => {
            "Tento preklad obsahuje len evanjeliá"
        }
        _ => "Táto kapitola v tomto preklade nie je",
    }
}

/// What a failed read tells the reader: the server's NLT sentence without the
/// socket details after it (`NLT nedostupné — API/internet: Heb.11: tcp…`).
/// Any other failure text is an English HTTP status, never shown.
pub fn failure_text(detail: &str) -> String {
    match detail.split(':').next().map(str::trim) {
        Some(sentence) if sentence.starts_with("NLT ") => sentence.to_string(),
        _ => "Nepodarilo sa načítať. Skontroluj pripojenie.".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Favourites + text size
// ---------------------------------------------------------------------------

/// The five slots from `study.favourites`; a missing or damaged value is the
/// default set.
pub fn parse_favourites(stored: Option<&str>) -> Vec<String> {
    let slots: Vec<String> = stored
        .unwrap_or_default()
        .split(',')
        .filter(|code| !code.is_empty())
        .map(str::to_string)
        .collect();
    if slots.len() == DEFAULT_FAVOURITES.len() {
        slots
    } else {
        DEFAULT_FAVOURITES
            .iter()
            .map(|code| code.to_string())
            .collect()
    }
}

/// `(slot, translation)` of each chip. A slot whose translation is not
/// installed here takes the next unused installed one; with too few
/// installed, the slot has no chip.
pub fn visible_favourites(slots: &[String], installed: &[String]) -> Vec<(usize, String)> {
    let mut spare = installed
        .iter()
        .filter(|code| !slots.contains(code))
        .cloned();
    slots
        .iter()
        .enumerate()
        .filter_map(|(slot, code)| {
            let code = installed
                .contains(code)
                .then(|| code.clone())
                .or_else(|| spare.next())?;
            Some((slot, code))
        })
        .collect()
}

/// `code` into `slot`. When another slot holds it, the two swap, so no chip
/// is ever a duplicate.
pub fn set_favourite(slots: &[String], slot: usize, code: &str) -> Vec<String> {
    let mut slots = slots.to_vec();
    if let (Some(other), Some(old)) = (
        slots.iter().position(|held| held == code),
        slots.get(slot).cloned(),
    ) {
        slots[other] = old;
    }
    if let Some(held) = slots.get_mut(slot) {
        *held = code.to_string();
    }
    slots
}

/// The reader's three text sizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextSize {
    Small,
    #[default]
    Medium,
    Large,
}

impl TextSize {
    pub const ALL: [Self; 3] = [Self::Small, Self::Medium, Self::Large];

    pub fn parse(stored: Option<&str>) -> Self {
        Self::ALL
            .into_iter()
            .find(|size| Some(size.key()) == stored)
            .unwrap_or_default()
    }

    /// The stored value, also `data-text-size` (the CSS keys on it).
    pub fn key(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Medium => "medium",
            Self::Large => "large",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Small => "Menšie",
            Self::Medium => "Stredné",
            Self::Large => "Väčšie",
        }
    }
}

// ---------------------------------------------------------------------------
// Gestures + selection
// ---------------------------------------------------------------------------

/// The chapter step of a finished horizontal drag (spec §6.3): a finger moving
/// left turns to the next chapter.
pub fn swipe_step(dx: f64, dy: f64) -> Option<Step> {
    if dx.abs() < SWIPE_MIN_PX || dy.abs() >= dx.abs() * SWIPE_MAX_SLOPE {
        return None;
    }
    Some(if dx < 0.0 { Step::Next } else { Step::Prev })
}

/// The selection after a tap on a verse number: the first tap selects, another
/// extends it to a range, the only selected verse deselects, and a tap inside
/// a range narrows it to that verse.
pub fn extend_selection(current: Option<(u16, u16)>, tapped: u16) -> Option<(u16, u16)> {
    match current {
        None => Some((tapped, tapped)),
        Some((start, end)) if start == end && start == tapped => None,
        Some((start, end)) if tapped < start => Some((tapped, end)),
        Some((start, end)) if tapped > end => Some((start, tapped)),
        Some(_) => Some((tapped, tapped)),
    }
}

// ---------------------------------------------------------------------------
// The search field
// ---------------------------------------------------------------------------

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
}

impl StudySuggest {
    /// What Enter opens: the first passage, or a book's first chapter.
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
/// offer the text search; so does anything else the resolver has no passage
/// for, except a hint about numbers ("Napíš aj knihu").
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
        && matches!(resolution, Resolution::NoBook { hint: None, .. });
    StudySuggest {
        view,
        grid,
        text_search: wordy.then(|| trimmed.to_string()),
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

/// A hit's text cut at the words the query matched. The server searches word
/// prefixes without diacritics, and so does this; `true` = marked.
pub fn mark_matches(text: &str, query: &str) -> Vec<(String, bool)> {
    let needles: Vec<String> = query
        .split_whitespace()
        .map(|word| {
            fold(
                &word
                    .chars()
                    .filter(|c| c.is_alphanumeric())
                    .collect::<String>(),
            )
        })
        .filter(|word| word.chars().count() >= MIN_SEARCH_CHARS)
        .collect();
    let mut parts: Vec<(String, bool)> = Vec::new();
    let mut push = |piece: &str, marked: bool| match parts.last_mut() {
        Some((held, was)) if *was == marked => held.push_str(piece),
        _ if piece.is_empty() => {}
        _ => parts.push((piece.to_string(), marked)),
    };
    let mut start = 0;
    for (index, c) in text.char_indices().chain([(text.len(), ' ')]) {
        if c.is_alphanumeric() {
            continue;
        }
        let word = &text[start..index];
        let folded = fold(word);
        let marked = needles
            .iter()
            .any(|needle| folded.starts_with(needle.as_str()));
        push(word, marked);
        if index < text.len() {
            push(&text[index..index + c.len_utf8()], false);
        }
        start = index + c.len_utf8();
    }
    parts
}

fn fold(word: &str) -> String {
    word.to_lowercase().chars().map(fold_char).collect()
}

fn fold_char(c: char) -> char {
    match c {
        'á' | 'ä' => 'a',
        'č' => 'c',
        'ď' => 'd',
        'é' | 'ě' => 'e',
        'í' => 'i',
        'ĺ' | 'ľ' => 'l',
        'ň' => 'n',
        'ó' | 'ô' => 'o',
        'ŕ' | 'ř' => 'r',
        'š' => 's',
        'ť' => 't',
        'ú' | 'ů' => 'u',
        'ý' => 'y',
        'ž' => 'z',
        other => other,
    }
}

/// One verse of a chapter, also the unit of the chapter cache.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StudyVerse {
    pub verse: u16,
    pub text: String,
}

/// The chapter cache's key and the reader's settle marker: `slk-seb:HEB.11`.
pub fn chapter_key(translation: &str, book: &str, chapter: u16) -> String {
    format!("{translation}:{book}.{chapter}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::bible::BibleBookDto;
    use crate::state::bible_console::versification;

    fn structure(name: &str) -> Versification {
        let raw = match name {
            "seb" => include_str!(
                "../../../presenter-core/src/bible/resolver/testdata/structure-slk-seb.json"
            ),
            _ => include_str!(
                "../../../presenter-core/src/bible/resolver/testdata/structure-eng-kjv.json"
            ),
        };
        let books: Vec<BibleBookDto> = serde_json::from_str(raw).expect("a structure fixture");
        versification(&books)
    }

    fn gospels() -> Versification {
        let books: Vec<BibleBookDto> = serde_json::from_value(serde_json::json!([
            { "book": "Ján", "code": "JHN", "number": 43,
              "chapters": [{ "number": 1, "verseCount": 51 }, { "number": 2, "verseCount": 25 }] },
            { "book": "Marek", "code": "MRK", "number": 41,
              "chapters": [{ "number": 1, "verseCount": 45 }] },
            { "book": "Lukáš", "code": "LUK", "number": 42,
              "chapters": [{ "number": 1, "verseCount": 80 }] },
            { "book": "Matúš", "code": "MAT", "number": 40,
              "chapters": [{ "number": 1, "verseCount": 25 }] }
        ]))
        .expect("books");
        versification(&books)
    }

    #[test]
    fn a_reader_url_round_trips_with_verses_and_a_sheet() {
        for url in [
            "/ui/study?t=slk-seb&p=HEB.11",
            "/ui/study?t=slk-seb&p=HEB.11&v=1",
            "/ui/study?t=eng-kjv&p=1JN.3&v=3-12",
            "/ui/study?t=slk-seb&p=HEB.11&v=1-3&s=akcie",
            "/ui/study?t=slk-seb&p=HEB.11&s=historia",
            "/ui/study?t=slk-seb&p=HEB.11&s=pismo",
            "/ui/study?t=slk-seb&p=HEB.11&s=preklad4",
            "/ui/study?view=search&t=slk-seb&q=viera+zarukou",
        ] {
            let query = url.split_once('?').expect("a query").1;
            let route = StudyRoute::parse(query).expect(url);
            assert_eq!(route.to_url(), url);
        }
    }

    #[test]
    fn a_query_with_diacritics_and_signs_survives_the_url() {
        let route = StudyRoute::search("slk-seb", "viera & zárukou");
        let url = route.to_url();
        assert!(!url.contains(' ') && !url[url.find('?').expect("?")..].contains("& "));
        let parsed = StudyRoute::parse(url.split_once('?').expect("?").1).expect("a route");
        assert_eq!(parsed, route);
    }

    #[test]
    fn the_parser_takes_a_lowercase_code_and_a_leading_question_mark() {
        let route = StudyRoute::parse("?t=slk-seb&p=heb.11").expect("a route");
        assert_eq!(route, StudyRoute::reader("slk-seb", "HEB", 11));
    }

    #[test]
    fn a_translation_can_be_missing_but_never_the_view() {
        let route = StudyRoute::parse("p=HEB.11").expect("a route");
        assert_eq!(route.translation, None);
        for broken in [
            "",
            "t=slk-seb",
            "p=HEB",
            "p=HEB.0",
            "p=HEB.x",
            "p=HEBREWS.1",
            "view=search",
            "view=search&q=%20",
            "view=sermons",
            "view=compare&p=HEB.11",
        ] {
            assert_eq!(StudyRoute::parse(broken), None, "{broken}");
        }
    }

    #[test]
    fn a_broken_verse_or_sheet_is_dropped_and_the_position_stays() {
        for query in [
            "p=HEB.11&v=0",
            "p=HEB.11&v=5-3",
            "p=HEB.11&v=x",
            "p=HEB.11&s=nie",
        ] {
            let route = StudyRoute::parse(query).expect(query);
            assert_eq!((route.verses, route.sheet), (None, None), "{query}");
        }
        assert_eq!(
            StudyRoute::parse("p=HEB.11&s=preklad5")
                .expect("a route")
                .sheet,
            None
        );
        assert_eq!(
            StudyRoute::parse("view=search&q=viera&v=3")
                .expect("a route")
                .verses,
            None
        );
    }

    #[test]
    fn stepping_crosses_books_and_stops_at_both_ends_of_the_bible() {
        let kjv = structure("kjv");
        let step = |book, chapter, dir| step_chapter(&kjv, book, chapter, dir);
        assert_eq!(step("HEB", 11, Step::Next), Some(("HEB".to_string(), 12)));
        assert_eq!(step("HEB", 11, Step::Prev), Some(("HEB".to_string(), 10)));
        assert_eq!(step("MAT", 28, Step::Next), Some(("MRK".to_string(), 1)));
        assert_eq!(step("MRK", 1, Step::Prev), Some(("MAT".to_string(), 28)));
        assert_eq!(step("GEN", 1, Step::Prev), None);
        assert_eq!(step("REV", 22, Step::Next), None);
        assert_eq!(step("GEN", 1, Step::Next), Some(("GEN".to_string(), 2)));
    }

    #[test]
    fn each_translation_steps_through_its_own_chapters() {
        // KJV Malachi has 4 chapters, SEB 3.
        assert_eq!(
            step_chapter(&structure("kjv"), "MAL", 3, Step::Next),
            Some(("MAL".to_string(), 4))
        );
        assert_eq!(
            step_chapter(&structure("seb"), "MAL", 3, Step::Next),
            Some(("MAT".to_string(), 1))
        );
    }

    #[test]
    fn a_position_the_translation_lacks_steps_to_its_nearest_chapter() {
        let gospels = gospels();
        // Malachi 4 opened in the KJV, then a switch to a gospels-only module.
        assert_eq!(
            step_chapter(&gospels, "MAL", 4, Step::Next),
            Some(("MAT".to_string(), 1))
        );
        assert_eq!(step_chapter(&gospels, "MAL", 4, Step::Prev), None);
        assert_eq!(
            step_chapter(&gospels, "HEB", 11, Step::Prev),
            Some(("JHN".to_string(), 2))
        );
        assert_eq!(step_chapter(&gospels, "HEB", 11, Step::Next), None);
        // A chapter past this translation's last one in a book it has.
        assert_eq!(
            step_chapter(&gospels, "JHN", 21, Step::Prev),
            Some(("JHN".to_string(), 2))
        );
    }

    #[test]
    fn the_bar_names_the_book_only_when_the_step_leaves_it() {
        let seb = structure("seb");
        assert_eq!(step_label(&seb, "HEB", ("HEB", 12)), "kap. 12");
        let other = seb.book("MRK").expect("Marek").name.clone();
        assert_eq!(step_label(&seb, "MAT", ("MRK", 1)), format!("{other} 1"));
    }

    #[test]
    fn a_gospels_only_translation_says_so() {
        assert_eq!(
            missing_text(Some(&gospels())),
            "Tento preklad obsahuje len evanjeliá"
        );
        assert_eq!(
            missing_text(Some(&structure("seb"))),
            "Táto kapitola v tomto preklade nie je"
        );
        assert_eq!(missing_text(None), "Táto kapitola v tomto preklade nie je");
    }

    #[test]
    fn a_failure_reads_as_a_sentence_without_socket_details() {
        assert_eq!(
            failure_text("NLT nedostupné — API/internet: Heb.11.1-40: error sending request"),
            "NLT nedostupné — API/internet"
        );
        for other in ["", "Bad Gateway"] {
            assert_eq!(
                failure_text(other),
                "Nepodarilo sa načítať. Skontroluj pripojenie."
            );
        }
    }

    #[test]
    fn labels_carry_the_short_code() {
        assert_eq!(reader_label("Hebrejom", 11, "slk-seb"), "Hebrejom 11 · SEB");
        assert_eq!(search_label("viera", "eng-kjv"), "Hľadanie: viera · KJV");
        assert_eq!(
            verse_reference("Hebrejom", 11, (1, 1), "slk-seb"),
            "Hebrejom 11:1 (SEB)"
        );
        assert_eq!(
            verse_reference("Hebrews", 11, (1, 3), "eng-nlt"),
            "Hebrews 11:1-3 (NLT)"
        );
    }

    #[test]
    fn a_copy_is_the_text_and_the_reference() {
        assert_eq!(
            copy_text(
                [(1, "Viera je zárukou.")].into_iter(),
                "Hebrejom 11:1 (SEB)"
            ),
            "Viera je zárukou.\nHebrejom 11:1 (SEB)"
        );
        assert_eq!(
            copy_text([(1, "Prvý."), (2, "Druhý.")].into_iter(), "Ján 1:1-2 (SEB)"),
            "1 Prvý.\n2 Druhý.\nJán 1:1-2 (SEB)"
        );
    }

    #[test]
    fn favourites_fall_back_to_the_default_set_when_damaged() {
        assert_eq!(parse_favourites(None), DEFAULT_FAVOURITES);
        assert_eq!(
            parse_favourites(Some("slk-seb,eng-kjv")),
            DEFAULT_FAVOURITES
        );
        assert_eq!(
            parse_favourites(Some("a,b,c,d,e")),
            ["a", "b", "c", "d", "e"]
        );
    }

    #[test]
    fn an_uninstalled_favourite_gives_its_chip_to_a_spare_translation() {
        let slots = parse_favourites(None);
        let installed: Vec<String> = ["eng-kjv", "slk-mil", "slk-seb"]
            .map(str::to_string)
            .to_vec();
        assert_eq!(
            visible_favourites(&slots, &installed),
            [
                (0, "slk-seb".to_string()),
                (1, "slk-mil".to_string()),
                (3, "eng-kjv".to_string())
            ]
        );
    }

    #[test]
    fn a_favourite_set_to_another_slots_translation_swaps_them() {
        let slots = parse_favourites(None);
        let moved = set_favourite(&slots, 0, "eng-kjv");
        assert_eq!(moved[0], "eng-kjv");
        assert_eq!(moved[3], "slk-seb");
        let new = set_favourite(&slots, 2, "slk-mil");
        assert_eq!(new[2], "slk-mil");
        assert_eq!(new[0], "slk-seb");
        assert_eq!(set_favourite(&slots, 2, "slk-sevp"), slots);
    }

    #[test]
    fn the_text_size_defaults_to_medium() {
        assert_eq!(TextSize::parse(None), TextSize::Medium);
        assert_eq!(TextSize::parse(Some("huge")), TextSize::Medium);
        for size in TextSize::ALL {
            assert_eq!(TextSize::parse(Some(size.key())), size);
        }
    }

    #[test]
    fn a_swipe_needs_60_px_and_less_than_30_degrees() {
        assert_eq!(swipe_step(-60.0, 0.0), Some(Step::Next));
        assert_eq!(swipe_step(60.0, 0.0), Some(Step::Prev));
        assert_eq!(swipe_step(-59.9, 0.0), None);
        // tan 30° × 100 = 57.7.
        assert_eq!(swipe_step(100.0, 57.0), Some(Step::Prev));
        assert_eq!(swipe_step(100.0, -57.0), Some(Step::Prev));
        assert_eq!(swipe_step(100.0, 58.0), None);
        assert_eq!(swipe_step(0.0, 200.0), None);
    }

    #[test]
    fn verse_taps_select_extend_and_clear() {
        assert_eq!(extend_selection(None, 5), Some((5, 5)));
        assert_eq!(extend_selection(Some((5, 5)), 5), None);
        assert_eq!(extend_selection(Some((5, 5)), 8), Some((5, 8)));
        assert_eq!(extend_selection(Some((5, 8)), 3), Some((3, 8)));
        assert_eq!(extend_selection(Some((3, 8)), 6), Some((6, 6)));
    }

    #[test]
    fn a_reference_with_a_verse_opens_there_on_enter() {
        let suggest = suggest("zid 11 1", &structure("seb"), "slk-seb");
        assert_eq!(
            suggest.enter_target(),
            Some(StudyTarget {
                book: "HEB".to_string(),
                chapter: 11,
                verses: Some((1, 1))
            })
        );
        assert_eq!(suggest.text_search, None);
    }

    #[test]
    fn a_whole_chapter_and_a_range_keep_their_shape() {
        let seb = structure("seb");
        let chapter = suggest("zid 13", &seb, "slk-seb").enter_target();
        assert_eq!(chapter.expect("a chapter").verses, None);
        let range = suggest("mt 5,3-12", &seb, "slk-seb").enter_target();
        assert_eq!(range.expect("a range").verses, Some((3, 12)));
    }

    #[test]
    fn g26_stays_a_reference_while_there_are_no_strongs_numbers() {
        let target = suggest("G26", &structure("seb"), "slk-seb")
            .enter_target()
            .expect("Genezis 26");
        assert_eq!((target.book.as_str(), target.chapter), ("GEN", 26));
    }

    #[test]
    fn words_offer_the_text_search_and_nothing_to_open() {
        let suggest = suggest("viera zarukou", &structure("seb"), "slk-seb");
        assert_eq!(suggest.text_search.as_deref(), Some("viera zarukou"));
        assert_eq!(suggest.enter_target(), None);
    }

    #[test]
    fn a_book_alone_shows_its_chapters_and_enter_opens_the_first() {
        let suggest = suggest("zid", &structure("seb"), "slk-seb");
        let grid = suggest.grid.clone().expect("a chapter grid");
        assert_eq!(grid.code, "HEB");
        assert_eq!(grid.chapters.len(), 13);
        assert_eq!(suggest.enter_target().expect("a target").chapter, 1);
    }

    #[test]
    fn numbers_without_a_book_never_become_a_text_search() {
        let suggest = suggest("11:1", &structure("seb"), "slk-seb");
        assert_eq!(suggest.text_search, None);
        assert_eq!(suggest.view.hint.as_deref(), Some("Napíš aj knihu"));
    }

    #[test]
    fn a_chapter_past_the_end_hints_and_searches_nothing() {
        let suggest = suggest("zid 14", &structure("seb"), "slk-seb");
        assert_eq!(
            suggest.view.hint.as_deref(),
            Some("Kniha má len 13 kapitol")
        );
        assert_eq!(suggest.text_search, None);
        assert_eq!(suggest.enter_target(), None);
    }

    #[test]
    fn without_the_structure_a_reference_gets_a_hint_and_words_still_search() {
        for reference in ["zid 11 1", "jan", "11:1", "1"] {
            let found = suggest_unresolved(reference);
            assert_eq!(found.text_search, None, "{reference}");
            assert!(found.view.hint.is_some(), "{reference}");
        }
        let words = suggest_unresolved("viera zarukou");
        assert_eq!(words.text_search.as_deref(), Some("viera zarukou"));
        assert_eq!(suggest_unresolved(" "), StudySuggest::default());
    }

    #[test]
    fn an_empty_field_suggests_nothing() {
        assert_eq!(
            suggest("  ", &structure("seb"), "slk-seb"),
            StudySuggest::default()
        );
    }

    #[test]
    fn matches_mark_whole_words_by_prefix_without_diacritics() {
        let parts = mark_matches("Viera je zárukou toho, v čo dúfame", "viera zaruk");
        assert_eq!(
            parts,
            [
                ("Viera".to_string(), true),
                (" je ".to_string(), false),
                ("zárukou".to_string(), true),
                (" toho, v čo dúfame".to_string(), false),
            ]
        );
    }

    #[test]
    fn marks_keep_every_character_of_the_text() {
        let text = "„Áno,“ riekol — a šiel.  Koniec";
        let parts = mark_matches(text, "ano siel");
        let joined: String = parts.iter().map(|(piece, _)| piece.as_str()).collect();
        assert_eq!(joined, text);
        let marked: Vec<&str> = parts
            .iter()
            .filter(|(_, marked)| *marked)
            .map(|(piece, _)| piece.as_str())
            .collect();
        assert_eq!(marked, ["Áno", "šiel"]);
    }

    #[test]
    fn a_query_with_nothing_to_mark_leaves_the_text_whole() {
        assert_eq!(
            mark_matches("Na počiatku.", "x"),
            [("Na počiatku.".to_string(), false)]
        );
    }

    #[test]
    fn the_settle_key_names_translation_book_and_chapter() {
        assert_eq!(chapter_key("slk-seb", "HEB", 11), "slk-seb:HEB.11");
    }
}
