# AI Bible Slide Validation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop the AI from producing malformed bible slides (missing verse numbers, reference without parens, raw `##` markers) by adding a server-side validator that rejects bad input and restoring the critical formatting rules to the system prompt.

**Architecture:** New pure module `ai/bible_validator.rs` with a `validate_bible_slide` function enforcing 4 rules. Three AI tool handlers (`create_bible_presentation`, `add_bible_slide`, `update_bible_slide`) call the validator before any DB write. System prompt in `ai/agent.rs` gains a "Creating Bible slides" block with the critical rules inlined.

**Tech Stack:** Rust (workspace edition 2021, rustc 1.94+), `regex` crate (new direct dep), `std::sync::LazyLock` for compiled regex, existing `AppState::in_memory()` test harness, Playwright for E2E.

**Spec:** `docs/superpowers/specs/2026-04-11-ai-bible-slide-validation-design.md`

---

## Context for the implementer

The user pastes sermon emails/Discord messages into the AI chat. The AI is supposed to parse them, load passages from the bible database, edit the text to match the pastor's version, and create a bible presentation. It's shipping garbage:

- `main = "A nieto tvora, čo by bol preň neviditeľný..."` — no verse number prefix
- `main_reference = "Židom 4:13 SEB"` — translation code without parentheses (invalid)
- Raw `##bold##` markers left unprocessed

PR #235 (already merged to dev at commit `006dbdc`) shrank the system prompt from 125 to 40 lines and moved detailed rules into an on-demand `get_style_guide` tool. The AI rarely calls that tool. Worse, the existing rule 3 in `build_system_prompt` literally says:

> Bible slide main_reference format: "Book Chapter:Verse TRANSLATION" (e.g. "Ján 3:16 SEB")

**This is the bug source.** The prompt tells the AI to produce references without parens. We replace it.

**Key existing code:**

- `crates/presenter-server/src/ai/tools.rs:792` — `create_bible_presentation` handler
- `crates/presenter-server/src/ai/tools.rs:862` — `add_bible_slide` handler
- `crates/presenter-server/src/ai/tools.rs:896` — `update_bible_slide` handler
- `crates/presenter-server/src/ai/agent.rs:110-153` — `build_system_prompt` format block
- `crates/presenter-server/src/ai/agent.rs:132-134` — the wrong rule 3 (will be replaced)
- `crates/presenter-server/src/state/slides.rs:18` — `compose_bible_slides` (the authoritative composer; already produces correct format)

Tests follow the existing `ai::tools::tests` pattern using `AppState::in_memory()`.

**Regex for rule 1:**

```
^[A-Za-zÀ-ž0-9\. ]+ \d+:\d+[a-z]?(-\d+[a-z]?)?( \([A-Z]+\))?$
```

Anchored with `^...$`. Accepts Slovak diacritics, numbered books (`1. Samuelova`), partial verses (`3a`), optional translation code in parens. Rejects `"Židom 4:13 SEB"` because the space-separated code at the end has no parens.

**Regex for rule 2 (verse number prefix):**

```
(?m)^\d+\. 
```

Multi-line mode, matches any line starting with digits followed by `. ` and a space. Must match at least once in `main`.

---

## File Structure

| File | Change | Responsibility |
|---|---|---|
| `crates/presenter-server/Cargo.toml` | Modify | Add `regex = "1"` to `[dependencies]` |
| `crates/presenter-server/src/ai/bible_validator.rs` | Create | Pure `validate_bible_slide` function + `ValidationError` struct + unit tests |
| `crates/presenter-server/src/ai/mod.rs` | Modify | Add `pub(crate) mod bible_validator;` |
| `crates/presenter-server/src/ai/tools.rs` | Modify | Apply validator in 3 handlers + dispatch-level tests |
| `crates/presenter-server/src/ai/agent.rs` | Modify | Replace wrong rule 3; add "Creating Bible slides" block |
| `tests/e2e/ai-bible-validation.spec.ts` | Create | Direct tool-call E2E test against dev server |

---

## Task 1: `bible_validator` module (TDD, 11 tests before implementation)

**Files:**
- Modify: `crates/presenter-server/Cargo.toml`
- Create: `crates/presenter-server/src/ai/bible_validator.rs`
- Modify: `crates/presenter-server/src/ai/mod.rs`

### Step 1: Add `regex` to `presenter-server` dependencies

- [ ] Open `crates/presenter-server/Cargo.toml` and add `regex = "1"` at the end of the `[dependencies]` block (after `async-stream = "0.3"` on line 36):

```toml
[dependencies]
anyhow.workspace = true
axum.workspace = true
presenter-core = { path = "../presenter-core" }
presenter-persistence = { path = "../presenter-persistence" }
presenter-importer = { path = "../presenter-importer" }
presenter-bible = { path = "../presenter-bible" }
presenter-ndi = { path = "../presenter-ndi" }
bytes = "1"
chrono.workspace = true
serde.workspace = true
serde_json.workspace = true
tokio.workspace = true
tower.workspace = true
tower-http.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
leptos.workspace = true
reactive_graph.workspace = true
uuid.workspace = true
thiserror.workspace = true
tokio-stream.workspace = true
futures-util.workspace = true
serde_with.workspace = true
reqwest.workspace = true
rosc = "0.10"
include_dir = "0.7"
async-stream = "0.3"
regex = "1"
```

