//! Candidate scoring (#850 spec §5.5). Every form of the generated table (and
//! the main translation's stored names) that the typed key meets is scored
//! `(tier, kind, penalty)`; books are ordered by that score, then by the
//! citation frequency of their family, then by code (so `1 Kor` precedes
//! `2 Kor`). Keys of at most two letters go through `SHORT_DEFAULTS` first.
//! Fuzzy matching runs only when nothing else matched and the key has 4+ letters.

use unicode_normalization::UnicodeNormalization;

use super::fold::FoldedBook;
use super::names_table::{BOOK_CODES, BOOK_KEYS, FREQUENCY_RANK, NO_FOLD, SHORT_DEFAULTS};
use super::types::{BookCandidate, MatchKind, Tier};

const SHORT_KEY_MAX: usize = 2;
const FUZZY_MIN_KEY: usize = 4;
const FUZZY_CAP: usize = 3;

/// A stored book name of the main translation, matched as a tier-1 form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredName {
    pub(crate) index: u8,
    pub(crate) ordinal: u8,
    pub(crate) key: String,
    pub(crate) form: String,
}

#[derive(Clone, Copy)]
struct Row<'a> {
    key: &'a str,
    form: &'a str,
    index: u8,
    tier: Tier,
    ordinal: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Score {
    tier: Tier,
    kind: MatchKind,
    penalty: u8,
}

#[derive(Clone, Copy)]
struct Best<'a> {
    score: Score,
    form: &'a str,
}

type Board<'a> = [Option<Best<'a>>; 66];

#[derive(Clone, Copy)]
enum Pass {
    Prefix,
    Fuzzy,
}

/// Book candidates for a folded book text, best first.
pub(crate) fn rank(folded: &FoldedBook, stored: &[StoredName]) -> Vec<BookCandidate> {
    let key = folded.key.as_str();
    if key.is_empty() {
        return Vec::new();
    }
    let ordinal = folded.ordinal.unwrap_or(0);
    let start = BOOK_KEYS.partition_point(|row| row.0 < key);
    let prefixed = BOOK_KEYS
        .get(start..)
        .unwrap_or_default()
        .iter()
        .take_while(|row| row.0.starts_with(key))
        .map(table_row);
    let mut board: Board<'_> = [None; 66];
    score_rows(&mut board, prefixed, key, ordinal, Pass::Prefix);
    let names = stored.iter().map(stored_row);
    score_rows(&mut board, names, key, ordinal, Pass::Prefix);
    if board.iter().all(Option::is_none) && key.len() >= FUZZY_MIN_KEY {
        let rows = BOOK_KEYS.iter().map(table_row);
        score_rows(&mut board, rows, key, ordinal, Pass::Fuzzy);
        let names = stored.iter().map(stored_row);
        score_rows(&mut board, names, key, ordinal, Pass::Fuzzy);
    }
    short_defaults_first(folded, order(&board))
        .into_iter()
        .map(|(index, best)| candidate(index, best.score, best.form))
        .collect()
}

/// `P.Š.` / `PŠ` (Song of Songs), matched before folding — folded it is `ps`.
pub(crate) fn no_fold_candidate(book_text: &str) -> Option<BookCandidate> {
    let key: String = book_text
        .nfc()
        .flat_map(char::to_lowercase)
        .filter(|c| *c != '.' && !c.is_whitespace())
        .collect();
    let &(_, index) = NO_FOLD.iter().find(|(form, _)| *form == key)?;
    let score = Score {
        tier: Tier::T1,
        kind: MatchKind::Exact,
        penalty: 0,
    };
    Some(candidate(index, score, book_text.trim()))
}

