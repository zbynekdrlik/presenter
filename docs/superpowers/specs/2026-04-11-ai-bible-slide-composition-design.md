# AI Bible Slide Composition Design

> **Status:** Proposed
> **Date:** 2026-04-11
> **Related:** PR #235 (bible slide validation), issue: AI-created bible presentations do not obey the per-slide character limit.

## Problem

There are two paths that build bible presentations:

- **Live mode** (`/bible/resolve` → `AppState::generate_bible_slides` → `compose_bible_slides` at `crates/presenter-server/src/state/slides.rs:18-142`) loads verses from the Bible DB and splits them into slides respecting a configurable `character_limit` (default `320`, stored in app setting `bible-preferences`).
- **AI mode** (`create_bible_presentation`, `add_bible_slide`, `update_bible_slide` in `crates/presenter-server/src/ai/tools.rs:457-625`) accepts slide `main` text verbatim from the LLM and writes it straight to the DB. No splitting. No length check. Bible validator has 4 rules, none about length.

Result: the AI ships bible slides that overflow the character limit and render badly on stage. The live path works correctly because the server does the splitting; the AI path is broken because the LLM decides the split points and the server trusts the LLM.

## Goal

Move slide-break decisions off the LLM and onto the server. The AI works at **verse granularity**; the server composes the final slides using the same `compose_bible_slides()` logic the live path uses.

## Architecture

```
Sermon text (pastor's wording with ##markers##)
            │
            ▼
  ┌─────────────────────────────────────────────┐
  │ AI: parse sermon, for each passage:          │
  │                                               │
  │   1. load_bible_verses(book,ch,start,end,tr) │
  │   2. for each loaded verse:                  │
  │      - compare DB text to sermon text        │
  │      - if different, use sermon text          │
  │      - apply ##word## → WORD inline           │
  │   3. build items[] in sermon order:           │
  │      - verse items in verse order             │
  │      - emphasis items where ##phrase## is     │
  │   4. create_bible_presentation(title,items)  │
  └─────────────────────────────────────────────┘
            │
            ▼
  ┌─────────────────────────────────────────────┐
  │ Server: compose_bible_items_into_slides()   │
  │                                               │
  │   For each item:                              │
  │     - emphasis       → flush + emit emphasis │
  │     - translation chg → flush + start new    │
  │     - verse (fits)    → append to current     │
  │     - verse (too big) → flush + start new    │
  │   Flush remainder.                            │
  │                                               │
  │   Validator runs on composed slides (5 rules)│
  │   Persist via append_bible_presentation_...  │
  └─────────────────────────────────────────────┘
```

## Tool Contract Changes

### `load_bible_verses` (new, replaces AI use of `resolve_bible_slides`)

**Input:**

```json
{
  "translation": "MIL",
  "book": "Ján",
  "chapter": 1,
  "verse_start": 1,
  "verse_end": 14
}
```

**Output:** Raw verses, **not split**:

```json
[
  {"number": 1, "text": "Na počiatku bolo Slovo...", "reference": "Ján 1:1 (MIL)"},
  {"number": 2, "text": "Ono bolo na počiatku...",   "reference": "Ján 1:2 (MIL)"},
  ...
]
```

The server tells the AI what DB thinks each verse says, one verse per element. No slides, no grouping, no character limit in play yet.

The existing `resolve_bible_slides` tool stays as-is for any non-AI caller but is removed from the AI tool list (not exposed in `tool_defs.rs` to the LLM).

### `create_bible_presentation` (rewritten signature)

**Input:**

```json
{
  "title": "Ján 1 — Slovo sa stalo telom",
  "venue": "Hlavná bohoslužba",
  "items": [
    {"kind": "verse",    "number": 1, "text": "Na počiatku bolo Slovo...",
     "book": "Ján", "chapter": 1, "translation": "MIL"},
    {"kind": "verse",    "number": 2, "text": "Ono bolo na počiatku...",
     "book": "Ján", "chapter": 1, "translation": "MIL"},
    {"kind": "emphasis", "text": "NOVÁ ZMLUVA"},
    {"kind": "verse",    "number": 3, "text": "Všetko vzniklo skrze neho...",
     "book": "Ján", "chapter": 1, "translation": "MIL"}
  ]
}
```

**Server behavior:**