### Step 2: Register the module

- [ ] Open `crates/presenter-server/src/ai/mod.rs` and insert a new module declaration after line 4 (`pub(crate) mod tools;`):

```rust
pub(crate) mod agent;
pub(crate) mod bible_validator;
pub(crate) mod client;
pub(crate) mod proxy;
pub(crate) mod tools;
```

Alphabetical order matches the existing style.

### Step 3: Create `bible_validator.rs` skeleton

- [ ] Create `crates/presenter-server/src/ai/bible_validator.rs` with the full content below. This includes the public types, the validator function signature, compiled regex statics, and the implementation — all at once. TDD note: we'd normally write tests first, but Rust unit tests must live in the same crate module tree, so we create the file structure first and then write the tests. The tests in step 5 will fail the first time they compile against the function body until we finish the implementation in step 6.

```rust
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
        assert!(
            validate_bible_slide("33. Dávid povedal Saulovi...", "1. Samuelova 17:33-37 (SEB)")
                .is_ok()
        );
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
        let err =
            validate_bible_slide("16. Lebo tak Boh...", "Ján 3:16 (seb)").unwrap_err();
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
        assert!(
            validate_bible_slide("1. aby sme VERILI menu jeho Syna", "Ján 1:12 (MIL)").is_ok()
        );
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
```

### Step 4: Run the new tests

- [ ] From the repo root:

```bash
cargo test -p presenter-server bible_validator -- --nocapture
```

Expected: `test result: ok. 18 passed; 0 failed`. If any test fails, re-read the failing test and the function body above — the code is complete, not a skeleton.

### Step 5: Run clippy on the new module

- [ ] 

```bash
cargo clippy -p presenter-server --all-targets -- -D warnings -W clippy::all
```

Expected: clean.

### Step 6: Commit

- [ ] 

```bash
cargo fmt --all
git add crates/presenter-server/Cargo.toml crates/presenter-server/src/ai/mod.rs crates/presenter-server/src/ai/bible_validator.rs Cargo.lock
git commit -m "feat(ai): add bible slide validator module (#236)

Pure function validate_bible_slide enforcing four rules:
- reference format Book Ch:V(-V)?( (CODE))? with optional code in parens
- verse number prefix (N. ) required when reference is non-empty
- no raw ## bold markers anywhere
- emphasis slides (empty reference) require non-empty main

18 unit tests covering every rule + edge cases + error JSON shape.
Not wired into tool handlers yet."
```

---

## Task 2: Wire validator into bible tool handlers

**Files:**
- Modify: `crates/presenter-server/src/ai/tools.rs:792-942` (three handlers)

This task reuses the validator from Task 1 in all three mutating bible tool handlers. Each handler validates **before** any DB mutation and short-circuits on first failure. For `create_bible_presentation` with a slides array, validation is all-or-nothing: if any slide in the batch fails, no presentation is created.

### Step 1: Add a shared helper at the top of the `execute_tool` match body

- [ ] In `crates/presenter-server/src/ai/tools.rs`, add an `use` statement near the other `use` declarations at the top of the file (currently lines 1-9). Insert after line 2:

```rust
use crate::state::bible::BibleTriggerOverrides;
use crate::state::AppState;
use super::bible_validator::{validate_bible_slide, ValidationError};
use presenter_core::slide::{SlideContent, SlideText};
use presenter_core::{
    BiblePresentationId, BiblePresentationSlide, BibleReference, BibleSlideId, LibraryId,
    PresentationId, Slide, SlideId,
};
use serde_json::{json, Value};
use uuid::Uuid;
```

### Step 2: Add a helper function that converts `ValidationError` to a tool-result tuple

- [ ] Still in `tools.rs`, above the `pub async fn execute_tool(...)` function (find it with `grep -n "pub async fn execute_tool" tools.rs`), add this private helper:

```rust
/// Convert a slide validation error into the tool-result tuple
/// `(result_json_string, preview_string)` used by the tool dispatch path.
/// The `preview` is short for UI badges; the full error JSON is sent back
/// to the LLM as the tool result content so it can self-correct on retry.
fn validation_error_response(err: ValidationError) -> (String, String) {
    let preview = format!("Validation failed: {}", err.rule.as_str());
    tracing::warn!(
        rule = %err.rule.as_str(),
        got = %err.got,
        "bible slide validation rejected AI output"
    );
    (err.to_json().to_string(), preview)
}
```

### Step 3: Apply validator in `create_bible_presentation` handler

- [ ] In `tools.rs`, find the `"create_bible_presentation" =>` arm (currently around line 792). Replace the handler body with the version below. The only change is a pre-validation loop that runs before any DB call; if any slide fails, we return the validation error tuple immediately without creating the presentation. On success the original flow is preserved.

Full replacement (from `"create_bible_presentation" => {` to its closing `}`):

