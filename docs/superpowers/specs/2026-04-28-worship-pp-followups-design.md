# Worship-PP Follow-ups — Design

**Date:** 2026-04-28
**Status:** Proposed
**Scope:** Frontend (presenter-ui WASM + CSS)

## Goal

Fix two follow-up issues reported after the worship-pp regression PR:

1. **Drag from search to playlist** doesn't work. The user drags a search result, but the playlist row never accepts it — no entry appears.
2. **Stage worship-pp sidebar** wastes screen space. Slides area should be wider (more readable main content), and playlist entry text needs to be much larger so the operator/leader can read song titles from across the room. Target: max 12 entries visible at once, sized for projector viewing.

## Non-goals

- No change to playlist persistence (the user confirmed dev-DB-replace-on-deploy is intentional).
- No change to the worship-snv layout, operator UI logic, server endpoints, drag-drop *between* libraries, or playlist CRUD.
- No change to the active-song highlight — the PR #268 pill (solid sky-blue + 4px accent bar) stays as-is.

## Approach

Two independent fixes, one PR.

---

### Issue 1 — Drag from search to playlist

**Investigation first, then targeted fix.** The static analysis says the drag *should* work:
- `crates/presenter-ui/src/components/search.rs` — search results are `<div draggable="true">`. The `on:dragstart` handler sets BOTH `application/x-presentation-id` AND `application/x-presenter-search` on the dataTransfer.
- `crates/presenter-ui/src/components/playlist_list.rs` — the playlist `<li>` `on:dragover` accepts either of those MIME types and calls `preventDefault`. The `on:drop` reads from the same keys and dispatches `replace_entries` via `spawn_local`.
- The state signals `search_dragging` and `dragging_from_search` are set by search.rs but **never read** by the drop handler — they don't gate anything.

So whatever's broken happens at runtime, not in the static logic. Most likely candidates (in order of probability):
1. **`effect_allowed("copy")` vs the playlist row's drop intent.** The search dragstart sets `effectAllowed = "copy"`. The playlist row's dragover doesn't set a matching `dropEffect`, which can cause the browser to deny the drop in some Chromium builds. Fix: set `event.data_transfer().set_drop_effect("copy")` in the playlist dragover when the type matches.
2. **A pointer-events / z-index issue** between the search-results column and the playlist-list column that makes the drag never reach the playlist row's hit region.
3. **A subtle timing bug** where the search result's dragstart fires but the dataTransfer types aren't yet visible to the playlist row's dragover by the time it runs.

