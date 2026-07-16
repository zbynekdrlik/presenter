# Implementation Plan — Slide Multi-Select + Copy/Cut/Paste (#554)

**Issue:** #554 (slide multi-select + copy/cut/paste in the operator slide editor)
**Spec (the contract — implement exactly this):** `docs/superpowers/specs/2026-07-15-slide-multiselect-copy-paste-design.md`
**Depends on (already merged):** #552 reorder error-handling + `slide_edit_seq` staleness guard, #553 Group-field save path, #556 (the file split: `slide_reorder.rs` / `slide_save.rs`). All present on `dev`.

> **REQUIRED SUB-SKILL:** Execute this plan with `superpowers:subagent-driven-development` (one subagent per task, sequential, in-session). Each task below is bite-sized, ends green, and has its own commit. Follow the repo's TDD rules: bug-style RED-before-GREEN where a behavior can be tested before implementing; every task ships its tests in the SAME commit.

> **Branch/version:** Work on `dev` (two-branch workflow). Local Rust builds are ALLOWED on this dev2 machine (`airuleset:local-builds=allowed`). Task 1 bumps the version FIRST.

---

## Goal

Inside one open presentation in the operator's slide editor: check-select several slides (or Shift+click a range), Copy or Cut them, and Paste at any gap (start / between any two / end) — via a selection panel, clickable insertion bars, block drag-drop, and Ctrl/Cmd+C/X/V — with **no data ever lost** by a failed or abandoned operation. Usable with mouse (PC) and touch (tablet). Client-side clipboard, one presentation for now (the `presentation_id` field is the seam for a later cross-presentation extension).

## Architecture

### Server (one new endpoint for copy; cut reuses reorder)

- **Paste-of-copy** — ONE new endpoint `POST /presentations/{id}/slides/paste` with body `{ "slideIds": [...], "position": N }`. Server clones the named slides' full content (main, translation, stage, group) — a multi-slide generalization of the existing `duplicate_slide`/`insert_blank_slide` plumbing in `edit_ops.rs` — inserts the clones as a contiguous block (in the presentation's own list order) at `position` (clamped `0..=len`), copies each clone's #515 stage-layout marker (non-fatal, exactly like `duplicate_slide`), and returns the full new slide list (same contract as reorder). Unknown ids ⇒ `422 Unprocessable Entity` (a stale clipboard must fail loudly, never half-paste). Reuses `replace_presentation_slides` (which bumps `updated_at` in one transaction) + `reconcile_stage_state_after_edit` + `cache_presentation_value` + `broadcast_stage_snapshots` + `BibleSlidesChanged` + **`nudge_sync()`** — so it propagates via #555 sync exactly like every other mutation.
- **Paste-of-cut** — NO new endpoint. A cut+paste within one presentation IS a multi-slide reorder: the client computes the final id order (block removed from old positions, spliced in at the target gap) and calls the EXISTING `POST /presentations/{id}/slides/reorder`, inheriting #552's error surfacing + `slide_edit_seq` staleness guard for free.

### Client state (new, in `crates/presenter-ui/src/state/operator.rs`)

- `ClipboardMode { Copy, Cut }` and `SlideClipboard { slide_ids: Vec<String>, mode: ClipboardMode, presentation_id: String }`.
- `selected_slide_ids: RwSignal<HashSet<String>>` (same shape as `state/bible.rs:53`).
- `clipboard: RwSignal<Option<SlideClipboard>>`.
- `selection_anchor_index: RwSignal<Option<usize>>` (anchor for Shift+click range).
- `paste_target_gap: RwSignal<Option<usize>>` (last hovered/selected gap for Ctrl/Cmd+V; else end).
- `dragging_clipboard: RwSignal<bool>` (block drag toward an insertion gap).
- Selection + clipboard + anchor + gap are cleared when the selected presentation changes.

### UI (new `slide_selection.rs` + `slide_selection_logic.rs`; minimal edits to `slide_list.rs`)

- **Checkbox** per slide card (edit mode only); plain click toggles + sets the anchor; Shift+click selects the inclusive range from the anchor. Checkbox is CONTROLLED (`prop:checked` from a per-card `Memo`; `on:click` `prevent_default`s the native toggle) so selection changes never rebuild the list.
- **Selection panel** (sticky above the list): "N selected", Copy, Cut, Paste, Clear.
- **Insertion bars**: one per gap (before first / between / after last), rendered ONLY when the clipboard is non-empty. Click = paste at that gap; also a drop target for the block drag; hovering one sets `paste_target_gap`. When the clipboard is non-empty the grid switches to a single column (`.operator__slides--clipboard`) so every inter-slide gap is one unambiguous full-width bar (a 2-D 3-column grid has no single "gap between slide i and i+1"; a focused single column is also better UX for a placement task). This is the one thing that reads `clipboard` in the list render closure, so a clipboard change rebuilds the list — acceptable: clipboard changes are deliberate and infrequent (never per-keystroke), and the C/X/V shortcuts are guarded so a rebuild never races in-flight typing (a button Copy blurs+saves the textarea first).
- **Cut marking**: cards whose id is in a `mode == Cut` clipboard get `operator__slide-card--cut` (dimmed + "cut" badge), driven by a per-card `Memo`.
- **Keyboard**: ONE `keydown` listener installed on `window` from the selection setup (mirrors the operator page's existing global handler in `operator.rs`; a container-scoped listener can't work — the slide grid never holds DOM focus, and a focused selection checkbox would wrongly suppress the very next Ctrl+C). Ctrl/Cmd+C/X/V + Escape, mapped by the pure `shortcut_action` helper. Guarded by `text_entry_focused()` so shortcuts are inert only while a TEXT field (textarea / non-checkbox input) is focused — a focused checkbox must NOT suppress the shortcut (you select with the checkbox, then Ctrl+C). This narrows the spec's "use `is_interactive_tag`" to its stated intent ("never fire from inside text fields"); `is_interactive_tag` stays the guard for the per-CARD click, where its breadth is correct.
- **Reactivity discipline** (per `.claude/skills/ui/SKILL.md`): selection checkbox + `--selected`/`--cut` classes read state through per-card `Memo`s (Copy, `PartialEq` types) — never a full rebuild; the list is keyed by index today (unchanged); pure order/shortcut logic lives in host-testable helpers.

### Error handling (per spec)

- Copy-paste request fails (non-422) → list unchanged, toast, clipboard KEPT (retry).
- Copy-paste `422` (stale clipboard) → toast "those slides no longer exist", clipboard CLEARED.
- Cut-paste (reorder) fails → originals stay in place, still marked; toast; retry or Escape.
- Abandoning a cut (Escape / Clear / copying something else) just removes the mark — nothing deleted.

## Tech stack

