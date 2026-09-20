//! Validator for AI-submitted bible slide content.
//!
//! The AI agent can call `create_bible_presentation`, `add_bible_slide`, and
//! `update_bible_slide` with arbitrary strings for `main` and `main_reference`.
//! Before PR #236 there was no validator and the agent shipped malformed slides
//! (missing verse number prefixes, reference format without parentheses, raw
//! `##bold##` markers). This module enforces five rules that the agent's
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
    MainExceedsCharacterLimit,
}

impl ValidationRule {
    /// snake_case string used in error JSON sent back to the LLM.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ReferenceFormatRequiresParens => "reference_format_requires_parens",
            Self::MissingVerseNumberPrefix => "missing_verse_number_prefix",
            Self::UnprocessedBoldMarkers => "unprocessed_bold_markers",
            Self::EmptyMainOnEmphasisSlide => "empty_main_on_emphasis_slide",
            Self::MainExceedsCharacterLimit => "main_exceeds_character_limit",
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
                 Ranges and non-contiguous verses may be listed: \
                 \"Daniel 10:2-3, 12-14 (ROH)\". \
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
            Self::MainExceedsCharacterLimit => {
                "Slide main text exceeds the character limit. The server composes \
                 slides from your verse items automatically. A LONE whole verse \
                 over the limit is accepted as-is (autofit shrinks it) — do NOT \
                 split a single verse. This error means the slide over-packed \
                 MULTIPLE distinct verses, or an emphasis/title slide is too long. \
                 Recovery: submit each verse as its own item and let the server \
                 pack them, or shorten the emphasis/title text."
            }
        }
    }
}

/// A validation failure — tells the LLM exactly what's wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    pub rule: ValidationRule,
    pub got: String,
    pub limit: Option<u32>,
}

impl ValidationError {
    pub fn new(rule: ValidationRule, got: impl Into<String>) -> Self {
        Self {
            rule,
            got: got.into(),
            limit: None,
        }
    }

    pub fn new_with_limit(rule: ValidationRule, got: impl Into<String>, limit: u32) -> Self {
        Self {
            rule,
            got: got.into(),
            limit: Some(limit),
        }
    }

    /// Serialize to the JSON shape the tool dispatch path returns as the
    /// tool-result content. The LLM sees this on its next iteration.
    ///
    /// For `MainExceedsCharacterLimit` with a known limit, the `expected`
    /// string is interpolated with the actual limit number so the LLM sees
    /// "exceeds the character limit (320 characters)" not a generic message.
    pub fn to_json(&self) -> serde_json::Value {
        let mut obj = serde_json::json!({
            "error": "slide_validation",
            "rule": self.rule.as_str(),
            "got": self.got,
            "expected": self.rule.expected(),
        });
        if let Some(limit) = self.limit {
            obj["limit"] = serde_json::json!(limit);
            if self.rule == ValidationRule::MainExceedsCharacterLimit {
                let with_limit = format!(
                    "Slide main text exceeds the character limit ({limit} characters). \
                     The server composes slides from your verse items automatically. A LONE \
                     whole verse over the limit is accepted as-is (autofit shrinks it) — do \
                     NOT split a single verse. This error means the slide over-packed MULTIPLE \
                     distinct verses, or an emphasis/title slide is too long. Recovery: submit \
                     each verse as its own item and let the server pack them, or shorten the \
                     emphasis/title text."
                );
                obj["expected"] = serde_json::json!(with_limit);
            }
        }
        obj
    }
}

// Rule 1 (reference format) is enforced by [`normalize_reference`] below (#784),
// which ACCEPTS and canonicalises every well-formed reference the domain
// produces (multi-range comma-lists, lowercase code, duplicated chapter) rather
// than the old single-range/uppercase-only `REFERENCE_RE` regex that rejected
// them and looped the agent. Its component regexes are defined just below.

