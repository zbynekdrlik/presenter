//! #819: the inline add / edit editor shared by the Resolume and Android list cards.
//!
//! Before #819 each card had ONE permanent add/edit form at its top, and a row's
//! **Edit** only loaded that form — measured 1170 px above the row the operator had
//! just clicked. Now **Edit** turns the clicked row itself into this editor (Save /
//! Cancel), and "+ Add …" opens the same editor as a new item at the top of the
//! list. One editor per card is open at a time ([`EditTarget`]): opening another one
//! discards the first one's unsaved changes.
//!
//! The draft ([`ConnectionDraft`]) lives at CARD level, outside the keyed `<For>`
//! rows, so neither the 5 s status poll nor a row rebuilt by an edit from another
//! tab can reset what the operator is typing.

use leptos::prelude::*;

use super::parse_port_in_range;
use crate::utils::keyboard::KEY_ESCAPE;

/// Which item a card's single inline editor is open on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) enum EditTarget {
    /// No editor open.
    #[default]
    Closed,
    /// The "+ Add …" editor at the top of the list.
    New,
    /// The row with this id is being edited in place.
    Item(String),
}

impl EditTarget {
    pub(super) fn is_new(&self) -> bool {
        matches!(self, Self::New)
    }

    pub(super) fn is_item(&self, id: &str) -> bool {
        matches!(self, Self::Item(open) if open == id)
    }

    /// The id a save UPDATES; `None` for the new item (a save creates) or no editor.
    pub(super) fn item_id(&self) -> Option<&str> {
        match self {
            Self::Item(id) => Some(id),
            Self::Closed | Self::New => None,
        }
    }
}

/// The validated label / host / port of an editor submission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ConnectionFields {
    pub(super) label: String,
    pub(super) host: String,
    pub(super) port: u16,
}

/// Validate the fields both editors share, in the order the operator sees them.
///
/// The port input has no native `min`/`max`, so an out-of-range value (99999)
/// reaches this check and gets the styled message instead of the browser silently
/// blocking the submit — this is the single authority for the 1..=65535 range (#455).
pub(super) fn validate_connection(
    label: &str,
    host: &str,
    port: &str,
) -> Result<ConnectionFields, &'static str> {
    let label = label.trim();
    if label.is_empty() {
        return Err("Label cannot be empty.");
    }
    let host = host.trim();
    if host.is_empty() {
        return Err("Host cannot be empty.");
    }
    let port = parse_port_in_range(port).ok_or("Port must be between 1 and 65535.")?;
    Ok(ConnectionFields {
        label: label.to_string(),
        host: host.to_string(),
        port,
    })
}

/// The open editor's values plus its message line. `Copy` (signals only), so every
/// handler and row closure can hold it.
#[derive(Clone, Copy)]
pub(super) struct ConnectionDraft {
    pub(super) label: RwSignal<String>,
    pub(super) host: RwSignal<String>,
    pub(super) port: RwSignal<String>,
    pub(super) enabled: RwSignal<bool>,
    /// The line under the editor: "Saving changes…" or a validation / save error.
    pub(super) message: RwSignal<String>,
    /// `idle` / `info` / `error` — the line's `data-state`; `error` also sets
    /// `aria-invalid` on the fields (#459).
    pub(super) state: RwSignal<String>,
    /// A save is in flight: Save is disabled and a second submit is ignored.
    pub(super) busy: RwSignal<bool>,
}

impl Default for ConnectionDraft {
    fn default() -> Self {
        Self {
            label: RwSignal::new(String::new()),
            host: RwSignal::new(String::new()),
            port: RwSignal::new(String::new()),
            enabled: RwSignal::new(true),
            message: RwSignal::new(String::new()),
            state: RwSignal::new(String::from("idle")),
            busy: RwSignal::new(false),
        }
    }
}

impl ConnectionDraft {
    /// Fill the editor with an item's current values.
    pub(super) fn load(self, label: &str, host: &str, port: u16, enabled: bool) {
        self.label.set(label.to_string());
        self.host.set(host.to_string());
        self.port.set(port.to_string());
        self.enabled.set(enabled);
        self.clear_message();
    }

