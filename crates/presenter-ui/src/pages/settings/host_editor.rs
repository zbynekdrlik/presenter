//! #819: the inline add / edit editor shared by the Resolume and Android list cards.
//!
//! Before #819 each card had ONE permanent add/edit form at its top, and a row's
//! **Edit** only loaded that form — measured 1170 px above the row the operator had
//! just clicked. Now **Edit** turns the clicked row itself into this editor (Save /
//! Cancel), and "+ Add …" opens the same editor as a new item at the top of the
//! list. One editor per card is open at a time ([`EditTarget`]): opening another one
//! discards the first one's unsaved changes.
//!
//! [`ListEditor`] is the card's whole editor state and save flow, so both cards share
//! the same guards: the draft lives at CARD level, outside the keyed `<For>` rows (the
//! 5 s status poll can never reset what the operator is typing); one save at a time;
//! a save that finishes late never closes a newer editor; focus returns to the button
//! that opened the editor.

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

    /// `true` when this is an item that is not among the listed `ids` any more.
    pub(super) fn is_item_missing<'a>(&self, mut ids: impl Iterator<Item = &'a str>) -> bool {
        match self {
            Self::Item(id) => !ids.any(|listed| listed == id),
            Self::Closed | Self::New => false,
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
    fn load(self, label: &str, host: &str, port: u16, enabled: bool) {
        self.label.set(label.to_string());
        self.host.set(host.to_string());
        self.port.set(port.to_string());
        self.enabled.set(enabled);
        self.clear_message();
    }

    fn validated(self) -> Result<ConnectionFields, &'static str> {
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

    fn clear_message(self) {
        self.show("idle", "");
    }
}

/// One card's inline-editor state: which item the single editor is open on, its
/// draft, and where focus goes back to when it closes. `Copy` (signals only).
#[derive(Clone, Copy)]
pub(super) struct ListEditor {
    pub(super) editing: RwSignal<EditTarget>,
    pub(super) draft: ConnectionDraft,
    /// Bumped on every open: a save that finishes after the operator re-opened an
    /// editor (even on the same row) must not close or overwrite that newer editor.
    generation: RwSignal<u64>,
    /// The item whose Edit button (or the "+ Add" button, for `New`) takes focus back
    /// when its editor closes.
    focus_return: RwSignal<Option<EditTarget>>,
}

impl Default for ListEditor {
    fn default() -> Self {
        Self {
            editing: RwSignal::new(EditTarget::Closed),
            draft: ConnectionDraft::default(),
            generation: RwSignal::new(0),
            focus_return: RwSignal::new(None),
        }
    }
}

/// What a save in flight was started from.
pub(super) struct SaveTicket {
    target: EditTarget,
    generation: u64,
}

impl SaveTicket {
    /// The id the save updates; `None` creates a new item.
    pub(super) fn updating(&self) -> Option<String> {
        self.target.item_id().map(str::to_string)
    }
}

impl ListEditor {
    /// Open the empty "+ Add …" editor.
    pub(super) fn open_new(self, default_port: u16) {
        self.draft.load("", "", default_port, true);
        self.open(EditTarget::New);
    }

    /// Open the editor on an existing row, filled with its values.
    pub(super) fn open_item(self, id: String, label: &str, host: &str, port: u16, enabled: bool) {
        self.draft.load(label, host, port, enabled);
        self.open(EditTarget::Item(id));
    }

    fn open(self, target: EditTarget) {
        self.generation.update(|g| *g += 1);
        self.editing.set(target);
    }

    /// Close the editor (Cancel, Escape, a successful save); focus goes back to the
    /// button that opened it.
    pub(super) fn close(self) {
        let closed = self.editing.get_untracked();
        self.discard();
        if closed != EditTarget::Closed {
            self.focus_return.set(Some(closed));
        }
    }

    /// Close without moving focus — the item it was open on is gone.
    fn discard(self) {
        self.editing.set(EditTarget::Closed);
        self.draft.clear_message();
    }

    /// The row being edited was deleted from this card.
    pub(super) fn discard_if_open_on(self, id: &str) {
        if self.editing.with_untracked(|t| t.is_item(id)) {
            self.discard();
        }
    }

    /// Every list refresh: an editor open on an item that is no longer listed
    /// (deleted here or in another tab) closes with it.
    pub(super) fn forget_missing<'a>(self, ids: impl Iterator<Item = &'a str>) {
        if self.editing.with_untracked(|t| t.is_item_missing(ids)) {
            self.discard();
        }
    }

    /// Tracked: the "+ Add …" editor is open.
    pub(super) fn is_new(self) -> bool {
        self.editing.with(EditTarget::is_new)
    }

    /// Tracked: the editor is open on row `id`.
    pub(super) fn is_open_on(self, id: &str) -> bool {
        self.editing.with(|t| t.is_item(id))
    }

    /// Validate and start a save. `None` when no editor is open, a save is already in
    /// flight (held Enter, a double click), or a shared field is invalid (its message is
    /// shown). The card may still reject its own fields before `mark_saving`.
    pub(super) fn begin_save(self) -> Option<(SaveTicket, ConnectionFields)> {
        let target = self.editing.get_untracked();
        if target == EditTarget::Closed || self.draft.busy.get_untracked() {
            return None;
        }
        match self.draft.validated() {
            Ok(fields) => {
                let generation = self.generation.get_untracked();
                Some((SaveTicket { target, generation }, fields))
            }
            Err(message) => {
                self.draft.show("error", message);
                None
            }
        }
    }

    /// The request is going out: Save stays disabled until [`Self::finish_save`].
    pub(super) fn mark_saving(self, message: &str) {
        self.draft.busy.set(true);
        self.draft.show("info", message);
    }

    /// A save finished (call it AFTER the list reload). Success closes the editor,
    /// failure shows `error` in it — only if it is still the editor the save started
    /// from. Returns whether it was, so the card can toast an error nobody saw. Save is
    /// re-enabled last, so a repeat submit cannot slip in before the editor closes.
    pub(super) fn finish_save(self, ticket: &SaveTicket, error: Option<&str>) -> bool {
        let current = self.generation.get_untracked() == ticket.generation
            && self.editing.get_untracked() == ticket.target;
        if current {
            match error {
                None => self.close(),
                Some(message) => self.draft.show("error", message),
            }
        }
        self.draft.busy.set(false);
        current
    }

    /// `true` (once) when the editor for `target` just closed. Tracked, so an Effect
    /// calling it re-runs on every close.
    fn take_focus_return(self, target: &EditTarget) -> bool {
        let wanted = self.focus_return.with(|r| r.as_ref() == Some(target));
        if wanted {
            self.focus_return.set(None);
        }
        wanted
    }
}

