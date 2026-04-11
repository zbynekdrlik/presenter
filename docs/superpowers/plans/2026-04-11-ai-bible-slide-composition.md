# AI Bible Slide Composition Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move bible-slide break decisions from the LLM to the server so AI-created bible presentations obey the configured `character_limit`, matching live mode behavior.

**Architecture:** AI works at verse granularity. It calls `load_bible_verses` to pull raw DB verses, edits them against the sermon (text overrides, uppercase markers, emphasis insertions), and submits a typed `items[]` stream to `create_bible_presentation`. The server composes slides with `compose_bible_items_into_slides` — the same character-limit logic the live path uses. A new validator rule `MainExceedsCharacterLimit` serves as a fail-safe.

**Tech Stack:** Rust 2021, `anyhow`, `serde_json`, `tokio`, existing `compose_bible_slides` logic, Playwright for E2E.

**Spec:** `docs/superpowers/specs/2026-04-11-ai-bible-slide-composition-design.md`

---

## Context for the implementer

This repo has two bible-slide code paths:

1. **Live mode** — `/bible/resolve` → `AppState::generate_bible_slides` at `crates/presenter-server/src/state/bible.rs:122-196` → `compose_bible_slides` at `crates/presenter-server/src/state/slides.rs:18-142`. Splits a set of DB passages into slides respecting a character limit. Works correctly.

2. **AI mode** — OpenAI-function-calling tools defined in `crates/presenter-server/src/ai/tool_defs.rs`, dispatched in `crates/presenter-server/src/ai/tools.rs:457-625`. Currently accepts slide text verbatim from the LLM. **This is the broken path.**

