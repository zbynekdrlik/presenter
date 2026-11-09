//! The `/ui/study` page's pure half (#851 spec §6): the URL of every view,
//! chapter stepping across books, the favourite translations, the swipe rule,
//! the verse selection and what the search field offers for an input. No
//! `web_sys` here, host-tested with `cargo test --lib`. The back / forward list
//! is in `state::study_history`.

use presenter_core::bible::canonical_book_by_code;
use presenter_core::bible::resolver::{Step, Versification};
use presenter_core::bible::sermon::translation_short_code;
use presenter_core::bible::{normalize_strongs, VerseMarkup};
use serde::{Deserialize, Serialize};

pub use crate::state::study_field::{
    chapter_grid, preview_target, suggest, suggest_unresolved, ChapterGrid, FieldEnter,
    StudySuggest, StudyTarget,
};
use crate::state::study_word::{normalize_scope, WordPlace, SCOPE_ALL};

/// localStorage / sessionStorage keys (spec §6.1), after `presenter:`.
pub const KEY_LAST: &str = "study.last";
pub const KEY_LAST_VERSE: &str = "study.lastVerse";
pub const KEY_FAVOURITES: &str = "study.favourites";
pub const KEY_TEXT_SIZE: &str = "study.textSize";
pub const KEY_HISTORY: &str = "study.history";
pub const KEY_ACTIVE_SERMON: &str = "study.activeSermon";

const PATH: &str = "/ui/study";
/// Where a first visit opens: the Gospel of John, in the first favourite.
pub const START_BOOK: &str = "JHN";
pub const START_CHAPTER: u16 = 1;
pub const DEFAULT_FAVOURITES: [&str; 5] = ["slk-seb", "slk-roh", "slk-sevp", "eng-kjv", "eng-nlt"];

/// The server's minimum for `/bible/search`.
pub(crate) const MIN_SEARCH_CHARS: usize = 2;
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
    /// Every installed translation, to read in one now (▾ after the chips, #921).
    Translations,
    /// Which sermon the passage goes to. It exists only for the passage in hand,
    /// so a URL never opens it.
    AddToSermon,
    /// The open sermon's stored slides.
    Preview,
    /// The open sermon's items in their order.
    Reorder,
    /// The cross-references of the first selected verse.
    CrossRefs,
    /// A tagged word (`StudyRoute::word`), over the reader or compare.
    Word,
    /// The named bookmarks (#915), from the header.
    Bookmarks,
    /// A name for a new bookmark at the first selected verse.
    SaveBookmark,
}

impl StudySheet {
    /// A sheet about the selected verses.
    pub fn needs_verses(self) -> bool {
        matches!(self, Self::Actions | Self::CrossRefs | Self::SaveBookmark)
    }

    /// `s=`. The word sheet has its own parameters (`w`, `wv`).
    fn to_param(self) -> Option<String> {
        Some(match self {
            Self::Actions => "akcie".to_string(),
            Self::History => "historia".to_string(),
            Self::TextSize => "pismo".to_string(),
            Self::Favourite(slot) => format!("preklad{slot}"),
            Self::Translations => "preklady".to_string(),
            Self::AddToSermon => "pridat".to_string(),
            Self::Preview => "nahlad".to_string(),
            Self::Reorder => "poradie".to_string(),
            Self::CrossRefs => "odkazy".to_string(),
            Self::Bookmarks => "zalozky".to_string(),
            Self::SaveBookmark => "zalozka".to_string(),
            Self::Word => return None,
        })
    }

    fn parse(param: &str) -> Option<Self> {
        match param {
            "akcie" => Some(Self::Actions),
            "historia" => Some(Self::History),
            "pismo" => Some(Self::TextSize),
            "nahlad" => Some(Self::Preview),
            "poradie" => Some(Self::Reorder),
            "odkazy" => Some(Self::CrossRefs),
            "zalozky" => Some(Self::Bookmarks),
            "zalozka" => Some(Self::SaveBookmark),
            "preklady" => Some(Self::Translations),
            // `pridat` is no sheet of a URL: the passage it adds is gone.
            other => other
                .strip_prefix("preklad")?
                .parse()
                .ok()
                .filter(|slot| *slot < DEFAULT_FAVOURITES.len())
                .map(Self::Favourite),
        }
    }
}

