//! Reference labels of sermon items (#851 spec §7.3). [`translation_short_code`]
//! is the shared "(SEB)" formatter; new code calls it, never a copy. Three older
//! copies remain today: the live composer's private one in
//! `state/slides/compose.rs` (P2a switches the composer to this function), an
//! inline one in `ai/tools/bible.rs`, and `resolume/handlers.rs`'s own.

/// "eng-kjv" → "KJV", "slk-seb" → "SEB": the part after the last dash, upper
/// case.
pub fn translation_short_code(code: &str) -> String {
    code.rsplit('-').next().unwrap_or(code).to_uppercase()
}

/// One verse of a label: its number and whether the label marks a cut at its
/// start (`b`) or at its end (`a`). [`render_item`](super::render_item) sets a
/// flag only where a label can express it (`b` on the first verse of a run,
/// `a` on the last); a plain label (the secondary line) sets none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerseMark {
    pub verse: u16,
    pub start_cut: bool,
    pub end_cut: bool,
}

impl VerseMark {
    pub const fn plain(verse: u16) -> Self {
        Self {
            verse,
            start_cut: false,
            end_cut: false,
        }
    }
}

/// The chapter-local part of a label: runs of consecutive verses joined by
/// ", " (`"1a"`, `"16b-18a"`, `"5-7, 9"`). `marks` must be in verse order.
pub fn verse_suffix(marks: &[VerseMark]) -> String {
    let mut runs: Vec<(VerseMark, VerseMark)> = Vec::new();
    for mark in marks {
        let extends = runs
            .last()
            .is_some_and(|(_, last)| last.verse.checked_add(1) == Some(mark.verse));
        if !extends {
            runs.push((*mark, *mark));
        } else if let Some((_, last)) = runs.last_mut() {
            *last = *mark;
        }
    }
    runs.into_iter()
        .map(|(first, last)| run_label(first, last))
        .collect::<Vec<_>>()
        .join(", ")
}

fn run_label(first: VerseMark, last: VerseMark) -> String {
    let (start, end) = (first.verse, last.verse);
    if start == end {
        let letter = match (first.start_cut, first.end_cut) {
            (false, true) => "a",
            (true, false) => "b",
            _ => "",
        };
        return format!("{start}{letter}");
    }
    let start_letter = if first.start_cut { "b" } else { "" };
    let end_letter = if last.end_cut { "a" } else { "" };
    format!("{start}{start_letter}-{end}{end_letter}")
}

/// "Hebrejom 11:1a (SEB)": the book name the translation stores, the chapter,
/// the suffix and the translation's short code.
pub fn item_label(book_name: &str, chapter: u16, suffix: &str, translation_code: &str) -> String {
    let short = translation_short_code(translation_code);
    format!("{book_name} {chapter}:{suffix} ({short})")
}
