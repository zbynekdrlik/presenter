//! The verse console's pure half (#850 B2a, spec §7): what a resolver
//! [`Resolution`] shows as rows and chips, and the requests the console sends.
//! No `web_sys` here — host-tested with `cargo test --lib`.

use presenter_core::bible::resolver::{
    segment_label, BookRef, BookStructure, Passage, Resolution, Segment, Versification,
};
use presenter_core::bible::sermon::translation_short_code;
use presenter_core::BibleSlideOutput;

use crate::api::bible::{BibleBookDto, BibleSlideDto, ResolveRequest, TriggerSlideRequest};

/// Chapters beyond this in a `/bible/books` row are malformed and skipped.
const MAX_CHAPTER: u16 = 200;

/// One passage the console can preview: a book of the MAIN translation and
/// one contiguous segment of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewTarget {
    pub book: BookRef,
    pub segment: Segment,
    /// `Hebrejom 11:1`.
    pub label: String,
}

/// What a tap on a row or a chip does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowTarget {
    /// Preview this passage (it never goes to the wall by itself).
    Preview(PreviewTarget),
    /// Rewrite the field: a fix for a reference that does not exist, or a
    /// book to complete with a chapter.
    Complete(String),
}

/// A result row or an alternative chip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleRow {
    /// Unique within one view; the keyed `<For>` key. It carries the label
    /// and detail too: a translation switch renames a row whose passage stays
    /// the same, and a keyed `<For>` never re-renders an unchanged key.
    pub key: String,
    /// Rows: `passage` | `ambiguous` | `book_only`. Chips: `alternative` |
    /// `fix` | `book`.
    pub kind: &'static str,
    pub label: String,
    /// `Žid · SEB` (the form that matched · the translation).
    pub detail: String,
    /// The first row: what the console previews when nothing was tapped.
    pub default: bool,
    pub target: RowTarget,
}

/// Everything the console shows under its field for one input.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConsoleView {
    pub rows: Vec<ConsoleRow>,
    pub chips: Vec<ConsoleRow>,
    pub hint: Option<String>,
}

impl ConsoleView {
    /// The passage previewed when the operator tapped nothing: the first row's.
    pub fn default_target(&self) -> Option<&PreviewTarget> {
        self.rows.iter().find_map(|row| match &row.target {
            RowTarget::Preview(target) => Some(target),
            RowTarget::Complete(_) => None,
        })
    }
}

/// Rows, chips and the hint for `resolution`, resolved against the main
/// translation `translation` (its code, `slk-seb`).
pub fn console_view(resolution: &Resolution, translation: &str) -> ConsoleView {
    let short = translation_short_code(translation);
    match resolution {
        Resolution::Passage {
            passage,
            alternatives,
        } => ConsoleView {
            rows: passage_rows(passage, "passage", &short),
            chips: alternatives.iter().filter_map(alternative_chip).collect(),
            hint: None,
        },
        Resolution::Ambiguous {
            default,
            alternatives,
        } => ConsoleView {
            rows: passage_rows(default, "ambiguous", &short),
            chips: alternatives.iter().filter_map(alternative_chip).collect(),
            hint: None,
        },
        Resolution::OutOfRange { hint, fix, .. } => ConsoleView {
            rows: Vec::new(),
            chips: fix
                .iter()
                .map(|fix| complete_row("fix", passage_text(fix), false))
                .collect(),
            hint: Some(hint.clone()),
        },
        Resolution::BookOnly { book, alternatives } => {
            let mut row = complete_row("book_only", format!("{} ", book.name), true);
            row.detail = "dopíš kapitolu".to_string();
            ConsoleView {
                rows: vec![row],
                chips: alternatives.iter().map(book_chip).collect(),
                hint: None,
            }
        }
        Resolution::Incomplete { books, .. } => ConsoleView {
            rows: Vec::new(),
            chips: books.iter().map(book_chip).collect(),
            hint: None,
        },
        Resolution::NoBook { hint, .. } => ConsoleView {
            rows: Vec::new(),
            chips: Vec::new(),
            hint: hint.clone(),
        },
    }
}