/// Give `button` focus when the editor for `target` closes, so a keyboard user lands
/// back on the Edit / "+ Add" button they started from instead of on `<body>`.
pub(super) fn focus_on_close(
    editor: ListEditor,
    button: NodeRef<leptos::html::Button>,
    target: EditTarget,
) {
    Effect::new(move || {
        if let Some(el) = button.get() {
            if editor.take_focus_return(&target) {
                let _ = el.focus();
            }
        }
    });
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
/// existing row (Save updates). `on_save` validates and saves; Cancel and Escape
/// close the editor without saving. Only one editor exists in a card at a time, so
/// the field `data-role`s and the `id`s stay unique on the page.
pub(super) fn render_connection_editor<S>(
    spec: EditorSpec,
    editor: ListEditor,
    creating: bool,
    on_save: S,
) -> AnyView
where
    S: Fn() + Copy + Send + 'static,
{
    let draft = editor.draft;
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
    // Escape cancels here and stops: the operator page's window-level Escape
    // (close the global search) must not fire as well.
    let on_keydown = move |ev: web_sys::KeyboardEvent| {
        if ev.key() == KEY_ESCAPE {
            ev.prevent_default();
            ev.stop_propagation();
            editor.close();
        }
    };

    let extra = spec
        .extra
        .map(|field| render_extra_field(field, spec.message_id, draft));

    view! {
        <form class="settings__form settings__form--inline-editor" data-role=role("editor")
            data-mode=mode aria-labelledby=role("editor-title")
            autocomplete="off" on:submit=on_submit on:keydown=on_keydown>
            <h3 class="settings__editor-title" id=role("editor-title")
                data-role=role("editor-title")>{title}</h3>
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
                        data-role=role("cancel") on:click=move |_| editor.close()>"Cancel"</button>
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
    fn an_editor_on_a_row_that_vanished_from_the_list_is_missing() {
        let row = EditTarget::Item("host-2".into());
        assert!(!row.is_item_missing(["host-1", "host-2"].into_iter()));
        assert!(row.is_item_missing(["host-1", "host-3"].into_iter()));
        assert!(row.is_item_missing(std::iter::empty()));
        // The new item and a closed editor never belong to a listed row.
        assert!(!EditTarget::New.is_item_missing(std::iter::empty()));
        assert!(!EditTarget::Closed.is_item_missing(std::iter::empty()));
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
