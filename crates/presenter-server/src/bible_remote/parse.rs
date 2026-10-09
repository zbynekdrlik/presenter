//! NLT API page → verses (#826).
//!
//! The API answers HTML. Each verse is one machine-generated
//! `<verse_export … ch="1" vn="1">…</verse_export>` element whose text starts
//! after `<span class="vn">1</span>`. Before that span sit the chapter heading
//! (`h2`/`h3.chapter-number`), section headings (`h3`/`h4.subhead`) and a
//! psalm's title (`p.psa-title`); inside the text sit footnote markers
//! (`a.a-tn`), footnote bodies (`span.tn`, with nested spans), poetry lines
//! (`p.poet1`/`p.poet2`), small-caps `Lord` (`span.sc`, the divine name) and
//! red-letter spans. The markup is flat and regular, so a few regexes plus a
//! balanced-span scan for the footnotes are enough — no HTML parser crate.

use regex::{Captures, Regex};
use std::sync::LazyLock;

/// One verse of an NLT page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NltVerse {
    pub(crate) chapter: u16,
    pub(crate) verse: u16,
    pub(crate) text: String,
}

// `Regex::new(...).ok()` is `None` only for a malformed literal — a programmer
// bug the parser tests catch at once. The parser then yields no verses, which
// the client reports as an NLT failure instead of showing wrong text.
static VERSE_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(?s)<verse_export\b([^>]*)>(.*?)</verse_export>").ok());
static CHAPTER_ATTR_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r#"\bch="(\d+)""#).ok());
static VERSE_ATTR_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r#"\bvn="(\d+)""#).ok());
static FOOTNOTE_MARK_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r#"(?s)<a\b[^>]*\bclass="a-tn"[^>]*>.*?</a>"#).ok());
static HEADING_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(?s)<h[1-6]\b[^>]*>.*?</h[1-6]>").ok());
static VERSE_NUMBER_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r#"(?s)^.*?<span\b[^>]*\bclass="vn"[^>]*>[^<]*</span>"#).ok());
static SMALL_CAPS_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r#"(?s)<span\b[^>]*\bclass="sc"[^>]*>([^<]*)</span>"#).ok());
static BLOCK_TAG_RE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?i)</?(?:p|br|div|section|blockquote|li|ul|ol|table|tr|td)\b[^>]*>").ok()
});
static TAG_RE: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"<[^>]*>").ok());

/// Opening tag of a footnote body.
const FOOTNOTE_OPEN: &str = r#"<span class="tn">"#;

/// Every `<verse_export>` of an NLT page, in page order. An element without a
/// numeric `ch`/`vn` is skipped (logged). An empty result means the page held
/// no verse at all.
pub(crate) fn parse_verses(html: &str) -> Vec<NltVerse> {
    let (Some(verse_re), Some(chapter_re), Some(verse_attr_re)) = (
        VERSE_RE.as_ref(),
        CHAPTER_ATTR_RE.as_ref(),
        VERSE_ATTR_RE.as_ref(),
    ) else {
        tracing::error!("NLT parser: a built-in regex failed to compile");
        return Vec::new();
    };
    verse_re
        .captures_iter(html)
        .filter_map(|caps| {
            let attributes = caps.get(1).map_or("", |m| m.as_str());
            let inner = caps.get(2).map_or("", |m| m.as_str());
            let chapter = capture_number(chapter_re, attributes);
            let verse = capture_number(verse_attr_re, attributes);
            match (chapter, verse) {
                (Some(chapter), Some(verse)) => Some(NltVerse {
                    chapter,
                    verse,
                    text: verse_text(inner),
                }),
                _ => {
                    tracing::warn!(attributes, "NLT parser: verse_export without ch/vn skipped");
                    None
                }
            }
        })
        .collect()
}

fn capture_number(re: &Regex, haystack: &str) -> Option<u16> {
    re.captures(haystack)?.get(1)?.as_str().parse().ok()
}

/// The display text of one verse's inner HTML: footnotes, headings, the psalm
/// title and the verse number removed; the divine name in capitals (`LORD`,
/// as eng-kjv writes it); poetry lines joined; entities decoded; whitespace
/// collapsed.
pub(super) fn verse_text(inner: &str) -> String {
    let mut text = remove_footnotes(inner);
    text = replace_all(FOOTNOTE_MARK_RE.as_ref(), &text, "");
    text = replace_all(HEADING_RE.as_ref(), &text, " ");
    if let Some(re) = VERSE_NUMBER_RE.as_ref() {
        text = re.replacen(&text, 1, "").into_owned();
    }
    if let Some(re) = SMALL_CAPS_RE.as_ref() {
        text = re
            .replace_all(&text, |caps: &Captures<'_>| {
                caps.get(1).map_or("", |m| m.as_str()).to_uppercase()
            })
            .into_owned();
    }
    text = replace_all(BLOCK_TAG_RE.as_ref(), &text, " ");
    text = replace_all(TAG_RE.as_ref(), &text, "");
    decode_entities(&text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn replace_all(re: Option<&Regex>, text: &str, with: &str) -> String {
    match re {
        Some(re) => re.replace_all(text, with).into_owned(),
        None => text.to_string(),
    }
}

/// `html` without its `<span class="tn">…</span>` footnote bodies. A body
/// nests spans (`tn-ref`), so each one is cut at its MATCHING `</span>`.
fn remove_footnotes(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find(FOOTNOTE_OPEN) {
        out.push_str(&rest[..start]);
        let footnote = &rest[start..];
        rest = &footnote[balanced_span_len(footnote)..];
    }
    out.push_str(rest);
    out
}

/// Byte length of the `<span …>` element `html` starts with, through its
/// matching `</span>` — the whole of `html` when it is never closed.
fn balanced_span_len(html: &str) -> usize {
    const OPEN: &str = "<span";
    const CLOSE: &str = "</span>";
    let mut depth = 0usize;
    let mut pos = 0usize;
    loop {
        let rest = &html[pos..];
        match (rest.find(OPEN), rest.find(CLOSE)) {
            (Some(open), Some(close)) if open < close => {
                depth += 1;
                pos += open + OPEN.len();
            }
            (_, Some(close)) => {
                depth = depth.saturating_sub(1);
                pos += close + CLOSE.len();
                if depth == 0 {
                    return pos;
                }
            }
            (_, None) => return html.len(),
        }
    }
}

/// Decode the HTML character references a page can carry: numeric (`&#8217;`,
/// `&#x201C;`) and the common named ones. Anything else is left as written.
pub(super) fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let decoded = rest
            .find(';')
            .filter(|semi| *semi <= 10)
            .and_then(|semi| decode_entity(&rest[1..semi]).map(|ch| (ch, semi)));
        match decoded {
            Some((ch, semi)) => {
                out.push(ch);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn decode_entity(name: &str) -> Option<char> {
    if let Some(number) = name.strip_prefix('#') {
        let code = match number.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => number.parse::<u32>().ok()?,
        };
        return char::from_u32(code);
    }
    let ch = match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => '\u{a0}',
        "lsquo" => '\u{2018}',
        "rsquo" => '\u{2019}',
        "ldquo" => '\u{201c}',
        "rdquo" => '\u{201d}',
        "ndash" => '\u{2013}',
        "mdash" => '\u{2014}',
        "hellip" => '\u{2026}',
        _ => return None,
    };
    Some(ch)
}