    /// An empty editor for a new item.
    pub(super) fn reset(self, default_port: u16) {
        self.load("", "", default_port, true);
    }

    pub(super) fn validated(self) -> Result<ConnectionFields, &'static str> {
        validate_connection(
            &self.label.get_untracked(),
            &self.host.get_untracked(),
            &self.port.get_untracked(),
        )
    }

    pub(super) fn show(self, state: &str, message: &str) {
        self.state.set(state.to_string());
        self.message.set(message.to_string());
    }

    pub(super) fn clear_message(self) {
        self.show("idle", "");
    }
}

/// An item-specific extra text field in the editor (Android's launch package).
#[derive(Clone, Copy)]
pub(super) struct ExtraField {
    pub(super) caption: &'static str,
    pub(super) role: &'static str,
    pub(super) placeholder: &'static str,
    pub(super) value: RwSignal<String>,
}

/// What differs between the Resolume and the Android editor.
#[derive(Clone, Copy)]
pub(super) struct EditorSpec {
    /// The `data-role` prefix: `host` → `host-label`, `host-submit`, `host-cancel`, …
    pub(super) role: &'static str,
    /// `id` of the message line every field's `aria-describedby` points at (#459).
    pub(super) message_id: &'static str,
    /// `data-role` of that message line.
    pub(super) message_role: &'static str,
    pub(super) label_placeholder: &'static str,
    pub(super) host_placeholder: &'static str,
    /// The editor heading and the Save button text for a NEW item.
    pub(super) new_title: &'static str,
    pub(super) new_submit: &'static str,
    /// The editor heading when editing an existing item in place.
    pub(super) edit_title: &'static str,
    pub(super) extra: Option<ExtraField>,
}

/// The inline editor. `creating` = the "+ Add …" item (Save creates); otherwise an
/// existing row (Save updates). `on_save` validates and saves, `on_cancel` closes the
/// editor without saving (also Escape). Only one editor exists in a card at a time,
/// so the field `data-role`s and the message `id` stay unique on the page.
pub(super) fn render_connection_editor<S, C>(
    spec: EditorSpec,
    draft: ConnectionDraft,
    creating: bool,
    on_save: S,
    on_cancel: C,
) -> AnyView
where
    S: Fn() + Copy + Send + 'static,
    C: Fn() + Copy + Send + 'static,
{
    let role = move |field: &str| format!("{}-{field}", spec.role);
    let invalid = move || (draft.state.get() == "error").to_string();
    let (mode, title, submit_text) = if creating {
        ("create", spec.new_title, spec.new_submit)
    } else {
        ("edit", spec.edit_title, "Save")
    };

    // The operator clicked Edit / "+ Add" to type: put the caret in the first field.
    let label_ref = NodeRef::<leptos::html::Input>::new();
    Effect::new(move || {
        if let Some(el) = label_ref.get() {
            let _ = el.focus();
        }
    });

    let on_submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        on_save();
    };
    let on_keydown = move |ev: web_sys::KeyboardEvent| {
        if ev.key() == KEY_ESCAPE {
            ev.prevent_default();
            on_cancel();
        }
    };

    let extra = spec
        .extra
        .map(|field| render_extra_field(field, spec.message_id, draft));

    view! {
        <form class="settings__form settings__form--inline-editor" data-role=role("editor")
            data-mode=mode autocomplete="off" on:submit=on_submit on:keydown=on_keydown>
            <p class="settings__editor-title" data-role=role("editor-title")>{title}</p>
            <div class="settings__form-row settings__form-row--connection">
                <label>
                    <span>"Label"</span>
                    <input type="text" node_ref=label_ref data-role=role("label")
                        placeholder=spec.label_placeholder required
                        aria-required="true"
                        aria-describedby=spec.message_id
                        aria-invalid=invalid
                        prop:value=move || draft.label.get()
                        on:input=move |ev| draft.label.set(event_target_value(&ev)) />
                </label>
                <label>
                    <span>"Hostname or DNS"</span>
                    <input type="text" data-role=role("host") placeholder=spec.host_placeholder required
                        aria-required="true"
                        aria-describedby=spec.message_id
                        aria-invalid=invalid
                        prop:value=move || draft.host.get()
                        on:input=move |ev| draft.host.set(event_target_value(&ev)) />
                </label>
                <label class="settings__form-control--small">
                    <span>"Port"</span>
                    // No native min/max/required — see `validate_connection` (#455).
                    <input type="number" data-role=role("port")
                        aria-required="true"
                        aria-describedby=spec.message_id
                        aria-invalid=invalid
                        prop:value=move || draft.port.get()
                        on:input=move |ev| draft.port.set(event_target_value(&ev)) />
                </label>
            </div>
            {extra}
            <div class="settings__form-row settings__form-row--inline">
                <label class="settings__form-checkbox">
                    <input type="checkbox" data-role=role("enabled")
                        prop:checked=move || draft.enabled.get()
                        on:change=move |ev| draft.enabled.set(event_target_checked(&ev)) />
                    <span>"Enabled"</span>
                </label>
                <div class="settings__form-actions">
                    <button type="submit" class="settings__button settings__button--primary"
                        data-role=role("submit") prop:disabled=move || draft.busy.get()>
                        {submit_text}
                    </button>
                    <button type="button" class="settings__button settings__button--ghost"
                        data-role=role("cancel") on:click=move |_| on_cancel()>"Cancel"</button>
                </div>
            </div>
            <p id=spec.message_id class="settings__form-status" data-role=spec.message_role
                data-state=move || draft.state.get()>
                {move || draft.message.get()}
            </p>
        </form>
    }
    .into_any()
}