// Rule 2 regex: multi-line mode, match any line starting with "N. ".
//
// `Regex::new(...).ok()` yields `None` only if this literal regex is malformed —
// a programmer bug which the unit tests in this module catch immediately
// (callers fail closed on `None`). It is effectively unreachable in production
// because the pattern is a compile-time constant.
static VERSE_PREFIX_RE: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"(?m)^\d+\. ").ok());

// --- #784 reference NORMALISATION regexes ---
//
// Same fallible-init + fail-closed rationale as `VERSE_PREFIX_RE`: each literal
// is a compile-time constant caught by this module's tests; callers fall back to
// rejecting on `None`.

// Split a trailing " (CODE)" off the reference. Case-insensitive so a lowercase
// code (`(roh)`) is captured and later upper-cased. Group 1 = the body, group 2
// = the code letters. A code NOT wrapped in parens (`… SEB`) never matches, so
// it stays part of the body and is rejected downstream (contract kept stable).
static REF_CODE_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(?i)^(.*?)\s*\(\s*([a-z]+)\s*\)\s*$").ok());

// Split the body into "book" + "chapter:verse-spec" at the FIRST `\d+:` token.
// `(.+?)` is non-greedy so the book stops at the first `<space><chapter>:`.
static REF_BOOK_SPEC_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"^(.+?)\s+(\d+\s*:.*)$").ok());

// A valid book name: Unicode letters, digits, dots, spaces only — `\p{L}` is
// the Unicode letter class (Slovak/Czech/other scripts) plus `0-9` for numbered
// books like "1. Samuelova"; it excludes symbols like `×` (U+00D7) / `÷`.
static REF_BOOK_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"^[\p{L}0-9.][\p{L}0-9. ]*$").ok());

// The book must contain at least one letter (rejects an all-digits "book").
static REF_BOOK_LETTER_RE: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"\p{L}").ok());

// A single verse-spec token: optional `chapter:` prefix, a verse (with optional
// `a`/`b` partial letter), and an optional `-verse` range end.
static REF_TOKEN_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"^(?:(\d+):)?(\d+[a-z]?)(?:-(\d+[a-z]?))?$").ok());

// Collapse whitespace around `:` and `-` inside the verse spec.
static REF_COLON_WS_RE: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"\s*:\s*").ok());
static REF_DASH_WS_RE: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"\s*-\s*").ok());

