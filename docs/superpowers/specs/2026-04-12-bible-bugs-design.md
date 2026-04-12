# Bible Bug Fixes Design

> **Issues:** #237, #232, #230
> **Date:** 2026-04-12

## Problem

Three independent bible-related bugs:

1. **#237 — Bible overlay on all stage layouts:** `<BibleOverlay>` is hardcoded into worship-snv, worship-pp, timer, and preach layouts. When any bible slide is triggered, bible text shows as a full-screen overlay on ALL layouts. Bible should only show on a dedicated bible stage layout.

2. **#232 — Bible edit reference blocks side-by-side:** In bible edit mode, the main_reference and translation_reference inputs are in a 2-column grid (`grid-template-columns: 1fr 1fr`). They should stack vertically and have auto-scaling like verse text.

3. **#230 — Cannot add multiple empty slides:** The backend permits empty slides (no constraints found). The bug is in the WASM UI — `AddEmptySlideButton` likely doesn't trigger a proper re-render after the first empty slide is added.

## Fix #237: Dedicated Bible Stage Layout

### Remove overlay from all layouts

Delete the `<super::bible_overlay::BibleOverlay overlay=ctx.bible_overlay />` line from:
- `crates/presenter-ui/src/components/stage/worship_snv.rs:125`
- `crates/presenter-ui/src/components/stage/worship_pp.rs:145`
- `crates/presenter-ui/src/components/stage/timer_layout.rs:53`
- `crates/presenter-ui/src/components/stage/preach_layout.rs:70`

### Create bible layout

New file `crates/presenter-ui/src/components/stage/bible_layout.rs` — a proper stage layout (not overlay) that renders bible content full-screen:
- Background: solid dark (matches current overlay style `rgba(0, 0, 0, 0.92)`)
- Main text centered, large font with auto-scaling
- Reference below the text, smaller font
- When no bible content active, show empty/waiting state
- Follow the same component pattern as other layouts (takes `StageContext` parameter)

### Register the layout

In `crates/presenter-core/src/stage_display.rs`, add to `built_in()`:
```
code: "bible"
name: "BIBLE"
description: "Full-screen Bible passage display"
```

### Wire into stage page

In `crates/presenter-ui/src/pages/stage.rs`, add the bible layout to the layout match/dispatch so it renders `BibleLayout` when layout code is "bible".

### Update Companion plugin

Add `{ id: "bible", label: "BIBLE" }` to `STAGE_LAYOUT_CHOICES` in `ops/companion/presenter/index.js`.

## Fix #232: Stack Reference Blocks Vertically

In `crates/presenter-ui/styles/operator.css` line 1353, change:
```css
grid-template-columns: 1fr 1fr;
```
to:
```css
grid-template-columns: 1fr;
```

This stacks main_reference and translation_reference vertically. Both inputs should use the same auto-scaling approach as verse text inputs (CSS `font-size` scaling or `clamp()`).

## Fix #230: Multiple Empty Slides

The backend permits empty slides — comment at `state/bible.rs:269-271` explicitly documents this. The bug is in the WASM UI's `AddEmptySlideButton` component at `crates/presenter-ui/src/pages/bible_slides.rs:40-86`.

Investigate and fix:
- Check if the click handler properly refreshes the slide list after adding
- Check if signal updates trigger a re-render
- Check if there's request deduplication preventing multiple calls
- Ensure each click creates a new slide with a unique ID and incremented order

## Testing

- **#237:** E2E test — switch to bible layout, trigger bible slide, verify it shows. Switch to worship-snv, trigger bible, verify it does NOT show on stage.
- **#232:** Visual verification in Playwright — open bible edit mode, verify reference blocks are stacked vertically.
- **#230:** E2E test — add 2+ empty slides to a bible presentation, verify all appear in the slide list.
- **All:** Rust unit tests where applicable (layout registration).

## Out of Scope

- Bible layout customization (fonts, colors) — uses same styling as current overlay
- Multi-translation display on bible layout — only main text for now
- Automatic layout switching when bible is triggered — operator manually switches
