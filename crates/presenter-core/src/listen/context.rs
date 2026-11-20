//! The Soniox context of the guest's transcript (#852 C3, spec §22.3, plan
//! Decision 12): the measured `general` pair (domain + topic) and the `terms`
//! that steer the recognition of names, in priority order — the 66 Bible
//! books (English long names), today's prepared references, today's šepkár
//! terms, then the curated Christian list. Deduplicated case-insensitively;
//! the whole JSON `context` object is kept within [`CONTEXT_MAX_CHARS`] by
//! dropping terms from the END of that order (curated first, then tip terms).
//! Built once per connection, so a reconnect picks up a tip sent meanwhile.

use std::collections::HashSet;

use crate::bible::resolver::names_table::BOOK_LONG_EN;

/// The measured `general` domain (eval config, spec §22.3).
pub const CONTEXT_DOMAIN: &str = "Christian church sermon";
/// The measured `general` topic (eval config, spec §22.3).
pub const CONTEXT_TOPIC: &str = "preacher reads and cites Bible books, chapters and verses";
/// The JSON `context` object's size limit, in characters. [unverified: Soniox's
/// own limit — the acceptance (C3-10) sends the largest real context once.]
pub const CONTEXT_MAX_CHARS: usize = 8_000;
/// Words a sermon uses that a general English model writes wrong or not at
/// all; the last priority group, dropped first by the cap.
pub const CHRISTIAN_TERMS: &[&str] = &[
    "Holy Spirit",
    "Holy Ghost",
    "Pentecost",
    "righteousness",
    "sanctification",
    "justification",
    "atonement",
    "redemption",
    "salvation",
    "grace",
    "Gethsemane",
    "Golgotha",
    "Calvary",
    "Nazareth",
    "Bethlehem",
    "Galilee",
    "Jerusalem",
    "Zion",
    "Israel",
    "Pharisees",
    "Sadducees",
    "apostle",
    "disciples",
    "epistle",
    "gospel",
    "covenant",
    "repentance",
    "resurrection",
    "crucifixion",
    "Messiah",
    "Christ",
    "Emmanuel",
    "hallelujah",
    "hosanna",
    "Yahweh",
    "Jehovah",
    "Psalm",
    "Abraham",
    "Moses",
    "David",
    "Elijah",
    "Paul",
    "Peter",
    "Mary",
    "Lazarus",
    "Pontius Pilate",
    "baptism",
    "communion",
    "tabernacle",
    "Sabbath",
    "Scripture",
    "prophecy",
    "prophet",
    "intercession",
    "sanctuary",
];

/// The book index of Psalms: one psalm is cited as "Psalm 23".
const PSALMS_INDEX: usize = 18;
/// `{"general":[`, `],"terms":[` and `]}` around the two lists.
const JSON_FRAME_CHARS: usize = 12 + 11 + 2;
/// `{"key":` + `,"value":` + `}` around one `general` pair.
const JSON_PAIR_FRAME_CHARS: usize = 7 + 9 + 1;

/// What changes from day to day; the books and the curated list are fixed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContextInputs {
    /// Today's prepared references ([`reference_term`]), in sermon order.
    pub prepared_refs: Vec<String>,
    /// Today's active šepkár source terms.
    pub tip_terms: Vec<String>,
}

/// The `context` sent with a connection's config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextTerms {
    /// `(key, value)` pairs: `domain`, `topic`.
    pub general: Vec<(String, String)>,
    pub terms: Vec<String>,
}

/// The context for one connection: priority order, deduplicated, capped.
pub fn context_terms(i: &ContextInputs) -> ContextTerms {
    let general = vec![
        ("domain".to_string(), CONTEXT_DOMAIN.to_string()),
        ("topic".to_string(), CONTEXT_TOPIC.to_string()),
    ];
    let ordered = BOOK_LONG_EN
        .iter()
        .copied()
        .chain(i.prepared_refs.iter().map(String::as_str))
        .chain(i.tip_terms.iter().map(String::as_str))
        .chain(CHRISTIAN_TERMS.iter().copied());
    let mut seen = HashSet::new();
    // Whitespace runs collapse to one space, so "Holy  Spirit" is "Holy Spirit".
    let mut terms: Vec<String> = ordered
        .map(|term| term.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|term| !term.is_empty() && seen.insert(term.to_lowercase()))
        .collect();
    let mut chars = context_json_chars(&general, &terms);
    while chars > CONTEXT_MAX_CHARS {
        let Some(last) = terms.pop() else { break };
        chars -= json_string_chars(&last) + usize::from(!terms.is_empty());
    }
    ContextTerms { general, terms }
}

/// The characters of the compact JSON `{"general":[{"key":…,"value":…},…],
/// "terms":[…]}` — the object the Soniox config carries (serde_json's
/// escaping).
pub fn context_json_chars(general: &[(String, String)], terms: &[String]) -> usize {
    let pairs: usize = general
        .iter()
        .map(|(key, value)| {
            JSON_PAIR_FRAME_CHARS + json_string_chars(key) + json_string_chars(value)
        })
        .sum();
    let terms_chars: usize = terms.iter().map(|term| json_string_chars(term)).sum();
    JSON_FRAME_CHARS
        + pairs
        + general.len().saturating_sub(1)
        + terms_chars
        + terms.len().saturating_sub(1)
}

/// One JSON string literal's characters, quotes and escapes included.
fn json_string_chars(text: &str) -> usize {
    2 + text
        .chars()
        .map(|c| match c {
            '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
            c if u32::from(c) < 0x20 => 6,
            _ => 1,
        })
        .sum::<usize>()
}

/// A reference as the preacher says it, with the English long book name:
/// "Romans 8", "Romans 8:37", "Romans 8:37-39" ("Psalm 23:1" for one psalm).
/// `book_index` = book number − 1; an unknown index gives an empty string
/// (left out of the terms). A verse end not after the start is ignored.
pub fn reference_term(
    book_index: u8,
    chapter: u16,
    verse_start: Option<u16>,
    verse_end: Option<u16>,
) -> String {
    let index = usize::from(book_index);
    let book = match BOOK_LONG_EN.get(index) {
        Some(_) if index == PSALMS_INDEX => "Psalm",
        Some(name) => name,
        None => return String::new(),
    };
    match (verse_start, verse_end) {
        (Some(start), Some(end)) if end > start => format!("{book} {chapter}:{start}-{end}"),
        (Some(start), _) => format!("{book} {chapter}:{start}"),
        (None, _) => format!("{book} {chapter}"),
    }
}

#[cfg(test)]
mod tests;
