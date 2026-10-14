//! Book-text folding (#850 spec §5.3) — the Rust twin of `fold()` in
//! `scripts/dev/gen_bible_book_names.py`. Both must fold every form the same
//! way: `tests/table.rs` re-folds each generated row's display form and runs the
//! shared `testdata/fold-cases.json`, which the Python unittest runs too.

use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

/// A typed or stored book name reduced to an ordinal and a lookup key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoldedBook {
    /// The leading book number (`1 Kor`, `I.`, `prvý`, `1 k.`), 1–5.
    pub ordinal: Option<u8>,
    /// The name without diacritics, filler words and punctuation (`[a-z0-9]`).
    pub key: String,
}

/// Spelled ordinals, tried in this order (`tretia` before `treti`).
const ORDINAL_WORDS: &[(&str, u8)] = &[
    ("prva", 1),
    ("prvy", 1),
    ("prvni", 1),
    ("prve", 1),
    ("druha", 2),
    ("druhy", 2),
    ("druhe", 2),
    ("tretia", 3),
    ("treti", 3),
    ("treta", 3),
    ("stvrta", 4),
    ("ctvrta", 4),
    ("piata", 5),
    ("pata", 5),
];

/// Roman ordinals; each needs a `.` or a space after it (`I Kor`, `II. Kor`).
const ROMAN_ORDINALS: &[(&str, u8)] = &[("iii", 3), ("ii", 2), ("iv", 4), ("i", 1)];

/// Words dropped from a book name (`Evanjelium podľa Jána` → `jana`).
const FILLER_WORDS: &[&str] = &[
    "kniha",
    "knihy",
    "list",
    "epistola",
    "cize",
    "evanjelium",
    "evangelium",
    "podla",
    "podle",
    "svateho",
    "svatych",
    "apostola",
    "apostolov",
    "pavla",
    "proroka",
    "prorok",
    "of",
    "the",
    "book",
    "gospel",
    "according",
    "to",
    "saint",
    "st",
];

/// Fold a book name: NFD without marks, lowercase, leading ordinal split off,
/// filler words dropped, then only `[a-z0-9]` kept.
pub fn fold_book_text(input: &str) -> FoldedBook {
    let lowered: String = strip_marks(input)
        .trim()
        .chars()
        .map(|c| {
            if ('\u{2010}'..='\u{2015}').contains(&c) {
                '-'
            } else {
                c
            }
        })
        .collect();
    let (ordinal, rest) = split_ordinal(&lowered);
    let key = drop_fillers(rest)
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        .collect();
    FoldedBook { ordinal, key }
}

/// NFD, combining marks dropped, lowercase.
pub(crate) fn strip_marks(input: &str) -> String {
    input
        .nfd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .collect()
}

fn split_ordinal(text: &str) -> (Option<u8>, &str) {
    match ordinal_word(text)
        .or_else(|| roman_ordinal(text))
        .or_else(|| digit_ordinal(text))
    {
        Some((ordinal, rest)) => (Some(ordinal), rest),
        None => (None, text),
    }
}

/// `prvá`, `druhý list`, `tretia kniha` … (the optional book word is consumed).
fn ordinal_word(text: &str) -> Option<(u8, &str)> {
    let (rest, ordinal) = ORDINAL_WORDS
        .iter()
        .find_map(|&(word, n)| text.strip_prefix(word).map(|rest| (rest, n)))?;
    let book_words: &[&str] = if ordinal <= 3 {
        &["kniha", "list", "epistola"]
    } else {
        &["kniha"]
    };
    let after_space = rest.trim_start();
    let rest = if after_space.len() < rest.len() {
        book_words
            .iter()
            .find_map(|word| after_space.strip_prefix(word))
            .unwrap_or(rest)
    } else {
        rest
    };
    Some((ordinal, rest.trim_start()))
}

/// `I.`, `II `, `iv.` … and `v.` (five needs the dot).
fn roman_ordinal(text: &str) -> Option<(u8, &str)> {
    for &(numeral, ordinal) in ROMAN_ORDINALS {
        if let Some(rest) = text.strip_prefix(numeral) {
            let mut chars = rest.chars();
            if chars.next().is_some_and(|c| c == '.' || c.is_whitespace()) {
                return Some((ordinal, chars.as_str().trim_start()));
            }
        }
    }
    text.strip_prefix("v.").map(|rest| (5, rest.trim_start()))
}

/// `1`, `1.`, `1 k.` (the "kniha" abbreviation needs a space after it).
fn digit_ordinal(text: &str) -> Option<(u8, &str)> {
    let mut chars = text.chars();
    let digit = chars.next()?.to_digit(10)?;
    let ordinal = u8::try_from(digit).ok().filter(|d| (1..=5).contains(d))?;
    let after = chars.as_str().trim_start();
    let after = after.strip_prefix('.').unwrap_or(after).trim_start();
    if let Some(rest) = after.strip_prefix('k') {
        let rest = rest.strip_prefix('.').unwrap_or(rest);
        if rest.starts_with(char::is_whitespace) {
            return Some((ordinal, rest));
        }
    }
    Some((ordinal, after))
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Drop whole filler words and a standalone `k` / `k.` followed by a space.
fn drop_fillers(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if !is_word_char(chars[i]) {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && is_word_char(chars[i]) {
            i += 1;
        }
        let word: String = chars[start..i].iter().collect();
        if !(FILLER_WORDS.contains(&word.as_str()) || is_kniha_abbreviation(&chars, start, i)) {
            out.push_str(&word);
        }
        out.push(' ');
    }
    out
}

/// `k` / `k.` with whitespace (or the start) before it and whitespace after it.
fn is_kniha_abbreviation(chars: &[char], start: usize, end: usize) -> bool {
    if end - start != 1 || chars[start] != 'k' {
        return false;
    }
    let before_ok = start == 0 || chars[start - 1].is_whitespace();
    let after = if chars.get(end) == Some(&'.') {
        end + 1
    } else {
        end
    };
    before_ok && chars.get(after).is_some_and(|c| c.is_whitespace())
}