The bible validator at `crates/presenter-server/src/ai/bible_validator.rs` has 4 rules (reference format, verse number prefix, no ## markers, non-empty emphasis). None of them check length.

The character limit is stored in DB under app setting `bible-preferences` (default 320). Read via `AppState::get_bible_preferences()`. In the AI path, the limit is already threaded to tool dispatch as `default_char_limit: u32` — see `execute_tool()` signature at `tools.rs:55-60`.

`BiblePresentationSlide` is the persisted type (see `crates/presenter-core/src/bible_presentation.rs` or wherever `presenter_core::BiblePresentationSlide` is defined). Fields: `id`, `order`, `main: SlideText`, `main_reference: String`, `secondary: SlideText`, `secondary_reference: String`, `metadata: Option<...>`. Persistence: `AppState::append_bible_presentation_slides(id, Vec<BiblePresentationSlide>)` at `state/bible.rs:238-254`.

The project CLAUDE.md says local `cargo build`, `cargo test`, `cargo clippy`, `cargo fmt` are allowed on this dev2 machine. Run them locally before pushing.

Production-code hard cap: 1000 lines per file. Warning cap: 800 lines. `tools.rs` is currently 1279 lines — WAIT, it is already over the hard cap? Check first thing: `wc -l crates/presenter-server/src/ai/tools.rs`. If it is over, we need to carve something out before adding to it. The existing 1279 is mostly tests (test module starts around line 657); production lines are ~655 which is under the cap. Adding ~150 lines of new handler + tests for the new path is safe if we remove `add_bible_slide` and `update_bible_slide` handlers (which we are anyway).

---

## File Structure

| File | Responsibility | Change type |
|------|----------------|-------------|
| `Cargo.toml` | Workspace version | Modify |
| `crates/presenter-server/src/state/slides.rs` | Add `BibleItem` enum + `compose_bible_items_into_slides` pure fn + unit tests | Modify |
| `crates/presenter-server/src/ai/bible_validator.rs` | Add `MainExceedsCharacterLimit` rule, extend `validate_bible_slide` signature with `character_limit: u32` | Modify |
| `crates/presenter-server/src/ai/tool_defs.rs` | Add `load_bible_verses` schema; rewrite `create_bible_presentation` schema to take `items`; remove `add_bible_slide` and `update_bible_slide` schemas | Modify |
| `crates/presenter-server/src/ai/tools.rs` | Add `load_bible_verses` handler; rewrite `create_bible_presentation` handler to parse `items[]`, run composer, run validator, persist; delete `add_bible_slide` and `update_bible_slide` handlers and their tests | Modify |
| `crates/presenter-server/src/ai/agent.rs` | Rewrite `## Creating Bible slides` prompt section for the new verse-granularity workflow | Modify |
| `tests/e2e/ai-chat-bible-composition.spec.ts` | New Playwright test exercising the AI path with a long passage + emphasis marker | Create |

---

## Task 1: Version bump

**Why first:** airuleset requires the version bump before any feature code so CI's version-check job doesn't waste a pipeline run on version-only failures.

**Files:**
- Modify: `Cargo.toml` (workspace `[workspace.package].version`)

- [ ] **Step 1: Inspect current version**

Run: `grep -A1 '\[workspace.package\]' Cargo.toml`

Expected: current version is `0.4.17` (set in the previous PR #235). If it differs, use the current `main` version + 1 patch.

- [ ] **Step 2: Bump to 0.4.18**

Edit `Cargo.toml`:

```toml
[workspace.package]
version = "0.4.18"
```

- [ ] **Step 3: Regenerate Cargo.lock**

Run: `cargo check --workspace 2>&1 | tail -5`

Expected: `Cargo.lock` updates with the new version. No compile errors (nothing else changed yet).

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore: bump version to 0.4.18"
```

---

## Task 2: Pure composer `compose_bible_items_into_slides`

**Files:**
- Modify: `crates/presenter-server/src/state/slides.rs` (add types + function + tests alongside existing `compose_bible_slides`)

### Step 1: Write the failing test for single-verse short enough

Add this test inside the existing `#[cfg(test)] mod tests { ... }` block in `crates/presenter-server/src/state/slides.rs` (at the end, before the closing `}`):

```rust
    // --- compose_bible_items_into_slides tests ---

    fn verse(number: u32, text: &str) -> BibleItem {
        BibleItem::Verse {
            number,
            text: text.to_string(),
            book: "Ján".to_string(),
            chapter: 1,
            translation: "SEB".to_string(),
        }
    }

    fn emphasis(text: &str) -> BibleItem {
        BibleItem::Emphasis {
            text: text.to_string(),
        }
    }

    #[test]
    fn compose_items_single_short_verse_emits_one_slide() {
        let items = vec![verse(1, "Na počiatku bolo Slovo.")];
        let slides = compose_bible_items_into_slides(&items, 320);
        assert_eq!(slides.len(), 1);
        assert_eq!(slides[0].main, "1. Na počiatku bolo Slovo.");
        assert_eq!(slides[0].main_reference, "Ján 1:1 (SEB)");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p presenter-server compose_items_single_short --lib -- --nocapture 2>&1 | tail -20`

Expected: FAIL with `cannot find type BibleItem in this scope` or `cannot find function compose_bible_items_into_slides`.

### Step 3: Add the types and stub function

Add to `crates/presenter-server/src/state/slides.rs` just **below** the existing `translation_short_code` helper (line 16) and **above** `compose_bible_slides` (line 18):

```rust
/// Typed input for the AI-facing bible composer. A stream of these is
/// produced by the LLM after it edits DB verses against the sermon text,
/// and the server composes slides out of the stream respecting the
/// character limit — same splitting rules as live mode.
#[derive(Debug, Clone)]
pub enum BibleItem {
    Verse {
        number: u32,
        text: String,
        book: String,
        chapter: u32,
        translation: String,
    },
    Emphasis {
        text: String,
    },
}

/// A slide produced by `compose_bible_items_into_slides`. Plain data —
/// the tool handler wraps it into `BiblePresentationSlide` for persistence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposedBibleSlide {
    pub main: String,
    pub main_reference: String,
}

/// Compose a stream of `BibleItem` into slides. Same greedy-packing rule
/// as `compose_bible_slides`: accumulate verses into one slide until the
/// next verse would overflow the character limit, then flush. Emphasis
/// items and translation/book/chapter changes force a slide break.
///
/// If a single verse item is longer than `character_limit`, it is emitted
/// as its own oversized slide — the validator's `MainExceedsCharacterLimit`
/// rule catches this downstream and the LLM sees a rule-keyed error.
pub fn compose_bible_items_into_slides(
    items: &[BibleItem],
    character_limit: u32,
) -> Vec<ComposedBibleSlide> {
    let limit = character_limit as usize;
    let mut slides: Vec<ComposedBibleSlide> = Vec::new();

    // Accumulator for the current verse slide.
    let mut cur_lines: Vec<String> = Vec::new();
    let mut cur_numbers: Vec<u32> = Vec::new();
    let mut cur_group: Option<(String, u32, String)> = None; // (book, chapter, translation)

    let flush_verses = |slides: &mut Vec<ComposedBibleSlide>,
                        lines: &mut Vec<String>,
                        numbers: &mut Vec<u32>,
                        group: &mut Option<(String, u32, String)>| {
        if lines.is_empty() {
            return;
        }
        let main = lines.join("\n");
        let (book, chapter, translation) = group.clone().expect("group set when lines present");
        let start = *numbers.first().unwrap();
        let end = *numbers.last().unwrap();
        let reference = if start == end {
            format!("{} {}:{} ({})", book, chapter, start, translation)
        } else {
            format!("{} {}:{}-{} ({})", book, chapter, start, end, translation)
        };
        slides.push(ComposedBibleSlide {
            main,
            main_reference: reference,
        });
        lines.clear();
        numbers.clear();
        *group = None;
    };

    for item in items {
        match item {
            BibleItem::Emphasis { text } => {
                flush_verses(&mut slides, &mut cur_lines, &mut cur_numbers, &mut cur_group);
                slides.push(ComposedBibleSlide {
                    main: text.clone(),
                    main_reference: String::new(),
                });
            }
            BibleItem::Verse {
                number,
                text,
                book,
                chapter,
                translation,
            } => {
                // Translation / book / chapter change forces a slide break.
                if let Some((cur_book, cur_chapter, cur_tr)) = &cur_group {
                    if cur_book != book || cur_chapter != chapter || cur_tr != translation {
                        flush_verses(
                            &mut slides,
                            &mut cur_lines,
                            &mut cur_numbers,
                            &mut cur_group,
                        );
                    }
                }

                let line = format!("{}. {}", number, text);
                let existing_len: usize = cur_lines.iter().map(String::len).sum();
                let newlines = if cur_lines.is_empty() { 0 } else { cur_lines.len() };
                let sep_before_new = if cur_lines.is_empty() { 0 } else { 1 };
                let prospective = existing_len + newlines + sep_before_new + line.len();

                if prospective > limit && !cur_lines.is_empty() {
                    flush_verses(
                        &mut slides,
                        &mut cur_lines,
                        &mut cur_numbers,
                        &mut cur_group,
                    );
                }

                cur_lines.push(line);
                cur_numbers.push(*number);
                cur_group = Some((book.clone(), *chapter, translation.clone()));
            }
        }
    }

    flush_verses(&mut slides, &mut cur_lines, &mut cur_numbers, &mut cur_group);
    slides
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p presenter-server compose_items_single_short --lib 2>&1 | tail -10`

Expected: PASS.

### Step 5: Write tests for the remaining behaviors

Add these tests to the same test module:

```rust
    #[test]
    fn compose_items_two_verses_that_fit_emit_one_slide_with_range() {
        let items = vec![
            verse(1, "Na počiatku bolo Slovo."),
            verse(2, "Ono bolo na počiatku u Boha."),
        ];
        let slides = compose_bible_items_into_slides(&items, 320);
        assert_eq!(slides.len(), 1);
        assert!(slides[0].main.contains("1. Na počiatku"));
        assert!(slides[0].main.contains("2. Ono bolo"));
        assert_eq!(slides[0].main_reference, "Ján 1:1-2 (SEB)");
    }

    #[test]
    fn compose_items_two_verses_that_overflow_emit_two_slides() {
        // limit = 30; each verse line is ~25 chars; together they exceed 30.
        let items = vec![
            verse(1, "Na počiatku bolo Slovo."), // "1. Na počiatku bolo Slovo." = 26
            verse(2, "Ono bolo na počiatku."),   // "2. Ono bolo na počiatku." = 24
        ];
        let slides = compose_bible_items_into_slides(&items, 30);
        assert_eq!(slides.len(), 2);
        assert_eq!(slides[0].main_reference, "Ján 1:1 (SEB)");
        assert_eq!(slides[1].main_reference, "Ján 1:2 (SEB)");
    }

    #[test]
    fn compose_items_emphasis_between_verses_breaks_slide() {
        let items = vec![
            verse(1, "Na počiatku."),
            emphasis("NOVÁ ZMLUVA"),
            verse(2, "Ono bolo."),
        ];
        let slides = compose_bible_items_into_slides(&items, 320);
        assert_eq!(slides.len(), 3);
        assert_eq!(slides[0].main, "1. Na počiatku.");
        assert_eq!(slides[0].main_reference, "Ján 1:1 (SEB)");
        assert_eq!(slides[1].main, "NOVÁ ZMLUVA");
        assert_eq!(slides[1].main_reference, "");
        assert_eq!(slides[2].main, "2. Ono bolo.");
        assert_eq!(slides[2].main_reference, "Ján 1:2 (SEB)");
    }

    #[test]
    fn compose_items_translation_change_forces_break() {
        let items = vec![
            BibleItem::Verse {
                number: 1,
                text: "Na počiatku bolo Slovo.".to_string(),
                book: "Ján".to_string(),
                chapter: 1,
                translation: "SEB".to_string(),
            },
            BibleItem::Verse {
                number: 2,
                text: "Ono bolo na počiatku.".to_string(),
                book: "Ján".to_string(),
                chapter: 1,
                translation: "MIL".to_string(),
            },
        ];
        let slides = compose_bible_items_into_slides(&items, 320);
        assert_eq!(slides.len(), 2);
        assert_eq!(slides[0].main_reference, "Ján 1:1 (SEB)");
        assert_eq!(slides[1].main_reference, "Ján 1:2 (MIL)");
    }

    #[test]
    fn compose_items_chapter_change_forces_break() {
        let items = vec![
            BibleItem::Verse {
                number: 14,
                text: "last verse ch1".to_string(),
                book: "Ján".to_string(),
                chapter: 1,
                translation: "SEB".to_string(),
            },
            BibleItem::Verse {
                number: 1,
                text: "first verse ch2".to_string(),
                book: "Ján".to_string(),
                chapter: 2,
                translation: "SEB".to_string(),
            },
        ];
        let slides = compose_bible_items_into_slides(&items, 320);
        assert_eq!(slides.len(), 2);
        assert_eq!(slides[0].main_reference, "Ján 1:14 (SEB)");
        assert_eq!(slides[1].main_reference, "Ján 2:1 (SEB)");
    }

    #[test]
    fn compose_items_book_change_forces_break() {
        let items = vec![
            BibleItem::Verse {
                number: 1,
                text: "first".to_string(),
                book: "Ján".to_string(),
                chapter: 1,
                translation: "SEB".to_string(),
            },
            BibleItem::Verse {
                number: 1,
                text: "second".to_string(),
                book: "Marek".to_string(),
                chapter: 1,
                translation: "SEB".to_string(),
            },
        ];
        let slides = compose_bible_items_into_slides(&items, 320);
        assert_eq!(slides.len(), 2);
        assert_eq!(slides[0].main_reference, "Ján 1:1 (SEB)");
        assert_eq!(slides[1].main_reference, "Marek 1:1 (SEB)");
    }

    #[test]
    fn compose_items_empty_returns_empty() {
        let slides = compose_bible_items_into_slides(&[], 320);
        assert!(slides.is_empty());
    }

    #[test]
    fn compose_items_single_verse_longer_than_limit_emits_oversized_slide() {
        // Limit 20; verse line is much longer. Composer still emits it —
        // the validator catches oversize downstream.
        let items = vec![verse(1, "Na počiatku bolo Slovo a Slovo bolo u Boha.")];
        let slides = compose_bible_items_into_slides(&items, 20);
        assert_eq!(slides.len(), 1);
        assert!(slides[0].main.len() > 20);
        assert_eq!(slides[0].main_reference, "Ján 1:1 (SEB)");
    }

    #[test]
    fn compose_items_adjacent_emphasis_emit_separate_slides() {
        let items = vec![
            emphasis("FIRST"),
            emphasis("SECOND"),
            verse(1, "verse"),
        ];
        let slides = compose_bible_items_into_slides(&items, 320);
        assert_eq!(slides.len(), 3);
        assert_eq!(slides[0].main, "FIRST");
        assert_eq!(slides[1].main, "SECOND");
        assert_eq!(slides[2].main_reference, "Ján 1:1 (SEB)");
    }
```

- [ ] **Step 6: Run all composer tests**

Run: `cargo test -p presenter-server --lib state::slides:: 2>&1 | tail -15`

Expected: all existing `compose_bible_slides_*` tests pass AND all 9 new `compose_items_*` tests pass.

- [ ] **Step 7: Format and commit**

```bash
cargo fmt --all
git add crates/presenter-server/src/state/slides.rs
git commit -m "feat(bible): add compose_bible_items_into_slides pure composer

Typed BibleItem enum + pure function that packs verse items into
slides respecting character_limit. Emphasis items, translation/book/
chapter changes force slide breaks. Mirrors the existing live-path
compose_bible_slides splitting rules. Unit-tested for all branch
combinations including oversized-single-verse fallthrough."
```

---

## Task 3: Validator length rule

**Files:**
- Modify: `crates/presenter-server/src/ai/bible_validator.rs` (add rule, extend signature, tests)

### Step 1: Write the failing test

Add this test inside the existing `#[cfg(test)] mod tests { ... }` block in `crates/presenter-server/src/ai/bible_validator.rs` (at the end):

```rust
    // -- Rule 5: character limit --

    #[test]
    fn length_rule_accepts_slide_at_exactly_limit() {
        let main = "1. ".to_string() + &"a".repeat(317); // 320 chars total
        assert!(validate_bible_slide(&main, "Ján 1:1 (SEB)", 320).is_ok());
    }

    #[test]
    fn length_rule_rejects_slide_one_char_over_limit() {
        let main = "1. ".to_string() + &"a".repeat(318); // 321 chars total
        let err = validate_bible_slide(&main, "Ján 1:1 (SEB)", 320).unwrap_err();
        assert_eq!(err.rule, ValidationRule::MainExceedsCharacterLimit);
        assert_eq!(err.limit, Some(320));
    }

    #[test]
    fn length_rule_rejects_slide_well_over_limit() {
        let main = "1. ".to_string() + &"a".repeat(1000);
        let err = validate_bible_slide(&main, "Ján 1:1 (SEB)", 320).unwrap_err();
        assert_eq!(err.rule, ValidationRule::MainExceedsCharacterLimit);
    }

    #[test]
    fn length_rule_applies_to_emphasis_slides() {
        let main = "a".repeat(400);
        let err = validate_bible_slide(&main, "", 320).unwrap_err();
        assert_eq!(err.rule, ValidationRule::MainExceedsCharacterLimit);
    }

    #[test]
    fn length_rule_error_json_includes_limit() {
        let err = ValidationError::new_with_limit(
            ValidationRule::MainExceedsCharacterLimit,
            "a".repeat(400),
            320,
        );
        let json = err.to_json();
        assert_eq!(json["rule"], "main_exceeds_character_limit");
        assert_eq!(json["limit"], 320);
        assert!(json["expected"].as_str().unwrap().contains("320"));
    }
```

Also, EVERY existing test in this file that calls `validate_bible_slide(main, main_reference)` must be updated to pass a character limit. Pass `320` (the default) to preserve their semantics. Find each call with:

Run: `grep -n 'validate_bible_slide(' crates/presenter-server/src/ai/bible_validator.rs`

For each matched line in a test, change `validate_bible_slide("main text", "ref")` to `validate_bible_slide("main text", "ref", 320)`.

- [ ] **Step 2: Run the new tests to verify they fail**

Run: `cargo test -p presenter-server --lib ai::bible_validator:: 2>&1 | tail -20`

Expected: compilation error — `validate_bible_slide` arity mismatch, `ValidationRule::MainExceedsCharacterLimit` not found, `ValidationError::new_with_limit` not found, `err.limit` field missing.

### Step 3: Add the rule and extend the signature

In `crates/presenter-server/src/ai/bible_validator.rs`, make these changes:

**1. Extend the `ValidationRule` enum** (add `MainExceedsCharacterLimit` as the 5th variant):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationRule {
    ReferenceFormatRequiresParens,
    MissingVerseNumberPrefix,
    UnprocessedBoldMarkers,
    EmptyMainOnEmphasisSlide,
    MainExceedsCharacterLimit,
}
```

**2. Extend the `as_str` match arm** for the new variant:

```rust
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ReferenceFormatRequiresParens => "reference_format_requires_parens",
            Self::MissingVerseNumberPrefix => "missing_verse_number_prefix",
            Self::UnprocessedBoldMarkers => "unprocessed_bold_markers",
            Self::EmptyMainOnEmphasisSlide => "empty_main_on_emphasis_slide",
            Self::MainExceedsCharacterLimit => "main_exceeds_character_limit",
        }
    }
