# AI bible composer: same full-passage reference on every slide

**Issue:** #292

**Date:** 2026-05-04

**Branch:** dev (workspace 0.4.58)

## Problem

When the AI composes a bible presentation that spans multiple slides (e.g. Numeri 13:17-20 split into two slides), each slide shows a per-slide reference like "Numeri 13:17-18", "Numeri 13:19-20" instead of the full passage "Numeri 13:17-20" on every slide. The user expects every slide of one passage to display the same full reference, matching the behavior of the non-AI bible-load flow and the documented style guide.

## Bug location

`crates/presenter-server/src/state/slides.rs:91-95` in `compose_bible_items_into_slides`:

```rust
let reference = if start == end {
    format!("{} {}:{} ({})", book, chapter, start, translation)
} else {
    format!("{} {}:{}-{} ({})", book, chapter, start, end, translation)
};
```

`start` and `end` come from `cur_numbers.first()`/`cur_numbers.last()` — the verses on the current slide only. The fix needs the full set of verses across the entire `items[]` for that `(book, chapter, translation)` group.

The non-AI composer `compose_bible_slides` (same file, line 173+) is already correct: it accepts `full_verse_start: u16, full_verse_end: u16` parameters and emits the full label on every slide. The style guide at `crates/presenter-server/src/ai/style_guide.md` lines 21-31 already documents this as the required behavior.

## Goal

Every slide produced by `compose_bible_items_into_slides` for a given `(book, chapter, translation)` group displays the same reference label, and that label covers the full set of cited verses across the entire `items[]`.

For the contiguous case (most common), the label uses range syntax: "Numeri 13:17-20 (SEB)".

For the non-contiguous case (rarer — e.g. pastor cites only verses 1, 3, 5), the label uses a comma-separated list: "Numeri 13:1, 3, 5 (SEB)".

A single verse stays single: "Numeri 13:17 (SEB)".

## Architecture

### Pass 1 — collect group verses

Before the existing flush loop, scan `items` once and build:

```rust
use std::collections::BTreeSet;

let mut group_verses: HashMap<(String, u32, String), BTreeSet<u32>> = HashMap::new();
for item in items {
    if let BibleItem::Verse { number, book, chapter, translation, .. } = item {
        group_verses
            .entry((book.clone(), *chapter, translation.clone()))
            .or_default()
            .insert(*number);
    }
}
```

`BTreeSet` keeps verses sorted and deduplicated. `HashMap` with the tuple key handles multiple distinct passages in the same `items[]`.

### Format helper

A free function next to `compose_bible_items_into_slides`:

```rust
fn format_verse_range(verses: &BTreeSet<u32>) -> String {
    let v: Vec<u32> = verses.iter().copied().collect();
    if v.is_empty() {
        return String::new();
    }
    let min = v[0];
    let max = v[v.len() - 1];
    let count = v.len() as u32;
    if min == max {
        format!("{}", min)
    } else if max - min + 1 == count {
        format!("{}-{}", min, max)
    } else {
        v.iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    }
}
```

### Pass 2 — flush uses group lookup

`flush_verses` no longer derives the reference from `cur_numbers.first()`/`cur_numbers.last()`. Instead it looks up the group's full verse set:

```rust
let reference = match group_verses.get(&(book.clone(), chapter, translation.clone())) {
    Some(verses) => format!(
        "{} {}:{} ({})",
        book,
        chapter,
        format_verse_range(verses),
        translation
    ),
    None => String::new(),
};
```

The closure becomes a regular helper that takes `&group_verses` as an extra parameter (or we inline the format-and-push logic — implementation detail to be decided in the plan). `cur_numbers` is still used as the non-empty guard but no longer drives the reference label.

## Testing

Five new unit tests in the existing `tests` module of `slides.rs`. Each test feeds a specific `items[]` shape and asserts the resulting slides' `main_reference` values:

1. **`compose_uses_full_passage_range_across_split_slides`** — items = `[Numeri 13:v17, v18, v19, v20]` with low character limit forcing two slides. Asserts both slides have `main_reference == "Numeri 13:17-20 (SEB)"`.
2. **`compose_handles_emphasis_between_verses_with_full_range`** — items = `[Numeri 13:v17, emphasis "WORD", v18, v19, v20]`. Asserts the emphasis slide has empty `main_reference`, and the surrounding verse slides both have `main_reference == "Numeri 13:17-20 (SEB)"`.
3. **`compose_two_distinct_passages_get_independent_ranges`** — items = `[Ján 1:v1, Ján 1:v2, Mat 5:v3]`. Asserts the Ján slides ref = "Ján 1:1-2 (SEB)", the Mat slide ref = "Mat 5:3 (SEB)".
4. **`compose_non_contiguous_verses_render_as_comma_list`** — items = `[Numeri 13:v1, v3, v5]` with low character limit forcing splits. Asserts every verse slide has `main_reference == "Numeri 13:1, 3, 5 (SEB)"`.
5. **`compose_mixed_gap_renders_as_comma_list`** — items = `[Numeri 13:v1, v2, v4, v5]`. Asserts `main_reference == "Numeri 13:1, 2, 4, 5 (SEB)"` (no mixed range/list syntax — just a flat list when not perfectly contiguous).

The existing test at line 525 (`compose_bible_slides_sets_reference_in_stage_and_group`) and the `compose_bible_slides_multi_verse_range_reference` test at line 571 belong to the non-AI `compose_bible_slides` function and continue to pass unchanged.

The two existing tests at line 855-864 (in `tools.rs`) that exercise `compose_bible_items_into_slides` indirectly (through `create_bible_presentation`) currently assert the OLD behavior — those need to be updated to the new full-range behavior, or marked as obsolete and replaced.

## File-level changes

| File | Change |
|------|--------|
| `crates/presenter-server/src/state/slides.rs` | Add Pass 1 group-verses precompute + `format_verse_range` helper; modify `flush_verses` closure to read from `group_verses` instead of `cur_numbers` for reference computation. ~30-40 lines changed. |
| `crates/presenter-server/src/state/slides.rs` (test module) | 5 new unit tests. |
| `crates/presenter-server/src/ai/tools.rs` (test module, lines 855-864) | Update assertions to expect full-range references. |

## Acceptance

- All 5 new unit tests pass.
- All existing tests pass (after the tools.rs test assertion updates).
- Mutation testing on `slides.rs` passes (no surviving mutants in the dedup or formatting logic).
- `cargo clippy --workspace --all-targets -- -D warnings -W clippy::all` clean.
- Manual verification on dev: ask the AI to compose a multi-slide bible passage; every slide displays the same full-range reference.
- Browser console clean during E2E.

## Out of scope

- The non-AI `compose_bible_slides` is already correct.
- No system prompt changes — the style guide already documents the correct behavior.
- No frontend changes.
- No persistence/migration concerns — references are computed at compose time and stored as plain strings.