Rust (axum + SeaORM server, Leptos/WASM `presenter-ui` — OUTSIDE the workspace, own `Cargo.lock`, wasm32), Playwright E2E. Banned in prod code: `unwrap()`/`expect()`/`panic!`/`std::thread::sleep` (the `presenter-ui` WASM crate is exempt from panic rules but MATCH the file's existing style — prefer `?`/`let ... else`/`.map_or`). No `test.skip()`, no arbitrary `sleep`/`waitForTimeout` for state waits (use `expect.poll` / `waitForResponse` / `waitForFunction`). Quality gates: file ≤1000 prod lines, fn ≤120 lines (tests NOT exempt), `quality-check.sh --strict`.

---

## Task 1 — Version bump (FIRST action, before any code)

**Modify:** `Cargo.toml` (workspace `[workspace.package].version`).

Steps:

- [ ] `git fetch origin && git merge origin/main` (sync base first).
- [ ] Determine the next free patch version DYNAMICALLY (do NOT hard-code — #555's PR #558 may land first and move `main`/the release):
  ```bash
  git fetch origin --tags
  gh release list -L 1              # latest published release, e.g. v0.4.200
  grep -m1 'version = ' Cargo.toml  # current dev workspace version
  git show origin/main:Cargo.toml | grep -m1 'version ='   # main's version
  ```
  Pick the SMALLEST `0.4.Z` that is strictly greater than BOTH the latest release AND `main`'s version AND the current `dev` version. (As of writing: dev `0.4.200`, release `v0.4.199` → bump to `0.4.201`. If #558 already moved dev/main to `0.4.201`, use `0.4.202`, etc.)
- [ ] Edit `Cargo.toml` `[workspace.package]` `version = "X.Y.Z"` to that value. (Only the workspace version is CI-gated by `version-check.yml`; the `presenter-ui` crate's own `0.1.x` version is independent — leave it.)
- [ ] Verify: `grep -m1 'version = ' Cargo.toml` shows the new value.

**Commit:** `chore: bump version to X.Y.Z for slide multi-select copy/paste (#554)`

---

## Task 2 — Pure client logic helpers (host-testable) [TDD: RED first]

**Create:** `crates/presenter-ui/src/components/slide_selection_logic.rs`
**Modify:** `crates/presenter-ui/src/components/mod.rs` (register `mod slide_selection_logic;`)

These three helpers hold ALL the trick logic (keyboard mapping, Shift-range selection, cut-splice ordering) as pure functions with no DOM/Leptos — so they run in the host `cargo test --lib` build.

Steps:

- [ ] Add `mod slide_selection_logic;` to `crates/presenter-ui/src/components/mod.rs` (near the other `slide_*` decls, lines 12-17). Keep it private (`mod`, not `pub mod`).
- [ ] Create the file with the helpers AND the tests below in ONE commit. Write the tests FIRST, watch them fail against an empty/`todo!()` body, then implement. Full file:

```rust
//! Pure, host-testable logic for the slide multi-select clipboard (#554):
//! keyboard-shortcut mapping, Shift+click range selection, and the cut+paste
//! "final order after splice" computation. NO DOM / Leptos here — these run in
//! the `cargo test --lib` host build (`presenter-ui` is excluded from the
//! workspace; run `cd crates/presenter-ui && cargo test --lib`).

use std::collections::HashSet;

/// The clipboard action a guarded keydown maps to (#554).
#[derive(Debug, PartialEq, Eq)]
pub(super) enum ShortcutAction {
    Copy,
    Cut,
    Paste,
    Clear,
}

/// Map a keydown (`key` + whether ctrl/meta is held) to a clipboard action.
/// Escape → Clear (no modifier needed); c/x/v WITH ctrl or meta → Copy/Cut/Paste.
/// Everything else → `None`. The caller applies the field-focus guard separately.
pub(super) fn shortcut_action(key: &str, ctrl_or_meta: bool) -> Option<ShortcutAction> {
    match key {
        "Escape" => Some(ShortcutAction::Clear),
        "c" | "C" if ctrl_or_meta => Some(ShortcutAction::Copy),
        "x" | "X" if ctrl_or_meta => Some(ShortcutAction::Cut),
        "v" | "V" if ctrl_or_meta => Some(ShortcutAction::Paste),
        _ => None,
    }
}

/// Selection after a Shift+click range select: adds every id in
/// `ids[min(anchor,clicked)..=max(anchor,clicked)]` to `current` (order of the
/// two indices does not matter). Existing selection is preserved (additive).
/// Out-of-range indices contribute nothing.
pub(super) fn range_select(
    ids: &[String],
    anchor: usize,
    clicked: usize,
    current: &HashSet<String>,
) -> HashSet<String> {
    let (lo, hi) = if anchor <= clicked {
        (anchor, clicked)
    } else {
        (clicked, anchor)
    };
    let mut next = current.clone();
    for id in ids.iter().skip(lo).take(hi.saturating_sub(lo) + 1) {
        next.insert(id.clone());
    }
    next
}

/// Final slide-id order for a cut+paste (#554): remove `cut_ids` from `current`
/// (keeping their relative order as the moved block) and splice that block in at
/// gap `position` (0..=current.len(), counted in the CURRENT list's gaps —
/// 0 = before first, len = after last). Returns `None` if `cut_ids` is empty or
/// names an id absent from `current` (a stale cut). The result is a permutation
/// of `current`, so the existing reorder endpoint accepts it unchanged.
pub(super) fn cut_splice_order(
    current: &[String],
    cut_ids: &[String],
    position: usize,
) -> Option<Vec<String>> {
    if cut_ids.is_empty() {
        return None;
    }
    let cut_set: HashSet<String> = cut_ids.iter().cloned().collect();
    if !cut_set.iter().all(|id| current.contains(id)) {
        return None;
    }
    let clamped = position.min(current.len());
    let mut block: Vec<String> = Vec::new();
    let mut remaining: Vec<String> = Vec::new();
    let mut removed_before = 0usize;
    for (idx, id) in current.iter().enumerate() {
        if cut_set.contains(id) {
            block.push(id.clone());
            if idx < clamped {
                removed_before += 1;
            }
        } else {
            remaining.push(id.clone());
        }
    }
    let insert_at = clamped - removed_before; // <= remaining.len() by construction
    remaining.splice(insert_at..insert_at, block);
    Some(remaining)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }
    fn set(items: &[&str]) -> HashSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn shortcut_escape_maps_to_clear_without_modifier() {
        assert_eq!(shortcut_action("Escape", false), Some(ShortcutAction::Clear));
    }

    #[test]
    fn shortcut_cxv_require_ctrl_or_meta() {
        assert_eq!(shortcut_action("c", true), Some(ShortcutAction::Copy));
        assert_eq!(shortcut_action("X", true), Some(ShortcutAction::Cut));
        assert_eq!(shortcut_action("v", true), Some(ShortcutAction::Paste));
        assert_eq!(shortcut_action("c", false), None);
        assert_eq!(shortcut_action("z", true), None);
    }

    #[test]
    fn range_select_is_inclusive_and_order_independent() {
        let all = ids(&["a", "b", "c", "d", "e"]);
        let forward = range_select(&all, 1, 3, &HashSet::new());
        assert_eq!(forward, set(&["b", "c", "d"]));
        let backward = range_select(&all, 3, 1, &HashSet::new());
        assert_eq!(backward, set(&["b", "c", "d"]));
    }

    #[test]
    fn range_select_preserves_existing_selection() {
        let all = ids(&["a", "b", "c", "d"]);
        let out = range_select(&all, 2, 3, &set(&["a"]));
        assert_eq!(out, set(&["a", "c", "d"]));
    }

    #[test]
    fn cut_splice_moves_block_to_start() {
        let cur = ids(&["a", "b", "c", "d", "e"]);
        let out = cut_splice_order(&cur, &ids(&["c", "d"]), 0).unwrap();
        assert_eq!(out, ids(&["c", "d", "a", "b", "e"]));
    }

    #[test]
    fn cut_splice_moves_block_to_end() {
        let cur = ids(&["a", "b", "c", "d", "e"]);
        // gap 5 = after last, in the CURRENT list.
        let out = cut_splice_order(&cur, &ids(&["a", "b"]), 5).unwrap();
        assert_eq!(out, ids(&["c", "d", "e", "a", "b"]));
    }

    #[test]
    fn cut_splice_moves_block_to_true_middle_accounting_for_removed_before() {
        // Cut a,b (both before the gap); target gap 4 (before "e"). Two ids are
        // removed from before the gap, so the block lands right before "e".
        let cur = ids(&["a", "b", "c", "d", "e"]);
        let out = cut_splice_order(&cur, &ids(&["a", "b"]), 4).unwrap();
        assert_eq!(out, ids(&["c", "d", "a", "b", "e"]));
    }

    #[test]
    fn cut_splice_block_order_follows_current_list_not_cut_ids_argument() {
        let cur = ids(&["a", "b", "c", "d"]);
        // cut_ids given out of list order — block must still be [b, c].
        let out = cut_splice_order(&cur, &ids(&["c", "b"]), 4).unwrap();
        assert_eq!(out, ids(&["a", "d", "b", "c"]));
    }

    #[test]
    fn cut_splice_result_is_a_permutation() {
        let cur = ids(&["a", "b", "c", "d", "e"]);
        let out = cut_splice_order(&cur, &ids(&["b", "d"]), 1).unwrap();
        let mut sorted = out.clone();
        sorted.sort();
        assert_eq!(sorted, ids(&["a", "b", "c", "d", "e"]));
        assert_eq!(out.len(), 5);
    }

    #[test]
    fn cut_splice_rejects_empty_and_unknown() {
        let cur = ids(&["a", "b"]);
        assert!(cut_splice_order(&cur, &[], 0).is_none());
        assert!(cut_splice_order(&cur, &ids(&["z"]), 0).is_none());
    }

    #[test]
    fn cut_splice_single_id_is_a_plain_move() {
        let cur = ids(&["a", "b", "c"]);
        let out = cut_splice_order(&cur, &ids(&["a"]), 3).unwrap();
        assert_eq!(out, ids(&["b", "c", "a"]));
    }
}
```

- [ ] RED then GREEN: `cd crates/presenter-ui && cargo test --lib slide_selection_logic`
  Expected: after writing tests against `todo!()` bodies they FAIL to compile/panic (RED); after implementing, `test result: ok. 11 passed`.

**Commit:** `test(slides): pure cut-splice / range / shortcut helpers for multi-select (#554)`

---

## Task 3 — Backend paste operation `AppState::paste_slides` [TDD: RED first]

**Modify:** `crates/presenter-server/src/state/slides/edit_ops.rs` (add `PasteSlidesError` + `paste_slides` + tests)
**Modify:** `crates/presenter-server/src/state/slides.rs` (re-export the error)

Steps:

- [ ] In `state/slides.rs`, re-export the new error so the router can name it (the module stays private): add `pub(crate) use edit_ops::PasteSlidesError;` next to `mod edit_ops;`.
- [ ] In `edit_ops.rs`, add the error type (top of the file, after the imports) and the method inside the existing `impl AppState { ... }` block. Write the tests FIRST (append to the existing `#[cfg(test)] mod tests`), watch them fail, then implement.

Error type + method:

```rust
/// Error surface for `paste_slides` so the router can distinguish a stale
/// clipboard (unknown ids → 422) from an internal failure (→ 500). (#554)
#[derive(Debug)]
pub enum PasteSlidesError {
    /// One or more requested slide ids are not in the presentation (stale
    /// clipboard) — the paste must fail loudly, never half-apply.
    UnknownSlides,
    /// Any other failure (cache read, persistence, stage reconcile).
    Internal(anyhow::Error),
}

impl From<anyhow::Error> for PasteSlidesError {
    fn from(err: anyhow::Error) -> Self {
        Self::Internal(err)
    }
}

impl std::fmt::Display for PasteSlidesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownSlides => write!(f, "one or more slides no longer exist"),
            Self::Internal(err) => write!(f, "{err}"),
        }
    }
}
```

```rust
    /// Paste-of-COPY (#554): clone the named slides' full content and insert the
    /// clones as a contiguous block at `position` (clamped `0..=len`). A
    /// multi-slide generalization of `duplicate_slide`: same persist → reconcile
    /// stage → cache → broadcast → publish → nudge_sync pipeline, so it bumps
    /// `updated_at` (inside `replace_presentation_slides`) and propagates via
    /// #555 sync exactly like every other slide mutation. Unknown ids →
    /// `UnknownSlides` (router maps to 422).
    pub async fn paste_slides(
        &self,
        presentation_id: PresentationId,
        source_ids: Vec<SlideId>,
        position: u32,
    ) -> Result<Vec<Slide>, PasteSlidesError> {
        let presentation_arc = self.presentation_from_cache(presentation_id).await?;
        let presentation = presentation_arc.as_ref();

        if source_ids.is_empty() {
            return Err(PasteSlidesError::UnknownSlides);
        }
        let requested: std::collections::HashSet<SlideId> = source_ids.iter().copied().collect();
        let present: std::collections::HashSet<SlideId> =
            presentation.slides.iter().map(|slide| slide.id).collect();
        if !requested.iter().all(|id| present.contains(id)) {
            return Err(PasteSlidesError::UnknownSlides);
        }

        // Clone the selected slides in the presentation's OWN list order so the
        // pasted block is contiguous and ordered like the source. Each clone
        // gets a fresh id via `Slide::new`; remember (source, clone) to copy the
        // #515 stage-layout marker afterwards.
        let mut block: Vec<Slide> = Vec::new();
        let mut marker_pairs: Vec<(SlideId, SlideId)> = Vec::new();
        for slide in presentation
            .slides
            .iter()
            .filter(|slide| requested.contains(&slide.id))
        {
            let clone = Slide::new(0, slide.content.clone());
            marker_pairs.push((slide.id, clone.id));
            block.push(clone);
        }

        let mut slides = presentation.slides.clone();
        let insert_at = (position as usize).min(slides.len());
        let tail = slides.split_off(insert_at);
        slides.extend(block);
        slides.extend(tail);
        Self::reindex_slides(&mut slides);

        self.repository
            .replace_presentation_slides(presentation_id, &slides)
            .await?;

        // #515: copy each source's stage-layout marker to its clone. Non-fatal —
        // the paste already committed (mirrors `duplicate_slide`).
        for (source_id, clone_id) in marker_pairs {
            match self.repository.get_slide_stage_layout(source_id).await {
                Ok(Some(code)) => {
                    if let Err(err) = self
                        .repository
                        .set_slide_stage_layout(presentation_id, clone_id, &code)
                        .await
                    {
                        tracing::warn!(?err, %source_id, "failed to copy stage-layout marker to pasted slide");
                    }
                }
                Ok(None) => {}
                Err(err) => {
                    tracing::warn!(?err, %source_id, "failed to read stage-layout marker while pasting slide");
                }
            }
        }

        self.reconcile_stage_state_after_edit(presentation_id, &slides)
            .await?;
        let mut updated_presentation = presentation.clone();
        updated_presentation.slides = slides.clone();
        self.cache_presentation_value(updated_presentation).await;
        self.broadcast_stage_snapshots().await?;
        self.live_hub.publish(LiveEvent::BibleSlidesChanged {
            presentation_id: presentation_id.to_string(),
        });
        self.nudge_sync();
        Ok(slides)
    }
```

Tests (append inside the existing `mod tests`; reuse the `presentation_with_slides` helper already there):

```rust
    #[tokio::test]
    async fn paste_clones_selected_slides_as_a_contiguous_block_at_position() {
        let state = AppState::in_memory().await.unwrap();
        let presentation = presentation_with_slides(&state, &["A", "B", "C"]).await;
        let ids: Vec<SlideId> = presentation.slides.iter().map(|s| s.id).collect();

        // Copy A and C, paste at gap 1 (after A). New block = clones of A,C in
        // list order → [A, A', C', B, C].
        let result = state
            .paste_slides(presentation.id, vec![ids[0], ids[2]], 1)
            .await
            .unwrap();

        let mains: Vec<String> = result
            .iter()
            .map(|s| s.content.main.value().to_string())
            .collect();
        assert_eq!(mains, vec!["A", "A", "C", "B", "C"]);
        assert_eq!(
            result.iter().map(|s| s.order).collect::<Vec<_>>(),
            vec![0, 1, 2, 3, 4],
            "paste must reindex order"
        );
        // Clones carry FRESH ids (not the sources').
        assert!(!result[1..3].iter().any(|s| s.id == ids[0] || s.id == ids[2]));
    }

    #[tokio::test]
    async fn paste_clones_full_content_including_group() {
        let state = AppState::in_memory().await.unwrap();
        let library = state.create_library("L").await.unwrap();
        let src = Slide::new(
            0,
            SlideContent::new(
                SlideText::new("main").unwrap(),
                SlideText::new("trans").unwrap(),
                SlideText::new("stage").unwrap(),
                Some(presenter_core::SlideGroup::new("Chorus".to_string())),
            ),
        );
        let (_, _, presentation, _) = state
            .create_presentation(library.id, "P", Some(&[src]))
            .await
            .unwrap();
        let src_id = presentation.slides[0].id;

        let result = state.paste_slides(presentation.id, vec![src_id], 1).await.unwrap();
        let clone = &result[1];
        assert_eq!(clone.content.main.value(), "main");
        assert_eq!(clone.content.translation.value(), "trans");
        assert_eq!(clone.content.stage.value(), "stage");
        assert_eq!(
            clone.content.group.as_ref().map(|g| g.name().to_string()),
            Some("Chorus".to_string()),
            "group must be cloned intact"
        );
    }

    #[tokio::test]
    async fn paste_clamps_position_past_the_end() {
        let state = AppState::in_memory().await.unwrap();
        let presentation = presentation_with_slides(&state, &["A", "B"]).await;
        let ids: Vec<SlideId> = presentation.slides.iter().map(|s| s.id).collect();
        // position 99 clamps to len (append).
        let result = state.paste_slides(presentation.id, vec![ids[0]], 99).await.unwrap();
        let mains: Vec<String> = result.iter().map(|s| s.content.main.value().to_string()).collect();
        assert_eq!(mains, vec!["A", "B", "A"]);
    }

    #[tokio::test]
    async fn paste_rejects_an_unknown_slide_id_with_unknownslides() {
        let state = AppState::in_memory().await.unwrap();
        let presentation = presentation_with_slides(&state, &["A"]).await;
        let ids: Vec<SlideId> = presentation.slides.iter().map(|s| s.id).collect();
        let result = state
            .paste_slides(presentation.id, vec![ids[0], SlideId::new()], 0)
            .await;
        assert!(matches!(result, Err(PasteSlidesError::UnknownSlides)));
        // The presentation must be UNCHANGED (no half-paste).
        let reloaded = state.presentation_detail(presentation.id).await.unwrap();
        let (_, _, reloaded) = reloaded.expect("still exists");
        assert_eq!(reloaded.slides.len(), 1);
    }

    #[tokio::test]
    async fn paste_persists_and_is_reloadable() {
        let state = AppState::in_memory().await.unwrap();
        let presentation = presentation_with_slides(&state, &["A", "B"]).await;
        let ids: Vec<SlideId> = presentation.slides.iter().map(|s| s.id).collect();
        state.paste_slides(presentation.id, vec![ids[0]], 0).await.unwrap();
        let reloaded = state.presentation_detail(presentation.id).await.unwrap();
        let (_, _, reloaded) = reloaded.expect("still exists");
        let mains: Vec<String> =
            reloaded.slides.iter().map(|s| s.content.main.value().to_string()).collect();
        assert_eq!(mains, vec!["A", "A", "B"], "paste must persist, not just return");
    }
```

- [ ] RED then GREEN: `cargo test -p presenter-server paste_` (from repo root).
  Expected: RED before the method exists; then `test result: ok` with all 5 paste tests passing.
- [ ] Confirm the fn is under the 120-line cap: `QC_TARGETS=crates/presenter-server/src/state/slides/edit_ops.rs python3 scripts/dev/fn_length_check.py .` → no fail (it is ~55 lines).

**Commit:** `feat(slides): AppState::paste_slides — clone a block at a position, 422 on stale ids (#554)`

---

## Task 4 — Router: paste route, handler, 422 mapping [TDD: RED first]

**Modify:** `crates/presenter-server/src/router.rs` (route + `AppError::unprocessable`)
**Modify:** `crates/presenter-server/src/router/presentations.rs` (request DTO + handler)
**Create:** `crates/presenter-server/src/router/presentations_paste_tests.rs` (router-level 200 + 422 tests)

Steps:

- [ ] Add a 422 constructor to `AppError` in `router.rs` (next to `bad_request_message`/`not_found`, ~line 515):
  ```rust
      fn unprocessable(message: impl Into<String>) -> Self {
          Self::new(
              StatusCode::UNPROCESSABLE_ENTITY,
              anyhow::anyhow!(message.into()),
          )
      }
  ```
- [ ] Register the route in `router.rs` right after the reorder route (~line 326):
  ```rust
          .route(
              "/presentations/{presentation_id}/slides/paste",
              post(presentations::paste_slides),
          )
  ```
  (`/slides/paste` is a static sibling of `/slides/reorder`; matchit prioritizes it over the dynamic `/slides/{slide_id}`, same as reorder does today.)
- [ ] In `router/presentations.rs`, add the request DTO (near the other request structs, ~line 37) and the handler (after `reorder_slides`, ~line 165):
  ```rust
  #[derive(Debug, serde::Deserialize)]
  #[serde(rename_all = "camelCase")]
  pub(super) struct PasteSlidesRequest {
      pub(super) slide_ids: Vec<uuid::Uuid>,
      pub(super) position: u32,
  }
  ```
  ```rust
  #[instrument(skip_all)]
  pub(super) async fn paste_slides(
      State(state): State<AppState>,
      Path(presentation_id): Path<String>,
      Json(payload): Json<PasteSlidesRequest>,
  ) -> Result<Json<Vec<Slide>>, AppError> {
      let presentation_uuid = super::parse_uuid("presentationId", &presentation_id)?;
      let source_ids = payload
          .slide_ids
          .into_iter()
          .map(SlideId::from_uuid)
          .collect();
      match state
          .paste_slides(
              PresentationId::from_uuid(presentation_uuid),
              source_ids,
              payload.position,
          )
          .await
      {
          Ok(slides) => Ok(Json(slides)),
          Err(crate::state::slides::PasteSlidesError::UnknownSlides) => {
              Err(AppError::unprocessable("one or more slides no longer exist"))
          }
          Err(crate::state::slides::PasteSlidesError::Internal(err)) => Err(err.into()),
      }
  }
  ```
- [ ] Register the test module in `router.rs` next to the other `#[cfg(test)] mod ...` decls (~line 577):
  ```rust
  #[cfg(test)]
  mod presentations_paste_tests;
  ```
- [ ] Create `router/presentations_paste_tests.rs` (own file so the huge `router/tests.rs` stays out of the diff — a `quality-check.sh` landmine). Write RED first. Full file:

```rust
//! Router-level tests for `POST /presentations/{id}/slides/paste` (#554):
//! a valid paste returns 200 + the new slide list; a stale-id paste returns 422.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use presenter_core::Slide;
use tower::ServiceExt;

use crate::router::build_router;
use crate::state::AppState;

async fn post_json(app: &axum::Router, uri: &str, body: serde_json::Value) -> axum::http::Response<Body> {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
}

/// Create a library + a 2-slide presentation via the state layer; return its id
/// and the slide ids.
async fn seed(state: &AppState) -> (String, Vec<String>) {
    let library = state.create_library("Paste Lib").await.unwrap();
    let slides = [
        presenter_core::Slide::new(
            0,
            presenter_core::SlideContent::new(
                presenter_core::SlideText::new("A").unwrap(),
                presenter_core::SlideText::new("").unwrap(),
                presenter_core::SlideText::new("").unwrap(),
                None,
            ),
        ),
        presenter_core::Slide::new(
            1,
            presenter_core::SlideContent::new(
                presenter_core::SlideText::new("B").unwrap(),
                presenter_core::SlideText::new("").unwrap(),
                presenter_core::SlideText::new("").unwrap(),
                None,
            ),
        ),
    ];
    let (_, _, presentation, _) = state
        .create_presentation(library.id, "P", Some(&slides))
        .await
        .unwrap();
    (
        presentation.id.to_string(),
        presentation.slides.iter().map(|s| s.id.to_string()).collect(),
    )
}

#[tokio::test]
async fn paste_valid_ids_returns_200_and_the_new_list() {
    let state = AppState::in_memory().await.unwrap();
    let (pres_id, slide_ids) = seed(&state).await;
    let app = build_router(state);

    let resp = post_json(
        &app,
        &format!("/presentations/{pres_id}/slides/paste"),
        serde_json::json!({ "slideIds": [slide_ids[0]], "position": 2 }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let slides: Vec<Slide> = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(slides.len(), 3, "one slide pasted → 3 total");
}

#[tokio::test]
async fn paste_unknown_id_returns_422() {
    let state = AppState::in_memory().await.unwrap();
    let (pres_id, _slide_ids) = seed(&state).await;
    let app = build_router(state);

    let bogus = uuid::Uuid::new_v4().to_string();
    let resp = post_json(
        &app,
        &format!("/presentations/{pres_id}/slides/paste"),
        serde_json::json!({ "slideIds": [bogus], "position": 0 }),
    )
    .await;
    assert_eq!(
        resp.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "a stale clipboard id must be 422, not 500"
    );
}
```

- [ ] RED then GREEN: `cargo test -p presenter-server presentations_paste_tests`
  Expected: RED before route/handler exist; then `test result: ok. 2 passed`.

**Commit:** `feat(slides): POST /slides/paste route + 422 on stale clipboard (#554)`

---

## Task 5 — Client API `paste_slides`

**Modify:** `crates/presenter-ui/src/api/presentations.rs`

Steps:

- [ ] Add the request struct + fn (after `reorder_slides`, ~line 192):
  ```rust
  #[derive(Serialize)]
  #[serde(rename_all = "camelCase")]
  struct PasteSlidesRequest {
      slide_ids: Vec<String>,
      position: u32,
  }

  /// Paste-of-copy: clone `slide_ids` as a contiguous block at `position`
  /// (0..=len). Returns the full new slide list (same contract as reorder).
  /// A `422` (`ApiError::Status(422, _)`) means a stale clipboard — the caller
  /// clears the clipboard and toasts.
  pub async fn paste_slides(
      pres_id: &str,
      slide_ids: Vec<String>,
      position: u32,
  ) -> Result<Vec<Slide>, ApiError> {
      post_json(
          &format!("/presentations/{pres_id}/slides/paste"),
          &PasteSlidesRequest { slide_ids, position },
      )
      .await
  }
  ```
- [ ] Verify it compiles (cheap wasm check): `cd crates/presenter-ui && cargo check --target wasm32-unknown-unknown`.
  Expected: `Finished` with no errors.

**Commit:** `feat(ui): api::presentations::paste_slides client (#554)`

---

## Task 6 — Client state: clipboard + selection signals

**Modify:** `crates/presenter-ui/src/state/operator.rs`

Steps:

- [ ] Add `use std::collections::HashSet;` (the file imports only `HashMap` today).
- [ ] Add the clipboard types above `OperatorState` (after `SaveStatus`):
  ```rust
  /// Clipboard mode for slide copy/cut (#554): `Copy` pastes clones, `Cut` MOVES
  /// the originals (only when a paste actually succeeds).
  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub enum ClipboardMode {
      Copy,
      Cut,
  }

  /// A slide clipboard captured within ONE presentation (#554). `presentation_id`
  /// guards against a stale clipboard after switching songs and is the seam for a
  /// future cross-presentation clipboard.
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct SlideClipboard {
      /// Ids of the captured slides, in list order at capture time.
      pub slide_ids: Vec<String>,
      pub mode: ClipboardMode,
      pub presentation_id: String,
  }
  ```
- [ ] Add these fields to `OperatorState` (after `groups_version`):
  ```rust
      /// Multi-selected slide ids in the operator editor (#554). Same shape as
      /// the Bible page's `selected_slide_ids`.
      pub selected_slide_ids: RwSignal<HashSet<String>>,
      /// The copy/cut clipboard; `None` when empty. Cleared on presentation switch.
      pub clipboard: RwSignal<Option<SlideClipboard>>,
      /// Index of the last checkbox-clicked slide — the anchor for Shift+click
      /// range selection (#554).
      pub selection_anchor_index: RwSignal<Option<usize>>,
      /// Gap index (0..=len) the user last hovered/selected as the Ctrl/Cmd+V
      /// paste target; `None` → paste at the end (#554).
      pub paste_target_gap: RwSignal<Option<usize>>,
      /// True while the clipboard block is being dragged toward an insertion gap.
      pub dragging_clipboard: RwSignal<bool>,
  ```
- [ ] Initialize them in `new()` (after `groups_version: RwSignal::new(0),`):
  ```rust
              selected_slide_ids: RwSignal::new(HashSet::new()),
              clipboard: RwSignal::new(None),
              selection_anchor_index: RwSignal::new(None),
              paste_target_gap: RwSignal::new(None),
              dragging_clipboard: RwSignal::new(false),
  ```
- [ ] Verify: `cd crates/presenter-ui && cargo check --target wasm32-unknown-unknown`.

**Commit:** `feat(ui): SlideClipboard + selection signals on OperatorState (#554)`

---

## Task 7 — Selection UI: panel, checkboxes, insertion bars, cut marking, keyboard, CSS

**Create:** `crates/presenter-ui/src/components/slide_selection.rs`
**Modify:** `crates/presenter-ui/src/components/mod.rs` (register `mod slide_selection;`)
**Modify:** `crates/presenter-ui/src/components/slide_list.rs` (mount panel; per-card checkbox + Memos; insertion bars; container class; setup calls)
**Modify:** `crates/presenter-ui/styles/operator.css` (panel, checkbox, `--selected`, `--cut`, insertion bar, single-column)

> **File-cap guard (`slide_list.rs` is 840 prod lines; hard cap 1000):** keep ALL non-trivial code in `slide_selection.rs`. `slide_list.rs` gains only: 2 helper calls per card (the checkbox view + the two Memos), 2 lines in the class closure, the insertion-bar interleave (~8 lines), the container class closure (~2), the panel mount (1), and two setup calls (2). Budget ≈ 30-40 lines → ~880. If your edits push it past ~950 prod lines (`bash scripts/dev/count_prod_lines.sh crates/presenter-ui/src/components/slide_list.rs`), extract the whole `<article>` card render into a `slide_selection::render_slide_card(...)` helper before finishing.

### 7a — `slide_selection.rs` (component + wiring)

- [ ] Add `mod slide_selection;` to `components/mod.rs` (near `mod slide_reorder;`). Keep private.
- [ ] Create the file. It owns: the `SlideSelectionPanel` component, the per-card `render_select_checkbox` + `selected_memo` + `cut_memo` helpers, the `render_insertion_bar` helper, `text_entry_focused`, the window keyboard setup, the clear-on-switch effect, and the copy/cut/paste/clear action fns. Full file:

```rust
//! Slide multi-select clipboard UI (#554): the selection panel, per-card
//! checkbox + selected/cut markers, the "paste here" insertion bars, the
//! window keyboard shortcuts, and the copy/cut/paste/clear actions. The pure
//! order/shortcut logic lives in `slide_selection_logic.rs`; this file is the
//! DOM/Leptos wiring. Reused from `slide_list.rs`.

use leptos::prelude::*;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

use crate::api;
use crate::api::ApiError;
use crate::state::operator::{ClipboardMode, OperatorState, SlideClipboard};
use crate::state::AppContext;

use super::slide_save::reconcile_after_seq_mismatch;
use super::slide_selection_logic::{cut_splice_order, range_select, shortcut_action, ShortcutAction};

/// Ordered slide ids of the currently open presentation (untracked snapshot).
fn current_slide_ids(ctx: &AppContext) -> Vec<String> {
    ctx.selected_presentation
        .get_untracked()
        .map(|p| p.slides.iter().map(|s| s.id.to_string()).collect())
        .unwrap_or_default()
}

/// The open presentation's id, or `None` when nothing is loaded.
fn current_pres_id(ctx: &AppContext) -> Option<String> {
    ctx.selected_presentation
        .get_untracked()
        .map(|p| p.id.to_string())
}

/// True while a TEXT-ENTRY field (a textarea, or a non-checkbox input) holds
/// focus — the guard that keeps Ctrl+C/X/V from firing while the user is typing
/// slide content. A focused selection CHECKBOX must NOT suppress the shortcut
/// (you select with the checkbox, then Ctrl+C), so it is excluded here. This is
/// the "never fire from inside text fields" intent of the spec; `is_interactive_tag`
/// stays the guard for the per-card CLICK, where its breadth (incl. checkbox) is
/// correct.
fn text_entry_focused() -> bool {
    let doc = crate::utils::window::document();
    let Some(active) = doc.active_element() else {
        return false;
    };
    let tag = active.tag_name().to_lowercase();
    if tag == "textarea" {
        return true;
    }
    if tag == "input" {
        return active.get_attribute("type").as_deref() != Some("checkbox");
    }
    false
}

/// Per-card `Memo`: is this slide currently selected? Copy + `PartialEq`, so the
/// checkbox `checked` and the `--selected` class update WITHOUT a list rebuild.
pub(super) fn selected_memo(op: &OperatorState, slide_id: String) -> Memo<bool> {
    let selected = op.selected_slide_ids;
    Memo::new(move |_| selected.with(|s| s.contains(&slide_id)))
}

/// Per-card `Memo`: is this slide part of a `Cut` clipboard (→ dim + badge)?
pub(super) fn cut_memo(op: &OperatorState, slide_id: String) -> Memo<bool> {
    let clipboard = op.clipboard;
    Memo::new(move |_| {
        clipboard.with(|c| {
            c.as_ref()
                .is_some_and(|cb| cb.mode == ClipboardMode::Cut && cb.slide_ids.contains(&slide_id))
        })
    })
}

/// The per-card select checkbox (edit mode only). Controlled: `prop:checked`
/// reads `selected_memo`; the click `prevent_default`s the native toggle and
/// drives the selection set. Plain click toggles + sets the anchor; Shift+click
/// selects the inclusive range from the anchor.
pub(super) fn render_select_checkbox(
    ctx: AppContext,
    op: OperatorState,
    slide_id: String,
    index: usize,
) -> impl IntoView {
    let checked = selected_memo(&op, slide_id.clone());
    view! {
        <input
            type="checkbox"
            class="operator__slide-select"
            data-role="slide-select-checkbox"
            data-slide-select-index=index
            prop:checked=move || checked.get()
            on:click=move |ev: web_sys::MouseEvent| {
                ev.prevent_default();
                let ids = current_slide_ids(&ctx);
                if ev.shift_key() {
                    if let Some(anchor) = op.selection_anchor_index.get_untracked() {
                        let next = range_select(
                            &ids,
                            anchor,
                            index,
                            &op.selected_slide_ids.get_untracked(),
                        );
                        op.selected_slide_ids.set(next);
                        return;
                    }
                }
                // Plain click (or Shift with no anchor yet): toggle this id.
                let sid = slide_id.clone();
                op.selected_slide_ids.update(|set| {
                    if !set.remove(&sid) {
                        set.insert(sid);
                    }
                });
                op.selection_anchor_index.set(Some(index));
            }
        />
    }
}

/// A "paste here" insertion bar for `gap` (0..=len). Rendered by `slide_list.rs`
/// only when the clipboard is non-empty. Click OR block-drop → paste at `gap`;
/// hover → remember `gap` for Ctrl/Cmd+V.
pub(super) fn render_insertion_bar(ctx: AppContext, op: OperatorState, gap: usize) -> impl IntoView {
    let ctx_click = ctx.clone();
    let ctx_drop = ctx.clone();
    view! {
        <div
            class="operator__slide-insert-bar"
            data-role="slide-insert-bar"
            data-insert-index=gap
            on:mouseenter=move |_| op.paste_target_gap.set(Some(gap))
            on:dragover=move |ev: web_sys::DragEvent| {
                if op.dragging_clipboard.get_untracked() {
                    ev.prevent_default();
                }
            }
            on:drop=move |ev: web_sys::DragEvent| {
                if op.dragging_clipboard.get_untracked() {
                    ev.prevent_default();
                    paste_at_gap(&ctx_drop, &op, gap);
                }
            }
            on:click=move |_| paste_at_gap(&ctx_click, &op, gap)
        >
            <span class="operator__slide-insert-hint">"Paste here"</span>
        </div>
    }
}

/// Copy the current selection into the clipboard (`Copy` mode). No-op if empty.
pub(super) fn copy_selection(ctx: &AppContext, op: &OperatorState) {
    set_clipboard(ctx, op, ClipboardMode::Copy);
}

/// Cut the current selection into the clipboard (`Cut` mode). Non-destructive:
/// the originals stay until a paste succeeds. No-op if empty.
pub(super) fn cut_selection(ctx: &AppContext, op: &OperatorState) {
    set_clipboard(ctx, op, ClipboardMode::Cut);
}

fn set_clipboard(ctx: &AppContext, op: &OperatorState, mode: ClipboardMode) {
    let Some(pres_id) = current_pres_id(ctx) else {
        return;
    };
    let selected = op.selected_slide_ids.get_untracked();
    if selected.is_empty() {
        return;
    }
    // Capture in list order.
    let slide_ids: Vec<String> = current_slide_ids(ctx)
        .into_iter()
        .filter(|id| selected.contains(id))
        .collect();
    if slide_ids.is_empty() {
        return;
    }
    op.clipboard.set(Some(SlideClipboard {
        slide_ids,
        mode,
        presentation_id: pres_id,
    }));
}

/// Clear the clipboard, selection, anchor, and paste target (Escape / Clear /
/// presentation switch). Nothing is deleted — a cut is simply abandoned.
pub(super) fn clear_selection_and_clipboard(op: &OperatorState) {
    op.clipboard.set(None);
    op.selected_slide_ids.set(std::collections::HashSet::new());
    op.selection_anchor_index.set(None);
    op.paste_target_gap.set(None);
}

/// Paste the clipboard at `gap` (0..=len). Copy → the paste endpoint; Cut → the
/// existing reorder endpoint via `cut_splice_order`.
pub(super) fn paste_at_gap(ctx: &AppContext, op: &OperatorState, gap: usize) {
    let Some(clipboard) = op.clipboard.get_untracked() else {
        return;
    };
    let Some(pres_id) = current_pres_id(ctx) else {
        return;
    };
    match clipboard.mode {
        ClipboardMode::Copy => paste_copy(ctx, op, pres_id, clipboard.slide_ids, gap),
        ClipboardMode::Cut => paste_cut(ctx, op, pres_id, clipboard.slide_ids, gap),
    }
}

/// Apply a server slide list to `selected_presentation` under the `slide_edit_seq`
/// staleness guard (#552/#556) — the same guard the reorder/insert/duplicate
/// paths use. `my_seq` is captured by the caller BEFORE the async request.
async fn apply_slides_guarded(
    ctx: &AppContext,
    op: &OperatorState,
    pres_id: String,
    slides: Vec<presenter_core::Slide>,
    my_seq: u64,
) {
    if op.slide_edit_seq.get_untracked() == my_seq {
        ctx.selected_presentation.update(|p| {
            if let Some(pres) = p.as_mut() {
                pres.slides = slides;
            }
        });
    } else {
        reconcile_after_seq_mismatch(
            pres_id,
            slides,
            ctx.selected_presentation,
            op.slide_edit_seq,
            op.save_status,
        )
        .await;
    }
}

fn paste_copy(ctx: &AppContext, op: &OperatorState, pres_id: String, ids: Vec<String>, gap: usize) {
    op.slide_edit_seq.update(|s| *s += 1);
    let my_seq = op.slide_edit_seq.get_untracked();
    let ctx = ctx.clone();
    let op = op.clone();
    leptos::task::spawn_local(async move {
        match api::presentations::paste_slides(&pres_id, ids, gap as u32).await {
            Ok(slides) => {
                apply_slides_guarded(&ctx, &op, pres_id, slides, my_seq).await;
                // Copy clipboard is KEPT so the user can paste again.
            }
            Err(ApiError::Status(422, _)) => {
                op.clipboard.set(None);
                ctx.show_toast("Those slides no longer exist", "error");
            }
            Err(_) => {
                // Clipboard KEPT so the user can retry.
                ctx.show_toast("Paste failed — try again", "error");
            }
        }
    });
}

fn paste_cut(ctx: &AppContext, op: &OperatorState, pres_id: String, ids: Vec<String>, gap: usize) {
    let current = current_slide_ids(ctx);
    let Some(final_order) = cut_splice_order(&current, &ids, gap) else {
        // A stale cut (an id vanished): drop the clipboard + marks.
        clear_selection_and_clipboard(op);
        ctx.show_toast("Those slides no longer exist", "error");
        return;
    };
    op.slide_edit_seq.update(|s| *s += 1);
    let my_seq = op.slide_edit_seq.get_untracked();
    let ctx = ctx.clone();
    let op = op.clone();
    leptos::task::spawn_local(async move {
        match api::presentations::reorder_slides(&pres_id, final_order).await {
            Ok(slides) => {
                apply_slides_guarded(&ctx, &op, pres_id, slides, my_seq).await;
                // The cut is consumed — clear clipboard + selection.
                clear_selection_and_clipboard(&op);
            }
            Err(_) => {
                // Originals stay in place, still marked; retry or Escape.
                ctx.show_toast("Move failed — try again", "error");
            }
        }
    });
}

/// Clear selection + clipboard whenever the open presentation changes (spec
/// scope 4). Called once from `SlideList`.
pub(super) fn setup_selection_clear_on_switch(ctx: AppContext, op: OperatorState) {
    let selected_presentation_id = ctx.selected_presentation_id;
    Effect::new(move |prev: Option<Option<String>>| {
        let current = selected_presentation_id.get();
        if let Some(prev) = prev {
            if prev != current {
                clear_selection_and_clipboard(&op);
            }
        }
        current
    });
}

/// Install the ONE window keydown listener for the clipboard shortcuts (mirrors
/// the operator page's existing global handler). Acts only in edit mode with a
/// presentation open, and never while a text field is focused. Called once from
/// `SlideList`.
pub(super) fn setup_clipboard_keyboard(ctx: AppContext, op: OperatorState) {
    let handler = Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(move |ev: web_sys::KeyboardEvent| {
        // Only in edit mode with a presentation open.
        if ctx.mode.get_untracked() == "live" || ctx.selected_presentation.get_untracked().is_none() {
            return;
        }
        let ctrl_or_meta = ev.ctrl_key() || ev.meta_key();
        let Some(action) = shortcut_action(&ev.key(), ctrl_or_meta) else {
            return;
        };
        // Never fire copy/cut/paste from inside a text field. Escape is allowed
        // to clear even from a field (matches the operator page's Escape).
        if !matches!(action, ShortcutAction::Clear) && text_entry_focused() {
            return;
        }
        match action {
            ShortcutAction::Copy => {
                if !op.selected_slide_ids.get_untracked().is_empty() {
                    ev.prevent_default();
                    copy_selection(&ctx, &op);
                }
            }
            ShortcutAction::Cut => {
                if !op.selected_slide_ids.get_untracked().is_empty() {
                    ev.prevent_default();
                    cut_selection(&ctx, &op);
                }
            }
            ShortcutAction::Paste => {
                if op.clipboard.get_untracked().is_some() {
                    ev.prevent_default();
                    let len = current_slide_ids(&ctx).len();
                    let gap = op.paste_target_gap.get_untracked().unwrap_or(len).min(len);
                    paste_at_gap(&ctx, &op, gap);
                }
            }
            ShortcutAction::Clear => {
                if op.clipboard.get_untracked().is_some()
                    || !op.selected_slide_ids.get_untracked().is_empty()
                {
                    clear_selection_and_clipboard(&op);
                }
            }
        }
    });
    let window = crate::utils::window::window();
    let _ = window.add_event_listener_with_callback("keydown", handler.as_ref().unchecked_ref());
    // Forget: the operator page has no client-side router, so this window
    // listener lives for the page's lifetime (same pattern as
    // `operator.rs`'s global keydown handler and the settings pollers).
    handler.forget();
}

/// The sticky selection panel above the slide list (edit mode). Shows the count
/// and Copy / Cut / Paste / Clear, plus a draggable block handle when the
/// clipboard is non-empty. Visible only when something is selected OR the
/// clipboard is non-empty.
#[component]
pub fn SlideSelectionPanel() -> impl IntoView {
    let ctx = use_ctx!(AppContext);
    let op = use_ctx!(OperatorState);

    let is_edit = move || ctx.mode.get() != "live";
    let count = move || op.selected_slide_ids.get().len();
    let has_clipboard = move || op.clipboard.get().is_some();
    let visible = move || is_edit() && (count() > 0 || has_clipboard());

    view! {
        <Show when=visible fallback=|| ()>
            {move || {
                let ctx_copy = ctx.clone();
                let ctx_cut = ctx.clone();
                let ctx_paste = ctx.clone();
                let op_copy = op.clone();
                let op_cut = op.clone();
                let op_paste = op.clone();
                let op_clear = op.clone();
                let op_drag = op.clone();
                let op_dragend = op.clone();
                view! {
                    <div class="operator__slide-selection" data-role="slide-selection-panel">
                        <span
                            class="operator__slide-selection-count"
                            data-role="slide-selection-count"
                        >
                            {move || format!("{} selected", op.selected_slide_ids.get().len())}
                        </span>
                        <button
                            type="button"
                            class="operator__list-action"
                            data-action="copy"
                            data-role="slide-copy"
                            on:click=move |_| copy_selection(&ctx_copy, &op_copy)
                        >"Copy"</button>
                        <button
                            type="button"
                            class="operator__list-action"
                            data-action="cut"
                            data-role="slide-cut"
                            on:click=move |_| cut_selection(&ctx_cut, &op_cut)
                        >"Cut"</button>
                        <button
                            type="button"
                            class="operator__list-action"
                            data-action="paste"
                            data-role="slide-paste"
                            prop:disabled=move || op_paste.clipboard.get().is_none()
                            on:click=move |_| {
                                let len = current_slide_ids(&ctx_paste).len();
                                let gap = op_paste
                                    .paste_target_gap
                                    .get_untracked()
                                    .unwrap_or(len)
                                    .min(len);
                                paste_at_gap(&ctx_paste, &op_paste, gap);
                            }
                        >"Paste"</button>
                        <button
                            type="button"
                            class="operator__list-action"
                            data-action="clear"
                            data-role="slide-clear"
                            on:click=move |_| clear_selection_and_clipboard(&op_clear)
                        >"Clear"</button>
                        <Show when=move || op_drag.clipboard.get().is_some() fallback=|| ()>
                            {
                                let op_drag = op_drag.clone();
                                let op_dragend = op_dragend.clone();
                                view! {
                                    <span
                                        class="operator__slide-selection-drag"
                                        data-role="clipboard-drag"
                                        draggable="true"
                                        title="Drag the block onto a gap"
                                        on:dragstart=move |ev: web_sys::DragEvent| {
                                            if let Some(dt) = ev.data_transfer() {
                                                let _ = dt.set_data("application/x-slide-clipboard", "1");
                                                dt.set_effect_allowed("move");
                                            }
                                            op_drag.dragging_clipboard.set(true);
                                        }
                                        on:dragend=move |_| op_dragend.dragging_clipboard.set(false)
                                    >"\u{2195} Drag block"</span>
                                }
                            }
                        </Show>
                    </div>
                }
            }}
        </Show>
    }
}
```

> Note: `is_some_and` is stable Rust; if the pinned toolchain rejects it, use `.map_or(false, |cb| ...)` (match the crate's style — `slide_save.rs` uses `.map(...).unwrap_or(...)`). Confirm at implementation time.

### 7b — `slide_list.rs` edits (minimal)

- [ ] Add imports at the top:
  ```rust
  use super::slide_selection::{
      render_insertion_bar, render_select_checkbox, cut_memo, selected_memo,
      setup_clipboard_keyboard, setup_selection_clear_on_switch, SlideSelectionPanel,
  };
  use crate::state::operator::ClipboardMode; // if referenced; else omit
  ```
- [ ] In `SlideList()` setup (near the other effects, after the scroll-to-top effect ~line 74), call the two setups once:
  ```rust
  setup_selection_clear_on_switch(ctx.clone(), op.clone());
  setup_clipboard_keyboard(ctx.clone(), op.clone());
  ```
- [ ] Mount the panel inside `.operator__slides-area`, ABOVE the `.operator__slides` div (just after the floating add-slide `<Show>` block, ~line 149):
  ```rust
                  <SlideSelectionPanel />
  ```
- [ ] Make the `.operator__slides` container class reactive so it switches to a single column when the clipboard is non-empty. Replace `class="operator__slides"` (~line 157) with a closure (clone an `op` for it):
  ```rust
                          class=move || {
                              if op_cls.clipboard.get().is_some() {
                                  "operator__slides operator__slides--clipboard"
                              } else {
                                  "operator__slides"
                              }
                          }
  ```
  (Add `let op_cls = op.clone();` alongside the existing `op_dragover`/`op_drop`/`op_bubble` clones.)
- [ ] Per card, add the two Memos + checkbox. Where the per-card clones are set up (~line 325, near `slide_id_class`):
  ```rust
                          let selected_marker = selected_memo(&op, slide_id.clone());
                          let cut_marker = cut_memo(&op, slide_id.clone());
  ```
- [ ] In the card `class=move || { ... }` closure (~line 336), after the `is-loading` block, add the selection/cut classes:
  ```rust
                                  if selected_marker.get() { c.push_str(" operator__slide-card--selected"); }
                                  if cut_marker.get() { c.push_str(" operator__slide-card--cut"); }
  ```
- [ ] Render the checkbox in the header's left group (edit mode only), just before the drag handle inside `.operator__slide-header-left` (~line 420):
  ```rust
                                  {is_edit.then(|| render_select_checkbox(ctx.clone(), op.clone(), slide_id.clone(), i))}
  ```
- [ ] Interleave insertion bars. The list currently ends `...).collect_view().into_any()`. Change the tail so that, when the clipboard is non-empty, a bar is emitted BEFORE each card and one AFTER the last. Capture `let clipboard_active = op.clipboard.get().is_some();` at the top of the render closure (this — and ONLY this — makes the list re-render on a clipboard change; documented tradeoff). Then wrap each card's returned view with a leading bar and append a trailing bar. Concretely, change the per-item closure to return a fragment and collect into a Vec, then push the trailing bar:
  ```rust
                      let clipboard_active = op.clipboard.get().is_some();
                      let mut rendered: Vec<AnyView> = raw_slides
                          .iter()
                          .cloned()
                          .zip(resolved.into_iter())
                          .enumerate()
                          .map(|(i, (raw_slide, resolved_slide))| {
                              // ... existing per-card body, ending in `view! { <article ...>...</article> }` ...
                              let card = view! { <article ...>...</article> };
                              view! {
                                  {clipboard_active.then(|| render_insertion_bar(ctx.clone(), op.clone(), i))}
                                  {card}
                              }.into_any()
                          })
                          .collect();
                      if clipboard_active {
                          let len = raw_slides.len();
                          rendered.push(render_insertion_bar(ctx.clone(), op.clone(), len).into_any());
                      }
                      rendered.collect_view().into_any()
  ```
  (Keep the existing per-card body verbatim; only the outer wrapping + trailing bar are new. `AnyView` is `leptos::prelude::AnyView`. If the exact `Vec<AnyView>` collect fights the type checker, fall back to building `Vec<_>` of `view!{...}.into_any()` and `.collect_view()` — the shape above is the intent.)

### 7c — CSS (`styles/operator.css`)

- [ ] Add near the slide-card rules (~line 1160). Match the existing palette/vars:
  ```css
  /* #554 slide multi-select + clipboard */
  .operator__slide-select {
    width: 1.05rem;
    height: 1.05rem;
    cursor: pointer;
    accent-color: var(--operator-accent-dark);
  }
  .operator__slide-card--selected {
    border-color: rgba(59, 124, 255, 0.55);
    box-shadow: 0 0 0 2px rgba(59, 124, 255, 0.22);
  }
  .operator__slide-card--cut {
    opacity: 0.55;
  }
  .operator__slide-card--cut .operator__slide-header-left::after {
    content: "cut";
    margin-left: 0.4rem;
    padding: 0 0.35rem;
    font-size: 0.65rem;
    border-radius: 6px;
    background: rgba(148, 163, 184, 0.25);
    color: var(--operator-muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .operator__slide-selection {
    position: sticky;
    top: 0;
    z-index: 3;
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.4rem 0.6rem;
    margin-bottom: 0.4rem;
    background: var(--operator-panel);
    border: 1px solid var(--operator-border);
    border-radius: 10px;
  }
  .operator__slide-selection-count {
    font-size: 0.8rem;
    color: var(--operator-muted);
    margin-right: auto;
  }
  .operator__slide-selection-drag {
    cursor: grab;
    font-size: 0.75rem;
    padding: 0.3rem 0.5rem;
    border-radius: 8px;
    background: rgba(59, 124, 255, 0.12);
    color: var(--operator-accent-dark);
  }
  /* Clipboard active: single column so every gap is one full-width bar. */
  .operator__slides--clipboard {
    grid-template-columns: 1fr;
  }
  .operator__slide-insert-bar {
    grid-column: 1 / -1;
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 1.5rem;
    border: 1px dashed rgba(59, 124, 255, 0.45);
    border-radius: 8px;
    background: rgba(59, 124, 255, 0.06);
    color: var(--operator-accent-dark);
    font-size: 0.72rem;
    cursor: pointer;
    transition: background 0.15s ease, border-color 0.15s ease;
  }
  .operator__slide-insert-bar:hover {
    background: rgba(59, 124, 255, 0.16);
    border-color: rgba(59, 124, 255, 0.7);
  }
  ```

### 7d — Build + verify locally

- [ ] Cheap check first: `cd crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings`.
  Expected: `Finished`, zero warnings. Fix any `move`-in-`view!` (bind closures to a `let`), any non-`Copy` capture (use `Memo`/pull out signals), and any redundant-closure clippy lint (`data-...=state_str`, not `move || state_str()`), per `.claude/skills/ui/SKILL.md`.
- [ ] Full WASM build (server embeds `dist/`): `bash scripts/build-ui.sh`.
  Expected: builds `dist/` with no error.

**Commit:** `feat(ui): slide multi-select panel, checkboxes, insertion bars + clipboard shortcuts (#554)`

---

## Task 8 — E2E matrix (real interactions, no skips)

**Create:** `tests/e2e/wasm-slide-multiselect.spec.ts`

Follow the drag-drop spec's harness EXACTLY (`tests/e2e/wasm-drag-drop.spec.ts`): `beforeAll` derives config + `refreshDevData` + `startTestServer`; each test builds a fresh presentation via the API so positions are deterministic; use `expect.poll` / `waitForFunction` / `waitForResponse` for state waits (NO `waitForTimeout` as a state gate); dispatch real events. Copy these self-contained helpers into the new spec (they are not exported): `createPresentationWithSlides`, `openPresentationInEditMode`, `domSlideOrder`. Add a `selectByCheckbox(page, slideId, {shift})` helper and a `mainTextsInOrder(page)` reader.

Cover the full spec Testing section — every test is a REAL interaction and NEVER `test.skip()`:

- [ ] **checkbox multi-select** — check 2 slides; assert `[data-role="slide-selection-count"]` reads "2 selected" and both cards have `operator__slide-card--selected`.
- [ ] **Shift+click range** — click slide 0's checkbox, then Shift+click slide 3's; assert slides 0-3 all selected (count "4 selected").
- [ ] **copy + paste at start** — select slide 4 (a distinct main text), Copy, click the `[data-role="slide-insert-bar"][data-insert-index="0"]` bar; `expect.poll(mainTextsInOrder)` shows the copied text now first, total count +1.
- [ ] **copy + paste at a true middle gap** (list ≥4) — paste at an interior bar; assert the clone lands at that gap; total +1.
- [ ] **copy + paste at end** — paste at the last bar (`data-insert-index = len`); clone is last; total +1.
- [ ] **cut + paste to a different position** — select 2 slides, Cut (assert `--cut` on both), paste at a gap; assert order changed, COUNT UNCHANGED, and after `page.reload()` + reopen the order persists.
- [ ] **cut then Escape** — Cut, `page.keyboard.press("Escape")`; assert no `--cut` cards remain, order unchanged, count unchanged.
- [ ] **Ctrl/Cmd+C / X / V shortcuts** — select via checkbox, `page.keyboard.press("Control+c")`; assert insertion bars appear (`[data-role="slide-insert-bar"]` count > 0 → clipboard set); set a target gap by hovering a bar, `Control+v`; assert paste happened (`waitForResponse` on `**/slides/paste`). Repeat a cut via `Control+x` + `Control+v` and assert the move.
- [ ] **shortcuts inert while typing in a slide textarea** — focus a slide's `[data-field="main"]` textarea, type text, `page.keyboard.press("Control+c")`; assert NO insertion bars appear (clipboard NOT set → `[data-role="slide-insert-bar"]` count === 0) and the slide count is unchanged.
- [ ] **Ctrl+C works right after a checkbox select** — check a slide (focus is on the checkbox), `Control+c`; assert insertion bars DO appear (validates `text_entry_focused` does not treat a checkbox as a text field).
- [ ] **paste into an empty-ish presentation (position 0 edge)** — 1-slide presentation, copy it, paste at bar index 0; assert 2 slides, copied text first.
- [ ] **failed paste (500) loses nothing, keeps clipboard** — `page.route('**/presentations/*/slides/paste', r => r.fulfill({status:500, body:"x"}))`; select + Copy + click a bar; assert a toast appears (`[data-role="toast"]` or the app's toast selector — confirm the toast DOM role in `operator.rs`/`mod.rs` at implementation time), the slide list is UNCHANGED (`domSlideOrder` equals the original), and the clipboard is still active (bars still present).
- [ ] **stale clipboard (422) clears the clipboard** — route the paste to `{status:422, body: JSON.stringify({message:"gone"})}`; Copy + click a bar; assert a toast appears AND the insertion bars DISAPPEAR (`[data-role="slide-insert-bar"]` count === 0 → clipboard cleared).
- [ ] **block drag onto a gap** — Copy, then dispatch the HTML5 drag chain from `[data-role="clipboard-drag"]` (dragstart) to a `[data-role="slide-insert-bar"]` (dragover with `preventDefault` asserted, then drop) — mirror the `dragSlide` DataTransfer pattern in `wasm-drag-drop.spec.ts`; assert the paste happened at that gap (`waitForResponse` on `**/slides/paste`, then `expect.poll(mainTextsInOrder)`).

Every test that mutates asserts persistence where the spec asks (reload + reopen for the cut case). Assert a clean browser console (no errors/warnings) on at least the copy/paste happy paths, matching the existing specs' `page.on("console", ...)` pattern (a WASM `console.warn` red-lines these — `.claude/skills/ui/SKILL.md`).

Steps:

- [ ] Write the spec.
- [ ] Run it locally against a freshly built server (build order matters — the server embeds `dist/`): `bash scripts/build-ui.sh` → build the test server with the E2E feature flags per the deploy skill (`cargo build --release -p presenter-server -p presenter-importer --features presenter-server/mock-integrations,presenter-server/test-helpers`) → `npx playwright test tests/e2e/wasm-slide-multiselect.spec.ts --project=chromium --reporter=line`.
  Expected: all tests green. (This lane runs on CI's self-hosted `e2e` job; no NDI/GPU needed for these UI tests.)

**Commit:** `test(e2e): slide multi-select copy/cut/paste matrix (#554)`

---

## Task 9 — Verification (full gate + quality-check --strict + DEV-ONLY post-deploy)

> **PROD/PP verification is DEFERRED:** SNV production is temporarily OFFLINE. Verify ONLY on DEV (`http://10.77.8.134:8080`). Do NOT attempt prod (`10.77.9.205`) or PP (`companion-pp.lan`) checks in this PR — note in the completion report that prod/PP verification is deferred until production returns.

Steps:

- [ ] **Format + lint (workspace):**
  - `cargo fmt --all --check`
  - `cargo clippy --workspace --all-targets -- -D warnings -W clippy::all`
- [ ] **WASM lint:** `cd crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings`
- [ ] **Unit tests:**
  - `cargo test -p presenter-server` (paste op + router 422 tests)
  - `cd crates/presenter-ui && cargo test --lib` (pure logic helpers) — note: if the full workspace `cargo test` SIGSEGVs in `presenter-persistence`, run `cargo clean -p presenter-persistence` then retry, or `cargo test -p presenter-persistence --lib` alone (MEMORY.md).
- [ ] **Strict quality gate (the CI "Quality Checks" job — run the EXACT command):**
  `./scripts/dev/quality-check.sh --strict --against origin/main`
  Must pass: no file >1000 prod lines, no fn >120 lines (tests included), no `continue-on-error`, cargo-deny/audit clean (re-run ONCE if deny/audit transiently fails — advisory-DB timing, per `.claude/skills/ci/SKILL.md`). Confirm `git diff --name-only origin/main...HEAD` does NOT drag in a pre-existing over-cap file (e.g. `state/mod.rs` #486). Check `slide_list.rs` prod lines specifically: `bash scripts/dev/count_prod_lines.sh crates/presenter-ui/src/components/slide_list.rs` (< 1000).
- [ ] **Build the UI + server, deploy to DEV** (this dev2 machine; `airuleset:local-builds=allowed`): `bash scripts/build-ui.sh` → rebuild + deploy per `.claude/skills/deploy/SKILL.md` → `sudo systemctl restart presenter-dev` → `curl http://10.77.8.134:8080/healthz` returns the NEW version + `"channel":"dev"`.
- [ ] **DEV functional verification with Playwright (real workflow, read real values):** open `http://10.77.8.134:8080/ui/operator`, enter edit mode, and drive the actual flows:
  - Select 2 slides via checkboxes → panel shows "2 selected".
  - Copy → paste at a middle gap → the DOM slide list grew by 2 and the clones carry the source text (read `[data-field="main"]` values, not just element count).
  - Cut 1 slide → paste elsewhere → count unchanged, order moved; reload the page and re-open → the new order persisted (proves the server round-trip + #555 `updated_at` bump landed).
  - Confirm the browser console is clean (zero errors/warnings).
  - Report the version read from the DOM footer (`v X.Y.Z (dev)`), matching `/healthz`.
- [ ] **Post the PR** (`dev` → `main`) and drive ALL gates green (CI incl. the self-hosted `e2e` job, `mergeable: true` + `mergeStateStatus: CLEAN`, `/review` + `/requesting-code-review` both 0/0/0). This is a FEATURE (not a bug fix) → no RED/GREEN regression-line required, but the E2E matrix is the behavior evidence. Auto-merge per `pr-merge-policy.md` (this project has no `airuleset:merge=manual` marker) once every gate is green; monitor the `main` deploy to DEV to terminal state.

**Commit (if any verification-driven fixes):** fold into the relevant task's commit style; do not add a "fixups" grab-bag.

---

## Self-review (run before declaring the plan done — DONE during authoring)

- **Spec coverage sweep** — every spec decision is a task: multi-select checkbox + Shift-range (T7 + `range_select` T2); selection panel N/Copy/Cut/Clear + Paste (T7); paste-of-copy endpoint with clone-full-content + clamp + 422 (T3/T4); paste-of-cut via reorder + `cut_splice_order` (T2/T7); insertion bars in every gap, clipboard-only (T7); block drag onto a gap (T7 + T8); Ctrl/Cmd+C/X/V + Escape, guarded (T2 `shortcut_action` + T7 keyboard + `text_entry_focused`); cut non-destructive until paste, Escape/Clear abandons (T7); error handling — failed paste keeps clipboard, 422 clears (T7/T8); `presentation_id` seam + clear-on-switch (T6/T7); `updated_at` + `nudge_sync` on paste (T3); full E2E matrix incl. empty presentation + failed-paste + inert-in-textarea (T8); Rust unit tests for the paste op (clamp, contiguous block, 422, group/content cloned) + the pure cut-splice helper (T3/T2). Out-of-scope items (cross-presentation clipboard, refresh persistence, live-selection block reorder) are correctly NOT built.
- **Placeholder scan** — no `TODO`/`...`/`unimplemented!()` in shipped code; every code step is real. (The two `view! { <article ...>...</article> }` ellipses in the T7b interleave step deliberately mean "keep the existing per-card body verbatim" — flagged inline, not a placeholder to write.)
- **Type-name consistency** — `SlideClipboard`, `ClipboardMode {Copy,Cut}`, `PasteSlidesError {UnknownSlides, Internal}`, `ShortcutAction {Copy,Cut,Paste,Clear}`, `PasteSlidesRequest {slideIds, position}` (camelCase on both client `Serialize` and server `Deserialize`), signals `selected_slide_ids`/`clipboard`/`selection_anchor_index`/`paste_target_gap`/`dragging_clipboard`, helpers `selected_memo`/`cut_memo`/`render_select_checkbox`/`render_insertion_bar`/`paste_at_gap`/`cut_splice_order`/`range_select`/`shortcut_action`/`text_entry_focused` — used consistently across tasks. Route `/presentations/{id}/slides/paste`. Data-roles: `slide-selection-panel`, `slide-selection-count`, `slide-select-checkbox`, `slide-insert-bar`, `clipboard-drag`, `slide-copy`/`slide-cut`/`slide-paste`/`slide-clear`.
- **Reactivity / quality landmines addressed** — selection/cut via per-card `Memo` (no rebuild); only clipboard-non-empty toggles a list rebuild (documented, safe); `slide_list.rs` file-cap guarded with an extraction fallback; new tests in own files to dodge the `quality-check.sh` diff-drag; keyboard guard narrowed to text-entry so checkbox→Ctrl+C works; window listener justified vs container; paste inherits `updated_at`/`nudge_sync` via `replace_presentation_slides`.