/// Canonicalise a well-formed bible reference, or reject genuinely malformed
/// text with [`ValidationRule::ReferenceFormatRequiresParens`] (#784).
///
/// Accepts and normalises every reference the domain produces — including the
/// composer's own non-contiguous comma-list output
/// (`state/slides/compose.rs::format_verse_range`, e.g. `Numeri 13:1, 3, 5`) and
/// the loose forms the model emitted at PP:
///
/// - `Daniel 10:2, 3, 12, 13, 14 (ROH)` → unchanged (already canonical)
/// - `Daniel 10:2-3 (roh)` → `Daniel 10:2-3 (ROH)` (code upper-cased)
/// - `Daniel 10:12-14 10:12 (ROH)` → `Daniel 10:12-14, 12 (ROH)` (dup chapter merged)
/// - stray leading/trailing/inner whitespace collapsed
///
/// Canonical form: `Book C:V[a](-V[a])?(, V[a](-V[a])?)*( (CODE))?`. A verse
/// segment that re-prefixes the SAME chapter drops its `C:`; a genuinely
/// different chapter keeps it. Rejects: no book, no `C:V`, a non-letter book
/// character, a code not wrapped in parens.
pub fn normalize_reference(reference: &str) -> Result<String, ValidationError> {
    let reject = || ValidationError::new(ValidationRule::ReferenceFormatRequiresParens, reference);

    let (code_re, book_spec_re, book_re, letter_re, token_re, colon_re, dash_re) = match (
        REF_CODE_RE.as_ref(),
        REF_BOOK_SPEC_RE.as_ref(),
        REF_BOOK_RE.as_ref(),
        REF_BOOK_LETTER_RE.as_ref(),
        REF_TOKEN_RE.as_ref(),
        REF_COLON_WS_RE.as_ref(),
        REF_DASH_WS_RE.as_ref(),
    ) {
        (Some(a), Some(b), Some(c), Some(d), Some(e), Some(f), Some(g)) => (a, b, c, d, e, f, g),
        // A literal regex failed to compile (programmer bug, caught by tests) —
        // fail closed rather than silently accepting.
        _ => return Err(reject()),
    };

    let trimmed = reference.trim();

    // Split off an optional trailing "(CODE)"; upper-case it.
    let (body, code) = match code_re.captures(trimmed) {
        Some(caps) => (
            caps.get(1).map_or("", |m| m.as_str()).trim().to_string(),
            Some(caps.get(2).map_or("", |m| m.as_str()).to_uppercase()),
        ),
        None => (trimmed.to_string(), None),
    };

    // Split "book" from "chapter:verse-spec".
    let caps = book_spec_re.captures(&body).ok_or_else(reject)?;
    let book = caps.get(1).map_or("", |m| m.as_str()).trim();
    let spec = caps.get(2).map_or("", |m| m.as_str());

    if !book_re.is_match(book) || !letter_re.is_match(book) {
        return Err(reject());
    }

    // Collapse whitespace around ':' and '-', then split into verse tokens on
    // any comma/whitespace run.
    let spec = colon_re.replace_all(spec, ":");
    let spec = dash_re.replace_all(&spec, "-");
    let tokens: Vec<&str> = spec
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.is_empty() {
        return Err(reject());
    }

    let mut segments: Vec<String> = Vec::with_capacity(tokens.len());
    let mut first_chapter: Option<u32> = None;
    let mut cur_chapter: u32 = 0;

    for (i, token) in tokens.iter().enumerate() {
        let tc = token_re.captures(token).ok_or_else(reject)?;
        let chapter = match tc.get(1) {
            Some(m) => Some(m.as_str().parse::<u32>().map_err(|_| reject())?),
            None => None,
        };
        // Verse (with optional partial letter) + optional range end.
        let verse = tc.get(2).map_or("", |m| m.as_str());
        let verse_seg = match tc.get(3) {
            Some(end) => format!("{verse}-{}", end.as_str()),
            None => verse.to_string(),
        };

        if i == 0 {
            // The first token MUST anchor the chapter.
            let ch = chapter.ok_or_else(reject)?;
            first_chapter = Some(ch);
            cur_chapter = ch;
            segments.push(verse_seg);
        } else {
            match chapter {
                Some(ch) if ch != cur_chapter => {
                    cur_chapter = ch;
                    segments.push(format!("{ch}:{verse_seg}"));
                }
                // Same chapter re-prefix, or no chapter → verse only.
                _ => segments.push(verse_seg),
            }
        }
    }

    let chapter = first_chapter.ok_or_else(reject)?;
    let mut result = format!("{book} {chapter}:{}", segments.join(", "));
    if let Some(code) = code {
        result.push_str(&format!(" ({code})"));
    }
    Ok(result)
}

/// True when `main` is a single whole verse on a verse slide — i.e. it has a
/// non-empty `main_reference` (so it is a verse, not an emphasis/title slide)
/// and exactly ONE line begins with a verse-number prefix (`\d+\. `). Such a
/// slide is kept whole even when it exceeds the character limit (issue #394);
/// autofit shrinks it for display. Two or more verse-prefixed lines means the
/// slide over-packs multiple verses, which is still a genuine packing error.
fn is_lone_whole_verse(main: &str, main_reference: &str) -> bool {
    if main_reference.is_empty() {
        return false;
    }
    let Some(re) = VERSE_PREFIX_RE.as_ref() else {
        return false;
    };
    main.lines().filter(|line| re.is_match(line)).count() == 1
}

