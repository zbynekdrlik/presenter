# AI Bible Slide Validation — Design Spec

**Date:** 2026-04-11
**Scope:** Force the AI to emit correctly-formatted bible slides by combining server-side hard validation with restored in-prompt rules. Follow-up to PR #235 (AI mode cleanup).
**Approach:** Server-side validator in the bible tool dispatch path + restored critical rules in the main system prompt. The AI still does the sermon parsing and the passage editing — the server just refuses to store malformed output.

## Problem

After PR #235 shipped, the user reported that AI-created bible presentations are malformed in three concrete ways:

1. **Missing verse number prefixes.** The AI writes `main = "A nieto tvora, čo by bol preň neviditeľný…"` instead of `main = "13. A nieto tvora…"`. Bible slides must have `N. ` prefixes so operators can see which verse they're reading on stage.

2. **Reference format has no parentheses around the translation code.** The AI writes `main_reference = "Židom 4:13 SEB"` instead of `"Židom 4:13 (SEB)"` — or alternatively, just `"Židom 4:13"` when it doesn't know the translation. The current bad output is the worst case: parens are missing AND the code is present, so the reference looks broken on stage.

3. **Bold markers (`##...##`) from the pastor's sermon email are not processed.** The pastor uses `##word##` inside verses to mark emphasis (should become UPPERCASE inside the verse text) and `##phrase##` on standalone lines to mark emphasis slides (should become a separate slide with UPPERCASE main and no reference). The AI is either leaving the raw markers in place or ignoring them entirely.

## Root cause

PR #235 moved the detailed formatting rules out of the system prompt and into an on-demand `get_style_guide` tool. The intent was to save prompt tokens. The outcome is that the LLM rarely calls `get_style_guide`, so it generates slides from memory and gets the format wrong.

Worse: the bible tools (`create_bible_presentation`, `add_bible_slide`, `update_bible_slide`) accept arbitrary `main`/`main_reference` strings and write them verbatim to the database. There is no validator between the AI and the DB. The model can ship any garbage it wants.

We already have an authoritative slide composer (`state/slides.rs::compose_bible_slides`) used by `resolve_bible_slides`, and it always produces correct format. But the AI typically does not route through it — it hand-constructs slides instead because the pastor's sermon often contains paraphrased or adjusted verse text that needs to match the sermon exactly, not the DB verbatim.

## Goals

- Impossible to store a malformed bible slide via AI tools. Any attempt returns an explicit rule-keyed error the LLM can act on.
- AI produces correct slides on the **first** tool call, not on a retry after rejection. System prompt must carry the critical rules inline.
- Emphasis / title slides (no reference, no verse numbers) remain possible through the same tools.
- No auto-fixing and no silent coercion. If the input is wrong, the server says so and names the broken rule.

## Non-goals

- No auto-migration of existing malformed slides in the database. User will clean those up separately if needed.
- No new AI tool for parsing sermon text server-side. The AI still does the parse-and-edit work itself.
- No change to `resolve_bible_slides` or `compose_bible_slides`. They already produce correct output for the cases where the AI wants verbatim DB text.
- No change to the worship-slide tools. Worship slides are free-form by design.
- No feature flag. Strict from day one.

## Architecture

Unchanged agent loop, unchanged tool dispatch shape. Two point-fixes:

1. **New module `ai/bible_validator.rs`** containing a pure function `validate_bible_slide(main: &str, main_reference: &str) -> Result<(), ValidationError>`. Pure string-in, error-out — no DB access, no state. Trivial to unit test and mutation test.

2. **`ai/tools.rs` dispatch path** for `create_bible_presentation`, `add_bible_slide`, and `update_bible_slide` calls `validate_bible_slide` on every incoming slide **before** any DB mutation. On failure it short-circuits and returns a tool-call error with `rule=<name>`, `got=<input>`, `expected=<explanation>`. The error flows through the existing tool-result path and the LLM sees it on the next iteration of the agent loop.

3. **`ai/agent.rs::build_system_prompt`** regains a ~30-line "Creating Bible slides" block covering the seven critical rules (parse sermon, load passage from DB, verse number prefixes, reference format with optional parens, shared full-range reference on multi-slide passages, bold marker handling, validator error handling).

4. **`ai/style_guide.md` and `get_style_guide`** stay as-is. They continue to hold the secondary reference material (Slovak book abbreviations, translation code mapping table, partial-verse syntax). The tool becomes genuinely optional — the hot-path rules are now in the main prompt.

