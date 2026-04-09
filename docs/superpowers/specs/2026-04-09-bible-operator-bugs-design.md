# Bible Operator Page Bug Fixes (#219)

> **Date:** 2026-04-09 | **Issue:** #219

## Overview

Six bugs on the Bible operator page (`/ui/operator/bible`) affecting search, persistence, Resolume integration, text display, and edit mode.

---

## Bug 1: Book Search Stuck on 2nd Attempt

**Symptom:** After selecting a book, the user cannot search for a different book by typing in the filter input. The list shows only the selected book with a "Change" button.

**Root Cause:** `BookList` component (`bible.rs:326-340`) collapses to a single-item view when `selected_book` is `Some`. The filter input still exists but is ignored because the collapsed view renders before the filtered list.

**Fix:** Add an `Effect` on `book_filter` — when the user types a non-empty filter while a book is selected, automatically clear `selected_book` so the filtered book list renders. This makes the filter always responsive: type to search, clear to return to selected book.

**Files:** `crates/presenter-ui/src/pages/bible.rs`

---

## Bug 2: Translation Selection Not Remembered After Refresh

**Symptom:** Changing the Main or Secondary translation dropdown in the Live tab is lost on page refresh.

**Root Cause:** The `TranslationSelectors` component (`bible.rs:215-235`) updates signals on change but never calls `bible::update_preferences()`. Preferences are only saved when clicking "Save" in the Settings tab (`bible.rs:908`). Users expect Live tab translation changes to auto-persist.

**Fix:** Add an `Effect` that watches `selected_translation` and `secondary_translation` signals. When either changes (and translations are loaded), auto-save preferences via `bible::update_preferences()`. Debounce or skip the initial load to avoid overwriting with defaults.

**Files:** `crates/presenter-ui/src/pages/bible.rs`

---

## Bug 3: Broom Button Doesn't Clear Resolume Bible Text

**Symptom:** Clicking the broom (🧹) button in the header clears worship slides from stage but leaves bible text in Resolume.

**Root Cause:** The broom button (`stage_preview.rs:49-61`) calls only `POST /stage/clear` (worship stage state). It does not call `POST /bible/clear`, so bible text persists in Resolume clips.

**Fix:** In the broom button's `on_clear` handler, also call `bible::clear_broadcast()` and update `ctx.active_bible_broadcast` to `None`.

**Files:** `crates/presenter-ui/src/components/stage_preview.rs`

---

## Bug 4: Verse Text Truncated with Dots

**Symptom:** Bible verse text in slides shows only the first characters followed by "..." instead of the full text.

**Root Cause:** `.operator__slide-text` in `operator.css:1199-1206` applies `text-overflow: ellipsis` + `overflow-x: hidden` with `white-space: pre`. This truncates any text wider than the container to a single line with ellipsis.

**Fix:** Add a Bible-specific override: `.operator__slide-card--bible .operator__slide-text` with `white-space: pre-wrap` and `text-overflow: clip` so Bible verse text wraps naturally within the slide card.

**Files:** `crates/presenter-ui/styles/bible.css`

---

## Bug 5: Preview Block Text Faded/Low Opacity

**Symptom:** The stage preview text in the Bible view is hard to read — text appears at ~60% opacity.

**Root Cause:** `.operator__stage-preview[data-active="false"]` in `operator.css:829-831` sets `opacity: 0.6` on the entire preview container when no slide is actively displayed. On the Bible page, this makes preview text nearly illegible against the dark background.

**Fix:** Increase the inactive preview opacity from 0.6 to 0.85. The dimming hint is preserved but text remains legible.

**Files:** `crates/presenter-ui/styles/operator.css`

---

## Bug 6: Edit Mode Text Blocks Too Small

**Symptom:** When switching to Edit mode, Bible slide textareas are too small. Users must manually stretch them to see content.

**Root Cause:** `bible.css:559` sets `min-height: 2.5rem` on Bible editor textareas, overriding the dynamic sizing from `operator.css` which uses `--bible-textarea-lines` (defaults to 4 lines). The fixed 2.5rem is roughly 1.5 lines of text.

**Fix:** Remove the `min-height: 2.5rem` from the Bible textarea rule in `bible.css` so the operator.css dynamic sizing (4 lines based on `--bible-textarea-lines`) takes effect.

**Files:** `crates/presenter-ui/styles/bible.css`

---

## Testing Strategy

- **E2E (Playwright):** New test in `tests/e2e/wasm-bible.spec.ts` covering:
  - Bug 1: Select book → type new book name → verify list shows matching books
  - Bug 2: Change translation → reload page → verify translation persisted
  - Bug 3: Broadcast bible slide → click broom → verify Resolume bible output is cleared via API
  - Bugs 4-6: Visual assertions on slide text not truncated, preview opacity, and edit mode textarea size
- **Unit tests:** No new unit tests needed — these are UI/CSS bugs
- **Console check:** Zero browser console errors/warnings in all tests
