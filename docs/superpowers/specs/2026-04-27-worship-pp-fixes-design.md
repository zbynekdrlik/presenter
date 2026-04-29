# Worship-PP Fixes — Design

**Date:** 2026-04-27
**Status:** Proposed
**Scope:** Frontend (presenter-ui WASM crate) + thin server DTO change for presentation-name enrichment

## Goal

Fix three regressions reported on the worship-pp stage layout and operator playlist UI after PR #268:

1. **Stage layout overlap** — the six worship-pp slide regions (current/next group/song/slide) are positioned absolutely against the page, and the playlist sidebar overlays them at 30% on the right. Slides and sidebar overlap, nothing is readable.
2. **Active-song highlight too subtle** — the current `--active` rule applies a 15%-opacity blue tint and bold text. From the projector that's invisible. The user wants the now-playing song to stand out at a glance.
3. **Operator playlist entries show no presentation name** — when a playlist is selected, the operator's playlist view rebuilds `ctx.presentations` from playlist entries with `String::new()` for the name. The entry name lookup then returns an empty string. Entries appear as blank rows with only the library tag visible.

## Non-goals

- No redesign of the worship-pp layout. The existing six-region structure stays; we only constrain it to the left 70%.
- No change to the worship-snv layout. It is unaffected.
- No change to drag-drop, playlist CRUD, or the GET /playlists/{id} fix from PR #268.

## Approach

Three independent fixes shipped together because they all surfaced from the same user session and share the worship-pp/playlist domain.

---

### Issue 1 — Stage layout: wrap regions in `.stage-pp__slides-area`

The CSS for `.stage-pp__slides-area` (left 70%, full height, `overflow: hidden`) and `.stage-pp__playlist-sidebar` (right 30%) is already defined. The bug is that `worship_pp.rs` doesn't wrap the six slide regions in `.stage-pp__slides-area` — they remain direct children of `.stage-container`, so their existing absolute positioning anchors against the page (full width).

**Change:**

- In `crates/presenter-ui/src/components/stage/worship_pp.rs`, wrap the six regions (`.stage__current-group`, `.stage__current-song`, `.stage__current-slide`, `.stage__next-group`, `.stage__next-song`, `.stage__next-slide`) inside a single `<div class="stage-pp__slides-area">` parent. The playlist sidebar stays as a sibling.
- In `crates/presenter-ui/styles/stage.css`, scope the worship-pp variants of those six region rules so their absolute positioning is relative to `.stage-pp__slides-area` (which already has `position: absolute`). Use scoped selectors like `.stage-pp__slides-area .stage__current-slide { ... }` so the SNV layout (which uses bare `.stage__current-slide`) is unaffected.
- Keep the original SNV positioning intact. Only override what worship-pp needs (effectively the same regions but with their `right`/`width` recalculated to fit a 100%-of-parent container which is itself 70% of the page).

The rendered geometry: slides occupy 70% width × 92% height on the left, playlist occupies 30% width × 92% height on the right with a 1px left border, status bar spans the bottom 8% as before. No overlap.

### Issue 2 — Active-song highlight: stronger, projector-visible

Replace the faint `.stage-pp__playlist-entry--active` styling with a high-contrast pill: solid accent background, white text, a 4px left-edge accent bar for at-a-glance scanning, and a small font-weight bump (no font-size change to avoid layout reflow).

**Change:**

- In `crates/presenter-ui/styles/stage.css`, update `.stage-pp__playlist-entry--active`:
  - `background: #38bdf8` (full-opacity sky-blue, matches existing accent palette)
  - `color: #0f172a` (dark on light pill — high contrast)
  - `font-weight: 700`
  - `border-left: 4px solid #0ea5e9` (deeper accent bar, sits inside the pill)
  - `padding-left: calc(0.6rem - 4px)` to compensate for the border so text alignment with non-active rows stays consistent.

Non-active entries keep their existing `color: #94a3b8` and transparent background so the contrast is dramatic at projector distance.

### Issue 3 — Operator playlist entry names: server enrichment

The server already has `fetch_presentation_names_for_playlist` (used to populate stage playlist sidebar). Use it for the API response too: each `Presentation` entry in the JSON gains a `presentation_name` field. The operator reads `entry.kind.presentation_name` directly when rendering, eliminating the empty-string rebuild.

**Change:**

Server side (`presenter-server`):

- Find the public Playlist DTO that's serialized in the GET/PATCH/PUT /playlists/{id} responses.
- Add a `presentation_name: Option<String>` field on the `PlaylistEntryKind::Presentation` variant in the *response* type only (the request type stays minimal — clients still PUT only `presentation_id`, the server fills the name on read).
- Where the playlist is serialized, run `fetch_presentation_names_for_playlist` and inject names into each `Presentation` entry before returning the JSON. Empty string when the name can't be resolved (presentation deleted).

Client side (`presenter-ui`):