/// Per book index: does a table form or a stored name carry `ordinal`? The
/// ordinal comes from the name, not the USFM code: `1` = `1. Mojžišova` (GEN)
/// as well as `1 Kor`, `3` also = Roháček's `Tretia kniha Kráľov` (1KI).
pub(crate) fn books_with_ordinal(ordinal: u8, stored: &[StoredName]) -> [bool; 66] {
    let mut books = [false; 66];
    let table = BOOK_KEYS
        .iter()
        .filter(|row| row.3 & 0x0F == ordinal)
        .map(|row| row.2);
    let names = stored
        .iter()
        .filter(|name| name.ordinal == ordinal)
        .map(|name| name.index);
    for index in table.chain(names) {
        if let Some(slot) = books.get_mut(usize::from(index)) {
            *slot = true;
        }
    }
    books
}

pub(crate) fn index_of(code: &str) -> Option<u8> {
    BOOK_CODES
        .iter()
        .position(|candidate| *candidate == code)
        .and_then(|index| u8::try_from(index).ok())
}

fn code_of(index: u8) -> &'static str {
    BOOK_CODES.get(usize::from(index)).copied().unwrap_or("")
}

fn rank_of(index: u8) -> u8 {
    FREQUENCY_RANK
        .get(usize::from(index))
        .copied()
        .unwrap_or(u8::MAX)
}

fn table_row(&(key, form, index, packed): &(&'static str, &'static str, u8, u8)) -> Row<'static> {
    let tier = match packed >> 4 {
        1 => Tier::T1,
        2 => Tier::T2,
        _ => Tier::T3,
    };
    Row {
        key,
        form,
        index,
        tier,
        ordinal: packed & 0x0F,
    }
}

fn stored_row(name: &StoredName) -> Row<'_> {
    Row {
        key: &name.key,
        form: &name.form,
        index: name.index,
        tier: Tier::T1,
        ordinal: name.ordinal,
    }
}

fn prefix_score(row: &Row<'_>, key: &str, ordinal: u8) -> Option<Score> {
    if !row.key.starts_with(key) || (ordinal != 0 && row.ordinal != ordinal) {
        return None;
    }
    // One-letter aliases (SEP J, L, R, G) carry no extra weight.
    let kind = if (key.len() == 1 && ordinal == 0) || row.key != key {
        MatchKind::Prefix
    } else if row.ordinal == ordinal {
        MatchKind::Exact
    } else {
        MatchKind::OrdinalStripped
    };
    Some(Score {
        tier: row.tier,
        kind,
        penalty: u8::from(ordinal == 0 && row.ordinal != 0),
    })
}

/// Damerau-Levenshtein ≤ 1 for keys of 4–6 letters, ≤ 2 for 7+, against the
/// whole form and its same-length prefix (`hebrjom` ~ `hebrejom`). Tier 3
/// forms and forms shorter than 4 letters never match fuzzily.
fn fuzzy_score(row: &Row<'_>, key: &str, ordinal: u8) -> Option<Score> {
    if row.tier == Tier::T3
        || row.key.len() < FUZZY_MIN_KEY
        || (ordinal != 0 && row.ordinal != ordinal)
    {
        return None;
    }
    let limit = if key.len() <= 6 { 1 } else { 2 };
    let (typed, form) = (key.as_bytes(), row.key.as_bytes());
    let same_length = form.get(..typed.len()).unwrap_or(form);
    let one_longer = form.get(..typed.len() + 1).unwrap_or(form);
    let distance = osa_distance(typed, form)
        .min(osa_distance(typed, same_length))
        .min(osa_distance(typed, one_longer));
    (distance <= limit).then(|| Score {
        tier: row.tier,
        kind: MatchKind::Fuzzy,
        penalty: u8::from(ordinal == 0 && row.ordinal != 0)
            .saturating_add(u8::try_from(distance).unwrap_or(u8::MAX)),
    })
}

