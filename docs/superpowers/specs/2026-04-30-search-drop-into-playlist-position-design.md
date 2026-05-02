# Drag Search Result into Open Playlist at Specific Position — Design

**Date:** 2026-04-30
**Status:** Proposed
**Scope:** Frontend (presenter-ui WASM) — single-file change to `presentation_list.rs`, one new E2E test
**Issue:** [#274](https://github.com/zbynekdrlik/presenter/issues/274) — "i want to be able to drag presentation directly to openned playlist, for example drag it from search directly to openned playlist to exact position and it needs to show line to between which presentations i am putting on what position"

## Goal

Let the operator drag a presentation from the search panel and drop it at a **specific position** inside the open playlist, with a visual line indicator showing where the entry will land — matching the existing within-playlist reorder UX.

## Bug / gap today

- Search results are already draggable (`crates/presenter-ui/src/components/search.rs:299-316` — sets MIME `application/x-presentation-id` + `application/x-presenter-search` on dragstart).
- Within-playlist reorder already shows an insertion-line indicator: each entry sets `data-drop-position="before"` / `"after"` on dragover, with a 3px blue line drawn by CSS pseudo-elements (`crates/presenter-ui/styles/operator.css:685-706`).
- Dropping a search result onto the playlist's *card* in the sidebar already works — appends to end (`crates/presenter-ui/src/components/playlist_list.rs:148-242`).

**The gap:** when the user drags a search result over an *entry* inside the open playlist, the entry-level `dragover` / `drop` handlers in `presentation_list.rs:194-240` only recognize the within-playlist `application/x-entry-id` MIME. The search drag falls through, so no line indicator appears and the drop bubbles up to the playlist-card handler, which appends to the end — losing the operator's intended position.

## Approach

Extend the existing entry-level handlers in `presentation_list.rs` to **also recognize `application/x-presentation-id`**. The infrastructure (line-indicator CSS, `replace_entries()` API, drag-state signals) is all already in place — this is mechanical wire-up, not new architecture.

## Components and data flow

1. **`dragstart`** (no change): `search.rs` already sets `application/x-presentation-id` with the presentation UUID and toggles `op.search_dragging` / `op.dragging_from_search`.

2. **`dragover` on a playlist entry** (extended):
   - Accept if `dataTransfer.types` contains EITHER `application/x-entry-id` (within-playlist reorder, existing) OR `application/x-presentation-id` (search drag, new).
   - On accept: `event.preventDefault()`, compute insertion side from cursor Y vs. entry's bounding-box midline, set `data-drop-position="before"` (cursor in top half) or `"after"` (bottom half) on the entry.
   - Same code path as today's reorder; only the MIME-acceptance check is widened.

3. **`dragleave`** (no change): clear `data-drop-position` on the entry.

4. **`drop` on a playlist entry** (extended):
   - Read `dataTransfer` MIME types.
   - **If `application/x-entry-id` is present** → existing reorder path. (No change.)
   - **If `application/x-presentation-id` is present (new path):**
     - Read the dragged presentation UUID from `event.dataTransfer.getData("application/x-presentation-id")`.
     - Read the target entry's `data-entry-index` (`u32`) and `data-drop-position` (`"before"` or `"after"`).
     - Compute target insertion index: `before` → `entry_index`, `after` → `entry_index + 1`.
     - Build a `Vec<PlaylistEntryPayload>` from the current `selected_playlist`'s entries (preserving each existing entry's `entry_id`), insert one new `PlaylistEntryPayload::Presentation { entry_id: None, presentation_id: <dragged uuid> }` at the target index. Duplicates are kept verbatim per the user's "allow duplicate" decision.
     - `let resp = api::playlists::replace_entries(playlist_id, new_entries).await?;`
     - Update `ctx.selected_playlist.set(Some(resp))` so the UI re-renders.
     - Show success toast "Added presentation to playlist" (reuse `toast_message` / `toast_variant` signals).
     - Clear `data-drop-position` + `op.search_dragging` + `op.dragging_from_search` (existing patterns).

5. **Error handling:** if `replace_entries()` returns an error, surface via the existing error toast pattern (`toast_variant.set("error")`, `toast_message.set(Some(format!("Error: {e}")))`). Local `selected_playlist` state stays unchanged — no optimistic update to roll back since the server response is the source of truth.

## Edge cases

