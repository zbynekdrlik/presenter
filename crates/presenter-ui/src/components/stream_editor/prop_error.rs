//! Inline property-form error, tagged with the element it belongs to (#787 reopen).
//!
//! A save runs asynchronously, so its 422 can land after the operator has
//! already opened a different element. The error therefore carries the id of
//! the element whose save failed, and the form shows it only while that element
//! is the one being edited. A late SUCCESS clears only its own element's error.
//! Pure + host-tested; the ctx holds it in `prop_error: RwSignal<Option<PropError>>`.

use leptos::prelude::*;

use super::StreamEditorCtx;

/// A failed save's message, bound to the element that was saved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PropError {
    pub element_id: i64,
    pub message: String,
}

/// The message to show in the form while `editing` is open — `None` when there
/// is no error, or when the error belongs to another element.
pub fn message_for(error: Option<&PropError>, editing: Option<i64>) -> Option<String> {
    match (error, editing) {
        (Some(e), Some(id)) if e.element_id == id => Some(e.message.clone()),
        _ => None,
    }
}

/// True when `error` belongs to `element_id` — a successful save of that
/// element clears it; a success of any other element leaves it alone.
pub fn belongs_to(error: Option<&PropError>, element_id: i64) -> bool {
    error.is_some_and(|e| e.element_id == element_id)
}

impl StreamEditorCtx {
    /// A save of `element_id` failed: remember the message FOR THAT ELEMENT, so
    /// a late 422 never shows under an element opened meanwhile. Logs AFTER
    /// storing it — the E2E uses the log line as its "422 handled" signal.
    pub(super) fn set_prop_error(self, element_id: i64, err: impl std::fmt::Display) {
        self.prop_error.set(Some(PropError {
            element_id,
            message: format!("Neplatné hodnoty: {err}"),
        }));
        let editing = self.selected_element.get_untracked();
        leptos::logging::log!(
            "stream editor: save of element {element_id} failed: {err} (editing {editing:?})"
        );
    }

    /// A save of `element_id` succeeded: clear the inline error only if it
    /// belongs to that element — another element's error stays.
    pub(super) fn clear_prop_error_of(self, element_id: i64) {
        if self
            .prop_error
            .with_untracked(|e| belongs_to(e.as_ref(), element_id))
        {
            self.prop_error.set(None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(id: i64) -> PropError {
        PropError {
            element_id: id,
            message: "Neplatné hodnoty: size".to_string(),
        }
    }

    #[test]
    fn shown_for_the_element_that_was_saved() {
        let e = err(7);
        assert_eq!(
            message_for(Some(&e), Some(7)).as_deref(),
            Some("Neplatné hodnoty: size")
        );
    }

    #[test]
    fn hidden_under_a_different_element() {
        let e = err(7);
        assert_eq!(message_for(Some(&e), Some(8)), None);
    }

    #[test]
    fn hidden_when_nothing_is_edited_or_no_error() {
        let e = err(7);
        assert_eq!(message_for(Some(&e), None), None);
        assert_eq!(message_for(None, Some(7)), None);
    }

    #[test]
    fn a_success_clears_only_its_own_element_error() {
        let e = err(7);
        assert!(belongs_to(Some(&e), 7));
        assert!(!belongs_to(Some(&e), 8));
        assert!(!belongs_to(None, 7));
    }
}