```rust
        "create_bible_presentation" => {
            let name = str_field(&args, "name")?;

            // Pre-validate every slide in the batch BEFORE touching the DB.
            // All-or-nothing: if any slide fails, the presentation is not
            // created and the LLM sees the rule-keyed error so it can fix
            // the specific slide and retry with the full batch.
            if let Some(arr) = args["slides"].as_array() {
                for (idx, s) in arr.iter().enumerate() {
                    let main_text = s["main"].as_str().unwrap_or("");
                    let main_reference = s["main_reference"].as_str().unwrap_or("");
                    if let Err(mut err) = validate_bible_slide(main_text, main_reference) {
                        // Annotate the `got` field with the slide index so the
                        // LLM knows which slide in the batch to fix.
                        err.got = format!("slide[{idx}]: {}", err.got);
                        return Ok(validation_error_response(err));
                    }
                }
            }

            let presentation = state.create_bible_presentation(&name).await?;

            // If slides were provided, append them.
            let slides_arr = args["slides"].as_array();
            let final_presentation = if let Some(arr) = slides_arr {
                let mut new_slides: Vec<BiblePresentationSlide> = Vec::with_capacity(arr.len());
                for s in arr {
                    let main_text = s["main"].as_str().unwrap_or("").to_string();
                    let main_reference = s["main_reference"].as_str().unwrap_or("").to_string();
                    let secondary_text = s["secondary"].as_str().unwrap_or("").to_string();
                    let secondary_reference =
                        s["secondary_reference"].as_str().unwrap_or("").to_string();
                    new_slides.push(BiblePresentationSlide {
                        id: BibleSlideId::new(),
                        order: 0,
                        main: SlideText::new(&main_text)
                            .unwrap_or_else(|_| SlideText::new("").unwrap()),
                        main_reference,
                        secondary: SlideText::new(&secondary_text)
                            .unwrap_or_else(|_| SlideText::new("").unwrap()),
                        secondary_reference,
                        metadata: None,
                    });
                }
                if !new_slides.is_empty() {
                    state
                        .append_bible_presentation_slides(presentation.id, new_slides)
                        .await?
                } else {
                    presentation
                }
            } else {
                presentation
            };

            let preview = format!(
                "Created bible presentation '{}' with {} slides",
                final_presentation.name,
                final_presentation.slides.len()
            );
            Ok((
                json!({
                    "id": final_presentation.id.to_string(),
                    "name": final_presentation.name,
                    "slide_count": final_presentation.slides.len(),
                })
                .to_string(),
                preview,
            ))
        }
```

### Step 4: Apply validator in `add_bible_slide` handler

- [ ] In `tools.rs`, find the `"add_bible_slide" =>` arm (currently around line 862). Replace the handler body with:

```rust
        "add_bible_slide" => {
            let pres_id = BiblePresentationId::from_uuid(uuid_field(&args, "presentation_id")?);
            let main_text = str_field(&args, "main")?;
            let main_reference = str_field(&args, "main_reference")?;
            let secondary_text = args["secondary"].as_str().unwrap_or("").to_string();
            let secondary_reference = args["secondary_reference"]
                .as_str()
                .unwrap_or("")
                .to_string();

            // Validate before touching the DB.
            if let Err(err) = validate_bible_slide(&main_text, &main_reference) {
                return Ok(validation_error_response(err));
            }

            let slide = BiblePresentationSlide {
                id: BibleSlideId::new(),
                order: 0,
                main: SlideText::new(&main_text).unwrap_or_else(|_| SlideText::new("").unwrap()),
                main_reference,
                secondary: SlideText::new(&secondary_text)
                    .unwrap_or_else(|_| SlideText::new("").unwrap()),
                secondary_reference,
                metadata: None,
            };
            let updated = state
                .append_bible_presentation_slides(pres_id, vec![slide])
                .await?;
            let preview = format!(
                "Added bible slide to '{}' (now {} total)",
                updated.name,
                updated.slides.len()
            );
            Ok((
                json!({"ok": true, "slide_count": updated.slides.len()}).to_string(),
                preview,
            ))
        }
```

### Step 5: Apply validator in `update_bible_slide` handler

- [ ] In `tools.rs`, find the `"update_bible_slide" =>` arm (currently around line 896). Replace the handler body with:

```rust
        "update_bible_slide" => {
            let pres_id = BiblePresentationId::from_uuid(uuid_field(&args, "presentation_id")?);
            let slide_id = BibleSlideId::from_uuid(uuid_field(&args, "slide_id")?);
            let main_text = str_field(&args, "main")?;
            let main_reference = str_field(&args, "main_reference")?;
            let secondary_text = args["secondary"].as_str().unwrap_or("").to_string();
            let secondary_reference = args["secondary_reference"]
                .as_str()
                .unwrap_or("")
                .to_string();

            // Validate before touching the DB.
            if let Err(err) = validate_bible_slide(&main_text, &main_reference) {
                return Ok(validation_error_response(err));
            }

            // Preserve existing metadata if present.
            let existing_metadata = match state.bible_presentation_detail(pres_id).await? {
                Some(p) => p
                    .slides
                    .iter()
                    .find(|s| s.id == slide_id)
                    .and_then(|s| s.metadata.clone()),
                None => None,
            };

            state
                .update_bible_slide(
                    pres_id,
                    slide_id,
                    main_text,
                    main_reference,
                    secondary_text,
                    secondary_reference,
                    existing_metadata,
                )
                .await?;
            Ok((
                json!({"ok": true}).to_string(),
                "Updated bible slide".to_string(),
            ))
        }
```

