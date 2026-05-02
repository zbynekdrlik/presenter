# Floating Song Bubble + Playlist Drop Edge-Case Fixes — Design

**Date:** 2026-04-30
**Status:** Proposed
**Scope:** Frontend (presenter-ui WASM) + the server-rendered settings page

**Issues addressed:**
- 3 bugs in PR #282 / issue #274 (clipboard search drop edge cases): empty playlist, drop above first entry, drop below last entry.
- [Issue #272](https://github.com/zbynekdrlik/presenter/issues/272) — replace the slides toolbar with a floating song-name bubble (draggable) + floating "+" add-slide button.

## Goal

Make the operator's drag-into-playlist UX work for every position (empty, first, middle, last) AND replace the `.operator__slides-toolbar` with floating elements so the slides area uses the full available height. Reuse the search-drop drop infrastructure from PR #282 — the new song-name bubble emits the same MIME and lands in the same drop handlers.

## Bugs to fix (PR #282 edge cases)

### 1. Empty playlist drop

Currently `presentation_list.rs:264-268` renders only an instructional `<li class="empty">"Playlist is empty…"` when `playlist.entries.is_empty()`. The `<li>` has no drag handlers, so a search drag falls through to nothing — no insert.

Fix: extend the empty-state `<li>` with the same `on:dragover` / `on:dragleave` / `on:drop` handlers as a regular entry, except the drop always inserts at index 0 (no `data-entry-index` to read; no before/after distinction). Show the existing CSS line indicator on dragover.

### 2. Drop above first entry / below last entry

Cursor in the strip immediately above entry 0 or below the last entry is OUTSIDE every entry's bounding box, so the entry-level `dragover` never fires. Search drag falls through to the playlist-card whole-list append handler, which puts the new presentation at the END regardless of intent.

Fix: render two transparent ~16-px-tall **spacer** `<li>` elements:
- **Head spacer** as the FIRST child of the entries list. Accepts search-drag dragover; on drop inserts at index 0.
- **Tail spacer** as the LAST child of the entries list. Accepts search-drag dragover; on drop inserts at `entries.len()`.

Both spacers reuse the same insertion-line CSS pseudo-elements. Head spacer always shows the line at its bottom (`data-drop-position="after"` — visually means "after the spacer = before entry 0"). Tail spacer always shows the line at its top (`data-drop-position="before"` — visually means "before the spacer = after the last entry").

Both spacers are `display: block`, `min-height: 16px`, transparent. They're visible to the cursor (capture dragover) but invisible visually until the line indicator appears.

## Feature: floating bubble + "+" (#272)

### Remove the existing slides toolbar

`crates/presenter-ui/src/components/slide_list.rs:238-260` currently renders `.operator__slides-toolbar` with two children:
- `<label class="operator__line-limit">` — Line limit number input.
- `<button class="operator__slides-add">` — "+" Add slide.

Remove the entire `<div class="operator__slides-toolbar">` block. The slides scroll area below grows to fill the available height.

### Add two floating elements over the slides area

Wrap the slides scroll area in a `position: relative` container. Inside that container, BEFORE the slides, render two absolutely-positioned overlay elements:

**Song-name bubble (top-left, `data-role="slides-song-bubble"`).** Visible only when a presentation is selected.

```html
<div class="operator__slides-bubble"
     data-role="slides-song-bubble"
     data-presentation-id="<uuid>"
     draggable="true"
     title="Drag into a playlist">
  <span class="operator__slides-bubble-name">{presentation.name}</span>
</div>
```

CSS: `position: absolute; top: 8px; left: 8px;`, rounded pill (`border-radius: 999px`), drop-shadow, `cursor: grab`, `z-index: 10`. On `:active`, `cursor: grabbing`.

`on:dragstart`: set MIME `application/x-presentation-id` to the active presentation's UUID (same as search.rs:299-316). Set `op.search_dragging = true` and `op.dragging_from_search = true` so the drop handlers in `presentation_list.rs` (extended in PR #282) accept it. Set `effectAllowed = "copy"`.

`on:dragend`: reset `op.search_dragging` and `op.dragging_from_search` to `false`. Reuse the same cleanup pattern as `search.rs`.

**"+" Add-slide button (top-right, `data-role="add-slide"`).** Visible only when a presentation is selected. Same `data-role` as today so existing E2E tests don't break.

```html
<button class="operator__slides-add-floating"
        data-role="add-slide"
        title="Add slide"
        on:click=add_slide>
  +
</button>
```

CSS: `position: absolute; top: 8px; right: 8px;`, circular (`border-radius: 50%`), `width: 36px; height: 36px;`, `z-index: 10`.

**Pointer-events isolation.** Wrap both elements in a child container of the slides area with `pointer-events: none` so the slides underneath stay clickable everywhere except where the two interactive elements actually sit. The two interactive children override with `pointer-events: auto`.

Alternative if pointer-events isolation is awkward: place each floating element directly as a sibling of the slides scroll container with `position: absolute`, with their own click/drag targets — they only cover ~36-160px regions, leaving the rest of the slides area unobstructed.

### Move "Line limit" to /ui/settings

The Line limit number input previously in the slides toolbar moves to a new "Preferences" section in `/ui/settings`.

The settings page (`crates/presenter-server/src/ui/settings.rs`) renders a section with:

```html
<section data-section="preferences">
  <h2>Preferences</h2>
  <label>
    <span>Operator line limit (max characters per line)</span>
    <input type="number"
           data-role="pref-line-limit"
           min="10"
           max="120"
           step="1"
           value="32" />
  </label>
  <p class="settings__hint">
    Stored in your browser's local storage. Slides with longer lines show a warning marker.
  </p>
</section>
```

`crates/presenter-server/src/settings_script.js` adds a small block that:
- Reads `localStorage.getItem("lineLimit") || "32"` on load and sets the input's value.
- On `input` change, writes `localStorage.setItem("lineLimit", value)`.

The operator UI continues to read `lineLimit` from localStorage in `OperatorState::new()` exactly as today (`crates/presenter-ui/src/state/operator.rs:58-60`). Reload the operator UI for the new value to take effect (acceptable — line limit changes are rare).

## File changes

| File | Change |
|------|--------|
| `crates/presenter-ui/src/components/presentation_list.rs` | (a) Empty-state `<li>` gets same dragover/drop handlers as regular entry (drop → insert at 0). (b) Add head spacer `<li>` before entry 0 and tail spacer after the last entry (drop → insert at 0 / `entries.len()`). |
| `crates/presenter-ui/src/components/slide_list.rs` | Remove `.operator__slides-toolbar` (lines 238-260). Wrap the slides container so it can host absolutely-positioned children. Add `<div class="operator__slides-bubble">` (top-left, draggable) and `<button class="operator__slides-add-floating">` (top-right). Both visible only when a presentation is selected. |
| `crates/presenter-ui/styles/operator.css` | New CSS for `.operator__slides-bubble` and `.operator__slides-add-floating` (positioning, look). New CSS for `.operator__list-spacer` (head/tail spacer, transparent until line indicator). Remove `.operator__slides-toolbar` rules (defunct). |
| `crates/presenter-server/src/ui/settings.rs` | Add a "Preferences" section with the line-limit input. |
| `crates/presenter-server/src/settings_script.js` | Add JS that reads/writes `localStorage["lineLimit"]`. |
| `tests/e2e/wasm-drag-drop.spec.ts` | 3 new tests: empty-playlist drop, drop on head spacer, drop on tail spacer. 1 new test: bubble drag from slides area into playlist (asserts insertion at chosen position). |

No changes to the search.rs dragstart, the playlist-card whole-list drop handler in `playlist_list.rs` (becomes redundant for the open playlist but stays for sidebar drops onto inactive playlists), or the API.

## Testing

### New Playwright E2E tests (in `tests/e2e/wasm-drag-drop.spec.ts`)

1. **Empty playlist drop:** open a playlist with zero entries (or seed one). Search for any presentation. Drag the search result onto the `<li class="empty">`. Assert the playlist now has 1 entry equal to the dragged presentation. Assert console clean.

2. **Drop on head spacer:** open a playlist with ≥1 entry. Search. Drag onto `[data-role="head-spacer"]` (the new spacer's data-role). Assert the new presentation is at index 0. Assert the original entry is now at index 1.

3. **Drop on tail spacer:** open a playlist with ≥1 entry. Search. Drag onto `[data-role="tail-spacer"]`. Assert the new presentation is at the last index. Assert original entries unmoved.

4. **Bubble drag into playlist:** select a library, click a presentation to open it (so the floating bubble appears), open a playlist with ≥1 entry, drag `[data-role="slides-song-bubble"]` over entry index 0 (top half), drop, assert the active presentation is now at index 0 of the playlist.

All 4 tests assert browser console is clean.

### Existing tests must keep passing

- `wasm-drag-drop.spec.ts:354` "drag search result into specific position in open playlist (#274)" — unchanged, covers middle position.
- The within-playlist reorder E2E — unchanged.
- Any test that uses `[data-role="add-slide"]` — keeps working because the new "+" button reuses that data-role.

## Risks / unknowns

- **Pointer-events isolation.** If the wrapper-with-`pointer-events: none` approach interferes with the slides' own dragover (slide-reorder), use the sibling approach instead — each floating element is positioned independently and they only cover ~36-160px regions, so they don't block slides anywhere else.
- **Bubble z-index conflicts.** Operator modals (presentation create, edit, etc.) use higher z-indexes; the bubble at z-10 stays under them. Verify by opening a modal while a presentation is selected — the bubble should disappear under the modal backdrop.
- **Spacer height visible when not dragging.** The 16px transparent spacer adds vertical whitespace to the entries list. If this looks off, reduce to 8px or use a smaller value when no drag is in progress (e.g. `height: 6px` default, expand to `16px` when `op.dragging_from_search.get_untracked()` is true). The CSS line indicator will still show.
- **Settings iframe localStorage.** The settings page is loaded via `<iframe src="/ui/settings">`. Same-origin iframes share localStorage with the parent, so writes from the settings page propagate immediately. Operator UI reads on next load — that's acceptable for a setting changed rarely.

## Out of scope

- Issue #271 (worship scroll behavior — "always show one row of slides to the future") — separate brainstorm.
- Animations on the floating bubble appearing/disappearing.
- Tooltip / first-time-user discovery hint for the bubble (operators will discover by trying).
- Server-persisted line limit (stays client-side localStorage).
- Drag onto an inactive playlist card in the sidebar from the new bubble — sidebar's whole-list drop handler still works for that case as today.