1. Load `character_limit` from `bible-preferences` app setting.
2. Run `compose_bible_items_into_slides(items, character_limit)` — see algorithm below.
3. Run the bible validator (now 5 rules) on each composed slide. If any rule fails, return a rule-keyed error to the LLM (same pattern as PR #235).
4. Persist with `append_bible_presentation_slides`.

`add_bible_slide` and `update_bible_slide` are removed from the AI tool list. If the AI needs to edit an existing presentation, it rebuilds the full `items` array and calls `create_bible_presentation` again. Rationale: the verse-granularity contract does not survive piecemeal edits, and these tools were rarely used.

## Composition Algorithm

New pure function at `crates/presenter-server/src/state/slides.rs`:

```rust
pub fn compose_bible_items_into_slides(
    items: &[BibleItem],
    character_limit: u32,
) -> Vec<ComposedSlide>
```

Where `BibleItem` is the typed enum:

```rust
pub enum BibleItem {
    Verse {
        number: u32,
        text: String,
        book: String,
        chapter: u32,
        translation: String,
    },
    Emphasis { text: String },
}
```

Algorithm (single pass, greedy):

```
current_verses = []
current_translation = None
current_book_chapter = None
slides = []

for item in items:
    match item:
        Emphasis(text):
            flush(current_verses, slides)
            slides.push(emphasis_slide(text))
            current_verses.clear()
            current_translation = None
            current_book_chapter = None

        Verse(number, text, book, chapter, translation):
            line = format!("{}. {}", number, text)

            # Translation or book/chapter change → forced break (design choice b)
            if current_translation.is_some() and
               (current_translation != Some(translation)
                or current_book_chapter != Some((book, chapter))):
                flush(current_verses, slides)
                current_verses.clear()

            # Compute prospective length (same formula as compose_bible_slides)
            prospective = sum(v.line.len() for v in current_verses)
                        + (len(current_verses) if current_verses else 0) # newlines
                        + line.len()
                        + (1 if current_verses else 0)  # newline before new line

            if prospective > character_limit and current_verses:
                flush(current_verses, slides)
                current_verses.clear()

            current_verses.push(Verse(number, line, translation, book, chapter))
            current_translation = Some(translation)
            current_book_chapter = Some((book, chapter))

flush(current_verses, slides)
return slides

fn flush(current_verses, slides):
    if current_verses is empty: return
    main = "\n".join(v.line for v in current_verses)
    start = current_verses[0].number
    end = current_verses[-1].number
    book = current_verses[0].book
    chapter = current_verses[0].chapter
    tr = current_verses[0].translation
    reference = if start == end {
        format!("{} {}:{} ({})", book, chapter, start, tr)
    } else {
        format!("{} {}:{}-{} ({})", book, chapter, start, end, tr)
    }
    slides.push(ComposedSlide { main, main_reference: reference })
```

**Edge cases:**

- **Single verse longer than `character_limit`**: emit as its own slide, even though it overflows. The validator's length rule will flag it; the LLM sees the error and can respond by splitting the verse text manually (by comma/phrase boundary) across multiple verse items with the same `number`. This is the one case where the LLM has to think about length — and only when a single verse is pathologically long (very rare).
- **Empty `items[]`**: reject at tool handler before calling composer; return a clear error.
- **Adjacent emphasis items**: allowed, each becomes its own slide.
- **Verse numbers out of order**: pass through as-is; the server does not reorder. If the pastor quotes `1, 14, 2` in that order, the slides render in that order.

## Validator Changes

File: `crates/presenter-server/src/ai/bible_validator.rs`

Add one rule to `ValidationRule`:

```rust
MainExceedsCharacterLimit,
```

Add the `expected()` text:

> `"Slide main text exceeds the character limit ({limit}). The server composes slides from verse items; this error means a single verse is longer than the limit on its own. Split the verse text across multiple verse items with the same verse number, or reduce the sermon's custom wording."`

The `ValidationError` struct gains an optional `limit: Option<u32>` field so the error payload can report the actual limit to the LLM.

Add rule logic in `validate_bible_slide(main, main_reference, character_limit)`:

```rust
if main.len() > character_limit as usize {
    return Err(ValidationError::with_limit(
        ValidationRule::MainExceedsCharacterLimit,
        main.to_string(),
        character_limit,
    ));
}
```

Existing signature changes from `validate_bible_slide(main, main_reference)` to `validate_bible_slide(main, main_reference, character_limit)`. Callers in `tools.rs` pass the limit from `bible-preferences`.

**Test coverage:**

- Slide exactly at limit passes.
- Slide one character over limit fails.
- Slide at 2× limit fails.
- Emphasis slide over limit also fails (consistent with verse slides).
- Existing 4 rules still pass their existing tests (no regression).

## System Prompt Changes

File: `crates/presenter-server/src/ai/agent.rs` — the `## Creating Bible slides` section added in PR #235.

Rewrite to describe the new verse-granularity workflow. Key points:

1. **You do not produce slides. You produce verse items and emphasis items. The server composes slides.**
2. For each passage mentioned in the sermon:
   - Call `load_bible_verses(book, chapter, verse_start, verse_end, translation)` to get raw DB verses.
   - For each loaded verse, compare the DB `text` with the sermon's wording. **The sermon is authoritative for text content.** If they differ, replace the `text` field with the sermon's wording. Keep the verse `number` from the sermon even if the pastor misquoted the reference.
   - For `##word##` markers inside a verse, replace `word` with `WORD` inside that verse's `text` field. Do not create a new slide for it.
   - For `##phrase##` markers that span standalone text (not wrapped inside a verse), emit an `emphasis` item with `text = phrase.to_uppercase()` at the position where the phrase appears in the sermon.
3. Assemble `items[]` in the order the sermon presents them: verse items in verse order, emphasis items inserted at the pastor's break points.
4. Call `create_bible_presentation(title, venue, items)`. The server splits into slides respecting the character limit. You cannot control where splits happen — that is the server's job.
5. The validator runs on the composed output. If it rejects, it returns a rule-keyed error — read the `rule` and `expected` fields and retry.

Remove any language that told the AI to "build slides", "split at character limit", or "produce slide text". Remove mentions of `create_bible_presentation` taking a `slides` array. Remove mentions of `add_bible_slide` / `update_bible_slide`.

## Testing Strategy

### Unit tests (pure composer)

File: `crates/presenter-server/src/state/slides.rs` (existing test module)

- `compose_single_verse_short_enough_emits_one_slide`
- `compose_two_verses_that_fit_emits_one_slide_with_range_reference`
- `compose_two_verses_that_overflow_emits_two_slides`
- `compose_emphasis_between_verses_breaks_slide`
- `compose_translation_change_forces_break`
- `compose_book_change_forces_break`
- `compose_chapter_change_forces_break`
- `compose_empty_items_returns_empty`
- `compose_single_verse_longer_than_limit_emits_one_oversized_slide` (validator will catch this downstream)
- `compose_adjacent_emphasis_items_emit_separate_slides`

### Unit tests (validator)

File: `crates/presenter-server/src/ai/bible_validator.rs`

Existing 4-rule tests keep their signature change (`character_limit` param). Add:

- `length_rule_accepts_slide_at_exactly_limit`
- `length_rule_rejects_slide_one_char_over_limit`
- `length_rule_rejects_slide_well_over_limit`
- `length_rule_error_payload_includes_limit`
- `length_rule_applies_to_emphasis_slides_too`

### Integration tests (tool dispatch)

File: `crates/presenter-server/src/ai/tools.rs` test module

- `load_bible_verses_returns_raw_verses_with_reference`
- `load_bible_verses_translation_not_found_returns_error`
- `create_bible_presentation_with_verse_items_composes_via_server`
- `create_bible_presentation_with_emphasis_items_inserts_emphasis_slides`
- `create_bible_presentation_with_overlong_text_triggers_server_split`
- `create_bible_presentation_mixed_translations_breaks_at_boundary`
- `create_bible_presentation_rejects_pathologically_long_single_verse`

### E2E test (Playwright)

File: `tests/e2e/ai-chat.spec.ts` (or whatever the existing AI chat e2e file is)

A test that sends a sermon with Ján 1:1-14 and a `##Slovo##` emphasis marker through the `/ai/chat` endpoint, waits for the presentation to appear, then asserts:

- Multiple slides were created (proving the server split the long passage)
- No slide's `main.length` exceeds the character limit
- A slide with `main = "SLOVO"` and empty `main_reference` exists
- Browser console is clean (zero errors/warnings)

### Manual verification on dev

Per airuleset: after deploy, open `http://10.77.8.134:8080/ui/operator`, trigger an AI chat with a real sermon, and confirm the rendered stage shows properly-split slides respecting the character limit.

## Migration / Rollout

1. Bump version on `dev`.
2. Implement the change (plan task-by-task via subagent-driven development).
3. CI green (all jobs, including mutation testing on the new composer and validator rule).
4. Deploy to dev, manual verification with a real sermon.
5. PR dev → main.
6. User approval → merge → production deploy → production verification.

No DB migration — the presentation storage format is unchanged (we still write `BiblePresentationSlide` rows with `main` / `main_reference`). Only the AI path's *input* shape changes; the *output* schema is identical to what live mode already writes.

No backward compatibility concerns for the AI tool contract: the AI agent is stateless across conversations and uses the current tool definitions on every turn.

## Non-Goals

- Re-splitting existing bible presentations created before this change. Operators can regenerate any broken ones via the AI chat.
- Changing the live path. `compose_bible_slides` stays as-is; the new `compose_bible_items_into_slides` is a sibling, not a replacement. The two could be unified later but this design keeps them separate to limit blast radius.
- Supporting mid-verse splits at punctuation boundaries. If a single verse is longer than the limit, the LLM is responsible for passing it as multiple verse items with the same `number`. This is a rare edge case in Slovak sermon texts.
- A UI for editing AI-composed bible presentations. Operators already have the live path for DB-driven presentations and can regenerate via AI chat.