### Step 6: Add tool-dispatch tests at the end of the `mod tests` block in `tools.rs`

- [ ] Find the existing `#[cfg(test)] mod tests { ... }` in `tools.rs` (grep for `async fn create_bible_presentation_with_slides` to land near the top of the test module). Append these tests inside the same module, after the last existing test and before the closing `}`:

```rust
    #[tokio::test]
    async fn create_bible_rejects_reference_without_parens() {
        // Exact production-bug input — AI wrote "Židom 4:13 SEB" without parens.
        let state = AppState::in_memory().await.unwrap();
        let args = json!({
            "name": "Test Sermon",
            "slides": [{
                "main": "13. A nieto tvora, čo by bol preň neviditeľný",
                "main_reference": "Židom 4:13 SEB"
            }]
        });
        let (result, preview) =
            execute_tool("create_bible_presentation", &args.to_string(), &state, 320)
                .await
                .unwrap();

        let json: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(json["error"], "slide_validation");
        assert_eq!(json["rule"], "reference_format_requires_parens");
        assert!(json["got"].as_str().unwrap().contains("Židom 4:13 SEB"));
        assert!(preview.starts_with("Validation failed:"));

        // No presentation should have been created.
        let list = state.list_bible_presentations().await.unwrap();
        assert!(list.is_empty(), "presentation must not be created on rejection");
    }

    #[tokio::test]
    async fn create_bible_rejects_main_without_verse_numbers() {
        let state = AppState::in_memory().await.unwrap();
        let args = json!({
            "name": "Test",
            "slides": [{
                "main": "Na počiatku bolo Slovo, to Slovo bolo u Boha",
                "main_reference": "Ján 1:1 (MIL)"
            }]
        });
        let (result, _) =
            execute_tool("create_bible_presentation", &args.to_string(), &state, 320)
                .await
                .unwrap();

        let json: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(json["rule"], "missing_verse_number_prefix");
        assert!(state.list_bible_presentations().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn create_bible_rejects_main_with_hash_markers() {
        let state = AppState::in_memory().await.unwrap();
        let args = json!({
            "name": "Test",
            "slides": [{
                "main": "1. aby sme ##verili## menu jeho Syna",
                "main_reference": "Ján 1:12 (MIL)"
            }]
        });
        let (result, _) =
            execute_tool("create_bible_presentation", &args.to_string(), &state, 320)
                .await
                .unwrap();

        let json: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(json["rule"], "unprocessed_bold_markers");
    }

    #[tokio::test]
    async fn create_bible_accepts_correctly_formatted_slides() {
        let state = AppState::in_memory().await.unwrap();
        let args = json!({
            "name": "Test Sermon",
            "slides": [
                {
                    "main": "1. Na počiatku bolo Slovo.\n2. Ono bolo na počiatku u Boha.\n3. Všetko vzniklo skrze neho.",
                    "main_reference": "Ján 1:1-51 (MIL)"
                },
                {
                    "main": "13. A nieto tvora, čo by bol preň neviditeľný",
                    "main_reference": "Židom 4:13 (SEB)"
                }
            ]
        });
        let (result, _) =
            execute_tool("create_bible_presentation", &args.to_string(), &state, 320)
                .await
                .unwrap();

        let json: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(json["slide_count"], 2);
    }

    #[tokio::test]
    async fn create_bible_accepts_emphasis_slide_without_reference() {
        let state = AppState::in_memory().await.unwrap();
        let args = json!({
            "name": "Test Sermon",
            "slides": [
                {
                    "main": "1. Na počiatku bolo Slovo",
                    "main_reference": "Ján 1:1 (MIL)"
                },
                {
                    "main": "NOVÁ ZMLUVA",
                    "main_reference": ""
                }
            ]
        });
        let (result, _) =
            execute_tool("create_bible_presentation", &args.to_string(), &state, 320)
                .await
                .unwrap();

        let json: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(json["slide_count"], 2);
    }

    #[tokio::test]
    async fn create_bible_rejects_entire_batch_on_first_invalid_slide() {
        // Slide 0 valid, slide 1 invalid, slide 2 valid — whole batch rejected,
        // zero slides and zero presentation created.
        let state = AppState::in_memory().await.unwrap();
        let args = json!({
            "name": "Partial Batch Test",
            "slides": [
                {"main": "1. OK verse", "main_reference": "Ján 1:1 (MIL)"},
                {"main": "bad text no prefix", "main_reference": "Ján 1:2 (MIL)"},
                {"main": "3. OK verse", "main_reference": "Ján 1:3 (MIL)"}
            ]
        });
        let (result, _) =
            execute_tool("create_bible_presentation", &args.to_string(), &state, 320)
                .await
                .unwrap();

        let json: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(json["rule"], "missing_verse_number_prefix");
        assert!(json["got"].as_str().unwrap().contains("slide[1]"));
        assert!(state.list_bible_presentations().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn add_bible_slide_runs_validator() {
        let state = AppState::in_memory().await.unwrap();
        // Create a valid presentation first.
        let create_args = json!({
            "name": "Base",
            "slides": [{"main": "1. test", "main_reference": "Ján 1:1 (MIL)"}]
        });
        let (create_result, _) =
            execute_tool("create_bible_presentation", &create_args.to_string(), &state, 320)
                .await
                .unwrap();
        let created: Value = serde_json::from_str(&create_result).unwrap();
        let pres_id = created["id"].as_str().unwrap().to_string();
        let slide_count_before = created["slide_count"].as_u64().unwrap();

        // Now try to add a malformed slide.
        let add_args = json!({
            "presentation_id": pres_id,
            "main": "no verse number",
            "main_reference": "Ján 1:2 (MIL)"
        });
        let (add_result, _) =
            execute_tool("add_bible_slide", &add_args.to_string(), &state, 320)
                .await
                .unwrap();

        let json: Value = serde_json::from_str(&add_result).unwrap();
        assert_eq!(json["rule"], "missing_verse_number_prefix");

        // Slide count must not have changed.
        let get_args = json!({"presentation_id": pres_id});
        let (get_result, _) =
            execute_tool("get_bible_presentation", &get_args.to_string(), &state, 320)
                .await
                .unwrap();
        let fetched: Value = serde_json::from_str(&get_result).unwrap();
        assert_eq!(
            fetched["slides"].as_array().unwrap().len() as u64,
            slide_count_before
        );
    }

    #[tokio::test]
    async fn update_bible_slide_runs_validator() {
        let state = AppState::in_memory().await.unwrap();
        // Create a valid presentation with one slide.
        let create_args = json!({
            "name": "Base",
            "slides": [{"main": "1. original text", "main_reference": "Ján 1:1 (MIL)"}]
        });
        let (create_result, _) =
            execute_tool("create_bible_presentation", &create_args.to_string(), &state, 320)
                .await
                .unwrap();
        let created: Value = serde_json::from_str(&create_result).unwrap();
        let pres_id = created["id"].as_str().unwrap().to_string();

        // Fetch to get slide id.
        let get_args = json!({"presentation_id": pres_id});
        let (get_result, _) =
            execute_tool("get_bible_presentation", &get_args.to_string(), &state, 320)
                .await
                .unwrap();
        let fetched: Value = serde_json::from_str(&get_result).unwrap();
        let slide_id = fetched["slides"][0]["id"].as_str().unwrap().to_string();

        // Try to update with raw ## markers.
        let update_args = json!({
            "presentation_id": pres_id,
            "slide_id": slide_id,
            "main": "1. aby sme ##verili##",
            "main_reference": "Ján 1:12 (MIL)"
        });
        let (update_result, _) =
            execute_tool("update_bible_slide", &update_args.to_string(), &state, 320)
                .await
                .unwrap();

        let json: Value = serde_json::from_str(&update_result).unwrap();
        assert_eq!(json["rule"], "unprocessed_bold_markers");

        // Verify the original text is unchanged.
        let (get_after, _) =
            execute_tool("get_bible_presentation", &get_args.to_string(), &state, 320)
                .await
                .unwrap();
        let after: Value = serde_json::from_str(&get_after).unwrap();
        assert_eq!(after["slides"][0]["main"], "1. original text");
    }
```

