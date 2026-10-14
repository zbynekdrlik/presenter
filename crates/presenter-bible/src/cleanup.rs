//! #851 D1 import fixes (spec §5.1): the spans a verse must not carry.
//!
//! - MySword (SEB, ROH, MIL): a translator's note is `<FI>[ … ]<Fi>`; it goes,
//!   with the whitespace before it. A BARE `[ … ]` is translation text (ROH
//!   Mt 17:21 "[a pôstom]", SEB 1 Thes 3:13 "[Amen.]") and stays. A
//!   `<TS>…<Ts>` span is a section heading (ROH, 2,785 verses); it goes. After
//!   the tags are stripped, whitespace before `. , ; : ! ?` goes (the modules'
//!   own spacing around `<FR>` quotes).
//! - USFM (KJV): `\f … \f*` footnotes and `\x … \x*` cross-references go with
//!   their text, a leading `¶` paragraph mark goes, and a heading line (`\s1`,
//!   `\d`, `\ms1`, …) is never verse text (PSA 119's acrostic letters and the
//!   epistles' subscriptions were glued to the verse before them).
//!
//! D2 replaces these helpers with the markup parsers (`markup/`), which keep
//! the notes and headings as study markup instead of dropping them.

use regex::Regex;
use std::borrow::Cow;
use std::sync::LazyLock;

static MYSWORD_NOTE_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"\s*<FI>\s*\[[^\]]*\]\s*<Fi>").ok());
static MYSWORD_HEADING_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(?is)<TS>.*?<Ts>").ok());
static USFM_NOTE_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"\\f\s.*?\\f\*|\\x\s.*?\\x\*").ok());
static SPACE_BEFORE_PUNCTUATION_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"\s+([.,;:!?])").ok());

/// USFM markers whose whole line is a heading or a title, never verse text.
const USFM_HEADING_MARKERS: [&str; 13] = [
    "s", "ms", "mr", "sr", "r", "d", "sp", "mt", "mte", "cl", "cd", "qa", "h",
];

/// `re` applied to `text`; the text unchanged when the pattern failed to
/// compile (it cannot: `every_pattern_compiles` pins all four).
fn replace_all<'a>(re: &Option<Regex>, text: &'a str, with: &str) -> Cow<'a, str> {
    match re {
        Some(re) => re.replace_all(text, with),
        None => Cow::Borrowed(text),
    }
}

/// MySword: the verse without its `<FI>[ … ]<Fi>` notes and `<TS>…<Ts>`
/// headings. Runs BEFORE the tags are stripped.
pub(crate) fn strip_mysword_notes_and_headings(input: &str) -> String {
    let without_notes = replace_all(&MYSWORD_NOTE_RE, input, "");
    replace_all(&MYSWORD_HEADING_RE, &without_notes, "").into_owned()
}

/// MySword: no whitespace before `. , ; : ! ?`. Runs AFTER the tags are
/// stripped and the whitespace is normalized.
pub(crate) fn drop_space_before_punctuation(text: &str) -> String {
    replace_all(&SPACE_BEFORE_PUNCTUATION_RE, text, "$1").into_owned()
}

/// USFM: the line without `\f … \f*` footnotes and `\x … \x*`
/// cross-references. An unterminated note is left as it is.
pub(crate) fn strip_usfm_notes(line: &str) -> Cow<'_, str> {
    replace_all(&USFM_NOTE_RE, line, "")
}

/// The text without a leading `¶` paragraph mark and the whitespace after it.
pub(crate) fn strip_leading_pilcrow(text: String) -> String {
    if let Some(rest) = text.strip_prefix('¶') {
        return rest.trim_start().to_string();
    }
    text
}

/// Whether a USFM line is a heading or title paragraph (`\s1 …`, `\d …`,
/// `\ms1 …`): the marker NAME decides (letters, then optional digits, then
/// whitespace or the end of the line), so `\sig` or `\s1x` is not a heading.
pub(crate) fn is_usfm_heading_line(line: &str) -> bool {
    let Some(rest) = line.strip_prefix('\\') else {
        return false;
    };
    let name_len = rest.chars().take_while(char::is_ascii_lowercase).count();
    let (Some(name), Some(after)) = (rest.get(..name_len), rest.get(name_len..)) else {
        return false;
    };
    let after = after.trim_start_matches(|c: char| c.is_ascii_digit());
    let marker_ends = after.is_empty() || after.starts_with(char::is_whitespace);
    marker_ends && USFM_HEADING_MARKERS.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pattern_compiles() {
        assert!(MYSWORD_NOTE_RE.is_some() && MYSWORD_HEADING_RE.is_some());
        assert!(USFM_NOTE_RE.is_some() && SPACE_BEFORE_PUNCTUATION_RE.is_some());
    }

    #[test]
    fn a_mysword_note_needs_its_fi_wrapper() {
        assert_eq!(
            strip_mysword_notes_and_headings("Rieka <FI>[ Pozn.: iný názov.]<Fi>. Tečie"),
            "Rieka. Tečie"
        );
        assert_eq!(
            strip_mysword_notes_and_headings("iba modlitbou [a pôstom]."),
            "iba modlitbou [a pôstom]."
        );
        assert_eq!(
            strip_mysword_notes_and_headings("<FI>slovo<Fi> ostáva"),
            "<FI>slovo<Fi> ostáva"
        );
    }

    #[test]
    fn a_mysword_heading_goes_in_any_case() {
        assert_eq!(
            strip_mysword_notes_and_headings("<TS>Nadpis<Ts>  Text verša."),
            "  Text verša."
        );
        assert_eq!(
            strip_mysword_notes_and_headings("<ts>Iný nadpis<ts> Text."),
            " Text."
        );
    }

    #[test]
    fn whitespace_before_punctuation_goes() {
        assert_eq!(
            drop_space_before_punctuation("slovom . Pán , áno ; nie : prečo ? hej !"),
            "slovom. Pán, áno; nie: prečo? hej!"
        );
        assert_eq!(
            drop_space_before_punctuation("bez zmeny – pomlčka"),
            "bez zmeny – pomlčka"
        );
    }

    #[test]
    fn usfm_notes_go_with_their_text() {
        assert_eq!(
            strip_usfm_notes(r"and years:\f + \fr 1.14 \ft the day…: Heb. between\f*  "),
            "and years:  "
        );
        assert_eq!(
            strip_usfm_notes(r"word\x - \xo 1.1 \xt Jn 1.1\x* rest"),
            "word rest"
        );
        assert_eq!(
            strip_usfm_notes(r"an open \f + note without its end"),
            r"an open \f + note without its end"
        );
    }

    #[test]
    fn a_pilcrow_goes_only_at_the_start() {
        assert_eq!(strip_leading_pilcrow("¶ For God".to_string()), "For God");
        assert_eq!(strip_leading_pilcrow("For ¶ God".to_string()), "For ¶ God");
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
            assert!(is_usfm_heading_line(line), "{line}");
        }
        for line in [
            r"\v 1 In the beginning",
            r"\q1",
            r"\p",
            r"\sig Paul",
            r"\s1x",
            "text",
        ] {
            assert!(!is_usfm_heading_line(line), "{line}");
        }
    }
}
