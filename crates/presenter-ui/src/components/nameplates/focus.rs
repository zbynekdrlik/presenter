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

use wasm_bindgen::JsCast;
use web_sys::{Element, HtmlElement, HtmlInputElement, Node};

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

/// Run `change`, a plate-list update that may move rows (a reorder made here,
/// a list from the server), keeping the focus of the list.
///
/// The focus goes back in a task queued AFTER `change`: the render effects it
/// woke (the `<For>` moving the rows) are queued first, so the task sees the
/// rows already moved (UI skill, "Moving focus after a signal change"). That
/// is still before the next input event, so a second ↑ goes to the handle.
pub fn keep_focus(change: impl FnOnce()) {
    let kept = KeptFocus::take();
    change();
    if let Some(kept) = kept {
        leptos::task::spawn_local(async move { kept.restore() });
    }
}

/// Whether `element` holds the focus.
pub fn holds_focus(element: &Element) -> bool {
    let node: &Node = element;
    document()
        .active_element()
        .is_some_and(|active| active.is_same_node(Some(node)))
}
