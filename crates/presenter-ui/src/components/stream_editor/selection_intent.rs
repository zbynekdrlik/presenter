//! Monotonic selection-intent counter for the stream editor (#787 reopen).
//!
//! Every local selection change (pick / deselect / scene open / panel close /
//! the start of an element create) bumps the counter. An async action that
//! wants to select something when its response lands (e.g. `add_element`
//! selecting the element it created) captures a ticket BEFORE awaiting and
//! applies the selection only if the ticket is still current — so a late
//! response can never override a newer choice the operator made meanwhile.
//! Pure + host-tested; the ctx holds it in a `StoredValue`.
//!
//! The ctx's selection-changing methods live here too, next to the one shared
//! dirty guard (`confirm_discard_draft`): every path that drops the element
//! draft — scene open, panel close, element pick / deselect, element create,
//! output switch — asks „Zahodiť neuložené zmeny prvku?" first (#787 reopen).

use leptos::prelude::*;

use super::prop_error::{self, PropError};
use super::StreamEditorCtx;

/// The counter. `Default` starts at 0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SelectionIntent {
    seq: u64,
}

impl SelectionIntent {
    /// Record a new local selection intent; returns its ticket.
    pub fn bump(&mut self) -> u64 {
        self.seq = self.seq.wrapping_add(1);
        self.seq
    }

    /// True when no newer intent was recorded since `ticket` was issued.
    pub fn is_current(&self, ticket: u64) -> bool {
        self.seq == ticket
    }
}

impl StreamEditorCtx {
    /// The dirty guard shared by every selection change: unsaved edits ask
    /// „Zahodiť neuložené zmeny prvku?" first. Returns whether to proceed
    /// (always true for a clean draft).
    pub(super) fn confirm_discard_draft(self) -> bool {
        if !self.draft_is_dirty() {
            return true;
        }
        crate::utils::window::window()
            .confirm_with_message("Zahodiť neuložené zmeny prvku?")
            .unwrap_or(true)
    }

    /// Open a scene for element authoring; clears any element selection + error.
    /// Unsaved element edits ask first (#787 reopen) — declining keeps them.
    pub fn select_scene(self, scene_id: i64) {
        if !self.confirm_discard_draft() {
            return;
        }
        self.bump_selection();
        self.selected_scene.set(Some(scene_id));
        self.selected_element.set(None);
        self.draft_element_id.set(None);
        self.prop_error.set(None);
    }

    /// Close the element panel (no scene selected). The raw reset, used by
    /// `switch_output` / `delete_scene` after their own decision — the „Zavrieť"
    /// button goes through the guarded [`Self::request_close_panel`].
    pub fn close_panel(self) {
        self.bump_selection();
        self.selected_scene.set(None);
        self.selected_element.set(None);
        self.draft_element_id.set(None);
        self.prop_error.set(None);
    }

    /// Open an element in the property form; clears any prior inline error.
    /// Switching away from an element with UNSAVED edits asks first (the draft is
    /// discarded on confirm, per the design's dirty guard).
    pub fn select_element(self, element_id: i64) {
        if let Some(current) = self.draft_element_id.get_untracked() {
            if current != element_id && !self.confirm_discard_draft() {
                return;
            }
        }
        self.bump_selection();
        self.selected_element.set(Some(element_id));
        self.prop_error.set(None);
    }

    /// Deselect the current element (canvas Escape / empty-canvas click, #787),
    /// keeping the scene open. Unsaved edits ask first, like [`Self::select_element`].
    /// Returns whether the element was actually deselected.
    pub fn deselect_element(self) -> bool {
        if self.selected_element.get_untracked().is_none()
            && self.draft_element_id.get_untracked().is_none()
        {
            return false;
        }
        if !self.confirm_discard_draft() {
            return false;
        }
        self.bump_selection();
        self.selected_element.set(None);
        self.draft_element_id.set(None);
        self.prop_error.set(None);
        true
    }

    /// The „Zavrieť" button: close the element panel through the same dirty
    /// guard as every other selection change (#787 reopen). Declining keeps the
    /// panel, the element and its unsaved edits.
    pub fn request_close_panel(self) {
        if self.confirm_discard_draft() {
            self.close_panel();
        }
    }

    /// A save of `element_id` failed: remember the message FOR THAT ELEMENT, so
    /// a late 422 never shows under an element opened meanwhile (#787 reopen).
    pub(super) fn set_prop_error(self, element_id: i64, err: impl std::fmt::Display) {
        let editing = self.selected_element.get_untracked();
        leptos::logging::log!(
            "stream editor: save of element {element_id} failed: {err} (editing {editing:?})"
        );
        self.prop_error.set(Some(PropError {
            element_id,
            message: format!("Neplatné hodnoty: {err}"),
        }));
    }

    /// A save of `element_id` succeeded: clear the inline error only if it
    /// belongs to that element — another element's error stays.
    pub(super) fn clear_prop_error_of(self, element_id: i64) {
        if self
            .prop_error
            .with_untracked(|e| prop_error::belongs_to(e.as_ref(), element_id))
        {
            self.prop_error.set(None);
        }
    }

    /// Record a new local selection intent; returns its ticket.
    pub(super) fn bump_selection(self) -> u64 {
        self.selection.try_update_value(|s| s.bump()).unwrap_or(0)
    }

    /// True when no selection change happened since `ticket` was issued.
    pub(super) fn selection_is_current(self, ticket: u64) -> bool {
        self.selection
            .try_with_value(|s| s.is_current(ticket))
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_ticket_is_current() {
        let mut s = SelectionIntent::default();
        let t = s.bump();
        assert!(s.is_current(t));
    }

    #[test]
    fn a_newer_intent_invalidates_an_older_ticket() {
        let mut s = SelectionIntent::default();
        let add = s.bump();
        let pick = s.bump();
        assert!(!s.is_current(add), "late add response must not select");
        assert!(s.is_current(pick));
    }

    #[test]
    fn tickets_are_strictly_increasing() {
        let mut s = SelectionIntent::default();
        let a = s.bump();
        let b = s.bump();
        assert!(b > a);
    }

    #[test]
    fn default_state_is_not_current_for_a_future_ticket() {
        let s = SelectionIntent::default();
        assert!(!s.is_current(1));
        assert!(s.is_current(0));
    }
}
