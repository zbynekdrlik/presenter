//! Validator for AI-submitted bible slide content.
//!
//! The AI agent can call `create_bible_presentation`, `add_bible_slide`, and
//! `update_bible_slide` with arbitrary strings for `main` and `main_reference`.
//! Before PR #236 there was no validator and the agent shipped malformed slides
//! (missing verse number prefixes, reference format without parentheses, raw
//! `##bold##` markers). This module enforces four rules that the agent's
//! dispatch path must call on every slide before any DB write.
//!
//! The validator is pure: no `AppState`, no DB, no IO. Trivial to unit test
//! and mutation test. See
//! `docs/superpowers/specs/2026-04-11-ai-bible-slide-validation-design.md`.

use regex::Regex;
use std::sync::LazyLock;

/// The set of rules that can fail validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationRule {
    ReferenceFormatRequiresParens,
    MissingVerseNumberPrefix,
    UnprocessedBoldMarkers,
    EmptyMainOnEmphasisSlide,
}

impl ValidationRule {
    /// snake_case string used in error JSON sent back to the LLM.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ReferenceFormatRequiresParens => "reference_format_requires_parens",
            Self::MissingVerseNumberPrefix => "missing_verse_number_prefix",
            Self::UnprocessedBoldMarkers => "unprocessed_bold_markers",
            Self::EmptyMainOnEmphasisSlide => "empty_main_on_emphasis_slide",
        }
    }

    /// Human-readable explanation included in the error so the LLM can self-
    /// correct on retry. These strings are part of the tool-result contract;
    /// changing them is a breaking change for the LLM's prompt memory.
    pub fn expected(&self) -> &'static str {
        match self {
            Self::ReferenceFormatRequiresParens => {
                "Format is \"Book Chapter:Verse(-Verse) (CODE)\" with parens \
                 around the translation code, or omit the code entirely. \
                 Correct: \"Židom 4:13 (SEB)\" or \"Židom 4:13\"."
            }
            Self::MissingVerseNumberPrefix => {
                "Verse slides must start each verse line with its verse \
                 number: \"13. A nieto tvora...\". Multi-verse slides use \
                 one line per verse, each with its number."
            }
            Self::UnprocessedBoldMarkers => {
                "Strip ## markers from slide text. ##word## inside a verse \
                 becomes WORD in uppercase: \"1. aby sme VERILI menu\". \
                 ##phrase## on a standalone line becomes a separate emphasis \
                 slide with main = phrase in uppercase and empty \
                 main_reference."
            }
            Self::EmptyMainOnEmphasisSlide => {
                "Emphasis or title slides must have non-empty main text. \
                 An empty slide is not allowed."
            }
        }
    }
}

/// A validation failure — tells the LLM exactly what's wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    pub rule: ValidationRule,
    pub got: String,
}

impl ValidationError {
    pub fn new(rule: ValidationRule, got: impl Into<String>) -> Self {
        Self {
            rule,
            got: got.into(),
        }
    }

    /// Serialize to the JSON shape the tool dispatch path returns as the
    /// tool-result content. The LLM sees this on its next iteration.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "error": "slide_validation",
            "rule": self.rule.as_str(),
            "got": self.got,
            "expected": self.rule.expected(),
        })
    }
}

// Rule 1 regex: "Book Ch:V(-V)?( (CODE))?".
//
// - `^[A-Za-zÀ-ž0-9\. ]+ ` — book name + trailing space. Includes Slovak
//   diacritics and digits for "1. Samuelova".
// - `\d+:\d+[a-z]?` — chapter:verse, optional partial letter ("3a").
// - `(-\d+[a-z]?)?` — optional verse range end.
// - `( \([A-Z]+\))?` — optional translation code in parens.
// - `$` — anchored.
static REFERENCE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[A-Za-zÀ-ž0-9\. ]+ \d+:\d+[a-z]?(-\d+[a-z]?)?( \([A-Z]+\))?$")
        .expect("reference regex is valid")
});

