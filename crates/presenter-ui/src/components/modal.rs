use leptos::prelude::Set;

use crate::state::operator::OperatorState;

/// Close any open modal.
pub fn close_modal(op: &OperatorState) {
    op.open_modal.set(None);
    op.modal_target_id.set(None);
    if let Some(body) = crate::utils::window::document_body() {
        body.dataset().delete("modalOpen");
    }
}

/// Open a named modal.
pub fn open_modal(op: &OperatorState, name: &str) {
    op.open_modal.set(Some(name.to_string()));
    if let Some(body) = crate::utils::window::document_body() {
        let _ = body.dataset().set("modalOpen", name);
    }
}
