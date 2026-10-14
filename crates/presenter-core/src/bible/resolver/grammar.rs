//! Hand-written reference grammar (#850 spec §5.4), no `regex` dependency.
//!
//! The book text runs to the first digit after an optional leading ordinal
//! (`1kor13:4`). Chapter–verse separator: `:` (then `,` and `.` list verses),
//! else `,` (then `.` lists), else the first `.`, else a space between two
//! numbers. `-`, `–`, `—`, `až`, `az` make ranges. The console resolves on
//! every keystroke, so trailing separators are ignored and text the grammar
//! cannot use is returned as `leftover`.

use std::cmp::Ordering;

use super::fold::strip_marks;
use super::types::RawSpec;

/// Chapter/verse tokens parsed at most; the rest of a long paste is leftover.
const MAX_SPEC_TOKENS: usize = 64;

pub(crate) struct SplitInput {
    pub(crate) book_text: String,
    pub(crate) bare_ordinal: Option<u8>,
    pub(crate) spec: Option<RawSpec>,
    pub(crate) leftover: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tok {
    Num(u16),
    Colon,
    Comma,
    Dot,
    Dash,
    Space,
    Word,
}

impl Tok {
    const fn is_separator(self) -> bool {
        matches!(self, Self::Colon | Self::Comma | Self::Dot | Self::Dash)
    }
}

pub(crate) fn split_input(input: &str) -> SplitInput {
    let text: String = input.trim().chars().map(normalize_dash).collect();
    let Some(cut) = spec_start(&text) else {
        return SplitInput {
            book_text: text,
            bare_ordinal: None,
            spec: None,
            leftover: String::new(),
        };
    };
    let book_text = text[..cut].trim().to_string();
    let spec_text = &text[cut..];
    let bare_ordinal = if book_text.is_empty() {
        bare_ordinal(spec_text)
    } else {
        None
    };
    if bare_ordinal.is_some() {
        return SplitInput {
            book_text,
            bare_ordinal,
            spec: None,
            leftover: String::new(),
        };
    }
    let (spec, leftover) = parse_spec(spec_text);
    SplitInput {
        book_text,
        bare_ordinal: None,
        spec,
        leftover,
    }
}

fn normalize_dash(c: char) -> char {
    if ('\u{2010}'..='\u{2015}').contains(&c) || c == '\u{2212}' {
        '-'
    } else {
        c
    }
}

/// Byte index of the first digit after an optional leading ordinal.
fn spec_start(text: &str) -> Option<usize> {
    let from = leading_ordinal_end(text).unwrap_or(0);
    text[from..]
        .find(|c: char| c.is_ascii_digit())
        .map(|index| from + index)
}

/// `1`–`5`, optional spaces and one `.`, then a char that is no digit,
/// space or separator: the book text starts with an ordinal (`1. Kor`).
fn leading_ordinal_end(text: &str) -> Option<usize> {
    let mut chars = text.char_indices();
    let (_, first) = chars.next()?;
    if !('1'..='5').contains(&first) {
        return None;
    }
    let mut seen_dot = false;
    for (index, c) in chars {
        if c.is_whitespace() {
            continue;
        }
        if c == '.' && !seen_dot {
            seen_dot = true;
            continue;
        }
        return (!c.is_ascii_digit() && !":,.-".contains(c)).then_some(index);
    }
    None
}

/// `1`, `2.`, `3 ` alone: an ordinal waiting for its book.
fn bare_ordinal(spec_text: &str) -> Option<u8> {
    let trimmed = spec_text.trim();
    match trimmed.strip_suffix('.').unwrap_or(trimmed).trim_end() {
        "1" => Some(1),
        "2" => Some(2),
        "3" => Some(3),
        _ => None,
    }
}

/// Tokens of the chapter/verse part and each token's END byte offset.
fn tokenize(text: &str) -> (Vec<Tok>, Vec<usize>) {
    let (mut toks, mut ends) = (Vec::new(), Vec::new());
    let mut chars = text.char_indices().peekable();
    while let Some(&(_, c)) = chars.peek() {
        if toks.len() == MAX_SPEC_TOKENS {
            break;
        }
        let tok = if c.is_ascii_digit() {
            let mut value: u16 = 0;
            while let Some(digit) = chars.peek().and_then(|&(_, d)| d.to_digit(10)) {
                let digit = u16::try_from(digit).unwrap_or(0);
                value = value.saturating_mul(10).saturating_add(digit);
                chars.next();
            }
            Tok::Num(value.max(1))
        } else if c.is_whitespace() {
            while chars.next_if(|&(_, w)| w.is_whitespace()).is_some() {}
            Tok::Space
        } else if c.is_alphabetic() {
            let mut word = String::new();
            while let Some((_, w)) = chars.next_if(|&(_, w)| w.is_alphabetic()) {
                word.push(w);
            }
            if strip_marks(&word) == "az" {
                Tok::Dash
            } else {
                Tok::Word
            }
        } else {
            chars.next();
            match c {
                ':' => Tok::Colon,
                ',' => Tok::Comma,
                '.' => Tok::Dot,
                '-' => Tok::Dash,
                _ => Tok::Word,
            }
        };
        toks.push(tok);
        ends.push(chars.peek().map_or(text.len(), |&(next, _)| next));
    }
    (toks, ends)
}

/// Parse the longest prefix the grammar understands; the rest is leftover.
fn parse_spec(text: &str) -> (Option<RawSpec>, String) {
    let (toks, ends) = tokenize(text);
    let usable = toks
        .iter()
        .position(|t| *t == Tok::Word)
        .unwrap_or(toks.len());
    let (toks, ends) = drop_loose_spaces(&toks[..usable], &ends[..usable]);
    let mut end = toks.len();
    while end > 0 {
        let used = trimmed_len(&toks[..end]);
        if used == 0 {
            break;
        }
        if let Some(spec) = parse_exact(&toks[..used]) {
            let rest = &text[ends[used - 1]..];
            let leftover = rest
                .trim_start_matches(|c: char| c.is_whitespace() || ":,.-".contains(c))
                .trim_end();
            return (Some(spec), leftover.to_string());
        }
        end = used - 1;
    }
    (None, text.trim().to_string())
}

/// Spaces next to a separator do not count (`11 : 1` = `11:1`).
fn drop_loose_spaces(toks: &[Tok], ends: &[usize]) -> (Vec<Tok>, Vec<usize>) {
    let (mut kept, mut kept_ends) = (Vec::new(), Vec::new());
    for (index, (&tok, &end)) in toks.iter().zip(ends).enumerate() {
        let next_is_separator = toks.get(index + 1).is_some_and(|t| t.is_separator());
        let previous_is_separator = kept.last().is_some_and(|t: &Tok| t.is_separator());
        if tok == Tok::Space && (next_is_separator || previous_is_separator) {
            continue;
        }
        kept.push(tok);
        kept_ends.push(end);
    }
    (kept, kept_ends)
}

/// Length without trailing separators and spaces (`11:` → `11`).
fn trimmed_len(toks: &[Tok]) -> usize {
    toks.iter()
        .rposition(|t| matches!(t, Tok::Num(_)))
        .map_or(0, |index| index + 1)
}

fn parse_exact(toks: &[Tok]) -> Option<RawSpec> {
    match toks {
        [Tok::Num(chapter)] => return Some(RawSpec::Chapter(*chapter)),
        [Tok::Num(start), Tok::Dash, Tok::Num(end)] => {
            return Some(if end <= start {
                RawSpec::Chapter(*start)
            } else {
                RawSpec::ChapterRange {
                    start: *start,
                    end: *end,
                }
            });
        }
        _ => {}
    }
    let separator = [Tok::Colon, Tok::Comma, Tok::Dot, Tok::Space]
        .into_iter()
        .find(|sep| toks.contains(sep))?;
    let [Tok::Num(chapter), sep, rest @ ..] = toks else {
        return None;
    };
    if *sep != separator || rest.is_empty() {
        return None;
    }
    if let [Tok::Num(verse_start), Tok::Dash, Tok::Num(end_chapter), sep_end, Tok::Num(verse_end)] =
        rest
    {
        if *sep_end == separator {
            return Some(cross_chapter(
                *chapter,
                *verse_start,
                *end_chapter,
                *verse_end,
            ));
        }
    }
    let list_separators: &[Tok] = match separator {
        Tok::Colon => &[Tok::Comma, Tok::Dot],
        Tok::Space => &[Tok::Space],
        _ => &[Tok::Dot],
    };
    let ranges = rest
        .split(|t| list_separators.contains(t))
        .map(verse_range)
        .collect::<Option<Vec<_>>>()?;
    Some(RawSpec::Verses {
        chapter: *chapter,
        ranges,
    })
}

/// `16` or `16-18`; a reversed range keeps only its start (`5-3` = `5`).
fn verse_range(item: &[Tok]) -> Option<(u16, u16)> {
    match item {
        [Tok::Num(verse)] => Some((*verse, *verse)),
        [Tok::Num(start), Tok::Dash, Tok::Num(end)] => Some((*start, (*end).max(*start))),
        _ => None,
    }
}

fn cross_chapter(chapter: u16, verse_start: u16, end_chapter: u16, verse_end: u16) -> RawSpec {
    match end_chapter.cmp(&chapter) {
        Ordering::Less => RawSpec::Verses {
            chapter,
            ranges: vec![(verse_start, verse_start)],
        },
        Ordering::Equal => RawSpec::Verses {
            chapter,
            ranges: vec![(verse_start, verse_end.max(verse_start))],
        },
        Ordering::Greater => RawSpec::CrossChapter {
            chapter,
            verse_start,
            end_chapter,
            verse_end,
        },
    }
}