| Case | Behavior |
|------|----------|
| **Empty playlist** | Drop never reaches an entry-level handler; falls through to the playlist-card handler which appends. Already works. |
| **Drop in the gap below the last entry** | No `data-entry-index` element under cursor; falls through to the playlist-card handler → append. Already works. |
| **Drop onto a separator entry** | Separators carry `data-entry-index` too, so before/after positioning applies the same as for presentations. Insertion respects the separator's index. |
| **Drop the same presentation already in the playlist** | Allowed. A second entry with the same `presentation_id` is added at the target position. |
| **`replace_entries()` network/server error** | Error toast shown; entries list unchanged. |
| **Drag a non-presentation search result (library / slide)** | Already filtered at dragstart — only `data-kind="presentation"` results are draggable. Non-presentation drags carry no `application/x-presentation-id` and the new branch ignores them. |

## File changes

| File | Change |
|------|--------|
| `crates/presenter-ui/src/components/presentation_list.rs` | Extend the entry-level `dragover` and `drop` closures (lines ~194-240) to recognize `application/x-presentation-id` in addition to the existing `application/x-entry-id`. ~80 lines added. |
| `tests/e2e/wasm-drag-drop.spec.ts` | Add one new test: open playlist with ≥2 presentations, drag a search result, assert the line indicator on the target entry during dragover, drop, assert the new presentation lands at the expected index. ~40 lines. |

No changes to:
- `search.rs` (dragstart already sets the right MIME)
- `playlist_list.rs` (whole-playlist drop handler keeps working for empty playlists / gap drops)
- The server (`PUT /playlists/{id}/entries` already accepts arbitrary-order Vec)
- CSS (`data-drop-position` styling already covers both reorder and the new search drop)
- The `api::playlists::replace_entries` client (existing signature is sufficient)

## Testing

### New Playwright E2E in `tests/e2e/wasm-drag-drop.spec.ts`

```typescript
test("drag search result into specific position in open playlist (#274)", async ({ page }) => {
  // 1. Open the operator, select a library, pick a playlist that already
  //    has at least 2 presentations (existing fixture or seeded data).
  // 2. Type into the global search box to surface a presentation that is
  //    NOT in the playlist (or one that IS — duplicates are allowed).
  // 3. Use Playwright's HTML5 drag-and-drop helper to drag
  //    [data-role="search-result-item"][data-kind="presentation"] over
  //    the SECOND entry in the playlist's entries list.
  // 4. Assert the second entry has data-drop-position="before" (cursor
  //    expected in the top half during the drag).
  // 5. Drop. Wait for the entries list to refresh.
  // 6. Read the rendered playlist entries (data-entry-index in order).
  //    Assert the dragged presentation now appears at index 1
  //    (i.e. inserted BEFORE the previous index-1 entry).
  // 7. Assert the existing within-playlist reorder still works
  //    (sanity that the new branch did not regress the existing one).
  // 8. Assert browser console is clean (no errors / warnings).
});
```

### Existing tests stay green

- Within-playlist reorder E2E in the same file (the existing `application/x-entry-id` branch is unchanged).
- Search-to-playlist append E2E (drop on the playlist card → append; the playlist-card handler is unchanged).

### No new unit tests

The change is pure DOM event wiring; logic is "look at MIME, branch, build Vec, call API". The E2E covers the user-visible behavior end-to-end.

## Risks / unknowns

- **Cursor-Y midline computation under fast scrolling.** The existing reorder code already handles this — reusing the same code path inherits whatever robustness is already there. No new risk.
- **`event.dataTransfer.types` checking semantics.** Some browsers report types as a `DOMStringList`. `presentation_list.rs` already deals with this for the reorder case; reuse the same pattern.
- **Performance with very long playlists.** The whole entries list is rebuilt and re-sent on every drop. Same as the existing reorder behavior — not a new concern. Realistic playlists are ≤ 30 entries.

## Out of scope

- Drag onto a **non-active** playlist card in the sidebar with insertion position — that path stays as today (append-to-end via the playlist-card handler).
- Multi-select drag (drag many presentations at once) — not in the issue.
- New server-side endpoint with a `position` parameter — unnecessary; `PUT /playlists/{id}/entries` already supports arbitrary Vec ordering.
- Animations / micro-transitions on the line indicator beyond what the existing CSS already does.
- Visual feedback that distinguishes a search-source drop from a within-playlist reorder — both share the same line indicator. Operator's mental model is "the line shows where the dragged thing will land", regardless of source.
