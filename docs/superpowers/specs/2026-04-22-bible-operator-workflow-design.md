# Bible Operator Workflow Fixes Design

> **Date:** 2026-04-22 | **Status:** Approved | **Issue:** #256

## Problem

The bible operator UI has four workflow bugs that cause users to lose state and waste time during a live service:

1. Clicking a book again (via the "Change" button) unselects it
2. Changing translation deselects the current book
3. Selecting a book resets chapter and verse fields
4. Typing in the filter while a book is selected auto-deselects the book

The user also wants two workflow improvements:

5. Selecting a book should clear the search filter so the next search can start immediately
6. Editing any passage field (chapter / verse start / verse end) should auto-load the passage after a short debounce — no need to press the "Load passage" button for every edit

## Design

### Book List Behavior

- Remove the "Change" button entirely. The book list DOM stops emitting it.
- When a book is selected, the list collapses to show only the selected book. Clicking the collapsed row is a no-op — it neither deselects nor expands.
- The filter input stays visible above the collapsed book.
- Typing in the filter expands the list with matching books. The currently-selected book stays highlighted in the expanded list.
- Typing in the filter does **not** clear the selection (remove the auto-clear at line 330-331 of `bible.rs`).
- Clicking a book in the expanded list:
  - Different book: selection changes (with chapter/verse clamping — see below), list collapses, filter clears
  - Same book (the already-selected one): list collapses, filter clears, no selection change

### Translation Change Behavior

When the translation changes:

- If the currently-selected book (matched by book code, e.g. `GEN`) exists in the new translation, preserve the selection.
- Clamp chapter to the new book's chapter count if it exceeds it.
- Clamp verse_start to the (possibly clamped) chapter's verse count if it exceeds it.
- Clamp verse_end the same way; if verse_end becomes `<= verse_start`, set it to `None` (single-verse).
- If the book does **not** exist in the new translation, clear the selection (existing fallback behavior).

### Book Change Behavior (picking a different book)

When the user picks a different book from the expanded list:

- Set the new book.
- Chapter: preserve if it fits within the new book's chapter count; otherwise clamp to the new book's max chapter.
- verse_start: preserve if it fits the (preserved or clamped) chapter's verse count; otherwise clamp to the last valid verse.
- verse_end: preserve if it fits; otherwise clamp; if it becomes `<= verse_start`, set to `None`.
- Collapse the list and clear the filter.

### Auto-Load Passage

- Any change to chapter, verse_start, or verse_end triggers the existing "load passage" action automatically.
- Debounce: 300 ms of inactivity. This means rapid typing only fires one request when the user stops.
- The existing "Load passage" button stays as a manual trigger (useful for re-loading the same passage or bypassing the debounce).

### Clamping Logic

The chapter/verse clamping is non-trivial and needs unit tests. Extract it as a pure function in `state/bible.rs` or a new `state/bible_clamp.rs`:

```rust
pub struct ClampedSelection {
    pub chapter: u16,
    pub verse_start: u16,
    pub verse_end: Option<u16>,
}

/// Clamp chapter/verse against a book's chapter/verse counts.
/// Returns the adjusted selection.
pub fn clamp_selection(
    chapter_count: u16,
    verse_counts: &[u16],
    chapter: u16,
    verse_start: u16,
    verse_end: Option<u16>,
) -> ClampedSelection { /* ... */ }
```

The function is used in three places:
1. Translation change (with the new translation's book structure)
2. Book change (with the new book's structure)
3. (Optional) field-blur on chapter to clamp verses, if that feels natural — not required by this spec

## Testing

### Unit Tests (for `clamp_selection`)

- Preserve chapter and verses when they fit
- Clamp chapter when it exceeds the new book's chapter count
- Clamp verse_start when it exceeds the (clamped) chapter's verse count
- Clamp verse_end when it exceeds the verse count
- Set verse_end to None when it becomes `<= verse_start`
- Edge case: empty book (0 chapters) — return chapter=1, verse_start=1, verse_end=None (defensive default)

### E2E Playwright Tests

- Selecting a book clears the filter and collapses the list
- Typing in the filter with a book selected expands the list; selection stays highlighted
- Clicking the collapsed book is a no-op (still selected, list stays collapsed)
- Picking a different book preserves chapter/verse when they fit
- Picking a book with fewer chapters clamps the chapter; verses clamp accordingly
- Changing translation preserves the book selection when it exists in the new translation
- Changing translation to one missing the selected book clears the selection
- Editing verse_end auto-loads the passage after debounce (verify the passage appears without clicking "Load passage")
- Zero browser console errors

## Files Changed

| File | Change |
|------|--------|
| `crates/presenter-ui/src/pages/bible.rs` | Remove Change button, remove auto-deselect on filter type, remove auto-reset on book click, add filter-clear on book selection, add debounced auto-load on field change |
| `crates/presenter-ui/src/state/bible.rs` | Add `clamp_selection` pure function; update translation-change effect to preserve+clamp instead of clearing |
| `crates/presenter-ui/src/state/bible_clamp.rs` (optional) | Extract clamp logic to its own module if `state/bible.rs` grows too large |
| `tests/e2e/wasm-bible.spec.ts` | Add new workflow tests (or extend existing bible spec file) |

## Out of Scope

- Performance caching of translations/books (user confirmed loading is already fast)
- Redesigning the passage preview or stage broadcast flow
- Any changes to the Bible translations admin UI
- Changes to how Bible data is imported or stored server-side
