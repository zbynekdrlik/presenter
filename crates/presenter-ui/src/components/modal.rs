use leptos::prelude::Set;

use crate::state::operator::OperatorState;
use crate::utils::window;

/// Open a named modal.
pub fn open_modal(op: &OperatorState, name: &str) {
    op.open_modal.set(Some(name.to_string()));
    if let Some(body) = window::document_body() {
        let _ = body.dataset().set("modalOpen", name);
    }
}

/// Close any open modal.
pub fn close_modal(op: &OperatorState) {
    op.open_modal.set(None);
    op.modal_target_id.set(None);
    if let Some(body) = window::document_body() {
        body.dataset().delete("modalOpen");
    }
}

/// Show a browser confirm dialog and return the result.
pub fn confirm(message: &str) -> bool {
    window::window()
        .confirm_with_message(message)
        .unwrap_or(false)
}