```

**3. Extend the `expected()` match arm** for the new variant:

```rust
            Self::MainExceedsCharacterLimit => {
                "Slide main text exceeds the character limit. The server composes \
                 slides from your verse items automatically — this error means a \
                 single verse is longer than the limit on its own. Recovery: split \
                 the verse text across multiple verse items with the same verse \
                 number, or reduce the sermon's custom wording."
            }
```

**4. Add a `limit` field to `ValidationError`** (optional so existing call sites with non-length rules don't need to supply it):

```rust
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

    pub fn to_json(&self) -> serde_json::Value {
        let mut obj = serde_json::json!({
            "error": "slide_validation",
            "rule": self.rule.as_str(),
            "got": self.got,
            "expected": self.rule.expected(),
        });
        if let Some(limit) = self.limit {
            obj["limit"] = serde_json::json!(limit);
            // Interpolate the limit into the expected text so the LLM sees the actual number.
            if self.rule == ValidationRule::MainExceedsCharacterLimit {
                let with_limit = format!(
                    "Slide main text exceeds the character limit ({limit} characters). \
                     The server composes slides from your verse items automatically — this \
                     error means a single verse is longer than the limit on its own. Recovery: \
                     split the verse text across multiple verse items with the same verse \
                     number, or reduce the sermon's custom wording."
                );
                obj["expected"] = serde_json::json!(with_limit);
            }
        }
        obj
    }
}
```

**5. Change the `validate_bible_slide` signature** to accept the limit and add the length check as the FIRST rule (before the others — fail fast on length, the most common error):

```rust
pub fn validate_bible_slide(
    main: &str,
    main_reference: &str,
    character_limit: u32,
) -> Result<(), ValidationError> {
    // Rule 5 — length check (applies to every slide, including emphasis).
    if main.len() > character_limit as usize {
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
```

- [ ] **Step 4: Run all validator tests**

Run: `cargo test -p presenter-server --lib ai::bible_validator:: 2>&1 | tail -30`

Expected: all existing 20+ tests pass (with their updated signature) AND the 5 new length-rule tests pass.

- [ ] **Step 5: Format and commit**

```bash
cargo fmt --all
git add crates/presenter-server/src/ai/bible_validator.rs
git commit -m "feat(bible-validator): add MainExceedsCharacterLimit rule

Validator now takes character_limit as a parameter and rejects any
slide whose main text exceeds it. ValidationError gains an optional
limit field, serialized in the tool-result JSON so the LLM knows
the actual limit without having to look it up in the prompt."
```

---

## Task 4: `load_bible_verses` tool

**Files:**
- Modify: `crates/presenter-server/src/ai/tool_defs.rs` (add new schema)
- Modify: `crates/presenter-server/src/ai/tools.rs` (add new handler + test)

### Step 1: Write the failing handler test

Append to the existing `#[cfg(test)] mod tests { ... }` block in `crates/presenter-server/src/ai/tools.rs`:

```rust
    #[tokio::test]
    async fn load_bible_verses_returns_raw_verses() {
        let state = AppState::in_memory().await.unwrap();
        // The in-memory state seeds no bible translations, so we expect
        // "translation not found" error. This still exercises the handler
        // path and proves the tool is registered.
        let args = json!({
            "translation": "slk-seb",
            "book": "Ján",
            "chapter": 1,
            "verse_start": 1,
            "verse_end": 3
        });
        let result = execute_tool("load_bible_verses", &args.to_string(), &state, 320).await;
        // Either Ok with error JSON, or Err — both are acceptable proofs the
        // handler exists. The key is that it is NOT the "unknown tool" path.
        match result {
            Ok((body, _preview)) => {
                assert!(
                    !body.contains("unknown tool"),
                    "tool must be registered, got body: {}",
                    body
                );
            }
            Err(_) => {
                // Acceptable — translation lookup failed. Handler exists.
            }
        }
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p presenter-server --lib ai::tools::tests::load_bible_verses 2>&1 | tail -20`

Expected: FAIL — the dispatch falls through to the `_` arm returning `"unknown tool: load_bible_verses"`, so the assertion fires.

### Step 3: Add the schema to tool_defs.rs

In `crates/presenter-server/src/ai/tool_defs.rs`, add this entry to the `tool_definitions()` vec immediately BEFORE the existing `resolve_bible_slides` entry (currently at line 186):

```rust
        tool_def(
            "load_bible_verses",
            "[BIBLE only] Load raw verse text from the database for a passage range. Returns an array of {number, text, reference} objects — NOT pre-split slides. Use this as the source of truth for verse text when building a bible presentation. Compare each returned verse to the sermon wording and override `text` where they differ.",
            json!({
                "type": "object",
                "properties": {
                    "translation": {"type": "string", "description": "Translation code (e.g. slk-seb)"},
                    "book": {"type": "string", "description": "Full book name (e.g. Ján)"},
                    "chapter": {"type": "integer"},
                    "verse_start": {"type": "integer"},
                    "verse_end": {"type": "integer"}
                },
                "required": ["translation", "book", "chapter", "verse_start", "verse_end"]
            }),
        ),
```

### Step 4: Add the handler to tools.rs

In `crates/presenter-server/src/ai/tools.rs`, add this `match` arm inside `execute_tool()`, placed immediately BEFORE the existing `"resolve_bible_slides"` arm (currently around line 315):

```rust
        "load_bible_verses" => {
            let translation = str_field(&args, "translation")?;
            let book = str_field(&args, "book")?;
            let chapter = args["chapter"].as_u64().unwrap_or(1) as u16;
            let verse_start = args["verse_start"].as_u64().unwrap_or(1) as u16;
            let verse_end = args["verse_end"].as_u64().unwrap_or(verse_start as u64) as u16;

            // Resolve the translation to get its short code for reference labels.
            let translations = state.list_bible_translations().await?;
            let main_trans = match translations
                .iter()
                .find(|t| t.code.eq_ignore_ascii_case(&translation))
            {
                Some(t) => t.clone(),
                None => {
                    return Ok((
                        json!({"error": "translation not found", "translation": translation})
                            .to_string(),
                        format!("Translation '{translation}' not found"),
                    ));
                }
            };
            let short_code = main_trans
                .code
                .rsplit('-')
                .next()
                .unwrap_or(&main_trans.code)
                .to_uppercase();

            // Load the passage range one verse at a time to build the
            // per-verse reference labels. We reuse find_bible_passage so
            // we do not depend on repository-level range APIs here.
            let mut verses: Vec<Value> = Vec::new();
            for v in verse_start..=verse_end {
                let reference = BibleReference {
                    book: book.clone(),
                    book_code: None,
                    book_number: None,
                    chapter,
                    verse_start: v,
                    verse_end: v,
                };
                if let Some(p) = state.find_bible_passage(&main_trans.code, &reference).await? {
                    verses.push(json!({
                        "number": p.reference.verse_start,
                        "text": p.text,
                        "reference": format!(
                            "{} {}:{} ({})",
                            p.reference.book, p.reference.chapter, p.reference.verse_start, short_code
                        ),
                    }));
                }
            }

            let preview = format!(
                "{} {}:{}-{} ({}) - {} verses",
                book,
                chapter,
                verse_start,
                verse_end,
                short_code,
                verses.len()
            );
            Ok((serde_json::to_string(&verses)?, preview))
        }
```

- [ ] **Step 5: Run the new test to verify it passes**

Run: `cargo test -p presenter-server --lib ai::tools::tests::load_bible_verses 2>&1 | tail -10`

Expected: PASS. The handler returns the "translation not found" JSON (not "unknown tool").

- [ ] **Step 6: Format and commit**

```bash
cargo fmt --all
git add crates/presenter-server/src/ai/tool_defs.rs crates/presenter-server/src/ai/tools.rs
git commit -m "feat(ai): add load_bible_verses tool

Returns raw per-verse objects {number, text, reference} for a passage
range. This is the new source-of-truth load step for the AI bible
workflow — the LLM compares each returned verse to the sermon text
and builds an items[] array from the results before calling
create_bible_presentation."
```

---

## Task 5: Rewrite `create_bible_presentation` to accept `items`

**Files:**
- Modify: `crates/presenter-server/src/ai/tool_defs.rs` (replace `create_bible_presentation` schema)
- Modify: `crates/presenter-server/src/ai/tools.rs` (replace handler, thread validator signature change through `add_bible_slide` and `update_bible_slide` for now — those go in Task 6)

### Step 1: Write the failing integration test

Append to the `#[cfg(test)] mod tests { ... }` block in `crates/presenter-server/src/ai/tools.rs` (right after the `load_bible_verses_returns_raw_verses` test):

```rust
    #[tokio::test]
    async fn create_bible_presentation_rejects_oversized_single_verse() {
        let state = AppState::in_memory().await.unwrap();
        let long_text = "a".repeat(400);
        let args = json!({
            "name": "Length Test",
            "items": [
                {
                    "kind": "verse",
                    "number": 1,
                    "text": long_text,
                    "book": "Ján",
                    "chapter": 1,
                    "translation": "SEB"
                }
            ]
        });
        let (body, _preview) =
            execute_tool("create_bible_presentation", &args.to_string(), &state, 320)
                .await
                .unwrap();
        let parsed: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["error"], "slide_validation");
        assert_eq!(parsed["rule"], "main_exceeds_character_limit");
        assert_eq!(parsed["limit"], 320);
    }

    #[tokio::test]
    async fn create_bible_presentation_with_items_composes_server_side() {
        let state = AppState::in_memory().await.unwrap();
        let args = json!({
            "name": "Server-side Composition",
            "items": [
                {"kind": "verse", "number": 1, "text": "Na počiatku bolo Slovo.",
                 "book": "Ján", "chapter": 1, "translation": "SEB"},
                {"kind": "verse", "number": 2, "text": "Ono bolo na počiatku u Boha.",
                 "book": "Ján", "chapter": 1, "translation": "SEB"},
                {"kind": "emphasis", "text": "NOVÁ ZMLUVA"},
                {"kind": "verse", "number": 3, "text": "Všetko vzniklo skrze neho.",
                 "book": "Ján", "chapter": 1, "translation": "SEB"}
            ]
        });
        let (body, _preview) =
            execute_tool("create_bible_presentation", &args.to_string(), &state, 320)
                .await
                .unwrap();
        let parsed: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["name"].as_str().unwrap(), "Server-side Composition");
        // 2 verses batched into 1 slide + 1 emphasis slide + 1 verse slide = 3 slides
        assert_eq!(parsed["slide_count"].as_u64().unwrap(), 3);

        // Verify actual persisted slides
        let pres_id_str = parsed["id"].as_str().unwrap();
        let pres_id = BiblePresentationId::from_uuid(Uuid::parse_str(pres_id_str).unwrap());
        let pres = state
            .bible_presentation_detail(pres_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(pres.slides.len(), 3);

        // Slide 0: verses 1-2 with range reference
        assert_eq!(pres.slides[0].main_reference, "Ján 1:1-2 (SEB)");
        assert!(pres.slides[0].main.value().contains("1. Na počiatku"));
        assert!(pres.slides[0].main.value().contains("2. Ono bolo"));

        // Slide 1: emphasis
        assert_eq!(pres.slides[1].main_reference, "");
        assert_eq!(pres.slides[1].main.value(), "NOVÁ ZMLUVA");

        // Slide 2: verse 3
        assert_eq!(pres.slides[2].main_reference, "Ján 1:3 (SEB)");
    }

    #[tokio::test]
    async fn create_bible_presentation_empty_name_rejected() {
        let state = AppState::in_memory().await.unwrap();
        let args = json!({"name": "", "items": []});
        // Empty name is still accepted by the underlying create. Empty items
        // is explicitly rejected as a misuse.
        let result =
            execute_tool("create_bible_presentation", &args.to_string(), &state, 320).await;
        // Either error JSON about empty items, or Err.
        let (body, _preview) = result.unwrap();
        let parsed: Value = serde_json::from_str(&body).unwrap();
        assert!(
            parsed["error"].is_string() || parsed["slide_count"].as_u64() == Some(0),
            "empty items should error or produce zero slides, got: {}",
            body
        );
    }
```

The existing test `create_bible_presentation_with_slides` (which uses the old `slides` array shape) must be DELETED — it is now invalid input for the rewritten tool. Find and delete it:

Run: `grep -n 'create_bible_presentation_with_slides' crates/presenter-server/src/ai/tools.rs`

Then remove the `#[tokio::test] async fn create_bible_presentation_with_slides() { ... }` block entirely.

- [ ] **Step 2: Run tests to verify the new ones fail**

Run: `cargo test -p presenter-server --lib ai::tools::tests::create_bible_presentation 2>&1 | tail -25`

Expected: FAIL (the handler does not know about `items` yet, it looks for `slides`).

### Step 3: Rewrite the schema in tool_defs.rs

In `crates/presenter-server/src/ai/tool_defs.rs`, **replace** the existing `create_bible_presentation` schema (currently lines 250-274) with:

```rust
        tool_def(
            "create_bible_presentation",
            "[BIBLE only] Create a Bible presentation from a stream of typed items. The server composes slides from your items — you do NOT decide where slide breaks happen. Emphasis items and translation/book/chapter changes force slide breaks; otherwise consecutive verse items pack together until the character limit. Always call load_bible_verses first to get DB verse text, edit the text to match the sermon where needed, then assemble items[] in sermon order.",
            json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "Presentation name (e.g. 'Sunday Sermon 2026-04-14')"},
                    "items": {
                        "type": "array",
                        "description": "Ordered stream of verse and emphasis items. The server composes slides respecting the character limit.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "kind": {"type": "string", "enum": ["verse", "emphasis"]},
                                "number": {"type": "integer", "description": "[verse] Verse number"},
                                "text": {"type": "string", "description": "Verse text (with any uppercase ##word## transformations applied) or the emphasis phrase"},
                                "book": {"type": "string", "description": "[verse] Full book name (e.g. Ján)"},
                                "chapter": {"type": "integer", "description": "[verse] Chapter number"},
                                "translation": {"type": "string", "description": "[verse] Short translation code (e.g. SEB, MIL, ROH)"}
                            },
                            "required": ["kind", "text"]
                        }
                    }
                },
                "required": ["name", "items"]
            }),
        ),
```

### Step 4: Rewrite the handler in tools.rs

First, add this import at the top of `crates/presenter-server/src/ai/tools.rs` (merge with the existing `use crate::state` import):

```rust
use crate::state::slides::{compose_bible_items_into_slides, BibleItem, ComposedBibleSlide};
```

Then **replace** the entire `"create_bible_presentation" => { ... }` arm in `execute_tool()` (currently lines 457-526) with:

```rust
        "create_bible_presentation" => {
            let name = str_field(&args, "name")?;
            let items_arr = match args["items"].as_array() {
                Some(arr) => arr,
                None => {
                    return Ok((
                        json!({"error": "missing_items", "expected": "items must be an array of verse/emphasis objects"}).to_string(),
                        "Missing items array".to_string(),
                    ));
                }
            };

            // Parse items into typed BibleItem values.
            let mut items: Vec<BibleItem> = Vec::with_capacity(items_arr.len());
            for (idx, raw) in items_arr.iter().enumerate() {
                let kind = raw["kind"].as_str().unwrap_or("");
                match kind {
                    "verse" => {
                        let number = raw["number"].as_u64().unwrap_or(0) as u32;
                        let text = raw["text"].as_str().unwrap_or("").to_string();
                        let book = raw["book"].as_str().unwrap_or("").to_string();
                        let chapter = raw["chapter"].as_u64().unwrap_or(0) as u32;
                        let translation = raw["translation"].as_str().unwrap_or("").to_string();
                        if number == 0 || text.is_empty() || book.is_empty() || chapter == 0 || translation.is_empty() {
                            return Ok((
                                json!({
                                    "error": "invalid_verse_item",
                                    "expected": "verse items require number>=1, non-empty text, book, chapter>=1, translation",
                                    "got": format!("item[{idx}]"),
                                })
                                .to_string(),
                                format!("Invalid verse item at index {idx}"),
                            ));
                        }
                        items.push(BibleItem::Verse {
                            number,
                            text,
                            book,
                            chapter,
                            translation,
                        });
                    }
                    "emphasis" => {
                        let text = raw["text"].as_str().unwrap_or("").to_string();
                        if text.trim().is_empty() {
                            return Ok((
                                json!({
                                    "error": "invalid_emphasis_item",
                                    "expected": "emphasis items require non-empty text",
                                    "got": format!("item[{idx}]"),
                                })
                                .to_string(),
                                format!("Invalid emphasis item at index {idx}"),
                            ));
                        }
                        items.push(BibleItem::Emphasis { text });
                    }
                    other => {
                        return Ok((
                            json!({
                                "error": "invalid_item_kind",
                                "expected": "kind must be 'verse' or 'emphasis'",
                                "got": format!("item[{idx}] kind={other}"),
                            })
                            .to_string(),
                            format!("Invalid kind '{other}' at index {idx}"),
                        ));
                    }
                }
            }

            // Compose slides server-side using the configured character limit.
            let composed: Vec<ComposedBibleSlide> =
                compose_bible_items_into_slides(&items, default_char_limit);

            // Validate each composed slide. Fail-safe; with a correct composer
            // only the oversized-single-verse case should ever trip this.
            for (idx, slide) in composed.iter().enumerate() {
                if let Err(mut err) =
                    validate_bible_slide(&slide.main, &slide.main_reference, default_char_limit)
                {
                    err.got = format!("composed_slide[{idx}]: {}", err.got);
                    return Ok(validation_error_response(err));
                }
            }

            // Persist.
            let presentation = state.create_bible_presentation(&name).await?;
            let final_presentation = if composed.is_empty() {
                presentation
            } else {
                let new_slides: Vec<BiblePresentationSlide> = composed
                    .into_iter()
                    .map(|c| BiblePresentationSlide {
                        id: BibleSlideId::new(),
                        order: 0,
                        main: SlideText::new(&c.main)
                            .unwrap_or_else(|_| SlideText::new("").unwrap()),
                        main_reference: c.main_reference,
                        secondary: SlideText::new("").unwrap(),
                        secondary_reference: String::new(),
                        metadata: None,
                    })
                    .collect();
                state
                    .append_bible_presentation_slides(presentation.id, new_slides)
                    .await?
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

Also, propagate the new validator signature to `add_bible_slide` and `update_bible_slide` handlers (they are still present at this point — Task 6 removes them). Find each `validate_bible_slide(&main_text, &main_reference)` call in the file and change it to `validate_bible_slide(&main_text, &main_reference, default_char_limit)`:

Run: `grep -n 'validate_bible_slide(' crates/presenter-server/src/ai/tools.rs`

Update each call site accordingly so the code still compiles.

- [ ] **Step 5: Run tests to verify the new ones pass**

Run: `cargo test -p presenter-server --lib ai::tools:: 2>&1 | tail -30`

Expected: the 3 new `create_bible_presentation_*` tests pass. Other existing `ai::tools::tests` still pass. If `add_bible_slide_appends` or similar tests fail because they pass slides without a length-compliant shape, let them — they are deleted in Task 6.

Note: `add_bible_slide_appends` and similar tests may temporarily break because the existing `add_bible_slide` tool handler still references `validate_bible_slide` with a new signature. They are removed in Task 6. For THIS task, just ensure the compile error is fixed and the 3 new tests pass. You may temporarily mark the failing old tests with `#[ignore]` and remove them in Task 6 — but cleaner to just make sure the signature change propagates and the tests compile, even if some assertions are now stale (Task 6 removes them outright).

- [ ] **Step 6: Format and commit**

```bash
cargo fmt --all
git add crates/presenter-server/src/ai/tool_defs.rs crates/presenter-server/src/ai/tools.rs
git commit -m "feat(ai): rewrite create_bible_presentation to take items[]

The tool now accepts a typed verse/emphasis item stream. Server
composes slides via compose_bible_items_into_slides respecting the
configured character limit, then runs the validator (now 5 rules)
on each composed slide before persisting. The LLM no longer decides
slide breaks — that responsibility moves to the server so the
character limit is always honored.

add_bible_slide and update_bible_slide handler calls are updated
to the new validator signature; the tools themselves are removed
in the next commit."
```

---

## Task 6: Remove `add_bible_slide` and `update_bible_slide`

**Why:** The verse-granularity contract doesn't survive piecemeal edits. Removing these tools forces the AI to rebuild full items arrays, which is the only way to guarantee the character limit holds across edits.

**Files:**
- Modify: `crates/presenter-server/src/ai/tool_defs.rs` (delete schemas)
- Modify: `crates/presenter-server/src/ai/tools.rs` (delete handlers + tests)

### Step 1: Remove the tool schemas

In `crates/presenter-server/src/ai/tool_defs.rs`, delete the `tool_def("add_bible_slide", ...)` block (lines ~298-312) and the `tool_def("update_bible_slide", ...)` block (lines ~313-328) entirely.

### Step 2: Remove the handlers

In `crates/presenter-server/src/ai/tools.rs`, delete the entire `"add_bible_slide" => { ... }` arm (~lines 545-582) and the entire `"update_bible_slide" => { ... }` arm (~lines 584-625) from the `match name` block in `execute_tool()`. The `_` fallthrough will handle any stray LLM calls to these names as "unknown tool".

### Step 3: Remove the old tests

In the same file's test module, delete these test functions:

- `add_bible_slide_appends`
- Any test named `update_bible_slide_*`
- Any other test that references `"add_bible_slide"` or `"update_bible_slide"` as the tool name to `execute_tool`

Find them with:

Run: `grep -n 'add_bible_slide\|update_bible_slide' crates/presenter-server/src/ai/tools.rs`

Delete each `#[tokio::test] async fn ...` block that references these tool names.

- [ ] **Step 4: Run the full test module**

Run: `cargo test -p presenter-server --lib ai:: 2>&1 | tail -25`

Expected: all remaining tests pass. No compilation errors. No leftover references to deleted symbols.

- [ ] **Step 5: Check file size is still under the cap**

Run: `wc -l crates/presenter-server/src/ai/tools.rs`

Expected: fewer lines than before (we removed more than we added). Still under 1000 production lines (the test module is exempt).

- [ ] **Step 6: Clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -15`

Expected: zero warnings.

- [ ] **Step 7: Format and commit**

```bash
cargo fmt --all
git add crates/presenter-server/src/ai/tool_defs.rs crates/presenter-server/src/ai/tools.rs
git commit -m "refactor(ai): remove add_bible_slide and update_bible_slide tools

These piecemeal-edit tools cannot maintain the character-limit
invariant across edits because each edit would need to re-run the
whole-presentation composer. The AI now rebuilds the full items
array and calls create_bible_presentation for any change."
```

---

## Task 7: System prompt rewrite

**Files:**
- Modify: `crates/presenter-server/src/ai/agent.rs` (replace `## Creating Bible slides` section)

### Step 1: Replace the prompt section

In `crates/presenter-server/src/ai/agent.rs`, replace the block from `## Creating Bible slides` (line 124) through the end of item 7 (line 165 — the `"rule" and "expected" fields` line) with this new block:

```rust
## Creating Bible slides

You do NOT decide where slides break. The server composes slides from a
typed stream of items you submit. You pick the items; the server decides
how many slides they become.

1. Parse the sermon text yourself: find passage references (##Book Ch:V##
   or ##Book Ch:V-V##), ##bold## markers inside verses, and any ##title##
   at the very start (use as presentation name).

2. For each passage: call `load_bible_verses(book, chapter, verse_start,
   verse_end, translation)` to get the raw DB verses as an array of
   {number, text, reference} objects. This is the source of truth for
   verse text. Never invent verses from memory.

3. For each loaded verse, compare its `text` to the sermon's wording.
   The sermon is authoritative for text content. If they differ, REPLACE
   the `text` field with the sermon's wording. If the pastor quotes a
   verse number that does not match the DB (e.g. says Ján 3:16 but quotes
   Ján 3:17 text), keep the sermon's text and the sermon's verse number.

4. Apply ##word## markers: inside a verse, replace `word` with `WORD`
   (uppercase) inline. The result stays as a single verse item — do NOT
   create a separate slide for in-verse emphasis.

5. Extract ##phrase## markers that appear as standalone emphasis (not a
   reference, not inside a verse): emit a separate `{"kind": "emphasis",
   "text": "PHRASE"}` item at the position where the phrase appears in
   the sermon. Phrase text goes uppercase.

6. Assemble an `items[]` array in sermon order:

   ```json
   [
     {"kind": "verse", "number": 1, "text": "Na počiatku bolo Slovo.",
      "book": "Ján", "chapter": 1, "translation": "SEB"},
     {"kind": "verse", "number": 2, "text": "Ono bolo na počiatku.",
      "book": "Ján", "chapter": 1, "translation": "SEB"},
     {"kind": "emphasis", "text": "NOVÁ ZMLUVA"},
     {"kind": "verse", "number": 3, "text": "Všetko vzniklo.",
      "book": "Ján", "chapter": 1, "translation": "SEB"}
   ]
   ```

   Verse items MUST include `number`, `text`, `book`, `chapter`,
   `translation` (short code like SEB, MIL, ROH). Emphasis items need
   only `kind` and `text`.

7. Call `create_bible_presentation(name, items)`. The server greedy-packs
   consecutive verse items into slides until the character limit ({char_limit}
   chars) would overflow, then flushes. Emphasis items and translation /
   book / chapter changes force slide breaks. The server auto-computes
   reference labels like "Ján 1:1-2 (SEB)".

8. If a single verse is longer than the character limit on its own
   (rare), the validator returns `main_exceeds_character_limit`. Recovery:
   split that verse into multiple verse items with the same `number` —
   the server will emit them as separate slides both labeled with the
   same verse number.

9. The server validates composed slides and returns a rule-keyed JSON
   error on failure (rules: `main_exceeds_character_limit`,
   `unprocessed_bold_markers`, `empty_main_on_emphasis_slide`,
   `reference_format_requires_parens`, `missing_verse_number_prefix`).
   Read the `rule` and `expected` fields, fix the item, and retry.
```

This replaces rules 1-7 of the old section. Keep the `## Rules` section (line 167+) unchanged.

### Step 2: Build and run the agent tests (if any exist)

Run: `cargo test -p presenter-server --lib ai::agent 2>&1 | tail -15`

Expected: passes (if there are agent tests) or "0 tests run" (if there aren't — the prompt change is textual only and compiles as a format string).

- [ ] **Step 3: Build the whole workspace to confirm everything compiles**

Run: `cargo build -p presenter-server 2>&1 | tail -15`

Expected: clean build.

- [ ] **Step 4: Format and commit**

```bash
cargo fmt --all
git add crates/presenter-server/src/ai/agent.rs
git commit -m "feat(ai): rewrite bible slide prompt for verse-granularity flow

The prompt now instructs the LLM to call load_bible_verses, compare
and override text against the sermon, assemble an items[] stream,
and call create_bible_presentation. The server is now responsible
for slide breaks — the LLM only picks items."
```

---

## Task 8: Playwright E2E test

**Files:**
- Create: `tests/e2e/ai-chat-bible-composition.spec.ts`

### Step 1: Explore existing AI chat E2E scaffolding

Run: `ls tests/e2e/ai*.spec.ts 2>&1; ls tests/e2e/*.ts | head -20`

Find an existing AI chat E2E test that already:
- Starts the server
- Has a helper for sending `/ai/chat` requests
- Uses Playwright's console-zero-errors pattern

If one exists, read it and copy the scaffolding. If none exists, use the existing `startTestServer` helper that other E2E tests use (find it with `grep -rn 'startTestServer' tests/e2e/ | head -5`).

### Step 2: Write the test

Create `tests/e2e/ai-chat-bible-composition.spec.ts`:

```typescript
import { test, expect } from '@playwright/test';
import { startTestServer, type TestServer } from './helpers/server';

test.describe('AI bible slide composition respects character limit', () => {
  let server: TestServer;

  test.beforeAll(async () => {
    server = await startTestServer();
  });

  test.afterAll(async () => {
    await server.stop();
  });

  test('AI-created bible slides obey the character limit', async ({ page }) => {
    const consoleMessages: string[] = [];
    page.on('console', (msg) => {
      if (msg.type() === 'error' || msg.type() === 'warning') {
        consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
      }
    });

    // 1. Hit the AI chat endpoint with a long bible passage request.
    // This is a direct API test because the AI chat UI may not be wired up.
    // The test exercises the dispatch path through execute_tool.
    const createResponse = await page.request.post(`${server.baseUrl}/ai/chat`, {
      data: {
        message: 'Create a bible presentation with Ján 1:1-14 from slk-seb, title "E2E Composition Test"',
      },
    });

    expect(createResponse.ok(), `create response not ok: ${await createResponse.text()}`).toBeTruthy();

    // 2. Fetch bible presentations and find the one we just created.
    const listResponse = await page.request.get(`${server.baseUrl}/bible/presentations`);
    expect(listResponse.ok()).toBeTruthy();
    const presentations = await listResponse.json();
    const created = presentations.find((p: any) => p.name === 'E2E Composition Test');
    expect(created, 'created presentation should be in the list').toBeTruthy();

    // 3. Fetch the full presentation with its slides.
    const detailResponse = await page.request.get(
      `${server.baseUrl}/bible/presentations/${created.id}`,
    );
    expect(detailResponse.ok()).toBeTruthy();
    const detail = await detailResponse.json();

    // 4. Assert: every slide's main fits under the character limit.
    const charLimit = 320;
    for (const slide of detail.slides) {
      expect(
        slide.main.length,
        `slide main='${slide.main}' is ${slide.main.length} chars, limit is ${charLimit}`,
      ).toBeLessThanOrEqual(charLimit);
    }

    // 5. Assert: there is more than one slide (proving the server split the long passage).
    expect(detail.slides.length).toBeGreaterThan(1);

    // 6. Assert clean console.
    expect(consoleMessages).toEqual([]);
  });
});
```

**Note on this test:** it depends on (a) `/ai/chat` being reachable from Playwright, (b) the dev server having Bible translations seeded with Ján 1:1-14 available, and (c) an `/ai/chat` endpoint that accepts the message shape shown above. If any of these assumptions are wrong in this repo, adapt to the actual endpoint — the CRITICAL assertions (character limit on all slides, multiple slides emitted) must survive the adaptation.

Discovery before writing — run these first:

```bash
grep -rn 'fn.*ai_chat\|"/ai/chat"' crates/presenter-server/src/router/
grep -rn '/bible/presentations' crates/presenter-server/src/router/
```

If `/ai/chat` expects a different body shape (e.g. `{"messages": [{"role": "user", "content": "..."}]}`), update the test payload to match.

If the AI chat requires a real LLM backend not available in CI, **rewrite the test to call `execute_tool` via a test-only HTTP shim OR mark the test `test.skip` with a clear comment.** Do NOT delete the assertion on character limit — that is the whole point of the test.

Preferred alternative if `/ai/chat` is not CI-reachable: write an **integration test in Rust** (not Playwright) at `crates/presenter-server/src/ai/tools.rs` test module that directly calls `execute_tool("create_bible_presentation", ...)` with pre-built items that would overflow a single slide at the default limit and asserts the composed slide count is > 1 and each slide fits. This exercises the exact same code path without requiring a live LLM.

### Step 3: Run the test

Run: `npm run test:playwright -- ai-chat-bible-composition 2>&1 | tail -30`

Expected: PASS. If the test fails due to infrastructure (LLM not reachable, endpoint shape wrong), switch to the Rust integration test alternative described above.

- [ ] **Step 4: Commit**

```bash
git add tests/e2e/ai-chat-bible-composition.spec.ts
git commit -m "test(e2e): add AI bible slide composition character limit E2E

Verifies that AI-created bible presentations obey the configured
character limit. Creates a long passage through /ai/chat, fetches
the resulting slides, asserts every slide's main fits under the
limit and that the server split the passage into multiple slides."
```

---

## Task 9: Local build, CI, PR, manual dev verification, merge, production verify

**Files:** no file changes in this task — this is the push-and-verify task.

### Step 1: Full local check

Run these in sequence:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
cargo test -p presenter-server --lib
cargo build --release -p presenter-server
```

Expected: all green. If any step fails, fix in a single new commit (don't amend).

### Step 2: Push to dev

```bash
git fetch origin
git status
git push origin dev
```

### Step 3: Monitor CI

```bash
gh run list --branch dev --limit 3
```

Pick the newest run id; view it until it reaches a terminal state:

```bash
gh run view <id> --json status,conclusion,jobs
```

**Do NOT poll in a tight loop.** Use a single `sleep 600 && gh run view <id>` and wait for the notification. If any job fails, `gh run view <id> --log-failed`, fix ALL failures in ONE commit, push, monitor again.

### Step 4: Manual verification on dev

After CI's deploy-dev job succeeds (dev URL `http://10.77.8.134:8080`), exercise the real flow:

```bash
# Health check
curl -s http://10.77.8.134:8080/healthz

# Expected: {"channel":"dev","status":"ok","version":"0.4.18"}
```

Then open `http://10.77.8.134:8080/ui/operator` in Playwright (via browser MCP) and:

1. Trigger an AI chat with a sermon containing a long Ján passage (use a real sermon text file from the project if available, or type one in).
2. Wait for the presentation to be created.
3. Open the generated bible presentation.
4. Inspect every slide's `main` text length — confirm NO slide exceeds 320 characters.
5. Confirm emphasis slides appear as standalone slides with empty references.
6. Confirm references use the range form "Ján 1:1-5 (SEB)" where verses are packed, and single form "Ján 1:6 (SEB)" where they are not.
7. Check browser console — must be clean (zero errors, zero warnings).

If any check fails, investigate root cause, fix on `dev`, re-push, re-verify.

### Step 5: Open the PR

```bash
gh pr create --base main --head dev --title "fix(ai): AI bible slides obey character limit (verse-granularity composer)" --body "$(cat <<'EOF'
## Summary

- Move bible-slide break decisions from LLM to server. AI now works at verse granularity via `load_bible_verses` + `create_bible_presentation(items[])`; server composes with `compose_bible_items_into_slides` respecting the configured character limit (matches live mode).
- Add 5th validator rule `MainExceedsCharacterLimit` as fail-safe.
- Remove `add_bible_slide` and `update_bible_slide` tools — the verse-granularity contract cannot survive piecemeal edits.
- System prompt rewritten for the new workflow: load, compare to sermon, edit text, emit items, submit.
- Translation / book / chapter change forces a slide break.

Spec: `docs/superpowers/specs/2026-04-11-ai-bible-slide-composition-design.md`
Plan: `docs/superpowers/plans/2026-04-11-ai-bible-slide-composition.md`

Fixes: AI-created bible presentations ignoring the character limit while live mode (DB-loaded) obeys it.

## Test plan

- [ ] Unit: `compose_bible_items_into_slides` — 9 tests covering fit/overflow/emphasis/translation-break/book-break/chapter-break/empty/oversized/adjacent-emphasis
- [ ] Unit: validator length rule — 5 tests at/over/well-over/emphasis/json-shape
- [ ] Integration: `create_bible_presentation` with items — 3 tests composing/rejecting
- [ ] E2E: Playwright ai-chat-bible-composition.spec.ts — real AI path through /ai/chat
- [ ] Manual: dev server verification with a real long Ján passage
- [ ] CI: all jobs green including mutation testing on the new composer and validator rule

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

### Step 6: Monitor the PR's CI

Wait for the PR's checks to go green (may duplicate the branch run — watch both).

Verify the PR is mergeable:

```bash
gh pr view <pr-number> --json mergeable,mergeStateStatus
```

Expected: `mergeable: true, mergeStateStatus: clean`.

### Step 7: Wait for user approval

**Do NOT merge.** Hand the PR URL to the user and wait for explicit merge instruction ("merge it", "approved", etc.). A green PR is not permission to merge.

### Step 8: After user says merge

```bash
gh pr merge <pr-number> --merge
git fetch origin
git log -1 origin/main
```

Then monitor the production deploy workflow:

```bash
gh run list --branch main --limit 3
```

Use a single `sleep 720 && gh run view <id>` wait. When it completes:

```bash
curl -s http://10.77.9.205/healthz
# Expected: {"channel":"release","status":"ok","version":"0.4.18"}
```

Open `http://10.77.9.205/ui/operator` in Playwright and re-run steps from §Step 4 against production. Report success with evidence.

---

## Verification Summary

| Check | How to verify |
|-------|---------------|
| Composer packs and splits correctly | Unit test `compose_items_two_verses_that_overflow_emit_two_slides` |
| Emphasis forces slide break | Unit test `compose_items_emphasis_between_verses_breaks_slide` |
| Translation change forces break | Unit test `compose_items_translation_change_forces_break` |
| Validator rejects oversized single verse | Unit test `length_rule_rejects_slide_one_char_over_limit` |
| Tool dispatch composes server-side | Integration test `create_bible_presentation_with_items_composes_server_side` |
| Real AI path respects limit | Playwright test + manual dev verification with real sermon |
| Production works | Manual check of http://10.77.9.205 after deploy |

---

## Notes for the implementer

- Every task commits independently so `git bisect` works cleanly if something regresses.
- Tasks 2 and 3 are independent — you could swap their order. Keep Task 2 first because Task 3's tests are simpler and Task 2's existence does not depend on Task 3.
- Do NOT skip Task 1 (version bump). CI's version-check job will fail the whole run if you forget.
- Do NOT combine commits. Each task is its own commit for readability and bisect.
- Do NOT delete the existing `compose_bible_slides` function. It is used by the live path and is not being replaced.
- Do NOT rename existing public types — only ADD new ones.
- If local `cargo test` produces flaky SIGSEGV in `presenter-persistence` (see project memory), run `cargo clean -p presenter-persistence` then retry. The new tests are all in `presenter-server` so this is unlikely to bite.