fn passage_rows(passage: &Passage, kind: &'static str, short: &str) -> Vec<ConsoleRow> {
    let detail = if kind == "ambiguous" {
        format!("predvolené · {} · {short}", passage.matched_form)
    } else {
        format!("{} · {short}", passage.matched_form)
    };
    passage
        .segments
        .iter()
        .enumerate()
        .map(|(index, segment)| {
            let target = target_of(&passage.book, *segment);
            ConsoleRow {
                key: format!("{}|{}|{detail}", target_key(kind, &target), target.label),
                kind,
                label: target.label.clone(),
                detail: detail.clone(),
                default: index == 0,
                target: RowTarget::Preview(target),
            }
        })
        .collect()
}

fn alternative_chip(passage: &Passage) -> Option<ConsoleRow> {
    let target = target_of(&passage.book, *passage.segments.first()?);
    Some(ConsoleRow {
        key: format!(
            "{}|{}",
            target_key("alternative", &target),
            passage.book.name
        ),
        kind: "alternative",
        label: passage.book.name.clone(),
        detail: String::new(),
        default: false,
        target: RowTarget::Preview(target),
    })
}

fn book_chip(book: &BookRef) -> ConsoleRow {
    complete_row("book", format!("{} ", book.name), false)
}

fn complete_row(kind: &'static str, text: String, default: bool) -> ConsoleRow {
    ConsoleRow {
        key: format!("{kind}:{text}"),
        kind,
        label: text.trim_end().to_string(),
        detail: String::new(),
        default,
        target: RowTarget::Complete(text),
    }
}

fn target_of(book: &BookRef, segment: Segment) -> PreviewTarget {
    PreviewTarget {
        book: book.clone(),
        label: segment_label(&book.name, &segment),
        segment,
    }
}

fn target_key(kind: &str, target: &PreviewTarget) -> String {
    let segment = &target.segment;
    format!(
        "{kind}:{}:{}:{}-{}",
        target.book.code,
        segment.chapter,
        segment.verse_start.unwrap_or(0),
        segment.verse_end.unwrap_or(0)
    )
}

/// A passage written back as field text that resolves to it again
/// (`Hebrejom 13`, `Hebrejom 11:40`, `Genezis 50:1-26`, cross-chapter `1:1-2:3`).
fn passage_text(passage: &Passage) -> String {
    let name = &passage.book.name;
    match (passage.segments.first(), passage.segments.last()) {
        (Some(first), Some(last)) if first.chapter != last.chapter => format!(
            "{name} {}:{}-{}:{}",
            first.chapter,
            first.verse_start.unwrap_or(1),
            last.chapter,
            last.verse_end.unwrap_or(1)
        ),
        (Some(first), _) => segment_label(name, first),
        _ => name.clone(),
    }
}

/// The main translation's structure from its `/bible/books` answer; each
/// verse count goes to its chapter NUMBER (the list may skip or reorder).
pub fn versification(books: &[BibleBookDto]) -> Versification {
    Versification::new(
        books
            .iter()
            .filter(|book| !book.code.is_empty())
            .map(|book| {
                let chapters = book
                    .chapters
                    .iter()
                    .filter(|chapter| (1..=MAX_CHAPTER).contains(&chapter.number));
                let last = chapters.clone().map(|c| c.number).max().unwrap_or(0);
                let mut verse_counts = vec![0; usize::from(last)];
                for chapter in chapters {
                    if let Some(slot) = verse_counts.get_mut(usize::from(chapter.number) - 1) {
                        *slot = chapter.verse_count;
                    }
                }
                BookStructure {
                    code: book.code.clone(),
                    number: book.number,
                    name: book.book.clone(),
                    verse_counts,
                }
            })
            .collect(),
    )
}

/// The `/bible/resolve` request for a preview (#702: a single verse sends
/// `verseEnd = verseStart`; a whole chapter sends verse 1 and no end, which
/// that endpoint reads as "to the end of the chapter").
pub fn resolve_request(
    target: &PreviewTarget,
    main: &str,
    secondary: Option<&str>,
    character_limit: u32,
) -> ResolveRequest {
    let (verse_start, verse_end) = match target.segment.verse_start {
        Some(start) => (
            start,
            Some(target.segment.verse_end.unwrap_or(start).max(start)),
        ),
        None => (1, None),
    };
    ResolveRequest {
        main_translation: main.to_string(),
        secondary_translation: secondary
            .filter(|code| !code.is_empty())
            .map(str::to_string),
        book: target.book.name.clone(),
        book_code: Some(target.book.code.clone()),
        chapter: target.segment.chapter,
        verse_start,
        verse_end,
        character_limit: Some(character_limit),
    }
}