## Validation rules

The validator enforces four rules. All are applied to every slide passed to `create_bible_presentation` (per slide in the array), `add_bible_slide`, and `update_bible_slide`.

### Rule 1 — Reference format

When `main_reference` is non-empty, it must match:

```
^[A-Za-zÀ-ž0-9\. ]+ \d+:\d+[a-z]?(-\d+[a-z]?)?( \([A-Z]+\))?$
```

Breakdown:

- `[A-Za-zÀ-ž0-9\. ]+` — book name (letters including Slovak diacritics, digits for "1. Samuelova", dots, spaces)
- ` \d+` — chapter number preceded by a space
- `:\d+` — verse number after colon
- `[a-z]?` — optional partial-verse suffix like `3a`
- `(-\d+[a-z]?)?` — optional verse range end
- `( \([A-Z]+\))?` — optional translation code in parentheses, preceded by a space

Acceptance examples:

- `"Ján 1:1-51 (MIL)"` ✅
- `"Žalm 26:3a (ROH)"` ✅
- `"Ján 3:16"` ✅ (no code — allowed per user instruction)
- `"1. Samuelova 17:33-37 (SEB)"` ✅
- `"Židom 4:13 (SEB)"` ✅

Rejection examples:

- `"Židom 4:13 SEB"` ❌ — code without parens (the exact bug in production)
- `"Ján 1:1-51 MIL"` ❌ — same
- `"John 3:16 (seb)"` ❌ — code must be uppercase
- `""` is NOT rejected — empty reference means emphasis/title slide (see rule 2)

Error response:

```json
{
  "error": "slide_validation",
  "rule": "reference_format_requires_parens",
  "got": "Židom 4:13 SEB",
  "expected": "Format is \"Book Chapter:Verse(-Verse) (CODE)\" with parens around the translation code, or omit the code entirely. Correct: \"Židom 4:13 (SEB)\" or \"Židom 4:13\"."
}
```

### Rule 2 — Verse number prefix

When `main_reference` is non-empty (slide is a verse slide, not an emphasis/title slide), `main` must contain at least one line that starts with `^\d+\. ` after trimming leading whitespace on that line. Multi-line `main` is allowed and common; every line is expected to begin with a verse number, but we only hard-assert that at least one does — this keeps the rule robust to wrapped lines and occasional formatting quirks.

Acceptance examples:

- `"1. Na počiatku bolo Slovo..."` ✅
- `"1. Na počiatku...\n2. Ono bolo...\n3. Všetko vzniklo..."` ✅
- `"13. A nieto tvora, čo by bol preň neviditeľný. Všetko je obnažené..."` ✅

Rejection examples:

- `"A nieto tvora, čo by bol preň neviditeľný..."` ❌ — the exact bug in production
- `"Na počiatku bolo Slovo..."` ❌

Error response:

```json
{
  "error": "slide_validation",
  "rule": "missing_verse_number_prefix",
  "got": "A nieto tvora, čo by bol preň neviditeľný...",
  "expected": "Verse slides must start each verse line with its verse number: \"13. A nieto tvora...\"."
}
```

### Rule 3 — No raw bold markers

Neither `main` nor `main_reference` may contain the substring `##` anywhere. Raw markers mean the AI forgot to process a `##word##` or `##phrase##` token from the pastor's sermon.

Acceptance examples:

- `"1. aby sme VERILI menu jeho Syna"` ✅ (properly converted to uppercase in-line)
- `"NOVÁ ZMLUVA"` ✅ (properly converted emphasis slide)

Rejection examples:

- `"1. aby sme ##verili## menu..."` ❌
- `"##NOVÁ ZMLUVA##"` ❌
- `"##Ján 1:1-51##"` should never appear — references should be stripped by the parser

Error response:

```json
{
  "error": "slide_validation",
  "rule": "unprocessed_bold_markers",
  "got": "1. aby sme ##verili## menu...",
  "expected": "Strip ## markers from slide text. ##word## inside a verse becomes WORD in uppercase: \"1. aby sme VERILI menu\". ##phrase## on a standalone line becomes a separate emphasis slide with main = phrase in uppercase and empty main_reference."
}
```

### Rule 4 — Emphasis / title slides

When `main_reference` is empty (`""`), the slide is treated as an emphasis or title slide:

- Rule 1 skipped (no reference to validate).
- Rule 2 skipped (no verse number required).
- Rule 3 still applied (no raw `##` markers).
- `main` must be non-empty after trimming.