/// The reader, the text search, the sermon list and one sermon's editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StudyView {
    Reader {
        book: String,
        chapter: u16,
    },
    Search {
        query: String,
        /// `all|ot|nt|<BOOK>`, in the URL unless it is `all`.
        scope: String,
    },
    /// One verse (a phone) or its chapter (wider) across translations. An empty
    /// `translations` = the favourites.
    Compare {
        book: String,
        chapter: u16,
        verse: u16,
        translations: Vec<String>,
    },
    /// Where a Strong's number occurs (`n`, not the spec's `s`: that is the
    /// sheets'). `scope` = `all|ot|nt|<BOOK>`.
    Strongs {
        number: String,
        scope: String,
    },
    Sermons,
    Sermon {
        id: String,
    },
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
    /// The word under `StudySheet::Word`.
    pub word: Option<WordPlace>,
    /// The sermon open beside the reader or the search on a wide screen.
    pub pane: Option<String>,
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
            word: None,
            pane: None,
        }
    }

    pub fn search(translation: &str, query: &str) -> Self {
        Self {
            translation: Some(translation.to_string()),
            view: StudyView::Search {
                query: query.to_string(),
                scope: SCOPE_ALL.to_string(),
            },
            verses: None,
            sheet: None,
            word: None,
            pane: None,
        }
    }

    pub fn compare(
        translation: Option<String>,
        (book, chapter, verse): (&str, u16, u16),
        translations: Vec<String>,
    ) -> Self {
        Self {
            translation,
            view: StudyView::Compare {
                book: book.to_string(),
                chapter,
                verse,
                translations,
            },
            verses: None,
            sheet: None,
            word: None,
            pane: None,
        }
    }

    pub fn strongs(translation: Option<String>, number: &str, scope: &str) -> Self {
        Self {
            translation,
            view: StudyView::Strongs {
                number: number.to_string(),
                scope: scope.to_string(),
            },
            verses: None,
            sheet: None,
            word: None,
            pane: None,
        }
    }

    pub fn sermons(translation: Option<String>) -> Self {
        Self {
            translation,
            view: StudyView::Sermons,
            verses: None,
            sheet: None,
            word: None,
            pane: None,
        }
    }

    pub fn sermon(translation: Option<String>, id: &str) -> Self {
        Self {
            translation,
            view: StudyView::Sermon { id: id.to_string() },
            verses: None,
            sheet: None,
            word: None,
            pane: None,
        }
    }

    pub fn with_verses(mut self, verses: Option<(u16, u16)>) -> Self {
        self.verses = verses;
        self
    }

    pub fn with_sheet(mut self, sheet: Option<StudySheet>) -> Self {
        self.sheet = sheet;
        self.word = None;
        self
    }

    /// The word sheet on this word.
    pub fn with_word(mut self, word: WordPlace) -> Self {
        self.sheet = Some(StudySheet::Word);
        self.word = Some(word);
        self
    }

    /// Only a sheet opened, closed or changed: the reader stays where it is.
    /// Another verse of the same chapter is a move, not a closed sheet.
    pub fn only_sheet_differs(&self, other: &Self) -> bool {
        (self.sheet != other.sheet || self.word != other.word)
            && self.without_sheet().with_verses(None) == other.without_sheet().with_verses(None)
    }

    /// The route for a location's query string (with or without `?`). `None`
    /// for anything that names no view; the page then opens the last position.
    pub fn parse(search: &str) -> Option<Self> {
        let mut translation = None;
        let mut passage = None;
        let mut place = None;
        let mut compared = Vec::new();
        let mut view = None;
        let mut query = None;
        let mut verses = None;
        let mut sheet = None;
        let mut id = None;
        let mut pane = None;
        let mut number = None;
        let mut scope = None;
        let mut word_number = None;
        let mut word_place = None;
        for pair in search.trim_start_matches('?').split('&') {
            let Some((name, value)) = pair.split_once('=') else {
                continue;
            };
            let value = decode(value);
            match name {
                "t" => translation = Some(value).filter(|code| !code.is_empty()),
                "p" => {
                    passage = parse_passage(&value);
                    place = parse_verse_place(&value);
                }
                "tt" => compared = parse_translations(&value),
                "view" => view = Some(value),
                "q" => query = Some(value),
                "v" => verses = parse_verses(&value),
                "s" => sheet = StudySheet::parse(&value),
                "id" => id = Some(value).filter(|id| !id.is_empty()),
                "pane" => pane = Some(value).filter(|id| !id.is_empty()),
                "n" => number = normalize_strongs(&value),
                "scope" => scope = Some(normalize_scope(&value)),
                "w" => word_number = Some(value),
                "wv" => word_place = Some(value),
                _ => {}
            }
        }
        let view = match view.as_deref() {
            Some("search") => StudyView::Search {
                query: query.filter(|text| !text.trim().is_empty())?,
                scope: scope.unwrap_or_else(|| SCOPE_ALL.to_string()),
            },
            Some("compare") => {
                let (book, chapter, verse) = place?;
                StudyView::Compare {
                    book,
                    chapter,
                    verse,
                    translations: compared,
                }
            }
            Some("strongs") => StudyView::Strongs {
                number: number?,
                scope: scope.unwrap_or_else(|| SCOPE_ALL.to_string()),
            },
            Some("sermons") => StudyView::Sermons,
            Some("sermon") => StudyView::Sermon { id: id? },
            None => {
                let (book, chapter) = passage?;
                StudyView::Reader { book, chapter }
            }
            Some(_) => return None,
        };
        let reading = matches!(view, StudyView::Reader { .. });
        // Compare covers the sermon, but keeps it for the way back to the text.
        let beside_text = keeps_sermon(&view);
        let verses = verses.filter(|_| reading);
        // These sheets are about the verses: none, no sheet.
        let sheet = sheet.filter(|sheet| !sheet.needs_verses() || verses.is_some());
        // A word sits in the text of the reader or of compare.
        let word = word_number
            .zip(word_place)
            .and_then(|(number, place)| WordPlace::parse(&number, &place))
            .filter(|_| sheet.is_none())
            .filter(|_| reading || matches!(view, StudyView::Compare { .. }));
        Some(Self {
            translation,
            view,
            verses,
            sheet: word.as_ref().map(|_| StudySheet::Word).or(sheet),
            word,
            pane: pane.filter(|_| beside_text),
        })
    }

    pub fn to_url(&self) -> String {
        let mut params = Vec::new();
        match &self.view {
            StudyView::Reader { .. } => {}
            StudyView::Search { .. } => params.push("view=search".to_string()),
            StudyView::Compare { .. } => params.push("view=compare".to_string()),
            StudyView::Strongs { .. } => params.push("view=strongs".to_string()),
            StudyView::Sermons => params.push("view=sermons".to_string()),
            StudyView::Sermon { .. } => params.push("view=sermon".to_string()),
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
            StudyView::Search { query, scope } => {
                params.push(format!(
                    "q={}",
                    urlencoding::encode(query).replace("%20", "+")
                ));
                if scope != SCOPE_ALL {
                    params.push(format!("scope={}", urlencoding::encode(scope)));
                }
            }
            StudyView::Compare {
                book,
                chapter,
                verse,
                translations,
            } => {
                params.push(format!("p={}.{chapter}.{verse}", urlencoding::encode(book)));
                if !translations.is_empty() {
                    let codes: Vec<_> = translations
                        .iter()
                        .map(|code| urlencoding::encode(code).into_owned())
                        .collect();
                    params.push(format!("tt={}", codes.join(",")));
                }
            }
            StudyView::Strongs { number, scope } => {
                params.push(format!("n={}", urlencoding::encode(number)));
                params.push(format!("scope={}", urlencoding::encode(scope)));
            }
            StudyView::Sermons => {}
            StudyView::Sermon { id } => params.push(format!("id={}", urlencoding::encode(id))),
        }
        if let Some(id) = &self.pane {
            params.push(format!("pane={}", urlencoding::encode(id)));
        }
        if let Some(param) = self.sheet.and_then(StudySheet::to_param) {
            params.push(format!("s={param}"));
        }
        if let Some(word) = &self.word {
            params.extend(word.to_params());
        }
        format!("{PATH}?{}", params.join("&"))
    }

    /// The sermon on this route, as the whole view or beside the text.
    pub fn open_sermon(&self) -> Option<&str> {
        match &self.view {
            StudyView::Sermon { id } => Some(id),
            _ => self.pane.as_deref(),
        }
    }

    /// Is a sermon on screen beside the text? Compare has the whole width.
    pub fn shows_pane(&self) -> bool {
        self.pane.is_some() && !matches!(self.view, StudyView::Compare { .. })
    }

    /// A move inside the text keeps the sermon beside it.
    pub fn keeping_pane(mut self, current: &Self) -> Self {
        if keeps_sermon(&self.view) && self.pane.is_none() {
            self.pane.clone_from(&current.pane);
        }
        self
    }

    /// Where this route belongs at this width, when that is somewhere else: a
    /// wide screen reads `reading` with the sermon beside it, a narrow one
    /// shows the sermon alone. So a rotation loses neither.
    pub fn for_width(&self, wide: bool, reading: Self) -> Option<Self> {
        match (&self.view, &self.pane) {
            (StudyView::Sermon { id }, _) if wide => Some(Self {
                translation: self.translation.clone().or(reading.translation),
                view: reading.view,
                verses: None,
                sheet: self.sheet,
                word: None,
                pane: Some(id.clone()),
            }),
            (_, Some(id)) if !wide && self.shows_pane() => {
                let mut alone = Self::sermon(self.translation.clone(), id);
                alone.sheet = self
                    .sheet
                    .filter(|sheet| matches!(sheet, StudySheet::Preview | StudySheet::Reorder));
                Some(alone)
            }
            _ => None,
        }
    }

    /// The same position with a sheet closed: what the history list shows.
    pub fn without_sheet(&self) -> Self {
        self.clone().with_sheet(None)
    }
}

