//! USFM (KJV) to segments (spec §5.2). `\w …|strong="…"` and nested `\+w` carry
//! a Strong's number, `\add` is italic, the `\ft` text of a `\f … \f*` is a
//! note, `\x … \x*` goes with its text, every other marker only goes.

use super::{finish, normalize, Spacing};
use presenter_core::bible::{normalize_strongs, MarkupSegment, SegmentKind};
use regex::Regex;
use std::sync::LazyLock;

static NOTE_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"\\f\s.*?\\f\*|\\x\s.*?\\x\*").ok());

/// USFM markers whose whole line is a heading or a title, never verse text.
const HEADING_MARKERS: [&str; 13] = [
    "s", "ms", "mr", "sr", "r", "d", "sp", "mt", "mte", "cl", "cd", "qa", "h",
];

/// Whether a USFM line is a heading or title paragraph (`\s1 …`, `\d …`,
/// `\ms1 …`): the marker NAME decides (letters, then optional digits, then
/// whitespace or the end of the line), so `\sig` or `\s1x` is not a heading.
pub(crate) fn is_heading_line(line: &str) -> bool {
    let Some(rest) = line.strip_prefix('\\') else {
        return false;
    };
    let name_len = rest.chars().take_while(char::is_ascii_lowercase).count();
    let (Some(name), Some(after)) = (rest.get(..name_len), rest.get(name_len..)) else {
        return false;
    };
    let after = after.trim_start_matches(|c: char| c.is_ascii_digit());
    let marker_ends = after.is_empty() || after.starts_with(char::is_whitespace);
    marker_ends && HEADING_MARKERS.contains(&name)
}

/// The words of a USFM line, without markers or notes.
fn plain(line: &str) -> String {
    finish(line_segments(line), Spacing::Plain).text
}

/// A heading line as Heading segments. A Psalm title's words keep their
/// Strong's numbers (424 tags), so a word search finds the title too.
pub(crate) fn heading(line: &str) -> Vec<MarkupSegment> {
    let mut segments = normalize(line_segments(line), Spacing::Plain);
    segments.retain(|segment| segment.kind.is_text());
    for segment in &mut segments {
        segment.kind = SegmentKind::Heading;
    }
    segments
}

/// One USFM line that stands alone.
pub(crate) fn line_segments(line: &str) -> Vec<MarkupSegment> {
    verse_line(line, &mut 0)
}

/// One USFM line of a verse; `italic` is the `\add` depth, which a line can
/// leave open for the next one. A note without its end marker stays in the text.
pub(crate) fn verse_line(line: &str, italic: &mut u8) -> Vec<MarkupSegment> {
    let mut run = Run {
        italic: *italic,
        ..Run::default()
    };
    let mut from = 0;
    if let Some(notes) = NOTE_RE.as_ref() {
        for found in notes.find_iter(line) {
            run.feed(&line[from..found.start()]);
            from = found.end();
            if found.as_str().starts_with("\\f") {
                run.note(found.as_str());
            }
        }
    }
    run.feed(&line[from..]);
    run.flush();
    *italic = run.italic;
    drop_leading_pilcrow(&mut run.segments);
    run.segments
}

/// A leading `¶` goes AFTER the markers: it sits behind `\wj` in the words of
/// Jesus.
fn drop_leading_pilcrow(segments: &mut [MarkupSegment]) {
    let first = segments
        .iter_mut()
        .find(|segment| segment.kind.is_text() && !segment.text.trim().is_empty());
    if let Some(first) = first {
        if let Some(rest) = first.text.trim_start().strip_prefix('¶') {
            first.text = rest.trim_start().to_string();
        }
    }
}

#[derive(Default)]
struct Run {
    segments: Vec<MarkupSegment>,
    text: String,
    strongs: Vec<String>,
    italic: u8,
}

