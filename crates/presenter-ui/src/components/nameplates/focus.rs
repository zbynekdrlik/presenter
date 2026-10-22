//! Keeping the focus in a plate list across a reorder (#871 review).
//!
//! The keyed `<For>` moves a reordered row with `insertBefore` (tachys
//! `view/keyed.rs`; a one-step swap re-inserts BOTH rows), and moving a
//! focused element drops its focus: it falls back to `<body>`, and Chrome
//! fires `blur` on the way. So ↑ / ↓ on the drag handle stopped after one
//! step, and a reorder from elsewhere pulled the operator out of a half-typed
//! name. [`keep_focus`] notes the focused element of a plate list (and an
//! input's caret / selection) before the list changes, and gives the focus
//! back once the rows have moved. The row DOM is reused across the move (the
//! key is the plate id), so the noted element is the one to focus again.
//!
//! While a move is under way [`rows_moving`] is true, so a field's blur
//! handler can tell the move's blur from the operator leaving the field or
//! the window ([`blur_kind`]).

use std::cell::Cell;

use wasm_bindgen::JsCast;
use web_sys::{Element, HtmlElement, HtmlInputElement, Node, VisibilityState};

use crate::utils::window::{document, focus_is_free};

/// Every plate list (the editor's Menovky tab, the operator's Menovky view).
const PLATE_LIST: &str = "[data-role=\"stream-nameplate-list\"]";

/// The focused element of a plate list, with an input's selection.
struct KeptFocus {
    element: HtmlElement,
    selection: Option<(u32, u32)>,
}

impl KeptFocus {
    /// The focused element, when it is inside a plate list.
    fn take() -> Option<Self> {
        let active = document().active_element()?;
        // Only a plate list's own focus: the add form or another view is
        // never touched.
        active.closest(PLATE_LIST).ok().flatten()?;
        let selection = active
            .dyn_ref::<HtmlInputElement>()
            .and_then(|input| Some((input.selection_start().ok()??, input.selection_end().ok()??)));
        let element = active.dyn_into::<HtmlElement>().ok()?;
        Some(Self { element, selection })
    }

    /// Focus the element again (and put the caret / selection back) when the
    /// move dropped its focus. Nothing when it has left the page (its plate
    /// was deleted) or the focus is somewhere else on purpose.
    fn restore(self) {
        if !self.element.is_connected() || !focus_is_free() {
            return;
        }
        if self.element.focus().is_err() {
            leptos::logging::log!("nameplates: the focus could not be put back after a reorder");
            return;
        }
        leptos::logging::log!("nameplates: focus put back after a reorder moved its row");
        if let (Some((start, end)), Some(input)) =
            (self.selection, self.element.dyn_ref::<HtmlInputElement>())
        {
            let _ = input.set_selection_range(start, end);
        }
    }
}

thread_local! {
    /// How many plate-list row moves are under way: from just before the
    /// list changes until the focus has been put back ([`keep_focus`]).
    static MOVES: Cell<u32> = const { Cell::new(0) };
}

/// Whether a plate list is moving its rows right now (#871 review). A blur
/// that fires meanwhile came from the move (`insertBefore` drops the focus of
/// the moved field), never from the operator: the move, its render effect and
/// the focus restore all run in one go of queued tasks, before the next key,
/// click or window event. Read it IN the blur handler, not later.
pub fn rows_moving() -> bool {
    MOVES.with(|moves| moves.get() > 0)
}

/// Run `change`, a plate-list update that may move rows (a reorder made here,
/// a list from the server), keeping the focus of the list.
///
/// The focus goes back in a task queued AFTER `change`: the render effects it
/// woke (the `<For>` moving the rows) are queued first, so the task sees the
/// rows already moved (UI skill, "Moving focus after a signal change"). That
/// is still before the next input event, so a second ↑ goes to the handle.
/// Until the focus is back, [`rows_moving`] says so.
pub fn keep_focus(change: impl FnOnce()) {
    let Some(kept) = KeptFocus::take() else {
        // Nothing in a plate list has the focus: no field can lose it.
        change();
        return;
    };
    MOVES.with(|moves| moves.set(moves.get() + 1));
    change();
    leptos::task::spawn_local(async move {
        kept.restore();
        MOVES.with(|moves| moves.set(moves.get().saturating_sub(1)));
    });
}

/// Why a plate field lost the focus (#871 review), decided after the focus
/// restore.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blur {
    /// A row move dropped the focus and it is back: the field was not left.
    Moved,
    /// The focus went away without the operator leaving the field: the
    /// WINDOW lost it (alt-tab, OBS, Companion, another tab, a locked phone;
    /// the field stays the page's active element), or a move could not give
    /// it back.
    Away,
    /// The operator left the field inside the page (Tab, a click elsewhere,
    /// Enter).
    Left,
}

/// [`Blur`] from what the page knows: `moving` = [`rows_moving`] when the
/// blur fired, `still_active` = the field is the document's active element
/// again, `page_focused` = `document.hasFocus()`.
///
/// Only a blur DURING a move can be a move: whether the window has already
/// lost the focus when the save decides depends on the browser, so a field
/// that is still active outside a move is a window left, never "moved".
pub fn blur_kind(moving: bool, still_active: bool, page_focused: bool) -> Blur {
    match (moving, still_active, page_focused) {
        (true, true, true) => Blur::Moved,
        (true, _, _) | (false, true, _) => Blur::Away,
        (false, false, _) => Blur::Left,
    }
}

/// [`blur_kind`] for `field`, read from the document now.
pub fn field_blur(moving: bool, field: &Element) -> Blur {
    let node: &Node = field;
    let still_active = document()
        .active_element()
        .is_some_and(|active| active.is_same_node(Some(node)));
    blur_kind(
        moving,
        still_active,
        document().has_focus().unwrap_or(false),
    )
}

/// The page is hidden (another tab or app, a locked phone), where a phone
/// browser may kill it at any moment.
pub fn page_hidden() -> bool {
    document().visibility_state() == VisibilityState::Hidden
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blur_during_a_move_with_the_focus_back_is_a_move() {
        assert_eq!(blur_kind(true, true, true), Blur::Moved);
    }

    #[test]
    fn a_still_active_field_outside_a_move_is_a_window_left_even_if_the_page_reports_focus() {
        // Safari / Firefox may still report the window's focus when the
        // save decides; Chrome no longer does. Both are a window left.
        assert_eq!(blur_kind(false, true, true), Blur::Away);
        assert_eq!(blur_kind(false, true, false), Blur::Away);
    }

    #[test]
    fn a_move_whose_focus_did_not_come_back_or_whose_window_left_is_away() {
        assert_eq!(blur_kind(true, false, true), Blur::Away);
        assert_eq!(blur_kind(true, true, false), Blur::Away);
        assert_eq!(blur_kind(true, false, false), Blur::Away);
    }

    #[test]
    fn a_field_that_lost_the_focus_inside_the_page_was_left() {
        assert_eq!(blur_kind(false, false, true), Blur::Left);
        assert_eq!(blur_kind(false, false, false), Blur::Left);
    }
}
