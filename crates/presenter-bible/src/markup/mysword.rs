//! MySword (SEB, ROH, MIL) to segments (spec §5.2): `<FI>[ … ]<Fi>` is a
//! translator's note, `<FI>word<Fi>` an italic, `<TS>…<Ts>` a heading, a BARE
//! `[ … ]` is translation text and stays (ROH Mt 17:21 "[a pôstom]"), every
//! other tag goes. Notes and italics pass the tag stripping as private-use
//! characters, so the spacing rules see the same string the wall text comes
//! from.

use presenter_core::bible::MarkupSegment;
use regex::{Captures, Regex};
use std::borrow::Cow;
use std::sync::LazyLock;
use tracing::warn;

use crate::parsers::{sanitize_markup_text, starts_with_ci};

static NOTE_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"\s*<FI>\s*\[([^\]]*)\]\s*<Fi>").ok());
static HEADING_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(?is)<TS>(.*?)<Ts>").ok());

const NOTE_OPEN: char = '\u{E000}';
const NOTE_CLOSE: char = '\u{E001}';
const ITALIC_ON: char = '\u{E002}';
const ITALIC_OFF: char = '\u{E003}';

/// The verse without characters the placeholders use: one in the source would
/// swallow the verse behind it.
fn without_reserved(input: &str) -> Cow<'_, str> {
    let reserved = [NOTE_OPEN, NOTE_CLOSE, ITALIC_ON, ITALIC_OFF];
    if !input.contains(reserved) {
        return Cow::Borrowed(input);
    }
    warn!("dropping private-use characters from a MySword verse");
    Cow::Owned(input.replace(reserved, ""))
}

pub(crate) fn segments(input: &str) -> Vec<MarkupSegment> {
    let input = without_reserved(input);
    let input = input.as_ref();
    let mut notes = Vec::new();
    let marked = match NOTE_RE.as_ref() {
        // A note with the whitespace around it leaves ONE space, so two words
        // it sat between never fuse.
        Some(re) => re
            .replace_all(input, |found: &Captures<'_>| {
                notes.push(sanitize_markup_text(&found[1]));
                format!(" {NOTE_OPEN}{}{NOTE_CLOSE}", notes.len() - 1)
            })
            .into_owned(),
        None => input.to_string(),
    };
    let mut headings = Vec::new();
    let body = match HEADING_RE.as_ref() {
        Some(re) => re
            .replace_all(&marked, |found: &Captures<'_>| {
                let text = sanitize_markup_text(&without_placeholders(&found[1]));
                headings.push(MarkupSegment::heading(text));
                ""
            })
            .into_owned(),
        None => marked,
    };
    let flat = sanitize_markup_text(&strip_tags(&body));
    headings.extend(split(&flat, &notes));
    headings
}

/// A note inside a heading goes with the heading's markup.
fn without_placeholders(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch == NOTE_OPEN {
            chars.by_ref().find(|next| *next == NOTE_CLOSE);
        } else {
            out.push(ch);
        }
    }
    out
}