impl Run {
    fn flush(&mut self) {
        if self.text.is_empty() {
            // The number came after a break in its word: it is that word's.
            let word = self.segments.iter_mut().rev().find(|s| s.kind.is_text());
            if let Some(word) = word {
                word.strongs.append(&mut self.strongs);
            }
            self.strongs.clear();
            return;
        }
        self.segments.push(MarkupSegment {
            text: std::mem::take(&mut self.text),
            strongs: std::mem::take(&mut self.strongs),
            italic: self.italic > 0,
            ..MarkupSegment::default()
        });
    }

    fn note(&mut self, span: &str) {
        self.flush();
        let text = span
            .find("\\ft")
            .and_then(|at| span.get(at..))
            .map(|body| body.trim_end_matches("\\f*"))
            .map(plain)
            .unwrap_or_default();
        if !text.is_empty() {
            self.segments.push(MarkupSegment::note(text));
        }
    }

    fn marker(&mut self, name: &str, closing: bool) {
        match name.trim_start_matches('+') {
            "w" => self.flush(),
            "add" => {
                self.flush();
                self.italic = match closing {
                    true => self.italic.saturating_sub(1),
                    false => self.italic.saturating_add(1),
                };
            }
            _ => {}
        }
    }

    fn feed(&mut self, input: &str) {
        let mut chars = input.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\\' {
                let mut name = String::new();
                let mut closing = false;
                while let Some(&next) = chars.peek() {
                    if next == '*' {
                        closing = true;
                        chars.next();
                        break;
                    }
                    if !(next.is_ascii_alphanumeric() || next == '+') {
                        break;
                    }
                    name.push(next);
                    chars.next();
                }
                self.marker(&name, closing);
                // The space after a closing marker is the verse's own.
                if !closing {
                    while chars.next_if(|next| next.is_whitespace()).is_some() {}
                }
            } else if ch == '|' {
                let mut attributes = String::new();
                while let Some(next) = chars.next_if(|next| *next != '\\') {
                    attributes.push(next);
                }
                self.strongs.extend(strongs_of(&attributes));
            } else {
                self.text.push(ch);
            }
        }
    }
}

