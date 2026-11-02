//! Verse parsers (#851 D2a, spec §5.2): a source line becomes
//! [`MarkupSegment`]s and `content` is read back from them, so the two cannot
//! drift. Whitespace is decided once, here, over the whole verse.

pub(crate) mod mysword;
pub(crate) mod usfm;

use presenter_core::bible::{MarkupSegment, VerseMarkup};

/// A finished verse: its text, and its markup when that says more.
pub(crate) struct ParsedVerse {
    pub text: String,
    pub markup: Option<VerseMarkup>,
}

/// What a source's whitespace rule adds to the common one.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Spacing {
    Plain,
    /// MySword: no whitespace before `. , ; : ! ?` (the modules' `<FR>` quote
    /// spacing), also across a note.
    NoneBeforePunctuation,
}

pub(crate) fn finish(segments: Vec<MarkupSegment>, spacing: Spacing) -> ParsedVerse {
    let markup = VerseMarkup(normalize(segments, spacing));
    ParsedVerse {
        text: markup.plain_text(),
        markup: markup.has_markup().then_some(markup),
    }
}

/// One space between words, none at the edges, over all Text segments. The
/// space stays in the segment where the whitespace was, so `faith` keeps a
/// clean word and the gap keeps its own segment.
pub(crate) fn normalize(segments: Vec<MarkupSegment>, spacing: Spacing) -> Vec<MarkupSegment> {
    let mut out: Vec<MarkupSegment> = Vec::with_capacity(segments.len());
    let mut gap: Option<usize> = None;
    let mut started = false;
    for segment in segments {
        if !segment.kind.is_text() {
            if !segment.text.trim().is_empty() {
                out.push(segment);
            }
            continue;
        }
        let index = out.len();
        let source = segment.text.clone();
        out.push(MarkupSegment {
            text: String::with_capacity(source.len()),
            ..segment
        });
        for ch in source.chars() {
            if ch.is_whitespace() {
                if started && gap.is_none() {
                    gap = Some(index);
                }
                continue;
            }
            if let Some(at) = gap.take() {
                let swallowed = spacing == Spacing::NoneBeforePunctuation
                    && matches!(ch, '.' | ',' | ';' | ':' | '!' | '?');
                if !swallowed {
                    out[at].text.push(' ');
                }
            }
            out[index].text.push(ch);
            started = true;
        }
    }
    coalesce(out)
}

/// No empty Text segment, and neighbouring plain ones are one.
fn coalesce(segments: Vec<MarkupSegment>) -> Vec<MarkupSegment> {
    let mut out: Vec<MarkupSegment> = Vec::with_capacity(segments.len());
    for segment in segments {
        if segment.kind.is_text() && segment.text.is_empty() {
            continue;
        }
        match out.last_mut() {
            Some(last) if mergeable(last, &segment) => last.text.push_str(&segment.text),
            _ => out.push(segment),
        }
    }
    out
}

fn mergeable(a: &MarkupSegment, b: &MarkupSegment) -> bool {
    a.kind.is_text()
        && b.kind.is_text()
        && a.strongs.is_empty()
        && b.strongs.is_empty()
        && a.italic == b.italic
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(segments: Vec<MarkupSegment>, spacing: Spacing) -> String {
        finish(segments, spacing).text
    }

    #[test]
    fn whitespace_is_one_space_across_segments_and_none_at_the_edges() {
        let verse = vec![
            MarkupSegment::text("  A  "),
            MarkupSegment::text("  "),
            MarkupSegment::text(" B "),
        ];
        assert_eq!(text(verse, Spacing::Plain), "A B");
    }

    #[test]
    fn a_gap_stays_outside_the_word_after_it() {
        let word = MarkupSegment {
            text: "faith".to_string(),
            strongs: vec!["G4102".to_string()],
            ..MarkupSegment::default()
        };
        let verse = vec![MarkupSegment::text("Now "), word.clone()];
        let done = finish(verse, Spacing::Plain).markup.expect("markup");
        assert_eq!(done.0, vec![MarkupSegment::text("Now "), word]);
    }

    #[test]
    fn the_punctuation_rule_reaches_across_a_note() {
        let verse = vec![
            MarkupSegment::text("Rieka "),
            MarkupSegment::note("Pozn."),
            MarkupSegment::text(" . Tečie"),
        ];
        assert_eq!(
            text(verse.clone(), Spacing::NoneBeforePunctuation),
            "Rieka. Tečie"
        );
        assert_eq!(text(verse, Spacing::Plain), "Rieka . Tečie");
    }

    #[test]
    fn an_empty_note_or_heading_is_no_markup() {
        let done = finish(
            vec![
                MarkupSegment::heading(""),
                MarkupSegment::text("A"),
                MarkupSegment::note(" "),
            ],
            Spacing::Plain,
        );
        assert!(done.markup.is_none());
    }

    #[test]
    fn plain_neighbours_merge_and_a_bare_verse_has_no_markup() {
        let done = finish(
            vec![MarkupSegment::text("A"), MarkupSegment::text(" B")],
            Spacing::Plain,
        );
        assert_eq!(done.text, "A B");
        assert!(done.markup.is_none());
    }
}