/// Drops `<f…</f>` spans and every tag; `<FI>` and `<Fi>` leave a placeholder.
/// Text after a `<` that never closes is cut off.
fn strip_tags(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(ch) = rest.chars().next() {
        if starts_with_ci(rest, "<f") {
            if let Some(end) = rest.find("</f>") {
                rest = &rest[end + 4..];
                continue;
            }
        }
        if ch == '<' {
            let Some(end) = rest.find('>') else {
                break;
            };
            match &rest[..=end] {
                "<FI>" => output.push(ITALIC_ON),
                "<Fi>" => output.push(ITALIC_OFF),
                _ => {}
            }
            rest = &rest[end + 1..];
            continue;
        }
        output.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    output
}

fn split(flat: &str, notes: &[String]) -> Vec<MarkupSegment> {
    let mut out = Vec::new();
    let mut current = MarkupSegment::default();
    let mut chars = flat.chars();
    let close = |current: &mut MarkupSegment, out: &mut Vec<MarkupSegment>, italic: bool| {
        let done = std::mem::replace(
            current,
            MarkupSegment {
                italic,
                ..MarkupSegment::default()
            },
        );
        if !done.text.is_empty() {
            out.push(done);
        }
    };
    while let Some(ch) = chars.next() {
        match ch {
            ITALIC_ON => close(&mut current, &mut out, true),
            ITALIC_OFF => close(&mut current, &mut out, false),
            NOTE_OPEN => {
                let italic = current.italic;
                close(&mut current, &mut out, italic);
                let index: String = chars
                    .by_ref()
                    .take_while(|next| *next != NOTE_CLOSE)
                    .collect();
                let note = index.parse::<usize>().ok().and_then(|i| notes.get(i));
                out.extend(note.map(MarkupSegment::note));
            }
            _ => current.text.push(ch),
        }
    }
    close(&mut current, &mut out, false);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markup::{finish, ParsedVerse, Spacing};
    use presenter_core::bible::SegmentKind;

    fn parse(input: &str) -> ParsedVerse {
        finish(segments(input), Spacing::NoneBeforePunctuation)
    }

    fn text(input: &str) -> String {
        parse(input).text
    }

    fn of_kind(input: &str, kind: SegmentKind) -> Vec<String> {
        parse(input)
            .markup
            .map(|m| m.0)
            .unwrap_or_default()
            .into_iter()
            .filter(|s| s.kind == kind)
            .map(|s| s.text)
            .collect()
    }

    #[test]
    fn a_note_needs_its_fi_wrapper() {
        let verse = "Rieka <FI>[ Pozn.: iný názov.]<Fi>. Tečie";
        assert_eq!(text(verse), "Rieka. Tečie");
        assert_eq!(of_kind(verse, SegmentKind::Note), ["Pozn.: iný názov."]);
        assert_eq!(
            text("iba modlitbou [a pôstom]."),
            "iba modlitbou [a pôstom]."
        );
        assert!(parse("iba modlitbou [a pôstom].").markup.is_none());
    }

    #[test]
    fn an_fi_word_is_italic_and_stays_in_the_text() {
        let done = parse("<FI>slovo<Fi> ostáva");
        assert_eq!(done.text, "slovo ostáva");
        assert!(done.markup.expect("markup").0[0].italic);
    }

    #[test]
    fn a_note_between_words_leaves_a_gap() {
        for verse in [
            "slovo <FI>[ Pozn.: iný názov.]<Fi>ďalšie",
            "slovo<FI>[ Pozn.]<Fi>ďalšie",
            "slovo  <FI> [ Pozn.] <Fi>ďalšie",
        ] {
            assert_eq!(text(verse), "slovo ďalšie", "{verse}");
        }
    }

    #[test]
    fn a_heading_in_any_case_comes_first_and_leaves_the_text() {
        let verse = "<TS>Nadpis<Ts>  Text verša.";
        assert_eq!(text(verse), "Text verša.");
        assert_eq!(of_kind(verse, SegmentKind::Heading), ["Nadpis"]);
        assert_eq!(text("<ts>Iný nadpis<ts> Text."), "Text.");
    }

    #[test]
    fn a_note_inside_a_heading_goes_with_the_heading() {
        let verse = "<TS>Nadpis <FI>[ Pozn.]<Fi> ďalej<Ts> Text.";
        assert_eq!(text(verse), "Text.");
        assert_eq!(of_kind(verse, SegmentKind::Heading), ["Nadpis ďalej"]);
        assert!(of_kind(verse, SegmentKind::Note).is_empty());
    }

    #[test]
    fn whitespace_before_punctuation_goes() {
        assert_eq!(
            text("slovom . Pán , áno ; nie : prečo ? hej !"),
            "slovom. Pán, áno; nie: prečo? hej!"
        );
        assert_eq!(text("bez zmeny – pomlčka"), "bez zmeny – pomlčka");
    }

    #[test]
    fn a_placeholder_character_in_the_source_swallows_nothing() {
        assert_eq!(
            text("slovo\u{E000} ďalšie\u{E002} text"),
            "slovo ďalšie text"
        );
    }

    #[test]
    fn an_empty_note_or_heading_leaves_no_markup() {
        assert!(parse("<TS><Ts>Text <FI>[ ]<Fi>ďalej").markup.is_none());
    }

    #[test]
    fn tags_and_footnotes_go() {
        assert_eq!(text("<b>Bold</b> text"), "Bold text");
        assert_eq!(text("Word<f>footnote content</f> rest"), "Word rest");
        assert_eq!(text("text <FI>[ Var.: + extra]<Fi> more"), "text more");
    }

    #[test]
    fn every_pattern_compiles() {
        assert!(NOTE_RE.is_some() && HEADING_RE.is_some());
    }
}