/// The views of the text: a sermon stays beside them.
fn keeps_sermon(view: &StudyView) -> bool {
    matches!(
        view,
        StudyView::Reader { .. }
            | StudyView::Search { .. }
            | StudyView::Compare { .. }
            | StudyView::Strongs { .. }
    )
}

fn decode(value: &str) -> String {
    let spaced = value.replace('+', " ");
    urlencoding::decode(&spaced)
        .map(|text| text.into_owned())
        .unwrap_or(spaced)
}

/// `HEB.11.1`: a chapter and a verse.
fn parse_verse_place(value: &str) -> Option<(String, u16, u16)> {
    let (chapter, verse) = value.rsplit_once('.')?;
    let (book, chapter) = parse_passage(chapter)?;
    let verse = verse.parse().ok().filter(|verse| *verse > 0)?;
    Some((book, chapter, verse))
}

/// `slk-seb,eng-kjv`.
fn parse_translations(value: &str) -> Vec<String> {
    value
        .split(',')
        .filter(|code| !code.is_empty())
        .map(ToString::to_string)
        .collect()
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

/// The cross-references shown before "Ďalšie (N)" (spec §6.7).
pub const FIRST_CROSSREFS: usize = 10;

/// What "Ďalšie (N)" still holds.
pub fn hidden_crossrefs(total: usize, shown: usize) -> usize {
    total.saturating_sub(shown)
}

/// The verses a cross-reference selects in the reader. A range leaving its
/// chapter selects its first verse: `v=` stays inside one chapter.
pub fn crossref_target(chapter: u16, verse: u16, chapter_end: u16, verse_end: u16) -> (u16, u16) {
    if chapter == chapter_end {
        (verse, verse_end.max(verse))
    } else {
        (verse, verse)
    }
}

/// `Porovnanie: Hebrejom 11:1`.
pub fn compare_label(book: &str, chapter: u16, verse: u16) -> String {
    format!("Porovnanie: {book} {chapter}:{verse}")
}

/// `Hľadanie: viera · SEB`.
pub fn search_label(query: &str, translation: &str) -> String {
    format!(
        "Hľadanie: {query} · {}",
        translation_short_code(translation)
    )
}

/// `slk-seb:viera`, or `slk-seb|HEB:viera` with a scope: a query can hold any
/// character, so the scope stands before it.
pub fn search_marker(translation: &str, query: &str, scope: &str) -> String {
    if scope == SCOPE_ALL {
        format!("{translation}:{query}")
    } else {
        format!("{translation}|{scope}:{query}")
    }
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

/// A hit's text cut at the words the query matched. The server searches word
/// prefixes without diacritics, and so does this; `true` = marked.
pub fn mark_matches(text: &str, query: &str) -> Vec<(String, bool)> {
    if let Some(words) = phrase_words(query) {
        return mark_phrase(text, &words);
    }
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

/// The folded words of a fully quoted query, the server's exact phrase.
fn phrase_words(query: &str) -> Option<Vec<String>> {
    let quote = |c: char| matches!(c, '"' | '„' | '“' | '”');
    let inner = query.trim().strip_prefix(quote)?.strip_suffix(quote)?;
    if inner.contains(quote) {
        return None;
    }
    // Combining marks count for nothing, as in the server's phrase.
    let inner: String = inner
        .chars()
        .filter(|c| !('\u{300}'..='\u{36F}').contains(c))
        .collect();
    let words: Vec<String> = inner
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(fold)
        .collect();
    (!words.is_empty()).then_some(words)
}

/// Each run of the phrase's words is one mark, the gaps inside it included.
fn mark_phrase(text: &str, phrase: &[String]) -> Vec<(String, bool)> {
    let mut words: Vec<(usize, usize)> = Vec::new();
    let mut start = None;
    for (index, c) in text.char_indices().chain([(text.len(), ' ')]) {
        match (c.is_alphanumeric(), start) {
            (true, None) => start = Some(index),
            (false, Some(from)) => {
                words.push((from, index));
                start = None;
            }
            _ => {}
        }
    }
    let folded: Vec<String> = words.iter().map(|&(a, b)| fold(&text[a..b])).collect();
    let mut parts = Vec::new();
    let mut done = 0;
    let mut at = 0;
    while at + phrase.len() <= folded.len() {
        if folded[at..at + phrase.len()] != *phrase {
            at += 1;
            continue;
        }
        let (from, to) = (words[at].0, words[at + phrase.len() - 1].1);
        if from > done {
            parts.push((text[done..from].to_string(), false));
        }
        parts.push((text[from..to].to_string(), true));
        done = to;
        at += phrase.len();
    }
    if done < text.len() {
        parts.push((text[done..].to_string(), false));
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
    /// The segments of a verse with anything more than text (spec §5.2). `text`
    /// stays what a copy or a sermon takes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub markup: Option<VerseMarkup>,
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
            "view=inde",
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
    fn the_verse_sheet_needs_verses() {
        let route = StudyRoute::parse("p=HEB.11&s=akcie").expect("a route");
        assert_eq!(route.sheet, None);
        let other = StudyRoute::parse("p=HEB.11&s=pismo").expect("a route");
        assert_eq!(other.sheet, Some(StudySheet::TextSize));
    }

    #[test]
    fn the_translation_list_has_its_own_url() {
        let route = StudyRoute::parse("t=slk-seb&p=HEB.11&s=preklady").expect("a route");
        assert_eq!(route.sheet, Some(StudySheet::Translations));
        assert!(route.to_url().contains("s=preklady"), "{}", route.to_url());
        // A slot's sheet still parses as the slot.
        let slot = StudyRoute::parse("t=slk-seb&p=HEB.11&s=preklad2").expect("a route");
        assert_eq!(slot.sheet, Some(StudySheet::Favourite(2)));
    }

    #[test]
    fn a_word_url_round_trips_over_the_reader_and_compare() {
        for url in [
            "/ui/study?t=eng-kjv&p=HEB.11&w=G4102&wv=1.3",
            "/ui/study?view=compare&t=slk-seb&p=HEB.11.1&w=G4102&wv=1.3",
        ] {
            let route = StudyRoute::parse(url.split_once('?').expect("query").1).expect(url);
            assert_eq!(route.sheet, Some(StudySheet::Word), "{url}");
            assert_eq!(route.to_url(), url);
        }
    }

    #[test]
    fn a_word_needs_both_parameters_and_a_text_under_it() {
        for query in [
            "t=eng-kjv&p=HEB.11&w=G4102",
            "t=eng-kjv&p=HEB.11&wv=1.3",
            "t=eng-kjv&p=HEB.11&w=nie&wv=1.3",
            "view=search&q=viera&w=G4102&wv=1.3",
        ] {
            let route = StudyRoute::parse(query).expect(query);
            assert_eq!((route.sheet, route.word), (None, None), "{query}");
        }
    }

    #[test]
    fn closing_the_word_sheet_forgets_the_word() {
        let route = StudyRoute::reader("eng-kjv", "HEB", 11);
        let word = WordPlace::parse("G4102", "1.3").expect("a word");
        let open = route.clone().with_word(word);
        assert_eq!(open.without_sheet(), route);
    }

    #[test]
    fn a_sheet_change_is_no_move_and_another_verse_or_chapter_is() {
        let route = StudyRoute::reader("eng-kjv", "HEB", 11);
        let word = |number: &str, place: &str| {
            route
                .clone()
                .with_word(WordPlace::parse(number, place).expect("a word"))
        };
        assert!(word("G4102", "1.3").only_sheet_differs(&route));
        // A derivation link and the Back from it.
        assert!(word("G3982", "1.3").only_sheet_differs(&word("G4102", "1.3")));
        assert!(!route.only_sheet_differs(&route));
        let verse = |number| {
            route
                .clone()
                .with_verses(Some((number, number)))
                .with_sheet(Some(StudySheet::Actions))
        };
        assert!(!verse(1).only_sheet_differs(&verse(2)));
        assert!(!word("G4102", "1.3").only_sheet_differs(&StudyRoute::reader("eng-kjv", "HEB", 12)));
    }

    #[test]
    fn a_strongs_url_round_trips_and_a_bad_number_names_no_view() {
        let route = StudyRoute::strongs(Some("slk-seb".to_string()), "G4102", "nt");
        let url = route.to_url();
        assert_eq!(url, "/ui/study?view=strongs&t=slk-seb&n=G4102&scope=nt");
        assert_eq!(
            StudyRoute::parse(url.split_once('?').expect("query").1),
            Some(route)
        );
        assert_eq!(StudyRoute::parse("view=strongs"), None);
        assert_eq!(StudyRoute::parse("view=strongs&n=nie"), None);
    }

    #[test]
    fn a_strongs_url_normalizes_its_number_and_scope() {
        let route = StudyRoute::parse("view=strongs&n=g0026&scope=heb").expect("a route");
        assert_eq!(
            route.view,
            StudyView::Strongs {
                number: "G26".to_string(),
                scope: "HEB".to_string()
            }
        );
        let bare = StudyRoute::parse("view=strongs&n=G26&scope=nie").expect("a route");
        assert!(bare.to_url().ends_with("scope=all"));
    }

    #[test]
    fn the_sermon_stays_beside_the_strongs_view() {
        let reader = StudyRoute::parse("t=eng-kjv&p=HEB.11&pane=abc").expect("a route");
        let view =
            StudyRoute::strongs(reader.translation.clone(), "G26", "all").keeping_pane(&reader);
        assert_eq!(view.pane.as_deref(), Some("abc"));
        assert!(view.to_url().contains("pane=abc"));
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
    fn g26_opens_the_number_and_keeps_the_resolvers_reading_as_a_row() {
        let found = suggest("G26", &structure("seb"), "slk-seb");
        assert_eq!(found.enter(), Some(FieldEnter::Strongs("G26".to_string())));
        let target = found.enter_target().expect("Genezis 26");
        assert_eq!((target.book.as_str(), target.chapter), ("GEN", 26));
    }

    #[test]
    fn g_26_with_a_space_stays_the_passage_and_offers_the_number() {
        let found = suggest("g 26", &structure("seb"), "slk-seb");
        let offer = found.strongs.clone().expect("the alternative");
        assert_eq!((offer.number.as_str(), offer.first), ("G26", false));
        assert!(matches!(found.enter(), Some(FieldEnter::Open(target)) if target.book == "GEN"));
    }

    #[test]
    fn a_number_needs_no_structure_and_words_still_search_on_enter() {
        let found = suggest_unresolved("G4102");
        assert_eq!(
            found.enter(),
            Some(FieldEnter::Strongs("G4102".to_string()))
        );
        assert_eq!(
            suggest("viera", &structure("seb"), "slk-seb").enter(),
            Some(FieldEnter::Search("viera".to_string()))
        );
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
    fn a_word_that_names_a_book_can_still_search_the_text() {
        let seb = structure("seb");
        for word in ["Mojžiš", "kráľ", "Izaiáš prorok"] {
            let found = suggest(word, &seb, "slk-seb");
            assert_eq!(found.text_search.as_deref(), Some(word), "{word}");
        }
        // Enter stays the book's first chapter.
        let book = suggest("Izaiáš", &seb, "slk-seb");
        assert_eq!(book.text_search.as_deref(), Some("Izaiáš"));
        assert_eq!(book.enter_target().expect("a target").book, "ISA");
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
    fn a_quoted_query_marks_the_whole_phrase_and_nothing_else() {
        let parts = mark_matches(
            "Viera, nádej. Viera je nádej, nádej viera",
            "„viera, je nádej“",
        );
        assert_eq!(
            parts,
            vec![
                ("Viera, nádej. ".to_string(), false),
                ("Viera je nádej".to_string(), true),
                (", nádej viera".to_string(), false),
            ]
        );
        let none = mark_matches("Viera je", "\"nádej\"");
        assert_eq!(none, vec![("Viera je".to_string(), false)]);
        let two = mark_matches("a b a b", "\"a b\"");
        assert_eq!(two.iter().filter(|(_, marked)| *marked).count(), 2);
    }

    #[test]
    fn a_decomposed_query_marks_what_the_server_found() {
        let parts = mark_matches("Viera a nádej", "\"na\u{301}dej\"");
        assert_eq!(parts[1], ("nádej".to_string(), true));
    }

    #[test]
    fn a_query_holding_a_bar_is_not_taken_for_a_scope() {
        assert_eq!(search_marker("slk-seb", "viera", "all"), "slk-seb:viera");
        assert_eq!(
            search_marker("slk-seb", "viera", "HEB"),
            "slk-seb|HEB:viera"
        );
        assert_ne!(
            search_marker("slk-seb", "a|HEB", "all"),
            search_marker("slk-seb", "a", "HEB")
        );
    }

    #[test]
    fn a_search_url_keeps_its_scope_and_all_stays_out_of_it() {
        let mut route = StudyRoute::search("slk-seb", "viera");
        assert!(!route.to_url().contains("scope"));
        if let StudyView::Search { scope, .. } = &mut route.view {
            *scope = "HEB".to_string();
        }
        let url = route.to_url();
        assert!(url.ends_with("q=viera&scope=HEB"), "{url}");
        let parsed = StudyRoute::parse(url.split_once('?').expect("a query").1).expect("a route");
        assert_eq!(parsed, route);
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
    fn the_sermon_views_round_trip() {
        let list = StudyRoute::sermons(Some("slk-seb".to_string()));
        assert_eq!(list.to_url(), "/ui/study?view=sermons&t=slk-seb");
        assert_eq!(StudyRoute::parse("view=sermons&t=slk-seb"), Some(list));
        let editor = StudyRoute::sermon(None, "abc-1");
        assert_eq!(editor.to_url(), "/ui/study?view=sermon&id=abc-1");
        assert_eq!(StudyRoute::parse("view=sermon&id=abc-1"), Some(editor));
    }

    #[test]
    fn a_sermon_view_needs_its_id() {
        assert_eq!(StudyRoute::parse("view=sermon"), None);
        assert_eq!(StudyRoute::parse("view=sermon&id="), None);
    }

    #[test]
    fn the_add_sheet_never_comes_from_a_url() {
        let route = StudyRoute::reader("slk-seb", "HEB", 11)
            .with_verses(Some((1, 1)))
            .with_sheet(Some(StudySheet::AddToSermon));
        let parsed = StudyRoute::parse(route.to_url().split_once('?').expect("query").1);
        assert_eq!(parsed.expect("a route").sheet, None);
    }

    #[test]
    fn the_settle_key_names_translation_book_and_chapter() {
        assert_eq!(chapter_key("slk-seb", "HEB", 11), "slk-seb:HEB.11");
    }

    #[test]
    fn a_pane_survives_the_url_beside_text_only() {
        let mut route = StudyRoute::reader("slk-seb", "HEB", 11);
        route.pane = Some("abc-1".to_string());
        let url = route.to_url();
        assert!(url.contains("pane=abc-1"), "{url}");
        assert_eq!(
            StudyRoute::parse(url.split_once('?').unwrap().1),
            Some(route)
        );
        let list = StudyRoute::parse("view=sermons&pane=abc-1").unwrap();
        assert_eq!(list.pane, None);
    }

    #[test]
    fn a_move_in_the_text_keeps_the_sermon_and_a_move_to_the_list_leaves_it() {
        let mut current = StudyRoute::reader("slk-seb", "HEB", 11);
        current.pane = Some("abc-1".to_string());
        let next = StudyRoute::reader("slk-seb", "HEB", 12).keeping_pane(&current);
        assert_eq!(next.open_sermon(), Some("abc-1"));
        let list = StudyRoute::sermons(None).keeping_pane(&current);
        assert_eq!(list.open_sermon(), None);
    }

    #[test]
    fn a_wide_screen_opens_the_sermon_beside_the_last_place() {
        let mut sermon = StudyRoute::sermon(Some("slk-seb".to_string()), "abc-1");
        sermon.sheet = Some(StudySheet::Reorder);
        let last = StudyRoute::reader("eng-kjv", "JHN", 3).with_verses(Some((16, 16)));
        let wide = sermon.for_width(true, last).unwrap();
        assert_eq!(wide.view, StudyRoute::reader("x", "JHN", 3).view);
        assert_eq!(wide.translation.as_deref(), Some("slk-seb"));
        assert_eq!(wide.verses, None);
        assert_eq!(wide.sheet, Some(StudySheet::Reorder));
        assert_eq!(wide.pane.as_deref(), Some("abc-1"));
    }

    #[test]
    fn a_narrow_screen_shows_the_sermon_alone_and_drops_a_verse_sheet() {
        let mut beside = StudyRoute::reader("slk-seb", "HEB", 11).with_verses(Some((1, 1)));
        beside.pane = Some("abc-1".to_string());
        beside.sheet = Some(StudySheet::Actions);
        let reading = StudyRoute::reader("slk-seb", "JHN", 3);
        let narrow = beside.for_width(false, reading.clone()).unwrap();
        assert_eq!(
            narrow.view,
            StudyView::Sermon {
                id: "abc-1".to_string()
            }
        );
        assert_eq!(narrow.sheet, None);
        assert_eq!(narrow.for_width(false, reading.clone()), None);
        assert_eq!(beside.for_width(true, reading), None);
    }

    #[test]
    fn a_compare_url_round_trips_with_its_verse_and_translations() {
        let route = StudyRoute::compare(
            Some("slk-seb".to_string()),
            ("HEB", 11, 1),
            vec!["slk-seb".to_string(), "eng-kjv".to_string()],
        );
        let url = route.to_url();
        assert_eq!(
            url,
            "/ui/study?view=compare&t=slk-seb&p=HEB.11.1&tt=slk-seb,eng-kjv"
        );
        assert_eq!(
            StudyRoute::parse(url.split_once('?').unwrap().1),
            Some(route)
        );
    }

    #[test]
    fn a_compare_url_without_a_list_means_the_favourites() {
        let route = StudyRoute::parse("view=compare&p=HEB.11.1").expect("a route");
        assert_eq!(
            route.view,
            StudyView::Compare {
                book: "HEB".to_string(),
                chapter: 11,
                verse: 1,
                translations: Vec::new(),
            }
        );
    }

    #[test]
    fn a_compare_url_needs_a_verse() {
        assert_eq!(StudyRoute::parse("view=compare&p=HEB.11"), None);
        assert_eq!(StudyRoute::parse("view=compare&p=HEB.11.0"), None);
    }

    #[test]
    fn the_cross_reference_sheet_has_a_url_and_needs_a_verse() {
        let route = StudyRoute::reader("slk-seb", "HEB", 11)
            .with_verses(Some((1, 1)))
            .with_sheet(Some(StudySheet::CrossRefs));
        let url = route.to_url();
        assert_eq!(
            StudyRoute::parse(url.split_once('?').unwrap().1),
            Some(route)
        );
        let bare = StudyRoute::parse("t=slk-seb&p=HEB.11&s=odkazy").expect("a route");
        assert_eq!(bare.sheet, None);
    }

    #[test]
    fn more_cross_references_count_what_is_left() {
        assert_eq!(hidden_crossrefs(23, FIRST_CROSSREFS), 13);
        assert_eq!(hidden_crossrefs(4, FIRST_CROSSREFS), 0);
        assert_eq!(hidden_crossrefs(23, 23), 0);
    }

    #[test]
    fn a_cross_reference_selects_its_verses_inside_one_chapter() {
        assert_eq!(crossref_target(4, 9, 4, 10), (9, 10));
        assert_eq!(crossref_target(4, 9, 5, 2), (9, 9));
    }

    #[test]
    fn compare_keeps_the_sermon_for_the_way_back_without_showing_it() {
        let reader = StudyRoute::parse("t=slk-seb&p=HEB.11&pane=abc").expect("a route");
        let compare = StudyRoute::compare(Some("slk-seb".to_string()), ("HEB", 11, 1), Vec::new())
            .keeping_pane(&reader);
        assert_eq!(compare.pane.as_deref(), Some("abc"));
        assert!(!compare.shows_pane());
        assert_eq!(
            compare.for_width(false, reader.clone()),
            None,
            "a rotation stays"
        );
        let back = StudyRoute::reader("slk-seb", "HEB", 11).keeping_pane(&compare);
        assert_eq!(back.pane.as_deref(), Some("abc"));
        assert!(back.shows_pane());
    }
}
