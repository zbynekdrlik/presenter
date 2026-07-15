# Slide Multi-Select + Copy/Cut/Paste — Design

**Issue:** #554
**Date:** 2026-07-15
**Status:** Accepted (design approved by user 2026-07-15)
**Depends on:** #552 (reorder error-handling + staleness guard), #553 (Group-field save path) — both must be merged first; this feature builds on the fixed reorder path.

## Problem

A worship-team volunteer preparing songs cannot copy several slides and paste them at the
end or into the middle of a presentation. The capability does not exist: the slide editor
(`crates/presenter-ui/src/components/slide_list.rs`) has no multi-select, no clipboard, and
no arbitrary-position insert. Today's closest tools are a single-slide Duplicate (always
inserts directly after its source) and a `+` button that always appends a blank slide at
the very end — even though the backend (`edit_ops.rs::insert_blank_slide`) already accepts
an arbitrary `position: Option<u32>` that no frontend caller uses.

## Goal

Within one open presentation: select multiple slides, copy or cut them, and paste them at
any position (start, between any two slides, end) — usable with a mouse on a PC and with
touch on a tablet, with no data ever lost by a failed or abandoned operation.

## Decisions (all confirmed with the user)

1. **Selection** — a checkbox on every slide card (same interaction pattern as the Bible
   page's verse picker) AND Shift+click to select the range between the last-clicked and
   the shift-clicked slide. A selection panel appears while ≥1 slide is selected showing
   "N selected" plus the action buttons.
2. **Paste placement** — BOTH of:
   - a clickable "paste here" insertion bar rendered in every gap between slides (and
     before the first / after the last), visible only while the clipboard is non-empty;
   - dragging the copied/cut block and dropping it on a gap (reusing the reorder drop
     mechanics).
   Ctrl/Cmd+V pastes at the insertion bar the user last hovered/selected, else at the end.
3. **Triggers** — BOTH selection-panel buttons (Copy / Cut / Paste) AND keyboard shortcuts
   Ctrl/Cmd+C, Ctrl/Cmd+X, Ctrl/Cmd+V. Shortcuts are ignored while focus is inside a
   textarea/input (typing in a slide's text field must never trigger slide-level copy).
4. **Scope** — within a single presentation NOW. Cross-presentation clipboard is a later
   extension; the clipboard state is designed so that extension needs no rework (see
   Clipboard below). Client-side clipboard only; it does not survive a page refresh, which
   is acceptable for this scope.
5. **Copy AND Cut as separate actions.** Cut is non-destructive until the paste lands:
   cut slides remain in place, visually marked (dimmed + "cut" badge), and are moved only
   when a paste actually succeeds. Abandoning a cut (Escape, clearing the selection,
   copying something else) simply removes the mark — nothing is deleted.

## Architecture

### Client state (new, in `crates/presenter-ui/src/state/operator.rs`)

```rust
pub struct SlideClipboard {
    /// Ids of the slides captured, in list order at capture time.
    pub slide_ids: Vec<String>,
    /// Copy = paste inserts copies; Cut = paste MOVES the originals.
    pub mode: ClipboardMode, // Copy | Cut
    /// Presentation the ids belong to (guards against stale clipboard after switching songs
    /// now, and becomes the cross-presentation key later).
    pub presentation_id: String,
}
```

- `selected_slide_ids: RwSignal<HashSet<String>>` — same shape as the Bible page's
  `state/bible.rs:53` pattern.
- `clipboard: RwSignal<Option<SlideClipboard>>`.
- Selection and clipboard are cleared when the selected presentation changes (scope 4).

### Server operations

- **Paste-of-copy** — ONE new endpoint:
  `POST /presentations/{id}/slides/paste` with body
  `{ "slideIds": [...], "position": N }` → clones the named slides (full content: main,
  translation, stage, group) and inserts the clones as a contiguous block at `position`.
  Implemented in `edit_ops.rs` as a multi-slide generalization of the existing
  `duplicate_slide`/`insert_blank_slide` plumbing. Returns the full new slide list (same
  contract as the reorder endpoint). Position is clamped to `0..=len`, ids not found in
  the presentation are rejected with 422 (a stale clipboard must fail loudly, not
  half-paste).
- **Paste-of-cut** — NO new endpoint. A cut+paste within one presentation is exactly a
  multi-slide reorder: the client computes the final id order (selected block removed
  from its old positions, spliced in at the target gap) and calls the EXISTING
  `POST /presentations/{id}/slides/reorder` — inheriting #552's error surfacing and
  `slide_edit_seq` staleness guard for free.

### UI (all in `slide_list.rs` + a small new `slide_selection.rs` component)

- Checkbox per slide card; Shift+click range logic in the checkbox click handler.
- Selection panel (sticky above the list): "N selected", Copy, Cut, Clear.
- Insertion bars: one per gap, rendered only when `clipboard.get().is_some()`; click =
  paste at that gap; also a valid drop target for the block drag.
- Cut marking: cards whose id is in a `mode == Cut` clipboard get a `--cut` CSS modifier
  (dimmed + badge).
- Keyboard: one `keydown` listener on the list container; C/X/V with ctrl/meta, guarded by
  the existing `is_interactive_tag` helper so shortcuts never fire from inside text fields;
  Escape clears the clipboard/selection.
- All mutations follow the post-#552 pattern: visible error surfacing (no silent
  `if let Ok` swallowing) and the `slide_edit_seq` staleness guard.

## Error handling

- Paste request fails → the list stays unchanged, a visible error toast appears (same
  mechanism the #552 fix uses), the clipboard is KEPT so the user can retry.
- Cut whose reorder fails → originals stay in place still marked; retry or Escape.
- Clipboard referencing a slide meanwhile deleted (by another tab) → server 422 → toast
  "slides no longer exist", clipboard cleared.

## Testing

Playwright E2E (real interactions, no `test.skip()`), per this repo's CLAUDE.md drag-drop
rule, covering at minimum:

- select 2 slides via checkboxes; select a range via Shift+click
- copy + paste at: start (above first), a true middle gap (list ≥4), end (below last)
- cut + paste to a different position — originals moved, count unchanged, order correct
  after reload (persistence proven)
- cut then Escape — nothing moved, marks cleared
- Ctrl/Cmd+C / X / V shortcut paths; shortcuts inert while typing in a slide textarea
- paste into an empty presentation (position 0 edge)
- failed paste (server stopped / 422 path) shows the error and loses nothing

Rust unit tests: the new `edit_ops.rs` paste operation (clamping, contiguous block, 422 on
unknown ids, group/content cloned intact) and the pure client-side "final order after cut
splice" helper.

## Out of scope (tracked, not built now)

- Cross-presentation clipboard (decision 4 — later extension; `presentation_id` field in
  the clipboard is the seam).
- Persisting the clipboard across page refresh.
- Multi-slide drag-reorder WITHOUT the cut/paste flow (block drag of a live selection);
  Cut+paste covers the need.
