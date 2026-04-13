# Worship Operator UI Rework Design

> **Issue:** #215
> **Date:** 2026-04-13

## Problem

The worship operator slides page looks like a "junk yard": random group text outside slides, groups missing on slides that should inherit, blank cards where slides should be, inconsistent styling. The bible operator UI is clean and polished by comparison. The user wants the worship UI to match the bible UI's quality.

## Root Causes (from codebase exploration)

1. **Phantom CSS class `stage-control__slide`** added to every worship card at `slide_list.rs:466`. No styles defined anywhere.
2. **Group headers rendered outside slide cards** at `slide_list.rs:462` as sibling `<div>` elements instead of inside the `<article>`.
3. **Group inheritance duplicated in UI** at `slide_list.rs:380-410` with manual state tracking. The data model already computes this via `resolve_sequence()` → `effective_group`, but worship doesn't use it.
4. **Conflicting CSS** — two `.operator__slide-group` definitions at `operator.css:1254` and `:1373` with opposite layout assumptions.
5. **Fixed textarea heights** — worship uses `rows="2"` while bible uses `field-sizing: content`.
6. **Empty slides from importer** — `importer/lib.rs:309` creates slides with empty main text when a ProPresenter slide has no text elements. UI renders them as blank cards.
7. **Monolithic 928-line component** vs bible's clean helper-function pattern.

## Architecture

Rewrite the worship slide list to match the bible slide list pattern:
- Use `ResolvedSlide` (with `effective_group` already computed) instead of raw `Slide`
- Split the monolithic `SlideList` into focused components and helper functions
- Match bible's zone-based card structure (trigger-zone + select-zone)
- Consistent CSS with no phantom classes

## Changes

### 1. Use `ResolvedSlide` Throughout the Worship UI

**Current:** The worship UI receives `Vec<Slide>` via the state signal and manually tracks group inheritance in the render closure.

**New:** Expose `ResolvedSlide` (or compute `effective_group` inline once per render). The data model's `resolve_sequence()` at `presenter-core/src/slide.rs:189-208` already propagates groups forward correctly — the UI just needs to call it.

**Two approaches:**

**Option A — Resolve in state layer (preferred):** The `slides` signal in `operator_context.rs` stores `Vec<ResolvedSlide>`. When fresh slides arrive from API/WebSocket, call `resolve_sequence()` once and store the result.

**Option B — Resolve in view:** Call `resolve_sequence()` inside the render closure. Simpler diff, but recomputes on every re-render.

I recommend **Option B** for this refactor — smaller blast radius, the compute is cheap (a single pass), and it keeps the signal type unchanged.

### 2. Rewrite Slide Card Rendering

Delete the monolithic per-slide closure (lines 380-906 in `slide_list.rs`). Replace with:

**New helpers** (mirroring bible pattern):
```rust
fn worship_slide_body_view(
    main: String,
    translation: String,
    stage: String,
    group: Option<String>,
    group_inherited: bool,
) -> impl IntoView { ... }

fn worship_slide_editor_view(
    main_sig: RwSignal<String>,
    trans_sig: RwSignal<String>,
    stage_sig: RwSignal<String>,
    group_sig: RwSignal<String>,
    group_placeholder: String,
) -> impl IntoView { ... }
```

**New component** `WorshipSlideCard` that takes a `ResolvedSlide` and renders:
```
<article class="operator__slide-card operator__slide-card--worship">
  <header class="operator__slide-header">
    <drag-handle />
    <slide-index />
    {group_badge}         // INSIDE the card, not before it
    {edit-controls}
  </header>
  <section class="operator__slide-bodies">
    {live_or_editor}
  </section>
</article>
```

The group badge is inside the header, not a sibling. Uses a single `.operator__slide-group` CSS class (the grid-based one, moved to inline-flex since it's not in a grid anymore).

### 3. Fix Group Rendering

**Remove:** The outside-the-card `<div class="operator__slide-group">` at `slide_list.rs:462`.

**Remove:** The inline `.operator__slide-group-label` at `slide_list.rs:684-691` and `:731-738` (redundant — the header group badge now covers this).

**Show group logic:**
- If `effective_group` differs from previous slide's `effective_group` → show as "explicit" (bright)
- If same as previous → show as "inherited" (dimmed) — so operator can still see what group the slide belongs to
- If `None` → no badge

**CSS:** Delete the conflicting `.operator__slide-group` at `operator.css:1254-1265`. Keep only the one at `:1373-1391` and adjust:
- Change `display: inline-flex` to fit inside the card header
- Remove `grid-column: 1 / -1` (not a grid)
- Keep the inherited variant for dimmed state

### 4. Fix Textareas in Edit Mode

Add `operator__slide-card--worship` variant in CSS with `field-sizing: content` for textareas, matching the bible rule at `operator.css:1333`.

### 5. Hide Empty Slides

In the importer (`presenter-importer/src/lib.rs:308-310`), change the fallback: if all text buckets are empty, SKIP creating the slide entirely instead of creating a blank one.

OR, safer: in the worship UI, filter `ResolvedSlide`s where all text fields are empty AND there's no group. Show a toast: "Hidden N empty slides from import" once per load.

I recommend the importer fix — it's the root cause and affects all consumers of the data.

### 6. Remove Phantom CSS Class

Delete `stage-control__slide` from line 466. Replace with `operator__slide-card--worship` for variant styling hooks.

### 7. Split `SlideList` File

The file is 928 lines. Split:
- `slide_list.rs` — top-level component (keeps pagination, selection state, drag-drop orchestration)
- `worship_slide_card.rs` — the per-slide card component (new file, ~300 lines)
- `worship_slide_helpers.rs` — shared view helpers (~150 lines)

## Testing

- **E2E Playwright:** Load a presentation known to have groups (e.g., `160 Ježiš je meno`), verify:
  - All slides display (no blank cards)
  - Slides with inherited groups show the group badge (dimmed)
  - Slides with new groups show the badge (bright)
  - No text appears outside slide cards
  - Edit mode textareas grow to fit content
- **Unit tests:** `resolve_sequence()` already has tests at `slide.rs:226-277`. Add UI-level tests for the new `worship_slide_body_view` helper.
- **Visual regression:** Take screenshots before/after, verify the "junk yard" appearance is gone

## Out of Scope

- Live event broadcasting changes (worship slide trigger flow stays the same)
- ProPresenter importer protocol changes (only fix the empty-slide fallback)
- Worship playlist / library panel rework (only the slides page)
- Worship live preview panel