The plan investigates by:
- Opening the operator UI with Playwright, finding a search result and a playlist row, and dispatching a real `dragstart → dragover → drop` sequence as the browser would.
- Reading the actual error (if any) from the browser console, the `dataTransfer.types` at each step, and the result of the drop handler.
- Applying the targeted fix (most likely #1 — add `set_drop_effect` to the playlist row's dragover).

**Regression guard:** add a Playwright E2E test that drags a real search-result row onto a playlist sidebar row and asserts the entry is added (similar to the existing `tests/e2e/wasm-playlist-operations.spec.ts` "drop a presentation" test, but with the source as a `[data-role="search-result-item"]` instead of `[data-role="presentation-item"]`).

---

### Issue 2 — Sidebar narrower, slides wider, much bigger song text, max 12 visible

Pure CSS in `crates/presenter-ui/styles/stage.css`. Changes are scoped to the worship-pp layout.

**Width split:**
- `.stage-pp__slides-area`: `width: 70% → 78%`
- `.stage-pp__playlist-sidebar`: `width: 30% → 22%`
- Slides area gains 8% of horizontal space; the autofit logic in worship-snv-derived regions naturally produces larger text because the container is wider.

**Entry sizing:**
- `.stage-pp__playlist-entry`:
  - `font-size: 0.9vw → 2.6vh` (≈28px @ 1080p, ≈37px @ 1440p — readable from a distance)
  - `padding: 0.4rem 0.6rem → 0.6vh 0.8rem` (vertical padding now scales with viewport height so 12 entries fill the 92vh sidebar)
  - `line-height: 1.1` (explicit, to keep row height predictable)
  - `margin-bottom: 2px` (unchanged)
- `.stage-pp__playlist-entry--active`: keep the existing solid-pill + 4px accent bar from PR #268. Just bump the same padding/font-size rules so the active row matches its inactive siblings dimensionally (the `padding-left: calc(0.8rem - 4px)` compensation stays).

**Math:** sidebar height = 92vh. Per-entry row = 2.6vh font + 1.2vh padding (top+bottom) + 1.1 line-height = roughly 7.0–7.5vh per row → 12 entries fit in ≈84–90vh. Margin/border-left absorb the remaining 2–8vh. If a playlist has more than 12 entries, the existing `overflow-y: auto` on the sidebar lets the rest scroll. If fewer, rows don't stretch — they keep their per-row size for visual consistency.

**Test:** the existing `tests/e2e/stage-worship-pp.spec.ts` already asserts no overlap between slides-area and sidebar. We extend it (or add a new test in the same file) to:
- Assert sidebar width is ~22% of viewport width.
- Compute `getBoundingClientRect()` on a `.stage-pp__playlist-entry` and assert font-size ≥ 24px (sanity floor for projector readability).

## Data flow

No data flow changes. Frontend-only.

## Testing strategy

1. **Unit tests:** none — both changes are CSS / event-handler tweaks with no isolated logic worth a unit test.
2. **E2E (Playwright)** — extend existing specs:
   - `tests/e2e/wasm-playlist-operations.spec.ts`: add a test that drags a `[data-role="search-result-item"]` onto a playlist row and asserts the entry is added (regression guard for issue 1).
   - `tests/e2e/stage-worship-pp.spec.ts`: assert the new sidebar width (22%) and that entry font-size is ≥ 24px computed (regression guard for issue 2).
3. **Live verification on dev** — after deploy:
   - Open `/ui/operator`, type a search query, drag a result onto a playlist row, confirm the entry is added.
   - Open `/stage`, ensure worship-pp layout is selected, confirm the sidebar is visibly narrower and the song text is large enough to read from the back of a room.
4. **Browser console:** zero errors / warnings on operator and stage.

## File-level overview

| File | Change |
|------|--------|
| `crates/presenter-ui/src/components/playlist_list.rs` | Investigate-then-fix: most likely add `dt.set_drop_effect("copy")` in the dragover handler when accepting search drops. |
| `crates/presenter-ui/styles/stage.css` | Resize `.stage-pp__slides-area` (78%) and `.stage-pp__playlist-sidebar` (22%); enlarge `.stage-pp__playlist-entry` font-size + tune padding/line-height; adjust active rule's padding-left compensation if needed. |
| `tests/e2e/wasm-playlist-operations.spec.ts` | Add "drag from search result to playlist" E2E test. |
| `tests/e2e/stage-worship-pp.spec.ts` | Extend with sidebar-width + entry font-size assertions. |
| `Cargo.toml` (workspace), `crates/presenter-ui/Cargo.toml` | Version bump to 0.4.38 / next presenter-ui patch. |

## Risks / unknowns

- The drag-from-search root cause is empirically uncertain. The plan's first task investigates live before writing a fix. If `set_drop_effect` doesn't resolve it, the next candidates are pointer-events on a parent container or a CSS `user-drag` style. The implementer reports findings before applying the fix.
- Setting `padding` in `vh` units is unusual but necessary so 12 entries fit a 92vh container at any aspect ratio. If this looks off in practice, the fallback is `min-height` on the entry plus `flex` distribution on the sidebar.
- Sidebar at 22% may look too narrow if a song title is long. The existing `text-overflow: ellipsis` handles this — we already truncate single-row entries.

## Out of scope

- Playlist persistence on dev (intentional design — DB replaced from prod on each deploy).
- Drag-drop reordering, separator drag, between-playlist moves, multi-select.
- Stage layout for non-worship-pp variants.
- Server-side changes.