// Rule 2 regex: multi-line mode, match any line starting with "N. ".
static VERSE_PREFIX_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\d+\. ").expect("verse prefix regex is valid"));

/// Validate a single bible slide's `main` and `main_reference` strings.
///
/// Rules:
/// - **Rule 3 (no raw bold markers)** applies to every slide: neither
///   `main` nor `main_reference` may contain `##`.
/// - If `main_reference` is empty (emphasis/title slide): `main` must be
///   non-empty after trimming. Rules 1 and 2 are skipped.
/// - If `main_reference` is non-empty (verse slide):
///   - **Rule 1 (reference format)**: `main_reference` must match
///     `Book Ch:V(-V)?( (CODE))?`.
///   - **Rule 2 (verse number prefix)**: `main` must contain at least one
///     line starting with `\d+\. `.
pub fn validate_bible_slide(main: &str, main_reference: &str) -> Result<(), ValidationError> {
    // Rule 3 — no raw bold markers (applies to every slide).
    if main.contains("##") {
        return Err(ValidationError::new(
            ValidationRule::UnprocessedBoldMarkers,
            main.to_string(),
        ));
    }
    if main_reference.contains("##") {
        return Err(ValidationError::new(
            ValidationRule::UnprocessedBoldMarkers,
            main_reference.to_string(),
        ));
    }

    if main_reference.is_empty() {
        // Emphasis / title slide — only rule: main non-empty.
        if main.trim().is_empty() {
            return Err(ValidationError::new(
                ValidationRule::EmptyMainOnEmphasisSlide,
                main.to_string(),
            ));
        }
        return Ok(());
    }

    // Verse slide — rule 1 (reference format).
    if !REFERENCE_RE.is_match(main_reference) {
        return Err(ValidationError::new(
            ValidationRule::ReferenceFormatRequiresParens,
            main_reference.to_string(),
        ));
    }

    // Rule 2 (verse number prefix).
    if !VERSE_PREFIX_RE.is_match(main) {
        return Err(ValidationError::new(
            ValidationRule::MissingVerseNumberPrefix,
            main.to_string(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Rule 1: reference format --

    #[test]
    fn reference_format_accepts_standard_range_with_code() {
        assert!(validate_bible_slide("1. Na počiatku bolo Slovo...", "Ján 1:1-51 (MIL)").is_ok());
    }

    #[test]
    fn reference_format_accepts_partial_verse_letter() {
        assert!(validate_bible_slide("3. Lebo tvoja milosť...", "Žalm 26:3a (ROH)").is_ok());
    }

    #[test]
    fn reference_format_accepts_single_verse_with_code() {
        assert!(validate_bible_slide("16. Lebo tak Boh miloval...", "Ján 3:16 (SEB)").is_ok());
    }

    #[test]
    fn reference_format_accepts_missing_code() {
        // User said: if AI doesn't know the translation, omit (CODE) entirely.
        assert!(validate_bible_slide("16. Lebo tak Boh miloval...", "Ján 3:16").is_ok());
    }

    #[test]
    fn reference_format_accepts_numbered_book() {
        assert!(validate_bible_slide(
            "33. Dávid povedal Saulovi...",
            "1. Samuelova 17:33-37 (SEB)"
        )
        .is_ok());
    }

    #[test]
    fn reference_format_rejects_code_without_parens() {
        // The exact production bug.
        let err = validate_bible_slide("13. A nieto tvora...", "Židom 4:13 SEB").unwrap_err();
        assert_eq!(err.rule, ValidationRule::ReferenceFormatRequiresParens);
        assert_eq!(err.got, "Židom 4:13 SEB");
    }

    #[test]
    fn reference_format_rejects_lowercase_code() {
        let err = validate_bible_slide("16. Lebo tak Boh...", "Ján 3:16 (seb)").unwrap_err();
        assert_eq!(err.rule, ValidationRule::ReferenceFormatRequiresParens);
    }

    #[test]
    fn reference_format_rejects_missing_chapter_colon() {
        let err = validate_bible_slide("1. Na počiatku...", "Ján 3 (SEB)").unwrap_err();
        assert_eq!(err.rule, ValidationRule::ReferenceFormatRequiresParens);
    }

    // -- Rule 2: verse number prefix --

    #[test]
    fn verse_prefix_accepts_single_verse_main() {
        assert!(validate_bible_slide("1. Na počiatku bolo Slovo...", "Ján 1:1 (MIL)").is_ok());
    }

    #[test]
    fn verse_prefix_accepts_multiline_main() {
        let main = "1. Na počiatku bolo Slovo.\n2. Ono bolo na počiatku.\n3. Všetko vzniklo.";
        assert!(validate_bible_slide(main, "Ján 1:1-3 (MIL)").is_ok());
    }

    #[test]
    fn verse_prefix_accepts_double_digit_verse() {
        assert!(
            validate_bible_slide("13. A nieto tvora, čo by bol...", "Židom 4:13 (SEB)").is_ok()
        );
    }

    #[test]
    fn verse_prefix_rejects_plain_text_main() {
        // The exact production bug — text has no "N. " prefix but reference is set.
        let err =
            validate_bible_slide("A nieto tvora, čo by bol...", "Židom 4:13 (SEB)").unwrap_err();
        assert_eq!(err.rule, ValidationRule::MissingVerseNumberPrefix);
    }

    // -- Rule 3: no raw bold markers --

    #[test]
    fn bold_markers_rejected_in_main() {
        let err =
            validate_bible_slide("1. aby sme ##verili## menu...", "Ján 1:12 (MIL)").unwrap_err();
        assert_eq!(err.rule, ValidationRule::UnprocessedBoldMarkers);
        assert!(err.got.contains("##verili##"));
    }

    #[test]
    fn bold_markers_rejected_in_reference() {
        let err = validate_bible_slide("1. test", "##Ján 1:1##").unwrap_err();
        assert_eq!(err.rule, ValidationRule::UnprocessedBoldMarkers);
    }

    #[test]
    fn bold_markers_accepted_when_stripped() {
        // Correct handling: ##verili## became VERILI in caps.
        assert!(validate_bible_slide("1. aby sme VERILI menu jeho Syna", "Ján 1:12 (MIL)").is_ok());
    }

    // -- Rule 4: emphasis slides --

    #[test]
    fn emphasis_slide_empty_reference_skips_verse_number_rule() {
        assert!(validate_bible_slide("NOVÁ ZMLUVA", "").is_ok());
    }

    #[test]
    fn emphasis_slide_still_rejects_bold_markers() {
        let err = validate_bible_slide("##NOVÁ ZMLUVA##", "").unwrap_err();
        assert_eq!(err.rule, ValidationRule::UnprocessedBoldMarkers);
    }

    #[test]
    fn emphasis_slide_rejects_empty_main() {
        let err = validate_bible_slide("", "").unwrap_err();
        assert_eq!(err.rule, ValidationRule::EmptyMainOnEmphasisSlide);
    }

    #[test]
    fn emphasis_slide_rejects_whitespace_only_main() {
        let err = validate_bible_slide("   \n  ", "").unwrap_err();
        assert_eq!(err.rule, ValidationRule::EmptyMainOnEmphasisSlide);
    }

    // -- Error JSON shape (contract with LLM) --

    #[test]
    fn error_json_has_stable_shape() {
        let err = ValidationError::new(
            ValidationRule::ReferenceFormatRequiresParens,
            "Židom 4:13 SEB",
        );
        let json = err.to_json();
        assert_eq!(json["error"], "slide_validation");
        assert_eq!(json["rule"], "reference_format_requires_parens");
        assert_eq!(json["got"], "Židom 4:13 SEB");
        assert!(json["expected"].as_str().unwrap().contains("parens"));
    }
}
