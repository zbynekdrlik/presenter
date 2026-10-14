//! Slovak hints and labels (#850 spec §5.6) — the wording the Bible page
//! already shows (`presenter-ui` `state/bible_range.rs`) and the corpus expects.

use super::types::Segment;

/// Numbers typed without a book (`11:1`).
pub const NO_BOOK_HINT: &str = "Napíš aj knihu";
/// The typed name means a book the main translation does not have (a partial
/// translation such as slk-mil).
pub const MISSING_BOOK_HINT: &str = "Táto kniha v hlavnom preklade nie je";

/// `n` with the Slovak noun form it takes: 1 → `one`, 2–4 → `few`, else `many`.
pub fn counted(n: u16, one: &str, few: &str, many: &str) -> String {
    let noun = match n {
        1 => one,
        2..=4 => few,
        _ => many,
    };
    format!("{n} {noun}")
}

/// "Kniha má len 13 kapitol".
pub fn chapters_hint(chapter_count: u16) -> String {
    format!(
        "Kniha má len {}",
        counted(chapter_count, "kapitolu", "kapitoly", "kapitol")
    )
}

/// "Kapitola má len 40 veršov".
pub fn verses_hint(verse_count: u16) -> String {
    format!(
        "Kapitola má len {}",
        counted(verse_count, "verš", "verše", "veršov")
    )
}

/// `Hebrejom 11:1`, `Hebrejom 11:1-3`, `Hebrejom 13` (whole chapter).
pub fn segment_label(book_name: &str, segment: &Segment) -> String {
    let chapter = segment.chapter;
    match (segment.verse_start, segment.verse_end) {
        (Some(start), Some(end)) if end > start => format!("{book_name} {chapter}:{start}-{end}"),
        (Some(start), _) => format!("{book_name} {chapter}:{start}"),
        (None, _) => format!("{book_name} {chapter}"),
    }
}