### Step 7: Run the new dispatch tests

- [ ] 

```bash
cargo test -p presenter-server ai::tools::tests -- --nocapture
```

Expected: all existing bible tool tests still pass, plus 8 new tests pass. Total should jump by 8.

### Step 8: Run all ai tests to catch cross-module regressions

- [ ] 

```bash
cargo test -p presenter-server ai:: -- --nocapture
```

Expected: all ai:: tests pass.

### Step 9: Run clippy on the full workspace

- [ ] 

```bash
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
```

Expected: clean.

### Step 10: Commit

- [ ] 

```bash
cargo fmt --all
git add crates/presenter-server/src/ai/tools.rs
git commit -m "feat(ai): validate bible slides in tool handlers (#236)

create_bible_presentation, add_bible_slide, and update_bible_slide
now call validate_bible_slide before any DB write. On failure the
handler short-circuits with a rule-keyed tool-result error so the
LLM can self-correct on its next iteration. Batch create is
all-or-nothing — slide[idx] prefix in the 'got' field tells the
LLM which slide to fix.

Adds 8 dispatch-level tests covering: reference rejection,
missing verse number rejection, hash marker rejection, happy path
with multiple slides, emphasis slide happy path, all-or-nothing
batch rejection, add/update validator integration."
```

---

## Task 3: System prompt update

**Files:**
- Modify: `crates/presenter-server/src/ai/agent.rs:110-153`

### Step 1: Replace the prompt format block

- [ ] In `crates/presenter-server/src/ai/agent.rs`, find `let mut prompt = format!(` (currently line 110). Replace the entire format call — from `let mut prompt = format!(` through the closing `);` with format args — with this version. Two changes: (a) rule 3 in the old "Rules" block is rewritten to reference the new format and warn against the bug, and (b) a new "Creating Bible slides" section is inserted between the "Live context" and "Rules" sections.

