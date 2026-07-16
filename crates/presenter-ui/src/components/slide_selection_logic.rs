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

/// Resolve a Shift+click range selection from an ANCHOR SLIDE ID rather than
/// a positional index (#558 V9): a raw index anchor is never invalidated
/// when the list shifts (a paste/delete/reorder before the next
/// Shift+click), so a later Shift+click could range-select the WRONG
/// slides. Looks up the anchor's CURRENT index in `ids`; when the anchor
/// slide no longer exists, returns `None` so the caller falls back to
/// treating the click as a plain click (select just the clicked slide, and
/// the clicked slide becomes the new anchor).
pub(super) fn range_select_by_anchor_id(
    ids: &[String],
    anchor_id: &str,
    clicked: usize,
    current: &HashSet<String>,
) -> Option<HashSet<String>> {
    let anchor_index = ids.iter().position(|id| id == anchor_id)?;
    Some(range_select(ids, anchor_index, clicked, current))
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
        assert_eq!(
            shortcut_action("Escape", false),
            Some(ShortcutAction::Clear)
        );
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
    fn range_select_by_anchor_id_resolves_the_anchor_after_a_list_shift() {
        // #558 V9: a POSITIONAL anchor (the old `selection_anchor_index`)
        // is never invalidated when the list shifts — anchoring at index 1
        // ("b") and then pasting a new slide above it moves "b" to index 2,
        // but a stale positional anchor would still say "1", sweeping in
        // the WRONG slide ("x", the pasted one) on the next Shift+click.
        // Anchoring by SLIDE ID resolves "b"'s CURRENT index instead.
        let before = ids(&["a", "b", "c"]);
        let anchor_id = before[1].clone(); // "b", captured at anchor time.

        // A paste inserts a new slide "x" above "b": list becomes [a, x, b, c].
        let after = ids(&["a", "x", "b", "c"]);

        // Shift+click "c" (now index 3): the correct range is b..=c.
        let out = range_select_by_anchor_id(&after, &anchor_id, 3, &HashSet::new())
            .expect("the anchor slide still exists in the shifted list");
        assert_eq!(
            out,
            set(&["b", "c"]),
            "must NOT include \"x\" — a stale positional anchor (1) would have"
        );

        // Proves the bug this replaces: the OLD positional anchor (index 1,
        // captured before the shift) sweeps in "x" too — the wrong range.
        let buggy_positional = range_select(&after, 1, 3, &HashSet::new());
        assert_eq!(
            buggy_positional,
            set(&["x", "b", "c"]),
            "a raw positional anchor demonstrably includes the wrong slide after a shift"
        );
    }

    #[test]
    fn range_select_by_anchor_id_signals_fallback_when_the_anchor_vanished() {
        // #558 V9: the anchor slide was deleted since it was set — there is
        // no honest range to resolve; the caller must fall back to a plain
        // click instead of guessing.
        let ids_now = ids(&["a", "c"]); // "b" (the anchor) was deleted.
        let out = range_select_by_anchor_id(&ids_now, "b", 1, &HashSet::new());
        assert!(
            out.is_none(),
            "a vanished anchor must signal fallback-to-plain-click, not a guessed range"
        );
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