/// Identifies what a preview shows: the passage, both translations and the
/// character limit. A preview answer is used only while this still matches.
pub fn preview_key(
    target: &PreviewTarget,
    main: &str,
    secondary: Option<&str>,
    character_limit: u32,
) -> String {
    format!(
        "{}|{main}|{}|{character_limit}",
        target_key("preview", target),
        secondary.unwrap_or("")
    )
}

/// The `/bible/trigger-slide` request for one previewed slide: exactly the
/// text and labels the preview card shows.
pub fn trigger_request(slide: &BibleSlideDto, main: &str) -> TriggerSlideRequest {
    let meta = slide.metadata.as_ref().and_then(|m| m.bible.as_ref());
    let non_empty = |text: &str| (!text.is_empty()).then(|| text.to_string());
    TriggerSlideRequest {
        main_text: slide.bible_main.clone(),
        main_reference: slide.bible_main_reference.clone(),
        secondary_text: non_empty(&slide.bible_translation),
        secondary_reference: non_empty(&slide.bible_translation_reference),
        translation_code: meta
            .and_then(|m| m.translation_code.clone())
            .or_else(|| non_empty(main)),
        book: meta.and_then(|m| m.book.clone()),
        book_code: meta.and_then(|m| m.book_code.clone()),
        book_number: meta.and_then(|m| m.book_number),
        chapter: meta.and_then(|m| m.chapter),
        verse_start: meta.and_then(|m| m.effective_verse_start()),
        verse_end: meta.and_then(|m| m.effective_verse_end()),
    }
}

/// The on-air strip: the reference without its "(SEB)" and the translations
/// on the wall ("SEB + NLT").
pub fn on_air_parts(output: &BibleSlideOutput) -> (String, String) {
    let (reference, main) = split_code(&output.main_reference);
    let (_, secondary) = split_code(&output.secondary_reference);
    let codes: Vec<String> = [main, secondary].into_iter().flatten().collect();
    (reference, codes.join(" + "))
}