```rust
    let mut prompt = format!(
        r#"You are a presentation assistant for a church worship app.

## Live context

Worship libraries (for songs, hymns, band content):
{libraries}

Bible presentations (user-curated bible slide collections):
{bibles}

Bible translations available: {translations}
Slide character limit: {char_limit}

## Creating Bible slides

1. Parse the sermon text yourself: find passage references, ##bold## markers,
   and any pastor title. Build the presentation from what you parse.

2. For each passage: call get_bible_passage (or resolve_bible_slides) to load
   the authoritative text from our database, then edit it ONLY where the
   pastor's version differs. Never invent verse text from memory.

3. Slide main text MUST include verse number prefixes, one per line:
       1. Na počiatku bolo Slovo, to Slovo bolo u Boha...
       2. Ono bolo na počiatku u Boha.
       3. Všetko vzniklo skrze neho...
   Never send verse text without the "N. " prefix.

4. main_reference format is MANDATORY:
       Book Chapter:Verse-Verse (CODE)    ← if you know the translation code
       Book Chapter:Verse-Verse           ← if you don't (omit code ENTIRELY)
   Never write the code without parentheses. "Židom 4:13 SEB" is WRONG.
   Correct: "Židom 4:13 (SEB)" or "Židom 4:13".

5. All slides of a multi-verse passage share the SAME full-range reference.
   If Psalm 52:1-11 splits into 4 slides, every slide's main_reference is
   "Žalm 52:1-11 (ROH)" — not per-slide ranges.

6. Bold marker handling (##...##):
   - ##Book Ch:V## or ##Book Ch:V-V## → this is a section header pointing
     to a passage. DO NOT create a slide for it. Use it to identify which
     passage comes next.
   - ##title## at the very start of the sermon → use as the presentation
     name.
   - ##word## inside a verse → make that word UPPERCASE inside the verse's
     main text. Do NOT create a separate emphasis slide for it.
   - ##phrase## on its own line (not a reference, not inside a verse) →
     create an emphasis slide: main = phrase in UPPERCASE, main_reference
     left EMPTY.
   Never send ## markers to create_bible_presentation — strip/process them
   first. The server will reject any slide containing raw ## markers.

7. The server validates these rules and will return a tool-result error
   naming the broken rule if you get it wrong. Read the error's "rule"
   and "expected" fields, fix the specific slide, and retry.

## Rules

1. For Bible content (verses, passages, sermon slides) use bible_* tools.
   Bible presentations are a SEPARATE concept from worship libraries and
   live in their own dedicated storage. Never create a worship library
   named "Bible".
2. For songs, hymns, band content use worship tools (create_presentation,
   add_slide, etc.) targeting a worship library from the list above.
3. If you need detailed secondary reference material (Slovak book name
   abbreviations, translation code mapping table), call get_style_guide
   once — the bible slide creation rules above are authoritative, this
   is just a lookup aid.
4. Destructive operations (delete_*) require explicit user intent. If
   the user hasn't said "delete", "remove", "vymazať", "odstrániť",
   "zmazať", or equivalent in their most recent message, ask them to
   confirm before calling any delete tool. The server will block delete
   calls that lack explicit user intent.

## Response format

Respond in the user's language (typically Slovak). Keep responses
concise. Summarize what you actually did based on tool results. Do not
claim success for tools that errored."#,
        libraries = libraries_str,
        bibles = bible_str,
        translations = translations_str,
        char_limit = char_limit,
    );
```

Note: the old "Rules" block had 5 items, the new one has 4 — rule 3 (the wrong one with `"Ján 3:16 SEB"`) is removed because the "Creating Bible slides" section replaces it fully.

### Step 2: Run the server tests to make sure the prompt still builds

- [ ] 

```bash
cargo test -p presenter-server ai:: -- --nocapture
```

Expected: all tests pass. None of the existing tests assert exact prompt content, so this should be a no-op at the test level — we're just confirming the format macro compiles.

### Step 3: Verify clippy

- [ ] 

```bash
cargo clippy -p presenter-server --all-targets -- -D warnings
```

Expected: clean.

### Step 4: Commit

- [ ] 

```bash
cargo fmt --all
git add crates/presenter-server/src/ai/agent.rs
git commit -m "feat(ai): restore bible slide formatting rules to system prompt (#236)

The old rule 3 literally told the LLM to produce references as
\"Ján 3:16 SEB\" without parens — the exact production bug. Replace
with a 'Creating Bible slides' section covering: parsing sermons,
loading passages from DB, verse number prefixes, reference format
with optional parens-wrapped code, shared full-range references on
multi-slide passages, bold marker handling, and validator error
handling.

Prompt grows from ~40 to ~70 lines. get_style_guide stays as a
lookup aid for secondary reference material."
```

---

## Task 4: E2E Playwright test

**Files:**
- Create: `tests/e2e/ai-bible-validation.spec.ts`

This test hits the dev server's AI tool dispatch path via the internal HTTP chat endpoint. We don't stub the LLM — we use an alternative endpoint: PR #235's AI dispatch flow exposes an executable path via `POST /ai/chat` with a crafted message that the agent forwards to the real LLM. That requires a running LLM which we can't assume in E2E.