This lets the AI create a slide like:

```json
{"main": "NOVÁ ZMLUVA", "main_reference": ""}
```

which becomes a standalone emphasis slide.

## System prompt changes

`build_system_prompt` in `ai/agent.rs` gains a "Creating Bible slides" block inserted after the bible presentations list and before the existing "Rules" section. The content is given verbatim below; numbered for clarity.

```text
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

7. The server validates these rules and will return an error naming the
   broken rule if you get it wrong. Fix the error and retry.
```

The existing seven-line "Rules" block stays — it covers worship/bible separation, the delete intent gate, and the `get_style_guide` pointer. Total system prompt grows from ~40 to ~70 lines. Worth it.

## Error handling

Validation errors short-circuit the tool dispatch **before** any DB write or `state` mutation. The error is returned through the existing tool-result path:

- `preview`: `"Validation failed: <rule_name>"` (short, for UI badges)
- `content`: full JSON error object above (sent back to the LLM as the tool result)

The LLM's next iteration sees the error and can self-correct. The delete intent gate and the existing error-path in the dispatch loop already handle this pattern, so the new errors integrate without new code in `agent.rs`.

Batch case (`create_bible_presentation` with a slides array): the validator runs on each slide in order and short-circuits on the first failure. The partial presentation is NOT created — the handler validates all slides before calling `create_bible_presentation` on the DB. This keeps the server side all-or-nothing and simplifies error recovery for the AI (no "I created 3 of 5, retry with the last 2" edge case).

## Data flow

```
user sermon text
       │
       ▼
┌──────────────┐
│   AI parses  │  ← system prompt tells it how
└──────┬───────┘
       │
       ├──► get_bible_passage / resolve_bible_slides   (load authoritative text)
       │
       ▼
┌──────────────┐
│   AI edits   │  ← to match pastor's version, adds verse numbers,
│  slides      │    applies bold markers, formats reference
└──────┬───────┘
       │
       ▼
┌──────────────────────────┐
│ create_bible_presentation │
│  or add_bible_slide       │
│  or update_bible_slide    │
└──────┬────────────────────┘
       │
       ▼
┌──────────────────────────┐
│ validate_bible_slide     │  ← per slide, pure function
│  (ai/bible_validator.rs) │
└──────┬───────────────────┘
       │
       ├── OK ──► state.append_bible_presentation_slides(...) ──► DB
       │
       └── Err ──► tool-result JSON ──► LLM retries
```

## Testing strategy

### Unit tests — validator (`ai/bible_validator.rs`)

Pure function, no state. One test per rule acceptance + one per rejection + edge cases:

- `reference_format_accepts_standard_range` — `"Ján 1:1-51 (MIL)"`
- `reference_format_accepts_partial_verse_letter` — `"Žalm 26:3a (ROH)"`
- `reference_format_accepts_single_verse` — `"Ján 3:16 (SEB)"`
- `reference_format_accepts_missing_code` — `"Ján 3:16"` (optional code)
- `reference_format_accepts_numbered_book` — `"1. Samuelova 17:33-37 (SEB)"`
- `reference_format_rejects_code_without_parens` — `"Židom 4:13 SEB"`
- `reference_format_rejects_lowercase_code` — `"Ján 3:16 (seb)"`
- `reference_format_rejects_missing_chapter_colon` — `"Ján 3 (SEB)"`
- `verse_prefix_accepts_single_verse_main` — `"1. Na počiatku..."`
- `verse_prefix_accepts_multiline_main` — `"1. Na...\n2. Ono...\n3. Všetko..."`
- `verse_prefix_accepts_double_digit_verse` — `"13. A nieto tvora..."`
- `verse_prefix_rejects_plain_text_main` — `"A nieto tvora..."` with non-empty reference
- `bold_markers_rejected_in_main` — `"1. aby sme ##verili## menu..."`
- `bold_markers_rejected_in_reference` — `main_reference = "##Ján 1:1##"`
- `bold_markers_accepted_when_stripped` — `"1. aby sme VERILI menu..."`
- `emphasis_slide_empty_reference_no_verse_number_required` — `main="NOVÁ ZMLUVA"`, `main_reference=""`
- `emphasis_slide_still_rejects_bold_markers` — `main="##NOVÁ##"`, `main_reference=""`
- `emphasis_slide_rejects_empty_main` — `main=""`, `main_reference=""`