fn split_code(label: &str) -> (String, Option<String>) {
    let label = label.trim();
    if let (Some(open), true) = (label.rfind(" ("), label.ends_with(')')) {
        let code = &label[open + 2..label.len() - 1];
        if !code.is_empty() && code.chars().count() <= 8 {
            return (label[..open].to_string(), Some(code.to_string()));
        }
    }
    (label.to_string(), None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use presenter_core::bible::resolver::lookup;

    fn seb() -> Versification {
        let books: Vec<BibleBookDto> = serde_json::from_str(include_str!(
            "../../../presenter-core/src/bible/resolver/testdata/structure-slk-seb.json"
        ))
        .expect("the SEB structure fixture is the /bible/books shape");
        versification(&books)
    }

    fn view(input: &str) -> ConsoleView {
        console_view(&lookup(input, &seb()), "slk-seb")
    }

    fn preview(row: &ConsoleRow) -> &PreviewTarget {
        match &row.target {
            RowTarget::Preview(target) => target,
            RowTarget::Complete(text) => panic!("expected a preview, got Complete({text:?})"),
        }
    }

    #[test]
    fn zid_11_1_is_one_hebrejom_row_named_by_its_matched_form_and_translation() {
        let view = view("zid 11 1");
        assert_eq!(view.rows.len(), 1, "{view:?}");
        let row = &view.rows[0];
        assert_eq!(
            (row.kind, row.label.as_str(), row.default),
            ("passage", "Hebrejom 11:1", true)
        );
        assert!(
            row.detail.ends_with(" · SEB") && !row.detail.starts_with(" ·"),
            "{}",
            row.detail
        );
        let target = view.default_target().expect("a preview target");
        assert_eq!(target.book.code, "HEB");
        assert_eq!(target.segment, Segment::verses(11, 1, 1));
        assert!(view.chips.is_empty() && view.hint.is_none());
    }

    #[test]
    fn jan_3_16_offers_1_jan_as_a_chip_that_previews_it() {
        let view = view("jan 3 16");
        assert_eq!(view.rows[0].label, "Ján 3:16");
        assert_eq!(view.chips.len(), 1, "{:?}", view.chips);
        let chip = &view.chips[0];
        assert_eq!((chip.kind, chip.label.as_str()), ("alternative", "1 Ján"));
        assert_eq!(preview(chip).label, "1 Ján 3:16");
        assert_ne!(chip.key, view.rows[0].key);
    }

    #[test]
    fn kor_13_is_marked_as_a_guess_with_2_korintanom_as_the_other_choice() {
        let view = view("kor 13");
        let row = &view.rows[0];
        assert_eq!(
            (row.kind, row.label.as_str()),
            ("ambiguous", "1 Korinťanom 13")
        );
        assert!(row.detail.starts_with("predvolené · "), "{}", row.detail);
        assert_eq!(view.chips[0].label, "2 Korinťanom");
    }

    #[test]
    fn zid_14_shows_the_hint_and_a_fix_chip_that_rewrites_the_field() {
        let view = view("zid 14");
        assert!(view.rows.is_empty());
        assert_eq!(view.hint.as_deref(), Some("Kniha má len 13 kapitol"));
        assert_eq!(view.chips.len(), 1);
        assert_eq!(view.chips[0].kind, "fix");
        assert_eq!(
            view.chips[0].target,
            RowTarget::Complete("Hebrejom 13".to_string())
        );
        assert!(view.default_target().is_none());
    }

    #[test]
    fn a_verse_past_the_chapter_end_offers_the_last_verse() {
        let view = view("zid 11 41");
        assert_eq!(view.hint.as_deref(), Some("Kapitola má len 40 veršov"));
        assert_eq!(
            view.chips[0].target,
            RowTarget::Complete("Hebrejom 11:40".to_string())
        );
    }

    #[test]
    fn a_verse_list_is_one_row_per_verse() {
        let view = view("mt 5:3.5.7");
        let labels: Vec<&str> = view.rows.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(labels, ["Matúš 5:3", "Matúš 5:5", "Matúš 5:7"]);
        let defaults: Vec<bool> = view.rows.iter().map(|row| row.default).collect();
        assert_eq!(defaults, [true, false, false]);
        let mut keys: Vec<&str> = view.rows.iter().map(|row| row.key.as_str()).collect();
        keys.dedup();
        assert_eq!(keys.len(), 3);
    }

    #[test]
    fn a_book_alone_completes_the_field_and_offers_the_other_books() {
        let view = view("jan");
        assert_eq!(view.rows.len(), 1);
        assert_eq!(view.rows[0].kind, "book_only");
        assert_eq!(view.rows[0].label, "Ján");
        assert_eq!(view.rows[0].target, RowTarget::Complete("Ján ".to_string()));
        assert!(view
            .chips
            .iter()
            .any(|chip| chip.target == RowTarget::Complete("1 Ján ".to_string())));
        assert!(view.default_target().is_none());
    }

    #[test]
    fn a_bare_ordinal_lists_the_numbered_books_to_complete() {
        let view = view("1");
        assert!(view.rows.is_empty());
        assert!(
            view.chips.iter().any(|chip| chip.label == "1 Ján"),
            "{:?}",
            view.chips
        );
        assert!(view.chips.iter().all(|chip| chip.kind == "book"));
    }

    #[test]
    fn numbers_without_a_book_ask_for_one_and_words_alone_show_nothing() {
        let numbers = view("11:1");
        assert_eq!(numbers.hint.as_deref(), Some("Napíš aj knihu"));
        assert!(numbers.rows.is_empty() && numbers.chips.is_empty());
        assert_eq!(view(""), ConsoleView::default());
    }

    #[test]
    fn a_single_verse_requests_itself_and_a_chapter_requests_from_verse_1() {
        let verse = view("zid 11 1");
        let request = resolve_request(verse.default_target().unwrap(), "slk-seb", Some(""), 320);
        assert_eq!(
            (request.chapter, request.verse_start, request.verse_end),
            (11, 1, Some(1))
        );
        assert_eq!(request.book_code.as_deref(), Some("HEB"));
        assert_eq!(request.secondary_translation, None);
        assert_eq!(request.character_limit, Some(320));

        let chapter = view("zid 13");
        let request = resolve_request(chapter.default_target().unwrap(), "slk-seb", None, 320);
        assert_eq!((request.verse_start, request.verse_end), (1, None));

        let range = view("mt 5,3-12");
        let request = resolve_request(
            range.default_target().unwrap(),
            "slk-seb",
            Some("eng-kjv"),
            320,
        );
        assert_eq!((request.verse_start, request.verse_end), (3, Some(12)));
        assert_eq!(request.secondary_translation.as_deref(), Some("eng-kjv"));
    }

    #[test]
    fn the_preview_key_changes_with_the_passage_the_translations_and_the_limit() {
        let first = view("zid 11 1");
        let target = first.default_target().unwrap();
        let key = preview_key(target, "slk-seb", None, 320);
        assert_eq!(key, preview_key(target, "slk-seb", None, 320));
        assert_ne!(key, preview_key(target, "slk-roh", None, 320));
        assert_ne!(key, preview_key(target, "slk-seb", Some("eng-kjv"), 320));
        assert_ne!(key, preview_key(target, "slk-seb", None, 200));
        let second = view("zid 11 2");
        assert_ne!(
            key,
            preview_key(second.default_target().unwrap(), "slk-seb", None, 320)
        );
    }

    #[test]
    fn the_trigger_request_sends_the_slide_exactly_as_previewed() {
        let slide: BibleSlideDto = serde_json::from_value(serde_json::json!({
            "id": "s1", "order": 0,
            "bibleMain": "1 Viera je podstatou toho, v čo dúfame",
            "bibleTranslation": "",
            "bibleMainReference": "Hebrejom 11:1 (SEB)",
            "bibleTranslationReference": "",
            "metadata": { "bible": {
                "translationCode": "slk-seb", "book": "Hebrejom", "bookCode": "HEB",
                "bookNumber": 58, "chapter": 11, "verses": [{ "start": 1, "end": 1 }]
            } }
        }))
        .expect("a resolve slide");
        let request = trigger_request(&slide, "slk-seb");
        assert_eq!(request.main_reference, "Hebrejom 11:1 (SEB)");
        assert_eq!(request.main_text, "1 Viera je podstatou toho, v čo dúfame");
        assert_eq!(
            (request.secondary_text, request.secondary_reference),
            (None, None)
        );
        assert_eq!(request.book_code.as_deref(), Some("HEB"));
        assert_eq!(
            (request.chapter, request.verse_start, request.verse_end),
            (Some(11), Some(1), Some(1))
        );
        assert_eq!(request.translation_code.as_deref(), Some("slk-seb"));
    }

    #[test]
    fn the_on_air_strip_splits_the_reference_and_names_both_translations() {
        let output = |main: &str, secondary: &str| {
            BibleSlideOutput::new("text", main, "", secondary, chrono::Utc::now())
        };
        assert_eq!(
            on_air_parts(&output("Hebrejom 11:1 (SEB)", "Hebrews 11:1 (NLT)")),
            ("Hebrejom 11:1".to_string(), "SEB + NLT".to_string())
        );
        assert_eq!(
            on_air_parts(&output("Hebrejom 11:1 (SEB)", "")),
            ("Hebrejom 11:1".to_string(), "SEB".to_string())
        );
        assert_eq!(
            on_air_parts(&output("Ján 3:16", "")),
            ("Ján 3:16".to_string(), String::new())
        );
    }

    #[test]
    fn a_translation_switch_renames_the_row_and_changes_its_key() {
        let kjv: Vec<BibleBookDto> = serde_json::from_str(include_str!(
            "../../../presenter-core/src/bible/resolver/testdata/structure-eng-kjv.json"
        ))
        .expect("the KJV structure fixture");
        let in_kjv = console_view(&lookup("heb 11 1", &versification(&kjv)), "eng-kjv");
        let in_seb = view("heb 11 1");
        assert_eq!(in_seb.rows[0].label, "Hebrejom 11:1");
        assert_eq!(in_kjv.rows[0].label, "Hebrews 11:1");
        assert!(in_kjv.rows[0].detail.ends_with(" · KJV"));
        assert_ne!(in_seb.rows[0].key, in_kjv.rows[0].key);
    }

    #[test]
    fn versification_puts_each_count_at_its_chapter_number() {
        let books: Vec<BibleBookDto> = serde_json::from_value(serde_json::json!([
            { "book": "Rút", "code": "RUT", "number": 8,
              "chapters": [{ "number": 2, "verseCount": 23 }, { "number": 1, "verseCount": 22 }] },
            { "book": "Bez kódu", "code": "", "number": 0, "chapters": [] }
        ]))
        .expect("books");
        let structure = versification(&books);
        assert_eq!(structure.books().len(), 1);
        let ruth = structure.book("RUT").expect("Rút");
        assert_eq!(
            (
                ruth.verse_count(1),
                ruth.verse_count(2),
                ruth.verse_count(3)
            ),
            (Some(22), Some(23), None)
        );
    }
}