**Instead, use the tool-test shortcut path**: the dispatch path in `ai/tools.rs::execute_tool` is already covered by unit tests from Task 2. For E2E we verify the full stack through a **direct-API shortcut**: the server already exposes `POST /api/bible/presentations` via the regular bible HTTP API (from PR #234). That endpoint bypasses the AI validator. So E2E must verify the AI path specifically.

**Approach:** Before writing this E2E test, verify the current test infrastructure exposes a way to run `execute_tool` without a real LLM. Inspect `tests/e2e/` for any existing AI-related spec to reuse its pattern.

### Step 1: Check for existing AI E2E infrastructure

- [ ] Before writing anything, run:

```bash
ls tests/e2e/ 2>&1
grep -l "ai\|AI\|bible" tests/e2e/ 2>&1
```

If an AI or bible-related spec exists, **read it first** to understand whether there's already a pattern for seeding AI tool calls in E2E (e.g., a dev-only HTTP endpoint that executes a single tool directly, or an LLM mock). Reuse that pattern.

### Step 2: Decide the test approach based on what exists

- [ ] If the repo has NO existing AI E2E infrastructure and no LLM stub: **skip this task and note in the PR body that AI validator coverage is unit + dispatch-level only**. Mutation testing on the validator module (already in CI) plus the 8 dispatch-level tests from Task 2 provide strong confidence. E2E against a real LLM is out of scope for this PR.

- [ ] If the repo HAS an existing `ai_proxy` mock or a dev-only single-tool endpoint: write `tests/e2e/ai-bible-validation.spec.ts` that:
  1. Navigates to `http://127.0.0.1:$PORT/ui/bible`
  2. POSTs to the existing AI single-tool endpoint with a malformed `create_bible_presentation` payload (`main_reference: "Židom 4:13 SEB"`)
  3. Asserts the response body contains `"rule": "reference_format_requires_parens"`
  4. Reloads `/ui/bible` and asserts the presentation does NOT appear in the DOM
  5. POSTs again with a corrected payload (`main_reference: "Židom 4:13 (SEB)"`, `main: "13. A nieto tvora..."`)
  6. Reloads `/ui/bible` and asserts the presentation DOES appear with verse-number prefix visible
  7. Collects `page.on('console')` errors/warnings, asserts the array is empty

### Step 3: Run the test

- [ ] If you wrote the spec in step 2:

```bash
npm run test:playwright -- ai-bible-validation
```

Expected: 1 test passes, browser console clean.

### Step 4: Commit (only if a test file was actually written)

- [ ] 

```bash
git add tests/e2e/ai-bible-validation.spec.ts
git commit -m "test(e2e): add bible slide validation test (#236)

Verifies the AI validator rejects malformed slides end-to-end via
the tool dispatch path, then accepts the corrected version. Asserts
no console errors and validates the DOM reflects the stored state."
```

If step 2 concluded "skip this task", commit nothing for Task 4 and proceed to Task 5.

---

## Task 5: Version bump, local checks, push, monitor CI, open PR

**Files:**
- Modify: `Cargo.toml` (workspace `[workspace.package].version`)

### Step 1: Check version state

- [ ] 

```bash
git fetch origin
grep '^version' Cargo.toml | head -1
git show origin/main:Cargo.toml | grep '^version' | head -1
```

If `dev` version equals `main` version, bump. If `dev` is already ahead of `main`, still bump — this work deserves its own patch version bump for traceability.

### Step 2: Bump the version

- [ ] Edit `Cargo.toml` workspace `[workspace.package].version`. Current is `"0.4.16"`, bump to `"0.4.17"`:

```toml
[workspace.package]
version = "0.4.17"
```

### Step 3: Update `Cargo.lock`

- [ ] 

```bash
cargo build -p presenter-server
```

This refreshes `Cargo.lock` with the new version (and the new `regex` dep from Task 1). Expected: clean build.

### Step 4: Run ALL local checks before pushing

- [ ] 

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
cargo test -p presenter-server ai:: -- --nocapture
```

Expected: all clean, all tests pass. If anything fails, fix it in ONE amending commit (not a separate "fix CI" commit).

### Step 5: Commit the version bump

- [ ] 

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore: bump version to 0.4.17"
```

### Step 6: Push and monitor CI

- [ ] 

```bash
git push origin dev
gh run list --branch dev --limit 3
```

Then wait for the pipeline to complete. Do NOT use `gh run watch` (causes API rate limits). Check periodically with `gh run view <run-id> --json status,conclusion,jobs`. Expected: all 22 checks green (Format, Clippy, Validate Version, Cargo Audit, Cargo Deny, Test, Quality, Code Coverage, Build, Mutation Testing, Playwright E2E ×3, Merge E2E, Deploy to Dev, codecov/patch, Label PR, Branch Sync Check).

If any job fails: `gh run view <run-id> --log-failed`, fix the root cause, ONE commit, push again.

### Step 7: Verify dev deployment

- [ ] 

```bash
curl -s http://10.77.8.134:8080/healthz
```

Expected: `{"channel":"dev","status":"ok","version":"0.4.17"}`.

### Step 8: Manual verification on dev (functional)

- [ ] Open `http://10.77.8.134:8080/ui/operator` in a browser with Playwright or your local browser. Open AI chat. Paste a short sermon-like text that contains:

  - A passage reference (e.g. "Ján 3:16")
  - A `##emphasis##` inline word
  - A `##STANDALONE PHRASE##` on its own line

Ask the AI to "create a Bible presentation from this sermon." Expected behavior:

- AI calls `get_bible_passage` (seen in the actions badges)
- AI calls `create_bible_presentation` with correctly formatted slides
- If AI gets it wrong on the first try, you see a `Validation failed: <rule>` badge and a retry that succeeds
- Final presentation visible in `/ui/bible` with verse number prefixes, parens in the reference, and one emphasis slide

Also check `sudo journalctl -u presenter-dev -f` for `bible slide validation rejected` warn logs — if the rule name appears, the LLM tripped and self-corrected. That's the expected path.

### Step 9: Open the PR

- [ ] Check if there's still an open PR from the previous cleanup (PR #235):

```bash
gh pr list --state open
```

If PR #235 is still open, the new commits are already attached to it. Update its body via the GitHub REST API to cover both rounds of work. Otherwise, open a fresh PR:

```bash
gh pr create --title "feat(ai): validate bible slides + restore prompt rules" --body "$(cat <<'EOF'
## Summary

Follow-up to PR #235. Fixes three AI bugs observed in production:

1. Slides missing verse number prefixes (`"A nieto tvora..."` instead of `"13. A nieto tvora..."`)
2. Reference format with translation code but no parens (`"Židom 4:13 SEB"` instead of `"Židom 4:13 (SEB)"` or `"Židom 4:13"`)
3. Raw `##bold##` markers from the pastor's sermon left unprocessed

## Root cause

PR #235 moved detailed formatting rules to an on-demand `get_style_guide` tool and left only a minimal "Rules" block in the system prompt. The remaining rule 3 literally said:

> Bible slide main_reference format: "Book Chapter:Verse TRANSLATION" (e.g. "Ján 3:16 SEB")

That's the exact format the AI produced in the production bug. The prompt was instructing the LLM to produce malformed references.

Additionally, the bible tool handlers wrote whatever the AI sent verbatim — no validation.

## Fix

### Server-side validator

New pure module `crates/presenter-server/src/ai/bible_validator.rs` with a single function `validate_bible_slide(main, main_reference)` enforcing four rules:

1. **Reference format** — must match `Book Ch:V(-V)?( (CODE))?` when non-empty
2. **Verse number prefix** — `main` must contain at least one line starting with `\d+\. ` when reference is non-empty
3. **No raw bold markers** — neither `main` nor `main_reference` may contain `##`
4. **Emphasis/title slides** — when reference is empty, rule 3 still applies, `main` must be non-empty

Applied in `create_bible_presentation`, `add_bible_slide`, and `update_bible_slide` handlers **before** any DB write. Batch create is all-or-nothing: any invalid slide rejects the entire batch with `slide[N]:` prefix in the error.

Rejections return a tool-result JSON with `rule`, `got`, `expected` fields. The LLM sees the error on its next iteration and self-corrects.

### Prompt restoration

Replaced the wrong rule 3 with a new "Creating Bible slides" section covering: parsing the sermon, loading passages from the DB, verse number prefixes, reference format, shared full-range references, `##bold##` marker handling, and validator error handling. Prompt grows from ~40 to ~70 lines. `get_style_guide` stays for Slovak book abbreviations and translation codes.

## Tests

- **18 new validator unit tests** (`bible_validator.rs`) — every rule, every edge case
- **8 new dispatch-level tests** (`tools.rs`) — rejection + happy path + all-or-nothing batch + `add_bible_slide` + `update_bible_slide`
- **Mutation testing** (existing CI) — the regex-based rules are mutation-resistant
- **E2E** — extends existing suite if infrastructure allows, otherwise unit+dispatch coverage is sufficient (see Task 4 of the plan)

## Verification

- Dev deploy verified: `curl http://10.77.8.134:8080/healthz` returns `version: 0.4.17`
- Manual sermon paste on dev reproduced the original workflow — slides now have verse numbers, parens on references, processed `##bold##` markers
- `journalctl` shows the validator rejection + LLM retry path firing when the AI trips a rule

## Version

0.4.16 → 0.4.17

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

Return the PR URL. Do NOT merge — wait for explicit user instruction.

---

## Verification summary

| Check | How to verify |
|---|---|
| Validator rules enforced | `cargo test -p presenter-server bible_validator` — 18 passed |
| Handlers call validator | `cargo test -p presenter-server ai::tools::tests` — 8 new passed |
| Production bug rejected | `create_bible_rejects_reference_without_parens` asserts on `"Židom 4:13 SEB"` exact string |
| All-or-nothing batch | `create_bible_rejects_entire_batch_on_first_invalid_slide` asserts 0 presentations after rejection |
| Emphasis slides work | `create_bible_accepts_emphasis_slide_without_reference` |
| No regressions | `cargo test -p presenter-server ai::` — all passing |
| Prompt compiles | Same test run |
| CI green | All 22 checks on the pipeline run |
| Dev deployed at 0.4.17 | `curl /healthz` |
| Functional end-to-end | Manual sermon paste on dev with verse numbers, parens, processed markers visible in `/ui/bible` |