### Tool dispatch tests (`ai/tools.rs` test module)

Exercise `execute_tool` end-to-end against `AppState::in_memory()`:

- `create_bible_rejects_reference_without_parens` — submits the production-bug input `"Židom 4:13 SEB"`, asserts tool-result JSON contains `rule: "reference_format_requires_parens"` and the presentation has 0 slides
- `create_bible_rejects_main_without_verse_numbers` — submits a slide without `N. ` prefix, asserts `rule: "missing_verse_number_prefix"`
- `create_bible_rejects_main_with_hash_markers` — submits slide with `##verili##`, asserts `rule: "unprocessed_bold_markers"`
- `create_bible_accepts_correctly_formatted_slides` — submits 3 slides all valid, asserts presentation created with 3 slides
- `create_bible_accepts_emphasis_slide_without_reference` — submits `{main: "NOVÁ ZMLUVA", main_reference: ""}`, asserts success
- `create_bible_rejects_entire_batch_on_first_invalid_slide` — submits 3 slides where slide 2 has bad reference, asserts 0 slides created (no partial state)
- `add_bible_slide_runs_validator` — submits invalid slide, asserts rejection with rule name
- `update_bible_slide_runs_validator` — updates existing valid slide with invalid content, asserts rejection and DB row unchanged

### E2E Playwright test (extends existing AI settings/chat spec)

Fallback-aware: if AI stubbing of `/v1/chat/completions` is clean in existing test infra, use it; otherwise use the direct tool-call path.

**Preferred path** — AI stub:
- Stub the chat completions endpoint to return a canned tool-call sequence that attempts `create_bible_presentation` with a malformed slide, sees the rejection, retries with a fixed slide, then completes
- Assert: final DB state has one bible presentation with one correctly-formatted slide
- Assert: the `actions` array on the assistant message shows the rejection + retry

**Fallback path** — direct tool invocation:
- POST to an AI tool endpoint (or the internal chat endpoint with a crafted message) submitting a malformed slide, assert 400/tool-error with rule name
- POST again with the corrected payload, assert 200 and the slide appears in `/ui/bible`
- Reload, verify via DOM that the reference has parens and the main has verse number prefix

Mutation testing (already in CI via `cargo-mutants`) will kill weak tests: flip `+` → `*` in the regex, remove an alternative branch, swap `contains("##")` for `starts_with("##")`. The rule-by-rule unit tests should catch all of these.

### Test not in scope

- No test for the `get_style_guide` tool (unchanged).
- No test for the delete intent gate (unchanged).
- No test that exercises the full LLM against a real sermon — too flaky, too slow, too expensive. The AI stub path approximates it deterministically.

## Observability

Add `tracing::warn!` inside the validator rejection path in `ai/tools.rs`:

```rust
tracing::warn!(
    tool = %tool_name,
    rule = %validation_error.rule,
    got = %validation_error.got,
    "bible slide validation rejected AI output"
);
```

This lands in `journalctl -u presenter-dev -f` on the dev server so we can see which rules the LLM trips most often after rollout. No new metrics endpoint and no dashboard — warn logs are enough for a first pass.

## Rollout

1. Single PR on `dev`, all tests green, mutation testing green, no feature flag
2. Deploy to dev, user pastes a real sermon, confirms slides look right (verse numbers, parens, emphasis slides, ALL CAPS on ##word##)
3. PR from `dev` to `main`, user merges
4. Production CI deploys to `presenter.lan`
5. Existing malformed slides in the DB are left alone. If the user wants them fixed, that's a separate cleanup pass after the new flow is proven.

## Open questions

None. All clarifications resolved in brainstorming:

- **Input type** — sermon email/Discord text (not passage refs list)
- **Enforcement** — hard reject with rule-keyed error
- **Bold parsing** — AI handles it from inline prompt rules; server only rejects raw `##`
- **Translation selection** — AI picks from sermon context, includes `(CODE)` only when confident; omits entirely when not

## Success criteria

- Pasting the exact production-bug sermon into AI chat produces slides with verse numbers, parens on references, processed bold markers, zero raw `##`
- All existing AI tests still pass
- All new validator tests pass
- All new tool-level tests pass
- Mutation testing clean on the new validator module
- Dev deploy shows AI retry loop working (see a rejection log → see a corrected retry → see stored slide)
- Manual sermon paste against dev produces a bible presentation that looks correct in `/ui/bible` with no manual cleanup needed