/// The card-specific extra text field (Android's launch package), on its own row.
fn render_extra_field(
    field: ExtraField,
    message_id: &'static str,
    draft: ConnectionDraft,
) -> impl IntoView {
    view! {
        <div class="settings__form-row">
            <label>
                <span>{field.caption}</span>
                <input type="text" data-role=field.role placeholder=field.placeholder required
                    aria-required="true"
                    aria-describedby=message_id
                    aria-invalid=move || (draft.state.get() == "error").to_string()
                    prop:value=move || field.value.get()
                    on:input=move |ev| field.value.set(event_target_value(&ev)) />
            </label>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::{validate_connection, ConnectionFields, EditTarget};

    #[test]
    fn edit_target_tells_the_new_item_from_an_existing_row() {
        assert!(EditTarget::New.is_new());
        assert!(!EditTarget::Closed.is_new());
        assert!(!EditTarget::Item("a".into()).is_new());

        let row = EditTarget::Item("host-1".into());
        assert!(row.is_item("host-1"));
        assert!(!row.is_item("host-2"));
        assert!(!EditTarget::New.is_item("host-1"));
        assert!(!EditTarget::Closed.is_item("host-1"));
    }

    #[test]
    fn only_an_existing_row_is_updated_on_save() {
        assert_eq!(EditTarget::Item("host-1".into()).item_id(), Some("host-1"));
        assert_eq!(EditTarget::New.item_id(), None);
        assert_eq!(EditTarget::Closed.item_id(), None);
        assert_eq!(EditTarget::default(), EditTarget::Closed);
    }

    #[test]
    fn valid_fields_are_trimmed() {
        assert_eq!(
            validate_connection("  Main Arena ", " resolume.lan ", " 8090 "),
            Ok(ConnectionFields {
                label: "Main Arena".into(),
                host: "resolume.lan".into(),
                port: 8090,
            })
        );
    }

    #[test]
    fn a_blank_label_or_host_is_rejected_in_field_order() {
        assert_eq!(
            validate_connection("   ", "", "x"),
            Err("Label cannot be empty.")
        );
        assert_eq!(
            validate_connection("Main", "  ", "x"),
            Err("Host cannot be empty.")
        );
    }

    #[test]
    fn an_out_of_range_port_is_rejected_not_truncated() {
        // #455: 99999 must show the range message, never save a wrapped / default port.
        for port in ["99999", "0", "", "80x"] {
            assert_eq!(
                validate_connection("Main", "resolume.lan", port),
                Err("Port must be between 1 and 65535."),
                "port {port:?}"
            );
        }
    }
}