/// Validate a single bible slide's `main` and `main_reference` strings.
///
/// Rules:
/// - **Rule 5 (character limit)** applies to every slide: `main` must not
///   exceed `character_limit` bytes. Checked first — cheap, common, fail-fast.
/// - **Rule 3 (no raw bold markers)** applies to every slide: neither
///   `main` nor `main_reference` may contain `##`.
/// - If `main_reference` is empty (emphasis/title slide): `main` must be
///   non-empty after trimming. Rules 1 and 2 are skipped.
/// - If `main_reference` is non-empty (verse slide):
///   - **Rule 1 (reference format)**: `main_reference` must NORMALISE via
///     [`normalize_reference`] to the canonical
///     `Book Ch:V[a](-V[a])?(, …)*( (CODE))?` form (multi-range/comma lists and
///     a lowercase code are accepted and canonicalised, not rejected — #784).
///   - **Rule 2 (verse number prefix)**: `main` must contain at least one
///     line starting with `\d+\. `.
///
/// On success returns the CANONICAL reference the caller writes back into the
/// slide (empty string for an emphasis/title slide) so Resolume gets one
/// canonical form (#784). Every failure is a [`ValidationError`].
pub fn validate_bible_slide(
    main: &str,
    main_reference: &str,
    character_limit: u32,
) -> Result<String, ValidationError> {
    // Rule 5 — length check (applies to every slide, including emphasis).
    // Cheap and common; fail fast before running any regex.
    //
    // Issue #394 exception: a LONE whole verse over the limit is NOT a packing
    // error — display autofit shrinks it. A verse slide (non-empty reference)
    // whose `main` is a single verse-number-prefixed line is accepted even when
    // it exceeds the limit. Multi-verse slides over the limit are still a real
    // over-packing error (the composer should have flushed before overflow),
    // and oversized emphasis/title slides (no reference) are still rejected.
    if main.len() > character_limit as usize && !is_lone_whole_verse(main, main_reference) {
        return Err(ValidationError::new_with_limit(
            ValidationRule::MainExceedsCharacterLimit,
            main.to_string(),
            character_limit,
        ));
    }

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
        return Ok(String::new());
    }

    // Verse slide — rule 1 (reference format): normalise to the canonical form,
    // or reject genuinely malformed text (#784).
    let normalized = normalize_reference(main_reference)?;

    // Rule 2 (verse number prefix). Fail closed if the regex is unavailable.
    if !VERSE_PREFIX_RE.as_ref().is_some_and(|re| re.is_match(main)) {
        return Err(ValidationError::new(
            ValidationRule::MissingVerseNumberPrefix,
            main.to_string(),
        ));
    }

    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Rule 1: reference format --

    #[test]
    fn reference_format_accepts_standard_range_with_code() {
        assert!(
            validate_bible_slide("1. Na počiatku bolo Slovo...", "Ján 1:1-51 (MIL)", 320).is_ok()
        );
    }

    #[test]
    fn reference_format_accepts_partial_verse_letter() {
        assert!(validate_bible_slide("3. Lebo tvoja milosť...", "Žalm 26:3a (ROH)", 320).is_ok());
    }

    #[test]
    fn reference_format_accepts_single_verse_with_code() {
        assert!(validate_bible_slide("16. Lebo tak Boh miloval...", "Ján 3:16 (SEB)", 320).is_ok());
    }

    #[test]
    fn reference_format_accepts_missing_code() {
        // User said: if AI doesn't know the translation, omit (CODE) entirely.
        assert!(validate_bible_slide("16. Lebo tak Boh miloval...", "Ján 3:16", 320).is_ok());
    }

    #[test]
    fn reference_format_accepts_numbered_book() {
        assert!(validate_bible_slide(
            "33. Dávid povedal Saulovi...",
            "1. Samuelova 17:33-37 (SEB)",
            320
        )
        .is_ok());
    }

    #[test]
    fn reference_format_rejects_code_without_parens_production_bug() {
        // Regression guard for the exact reference produced by the AI in
        // production before this validator shipped.
        let err = validate_bible_slide("13. A nieto tvora...", "Židom 4:13 SEB", 320).unwrap_err();
        assert_eq!(err.rule, ValidationRule::ReferenceFormatRequiresParens);
        assert_eq!(err.got, "Židom 4:13 SEB");
    }

    #[test]
    fn reference_format_rejects_unicode_symbols_in_book_name() {
        // Regression guard for the earlier character class `[À-ž]` which
        // accidentally admitted U+00D7 (`×`) and U+00F7 (`÷`). Switching
        // to `\p{L}` excludes all Unicode symbols.
        let err = validate_bible_slide("1. test", "Bo×k 1:1 (MIL)", 320).unwrap_err();
        assert_eq!(err.rule, ValidationRule::ReferenceFormatRequiresParens);
        let err2 = validate_bible_slide("1. test", "Bo÷k 1:1 (MIL)", 320).unwrap_err();
        assert_eq!(err2.rule, ValidationRule::ReferenceFormatRequiresParens);
    }

    #[test]
    fn reference_format_normalizes_lowercase_code() {
        // #784: a lowercase translation code is no longer REJECTED — it is
        // upper-cased and written back. (Was `reference_format_rejects_lowercase_code`;
        // the design deliberately reverses this — a lowercase code is well-formed,
        // just non-canonical, so it normalises instead of looping the agent.)
        assert_eq!(
            validate_bible_slide("16. Lebo tak Boh...", "Ján 3:16 (seb)", 320).unwrap(),
            "Ján 3:16 (SEB)"
        );
    }

    // -- Rule 1 NORMALISATION (#784) --
    //
    // The PP loop (2026-09-20) was the validator REJECTING well-formed refs the
    // composer (`state/slides/compose.rs::format_verse_range`) and Gemini
    // actually produce: non-contiguous comma-lists, a lowercase code, a stray
    // duplicated chapter token. They must now NORMALISE to one canonical form,
    // not reject — while genuine garbage still fails.

    #[test]
    fn normalize_accepts_the_pp_multi_range_comma_list() {
        // The exact string the composer emitted at PP for Daniel 10:2-3, 12-14
        // (ROH). Already canonical → unchanged.
        assert_eq!(
            normalize_reference("Daniel 10:2, 3, 12, 13, 14 (ROH)").unwrap(),
            "Daniel 10:2, 3, 12, 13, 14 (ROH)"
        );
    }

    #[test]
    fn normalize_upper_cases_a_lowercase_translation_code() {
        assert_eq!(
            normalize_reference("Daniel 10:2-3 (roh)").unwrap(),
            "Daniel 10:2-3 (ROH)"
        );
    }

    #[test]
    fn normalize_merges_a_duplicated_chapter_token() {
        // "10:12-14 10:12" — a stray re-prefixed SAME chapter, space-separated;
        // the second chapter prefix drops and the verse joins the list.
        assert_eq!(
            normalize_reference("Daniel 10:12-14 10:12 (ROH)").unwrap(),
            "Daniel 10:12-14, 12 (ROH)"
        );
    }

    #[test]
    fn normalize_collapses_stray_whitespace() {
        assert_eq!(
            normalize_reference("  Daniel   10:2 ,  3 ,  12  (ROH)  ").unwrap(),
            "Daniel 10:2, 3, 12 (ROH)"
        );
    }

    #[test]
    fn normalize_is_idempotent_on_already_canonical_refs() {
        for r in [
            "Ján 1:1-51 (MIL)",
            "Žalm 26:3a (ROH)",
            "Ján 3:16 (SEB)",
            "Ján 3:16",
            "1. Samuelova 17:33-37 (SEB)",
            "Numeri 13:1, 3, 5 (SEB)",
        ] {
            assert_eq!(
                normalize_reference(r).unwrap(),
                r,
                "must be idempotent: {r}"
            );
        }
    }

    #[test]
    fn normalize_rejects_genuine_garbage() {
        // No book, no chapter:verse, empty, missing colon, code without parens,
        // a Unicode symbol in the book name — all still rejected.
        for g in [
            "Daniel",
            "10:2",
            "",
            "Ján 3 (SEB)",
            "Židom 4:13 SEB",
            "Bo×k 1:1 (MIL)",
            "Ján 3:x (SEB)",
        ] {
            let err = normalize_reference(g).unwrap_err();
            assert_eq!(
                err.rule,
                ValidationRule::ReferenceFormatRequiresParens,
                "must reject: {g:?}"
            );
        }
    }

    #[test]
    fn validate_writes_back_the_normalised_reference() {
        // validate_bible_slide now RETURNS the canonical reference so the caller
        // (create_bible_presentation) writes it back into the slide — Resolume
        // then gets one canonical form.
        let normalized = validate_bible_slide("2. text\n3. text", "Daniel 10:2-3 (roh)", 320)
            .expect("a well-formed multi-range ref must be accepted");
        assert_eq!(normalized, "Daniel 10:2-3 (ROH)");
    }

    #[test]
    fn validate_returns_empty_reference_for_emphasis_slide() {
        assert_eq!(validate_bible_slide("NOVÁ ZMLUVA", "", 320).unwrap(), "");
    }

    #[test]
    fn reference_format_rejects_missing_chapter_colon() {
        let err = validate_bible_slide("1. Na počiatku...", "Ján 3 (SEB)", 320).unwrap_err();
        assert_eq!(err.rule, ValidationRule::ReferenceFormatRequiresParens);
    }

    // -- Rule 2: verse number prefix --

    #[test]
    fn verse_prefix_accepts_single_verse_main() {
        assert!(validate_bible_slide("1. Na počiatku bolo Slovo...", "Ján 1:1 (MIL)", 320).is_ok());
    }

    #[test]
    fn verse_prefix_accepts_multiline_main() {
        let main = "1. Na počiatku bolo Slovo.\n2. Ono bolo na počiatku.\n3. Všetko vzniklo.";
        assert!(validate_bible_slide(main, "Ján 1:1-3 (MIL)", 320).is_ok());
    }

    #[test]
    fn verse_prefix_accepts_double_digit_verse() {
        assert!(
            validate_bible_slide("13. A nieto tvora, čo by bol...", "Židom 4:13 (SEB)", 320)
                .is_ok()
        );
    }

    #[test]
    fn verse_prefix_rejects_plain_text_main_production_bug() {
        // Regression guard: AI shipped verse text with no "N. " prefix
        // while still setting a valid reference. Both conditions reproduced.
        let err = validate_bible_slide("A nieto tvora, čo by bol...", "Židom 4:13 (SEB)", 320)
            .unwrap_err();
        assert_eq!(err.rule, ValidationRule::MissingVerseNumberPrefix);
    }

    // -- Rule 3: no raw bold markers --

    #[test]
    fn bold_markers_rejected_in_main() {
        let err = validate_bible_slide("1. aby sme ##verili## menu...", "Ján 1:12 (MIL)", 320)
            .unwrap_err();
        assert_eq!(err.rule, ValidationRule::UnprocessedBoldMarkers);
        assert!(err.got.contains("##verili##"));
    }

    #[test]
    fn bold_markers_rejected_in_reference() {
        let err = validate_bible_slide("1. test", "##Ján 1:1##", 320).unwrap_err();
        assert_eq!(err.rule, ValidationRule::UnprocessedBoldMarkers);
    }

    #[test]
    fn bold_markers_accepted_when_stripped() {
        // Correct handling: ##verili## became VERILI in caps.
        assert!(
            validate_bible_slide("1. aby sme VERILI menu jeho Syna", "Ján 1:12 (MIL)", 320).is_ok()
        );
    }

    // -- Rule 4: emphasis slides --

    #[test]
    fn emphasis_slide_empty_reference_skips_verse_number_rule() {
        assert!(validate_bible_slide("NOVÁ ZMLUVA", "", 320).is_ok());
    }

    #[test]
    fn emphasis_slide_still_rejects_bold_markers() {
        let err = validate_bible_slide("##NOVÁ ZMLUVA##", "", 320).unwrap_err();
        assert_eq!(err.rule, ValidationRule::UnprocessedBoldMarkers);
    }

    #[test]
    fn emphasis_slide_rejects_empty_main() {
        let err = validate_bible_slide("", "", 320).unwrap_err();
        assert_eq!(err.rule, ValidationRule::EmptyMainOnEmphasisSlide);
    }

    #[test]
    fn emphasis_slide_rejects_whitespace_only_main() {
        let err = validate_bible_slide("   \n  ", "", 320).unwrap_err();
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

    // -- Rule 5: character limit --

    #[test]
    fn length_rule_accepts_slide_at_exactly_limit() {
        // "1. " is 3 chars; 317 "a"s brings total to 320.
        let main = format!("1. {}", "a".repeat(317));
        assert_eq!(main.len(), 320);
        assert!(validate_bible_slide(&main, "Ján 1:1 (SEB)", 320).is_ok());
    }

    #[test]
    fn length_rule_accepts_multi_verse_slide_at_exactly_limit() {
        // A MULTI-verse slide (is_lone_whole_verse is false, so it does NOT take
        // the #394 lone-verse exception) sitting EXACTLY at the limit must be
        // ACCEPTED — the rule rejects only main.len() STRICTLY GREATER than the
        // limit. This pins the boundary at `>` (not `>=`): "1. " (3) + 156 a's +
        // "\n2. " (4) + 157 b's = 320 bytes, two verse-prefixed lines.
        let main = format!("1. {}\n2. {}", "a".repeat(156), "b".repeat(157));
        assert_eq!(main.len(), 320);
        assert!(validate_bible_slide(&main, "Ján 1:1-2 (SEB)", 320).is_ok());
    }

    #[test]
    fn length_rule_accepts_lone_verse_one_char_over_limit() {
        // Issue #394: even one char over the limit, a LONE whole verse is kept
        // whole and accepted (autofit shrinks it) — it is never rejected, which
        // is what previously forced the LLM to split the verse mid-text.
        let main = format!("1. {}", "a".repeat(318));
        assert_eq!(main.len(), 321);
        assert!(validate_bible_slide(&main, "Ján 1:1 (SEB)", 320).is_ok());
    }

    #[test]
    fn length_rule_accepts_lone_oversized_verse() {
        // Issue #394: a single whole verse over the limit is autofit's job
        // (display shrink), NOT a packing error. The slide has exactly ONE
        // verse-number-prefixed line, so it must be ACCEPTED rather than
        // rejected with MainExceedsCharacterLimit — otherwise the LLM retry
        // loop is forced to split the verse mid-text.
        let main = format!("1. {}", "a".repeat(1000));
        assert!(
            validate_bible_slide(&main, "Ján 1:1 (SEB)", 320).is_ok(),
            "a lone whole verse over the limit must be accepted, not rejected"
        );
    }

    #[test]
    fn length_rule_rejects_multi_verse_slide_over_limit() {
        // A slide that over-packs MULTIPLE verses past the limit is a genuine
        // packing error (the composer should have flushed before overflow) —
        // still rejected. Two verse-number-prefixed lines, total over 320.
        let main = format!("1. {}\n2. {}", "a".repeat(200), "b".repeat(200));
        let err = validate_bible_slide(&main, "Ján 1:1-2 (SEB)", 320).unwrap_err();
        assert_eq!(err.rule, ValidationRule::MainExceedsCharacterLimit);
    }

    #[test]
    fn length_rule_applies_to_emphasis_slides() {
        let main = "a".repeat(400);
        let err = validate_bible_slide(&main, "", 320).unwrap_err();
        assert_eq!(err.rule, ValidationRule::MainExceedsCharacterLimit);
    }

    #[test]
    fn length_rule_error_json_includes_limit_and_interpolated_message() {
        let err = ValidationError::new_with_limit(
            ValidationRule::MainExceedsCharacterLimit,
            "a".repeat(400),
            320,
        );
        let json = err.to_json();
        assert_eq!(json["rule"], "main_exceeds_character_limit");
        assert_eq!(json["limit"], 320);
        assert!(
            json["expected"].as_str().unwrap().contains("320"),
            "expected text should interpolate the limit, got: {}",
            json["expected"]
        );
    }
}