/// Optimal-string-alignment distance, capped at 3 (and 3 when the lengths
/// differ by more than 2).
pub(crate) fn osa_distance(a: &[u8], b: &[u8]) -> usize {
    if a.len().abs_diff(b.len()) > 2 {
        return FUZZY_CAP;
    }
    let width = b.len() + 1;
    let mut d = vec![0_usize; (a.len() + 1) * width];
    for (j, cell) in d.iter_mut().take(width).enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        d[i * width] = i;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut best = (d[(i - 1) * width + j] + 1)
                .min(d[i * width + j - 1] + 1)
                .min(d[(i - 1) * width + j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                best = best.min(d[(i - 2) * width + j - 2] + 1);
            }
            d[i * width + j] = best;
        }
    }
    d[a.len() * width + b.len()].min(FUZZY_CAP)
}

/// Score `rows` into `board`, keeping each book's best form. `'r: 'a` lets the
/// `'static` table rows and the borrowed stored names share one board.
fn score_rows<'a, 'r: 'a>(
    board: &mut Board<'a>,
    rows: impl Iterator<Item = Row<'r>>,
    key: &str,
    ordinal: u8,
    pass: Pass,
) {
    for row in rows {
        let found = match pass {
            Pass::Prefix => prefix_score(&row, key, ordinal),
            Pass::Fuzzy => fuzzy_score(&row, key, ordinal),
        };
        if let Some(score) = found {
            keep_best(board, row, score);
        }
    }
}

fn keep_best<'a>(board: &mut Board<'a>, row: Row<'a>, score: Score) {
    if let Some(slot) = board.get_mut(usize::from(row.index)) {
        match slot {
            Some(best) if best.score <= score => {}
            _ => {
                *slot = Some(Best {
                    score,
                    form: row.form,
                });
            }
        }
    }
}

fn order<'a>(board: &Board<'a>) -> Vec<(u8, Best<'a>)> {
    let scored: Vec<(u8, Best<'a>)> = board
        .iter()
        .zip(0_u8..)
        .filter_map(|(best, index)| best.map(|best| (index, best)))
        .collect();
    let mut keyed: Vec<_> = scored
        .iter()
        .map(|&(index, best)| {
            let family = family_rank(index, best.score.penalty, &scored);
            ((best.score, family, code_of(index)), (index, best))
        })
        .collect();
    keyed.sort_by_key(|(key, _)| *key);
    keyed.into_iter().map(|(_, entry)| entry).collect()
}

/// A numbered book matched without a typed ordinal ranks by its family's
/// most cited member (`kor` → the Korinťanom family at 1 Kor's rank).
fn family_rank(index: u8, penalty: u8, scored: &[(u8, Best<'_>)]) -> u8 {
    let code = code_of(index);
    if penalty == 0 || !code.starts_with(|c: char| c.is_ascii_digit()) {
        return rank_of(index);
    }
    let family = code.get(1..);
    scored
        .iter()
        .filter(|(other, _)| code_of(*other).get(1..) == family)
        .map(|(other, _)| rank_of(*other))
        .min()
        .unwrap_or_else(|| rank_of(index))
}

fn short_defaults_first<'a>(
    folded: &FoldedBook,
    ordered: Vec<(u8, Best<'a>)>,
) -> Vec<(u8, Best<'a>)> {
    if folded.key.len() > SHORT_KEY_MAX {
        return ordered;
    }
    let full_key = match folded.ordinal {
        Some(ordinal) => format!("{ordinal}{}", folded.key),
        None => folded.key.clone(),
    };
    let Some(&(_, preferred)) = SHORT_DEFAULTS.iter().find(|(key, _)| *key == full_key) else {
        return ordered;
    };
    let mut head: Vec<(u8, Best<'a>)> = preferred
        .iter()
        .filter_map(|index| ordered.iter().find(|(i, _)| i == index).copied())
        .collect();
    head.extend(
        ordered
            .iter()
            .filter(|(i, _)| !preferred.contains(i))
            .copied(),
    );
    head
}

fn candidate(index: u8, score: Score, form: &str) -> BookCandidate {
    BookCandidate {
        code: code_of(index).to_string(),
        number: u16::from(index) + 1,
        tier: score.tier,
        kind: score.kind,
        penalty: score.penalty,
        matched_form: form.to_string(),
    }
}