fn strongs_of(attributes: &str) -> Option<String> {
    let (_, value) = attributes.split_once("strong=\"")?;
    let (value, _) = value.split_once('"')?;
    normalize_strongs(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use presenter_core::bible::SegmentKind;

    fn verse(line: &str) -> String {
        plain(line)
    }

    fn word(text: &str, strongs: &str) -> MarkupSegment {
        MarkupSegment {
            text: text.to_string(),
            strongs: vec![strongs.to_string()],
            ..MarkupSegment::default()
        }
    }

    #[test]
    fn a_word_carries_its_normalized_strongs_number() {
        let line =
            r#"\w Now|strong="G1161"\w* \w faith|strong="G4102"\w* the \w God|strong="H0430"\w*"#;
        let done = finish(line_segments(line), Spacing::Plain);
        assert_eq!(done.text, "Now faith the God");
        assert_eq!(
            done.markup.expect("markup").0,
            vec![
                word("Now", "G1161"),
                MarkupSegment::text(" "),
                word("faith", "G4102"),
                MarkupSegment::text(" the "),
                word("God", "H430"),
            ]
        );
    }

    #[test]
    fn a_nested_word_inside_a_divine_name_keeps_its_number() {
        let done = finish(
            line_segments(r#"\nd \+w LORD|strong="H3068"\+w*\nd* said"#),
            Spacing::Plain,
        );
        assert_eq!(done.text, "LORD said");
        assert_eq!(done.markup.expect("markup").0[0], word("LORD", "H3068"));
    }

    #[test]
    fn add_is_italic_also_with_a_word_inside() {
        let segments = line_segments(r#"river \add is \+w it|strong="H1"\+w*\add* there"#);
        let italic: Vec<_> = segments
            .iter()
            .filter(|s| s.italic)
            .map(|s| s.text.as_str())
            .collect();
        assert_eq!(italic, ["is ", "it"]);
        assert!(!segments.last().expect("segment").italic);
    }

    #[test]
    fn a_footnote_becomes_a_note_with_only_its_ft_text() {
        let line = r"years:\f + \fr 1.14 \ft the day…: Heb. \+nd LORD\+nd* between\f*  and";
        let done = finish(line_segments(line), Spacing::Plain);
        assert_eq!(done.text, "years: and");
        let markup = done.markup.expect("markup");
        let note = markup
            .0
            .iter()
            .find(|s| s.kind == SegmentKind::Note)
            .expect("note");
        assert_eq!(note.text, "the day…: Heb. LORD between");
    }

    #[test]
    fn a_cross_reference_goes_with_its_text() {
        let done = finish(
            line_segments(r"word\x - \xo 1.1 \xt Jn 1.1\x* rest"),
            Spacing::Plain,
        );
        assert_eq!(done.text, "word rest");
        assert!(done.markup.is_none());
    }

    #[test]
    fn an_open_note_stays_in_the_text() {
        assert_eq!(verse(r"an open \f + note"), "an open + note");
    }

    #[test]
    fn a_marker_ends_at_its_star() {
        assert_eq!(
            verse(r#"\w afrai|strong="G5399"\w*d \w wondered|strong="G2296"\w*"#),
            "afraid wondered"
        );
    }

    #[test]
    fn a_pilcrow_goes_only_at_the_start() {
        assert_eq!(verse(r"\wj ¶ For God\wj*"), "For God");
        assert_eq!(verse("For ¶ God"), "For ¶ God");
    }

    #[test]
    fn markers_and_whitespace_leave_plain_words() {
        assert_eq!(verse("Hello world"), "Hello world");
        assert_eq!(verse(r"\v 1 In the beginning"), "1 In the beginning");
        assert_eq!(verse("  hello   world  "), "hello world");
        assert_eq!(verse(""), "");
    }

    #[test]
    fn a_title_word_keeps_its_number() {
        let done = heading(r#"\d A \w Psalm|strong="H4210"\w* of David"#);
        assert!(done.iter().all(|s| s.kind == SegmentKind::Heading));
        assert_eq!(done[1].strongs, ["H4210"]);
        let text: String = done.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(text, "A Psalm of David");
    }

    #[test]
    fn nested_add_stays_italic_until_the_outer_end() {
        let segments = line_segments(r"\add a \+add b\+add* c\add* d");
        let italic: Vec<_> = segments.iter().map(|s| (s.text.trim(), s.italic)).collect();
        assert_eq!(
            italic,
            [("a", true), ("b", true), ("c", true), ("d", false)]
        );
    }

    #[test]
    fn add_can_span_two_lines_of_a_verse() {
        let mut depth = 0;
        let first = verse_line(r"\q1 \add word", &mut depth);
        let second = verse_line(r"\q2 more\add* rest", &mut depth);
        assert!(first[0].italic);
        assert!(second[0].italic);
        assert!(!second[1].italic);
        assert_eq!(depth, 0);
    }

    #[test]
    fn a_note_inside_a_word_leaves_the_number_on_the_word() {
        let segments = line_segments(r#"\w a\f + \fr 1.1 \ft x\f*|strong="G1"\w*"#);
        assert_eq!(segments[0], word("a", "G1"));
        assert_eq!(segments[1].kind, SegmentKind::Note);
    }

    #[test]
    fn a_heading_is_its_words() {
        let done = heading(r"\s1 \tl  ב BETH.\tl*  ");
        assert_eq!(done, [MarkupSegment::heading("ב BETH.")]);
        assert!(heading(r"\s").is_empty());
    }

    #[test]
    fn heading_lines_are_recognised_by_the_marker_name() {
        for line in [
            r"\s1 Written to the Hebrews from Italy by Timothy.",
            r"\s1 \tl  ב BETH.\tl*",
            r"\d A Psalm of David.",
            r"\ms1 The Epistle of Jeremy",
            r"\qa Aleph",
            r"\s",
        ] {
            assert!(is_heading_line(line), "{line}");
        }
        for line in [
            r"\v 1 In the beginning",
            r"\q1",
            r"\p",
            r"\sig Paul",
            r"\s1x",
            "text",
        ] {
            assert!(!is_heading_line(line), "{line}");
        }
    }

    #[test]
    fn every_pattern_compiles() {
        assert!(NOTE_RE.is_some());
    }
}