- Update `PlaylistEntryKind::Presentation` deserialization to include `presentation_name: Option<String>`.
- In `presentation_list.rs`, line ~352-356, replace the lookup-from-`ctx.presentations` with `entry.presentation_name.clone().unwrap_or_default()`.
- Drop the now-unused `rebuild_playlist_presentations_with_signal` calls (and the helper itself) from `operator.rs:494-508` and `presentation_list.rs:605-623`. The operator UI no longer needs to fake a presentations list when a playlist is selected — names come from the playlist response directly.
- Library list is still the source of `ctx.presentations` for non-playlist mode (clicking a library), so keep that flow untouched.

This is the cleanest fix because:
- The server already has all the data and the lookup function.
- One round-trip per playlist load (no N+1 lazy fetches).
- No client-side cache to invalidate when a presentation gets renamed.
- Removes a chunk of WASM-side complexity (rebuild functions).

## Data flow after fix

```
operator clicks playlist row
  → GET /playlists/{id}
  ← { entries: [ { kind: { type: "presentation", presentation_id, presentation_name }, ... }, ... ] }
operator renders entries directly from the response — names already there.

worship-pp stage gets snapshot
  ← { playlist_entries: [ { name, presentation_id, is_active, entry_type }, ... ] }   (unchanged)
stage renders sidebar; active entry gets the new high-contrast --active pill.
```

## Testing strategy

1. **Unit test (server):** `replace_playlist_entries` and `get_playlist` responses include `presentation_name` for `Presentation` entries (assert against a known seeded presentation). Empty string when the presentation is missing.
2. **Unit test (server):** existing tests still pass — adding the field is additive, request types unchanged.
3. **Unit test (presenter-ui):** `PlaylistEntryKind::Presentation` round-trips `presentation_name` through serde.
4. **E2E (Playwright):** Extend `wasm-playlist-operations.spec.ts` — after dragging a presentation onto a playlist, click the playlist row and assert the rendered entry has visible non-empty text matching the presentation name. (Today this would render an empty span.)
5. **E2E (Playwright):** Open `/stage` with worship-pp layout active and a playlist with entries. Assert:
   - `.stage-pp__slides-area` exists and contains all six slide regions
   - `.stage-pp__slides-area`'s right edge ≤ `.stage-pp__playlist-sidebar`'s left edge (no overlap, computed from `getBoundingClientRect`).
   - When an active presentation is set, the matching `.stage-pp__playlist-entry--active` row has a `background-color` distinct from the non-active rows (computed style, not just class presence).
6. **Manual visual check:** open dev stage with a playlist + active presentation, confirm the highlight is obvious from a normal viewing distance.
7. **Browser console:** zero errors, zero warnings on operator and stage pages.

## File-level overview

| File | Change |
|------|--------|
| `crates/presenter-ui/src/components/stage/worship_pp.rs` | Wrap six regions in `<div class="stage-pp__slides-area">` |
| `crates/presenter-ui/styles/stage.css` | Scope worship-pp region positioning to `.stage-pp__slides-area`; rewrite `.stage-pp__playlist-entry--active` with high-contrast styling |
| `crates/presenter-server/src/router/playlists.rs` (or whichever module owns the response DTO) | Add `presentation_name: Option<String>` to the response variant; enrich responses with name lookup |
| `crates/presenter-server/src/state/...` | Plumb the existing name-lookup into the response builder |
| `crates/presenter-core/src/playlist.rs` (if the DTO lives there) | Add field to the response model |
| `crates/presenter-ui/src/api/playlists.rs` and/or `presenter-core` | Mirror the new field for deserialization |
| `crates/presenter-ui/src/components/presentation_list.rs` | Read name from entry directly; remove `rebuild_playlist_presentations_with_signal` |
| `crates/presenter-ui/src/pages/operator.rs` | Remove the `String::new()` summaries-rebuild block at line ~494-508 |
| `tests/e2e/wasm-playlist-operations.spec.ts` | Add operator playlist-name visibility check after drag-drop |
| `tests/e2e/` (new or extended stage spec) | Add worship-pp layout + active-highlight E2E |

## Risks / unknowns

- The exact location of the response DTO depends on whether playlist response uses the same struct as the storage model or a separate response struct. The plan must verify and choose the right insertion point.
- If the response DTO is shared with the request body, splitting it (server-side response variant vs request variant) is part of this work.
- Edge case: presentation referenced by playlist was deleted. Server returns `presentation_name: None` (or `Some("")`); operator and stage should both render the entry without crashing (e.g., as `[deleted]` or just blank — pick blank to match existing behavior).

## Out of scope

- Cross-library global presentation index (Alt B for issue 3).
- Lazy per-entry name fetch (Alt C for issue 3).
- Adding ▶ marker or font-size scaling for the active row (Alts B/C for issue 2).
- Any change to worship-snv, preach, bible, NDI, timer, or API stage layouts.
