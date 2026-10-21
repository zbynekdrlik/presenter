//! #819: the inline add / edit editor shared by the settings list cards — the
//! Resolume connections, the Android stage displays and (#852 C1b) the listening
//! microphones.
//!
//! Before #819 each card had ONE permanent add/edit form at its top, and a row's
//! **Edit** only loaded that form — measured 1170 px above the row the operator had
//! just clicked. Now **Edit** turns the clicked row itself into this editor (Save /
//! Cancel), and "+ Add …" opens the same editor as a new item at the top of the
//! list. One editor per card is open at a time ([`EditTarget`]).
//!
//! [`ListEditor`] is the card's editor state machine; the card plumbing around it
//! (fetch / save / delete / rows) is `list_card`. What the editor edits is the
//! card's [`EditorForm`] (#852 C1b): `connection_form::ConnectionForm` — label,
//! host, port, an optional extra field, "Enabled" — for Resolume and Android,
//! `listen_form::MicForm` for the microphones. The draft lives at CARD level, outside
//! the keyed `<For>` rows, so the 5 s status poll can never reset what the
//! operator is typing. Its guards ([`trigger_lock`], round 2):
//! - one save at a time; while it is in flight no editor opens or switches and the
//!   operator cannot close it (every Edit and "+ Add" is disabled, Escape and Cancel
//!   are ignored) — only a row deleted elsewhere still closes its editor;
//! - unsaved changes lock the other triggers until they are saved or cancelled;
//! - a save that finishes late never closes a newer editor ([`save_is_current`]);
//! - only a real open focuses the first field, never a remount ([`takes_focus`]);
//! - focus returns to the button that opened the editor, or to "+ Add" when the
//!   row was deleted elsewhere — once that button is unlocked ([`focus_on_close`]).

use leptos::prelude::*;

use crate::utils::keyboard::KEY_ESCAPE;
use crate::utils::window::focus_is_free;

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

/// What one card's inline editor edits (#852 C1b). The target, the locks, the save
/// ticket, the focus and the message line are the same for every card
/// ([`ListEditor`]); the fields, their values and their checks are the form's.
pub(super) trait EditorForm: Copy + Send + Sync + 'static {
    /// What the fields hold: the editor has unsaved changes exactly while they
    /// differ from the values it was opened with.
    type Values: Clone + Default + PartialEq + Send + Sync + 'static;
    /// A validated submission.
    type Submission: 'static;
    /// What differs between two cards using the same form (placeholders, the
    /// extra field, the new item's port).
    type Config: Copy + Send + Sync + 'static;

    /// The form's signals (created once per card).
    fn new(config: Self::Config) -> Self;
    /// The values a new item ("+ Add …") starts with.
    fn new_values(self) -> Self::Values;
    fn load(self, values: &Self::Values);
    /// Tracked: the values on screen.
    fn values(self) -> Self::Values;
    fn values_untracked(self) -> Self::Values;
    /// Check the values in the order the operator sees the fields.
    fn validate(self, values: &Self::Values) -> Result<Self::Submission, &'static str>;
    /// The field rows. `first` is the input that takes the caret once per open;
    /// every field's `aria-describedby` is `spec.message_id`, and it is
    /// `aria-invalid` while `invalid` holds (the message line shows an error).
    fn render_fields(
        self,
        spec: EditorSpec,
        first: NodeRef<leptos::html::Input>,
        invalid: Signal<bool>,
    ) -> AnyView;
    /// Controls in front of Save / Cancel on the actions row (the connection
    /// form's "Enabled"); an empty view for none.
    fn render_action_lead(self, spec: EditorSpec) -> AnyView;
}

/// The editor's message line and its save-in-flight flag. `Copy` (signals only).
#[derive(Clone, Copy)]
struct EditorStatus {
    /// The line under the editor: "Saving changes…" or a validation / save error.
    message: RwSignal<String>,
    /// `idle` / `info` / `error` — the line's `data-state`; `error` also sets
    /// `aria-invalid` on the fields (#459).
    state: RwSignal<String>,
    /// A save is in flight: see [`trigger_lock`].
    busy: RwSignal<bool>,
}

impl Default for EditorStatus {
    fn default() -> Self {
        Self {
            message: RwSignal::new(String::new()),
            state: RwSignal::new(String::from("idle")),
            busy: RwSignal::new(false),
        }
    }
}

impl EditorStatus {
    fn show(self, state: &str, message: &str) {
        self.state.set(state.to_string());
        self.message.set(message.to_string());
    }

    fn clear(self) {
        self.show("idle", "");
    }
}

/// Why a row's Edit button, or "+ Add …", cannot open the editor right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TriggerLock {
    /// A save is in flight: no editor opens or switches (and Cancel / Escape cannot
    /// close it) until the save settles, so its late result can never land on another
    /// editor. Only a row deleted elsewhere still closes its editor meanwhile.
    Saving,
    /// The open editor has unsaved changes: save or cancel them first, instead of one
    /// click throwing them away.
    Unsaved,
    /// This trigger's own editor is already open ("+ Add" while adding).
    Open,
}

impl TriggerLock {
    /// The button's `data-lock`.
    pub(super) fn data_lock(self) -> &'static str {
        match self {
            Self::Saving => "saving",
            Self::Unsaved => "unsaved",
            Self::Open => "open",
        }
    }

    /// The button's tooltip, in the card's words.
    pub(super) fn title(self, spec: &EditorSpec) -> Option<&'static str> {
        match self {
            Self::Saving => Some(spec.saving_title),
            Self::Unsaved => Some(spec.unsaved_title),
            Self::Open => None,
        }
    }
}

/// May `trigger` (a row's Edit, or "+ Add" for [`EditTarget::New`]) open its editor
/// while `open` is the open one? A save in flight locks everything; unsaved changes
/// lock every other trigger.
fn trigger_lock(
    open: &EditTarget,
    busy: bool,
    dirty: bool,
    trigger: &EditTarget,
) -> Option<TriggerLock> {
    if busy {
        Some(TriggerLock::Saving)
    } else if *open != EditTarget::Closed && open == trigger {
        Some(TriggerLock::Open)
    } else if dirty {
        Some(TriggerLock::Unsaved)
    } else {
        None
    }
}

/// The open editor shows values other than the ones it was opened with.
fn is_dirty<V: PartialEq>(open: &EditTarget, current: &V, loaded: &V) -> bool {
    *open != EditTarget::Closed && current != loaded
}

/// An editor mount takes focus only for an open that has not been focused yet. A
/// remount of the same editor (its row re-keyed by an edit elsewhere) must not pull
/// the caret back to the first field while the operator types in another one.
fn takes_focus(focused_generation: u64, generation: u64) -> bool {
    focused_generation != generation
}

/// One card's inline-editor state machine over its form `F`. `Copy` (signals only).
pub(super) struct ListEditor<F: EditorForm> {
    editing: RwSignal<EditTarget>,
    form: F,
    status: EditorStatus,
    /// What the open editor was filled with; [`is_dirty`] compares against it.
    loaded: RwSignal<F::Values>,
    dirty: Memo<bool>,
    /// Bumped on every open: a save that finishes after the operator re-opened an
    /// editor (even on the same row) must not close or overwrite that newer editor.
    generation: RwSignal<u64>,
    /// The generation whose editor already took focus ([`takes_focus`]).
    focused: StoredValue<u64>,
    /// The item whose Edit button (or the "+ Add" button, for `New`) takes focus back
    /// when its editor closes.
    focus_return: RwSignal<Option<EditTarget>>,
}

impl<F: EditorForm> Clone for ListEditor<F> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<F: EditorForm> Copy for ListEditor<F> {}

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

impl<F: EditorForm> ListEditor<F> {
    /// A card's editor over a form built from the card's `config`.
    pub(super) fn new(config: F::Config) -> Self {
        let editing = RwSignal::new(EditTarget::Closed);
        let form = F::new(config);
        let loaded = RwSignal::new(F::Values::default());
        let dirty = Memo::new(move |_| {
            let current = form.values();
            editing.with(|open| loaded.with(|opened_with| is_dirty(open, &current, opened_with)))
        });
        Self {
            editing,
            form,
            status: EditorStatus::default(),
            loaded,
            dirty,
            generation: RwSignal::new(0),
            focused: StoredValue::new(0),
            focus_return: RwSignal::new(None),
        }
    }

    /// Open the "+ Add …" editor with the form's new-item values.
    pub(super) fn open_new(self) {
        let values = self.form.new_values();
        self.open(EditTarget::New, values);
    }

    /// Open the editor on an existing row, filled with its `values`.
    pub(super) fn open_item(self, id: String, values: F::Values) {
        self.open(EditTarget::Item(id), values);
    }

    /// The same guard the disabled buttons show: a locked trigger never opens.
    fn open(self, target: EditTarget, values: F::Values) {
        if self.trigger_lock_untracked(&target).is_some() {
            return;
        }
        self.form.load(&values);
        self.status.clear();
        self.loaded.set(values);
        self.generation.update(|g| *g += 1);
        self.editing.set(target);
    }

    /// Cancel / Escape: close without saving. Ignored while a save is in flight —
    /// the save settles the editor itself.
    pub(super) fn cancel(self) {
        if !self.status.busy.get_untracked() {
            self.close();
        }
    }

    /// Close the editor; focus goes back to the button that opened it.
    fn close(self) {
        let closed = self.editing.get_untracked();
        self.discard();
        if closed != EditTarget::Closed {
            self.focus_return.set(Some(closed));
        }
    }

    /// Close without moving focus.
    fn discard(self) {
        self.editing.set(EditTarget::Closed);
        self.status.clear();
    }

    /// The row being edited was deleted from this card.
    pub(super) fn discard_if_open_on(self, id: &str) {
        if self.editing.with_untracked(|t| t.is_item(id)) {
            self.discard();
        }
    }

    /// Every list refresh: an editor open on an item that is no longer listed (deleted
    /// in another tab) closes, and focus goes to "+ Add". Returns whether it did, so
    /// the card can tell the operator why their editor vanished.
    pub(super) fn forget_missing<'a>(self, ids: impl Iterator<Item = &'a str>) -> bool {
        let missing = self.editing.with_untracked(|t| t.is_item_missing(ids));
        if missing {
            self.discard();
            self.focus_return.set(Some(EditTarget::New));
        }
        missing
    }

    /// Tracked: the "+ Add …" editor is open.
    pub(super) fn is_new(self) -> bool {
        self.editing.with(EditTarget::is_new)
    }

    /// Tracked: the editor is open on row `id`.
    pub(super) fn is_open_on(self, id: &str) -> bool {
        self.editing.with(|t| t.is_item(id))
    }

    /// Tracked: why `trigger` (a row's Edit, or "+ Add" for `New`) is locked now.
    pub(super) fn trigger_lock(self, trigger: &EditTarget) -> Option<TriggerLock> {
        let busy = self.status.busy.get();
        let dirty = self.dirty.get();
        self.editing
            .with(|open| trigger_lock(open, busy, dirty, trigger))
    }

    fn trigger_lock_untracked(self, trigger: &EditTarget) -> Option<TriggerLock> {
        let busy = self.status.busy.get_untracked();
        let dirty = self.dirty.get_untracked();
        self.editing
            .with_untracked(|open| trigger_lock(open, busy, dirty, trigger))
    }

    /// Validate and start a save. `None` when no editor is open, a save is already in
    /// flight (held Enter, a double click), or a field is invalid (its message is
    /// shown).
    pub(super) fn begin_save(self) -> Option<(SaveTicket, F::Submission)> {
        let target = self.editing.get_untracked();
        if target == EditTarget::Closed || self.status.busy.get_untracked() {
            return None;
        }
        match self.form.validate(&self.form.values_untracked()) {
            Ok(submission) => {
                let generation = self.generation.get_untracked();
                Some((SaveTicket { target, generation }, submission))
            }
            Err(message) => {
                self.status.show("error", message);
                None
            }
        }
    }

    /// The request is going out: Save, Cancel and every trigger stay locked until
    /// [`Self::finish_save`].
    pub(super) fn mark_saving(self, message: &str) {
        self.status.busy.set(true);
        self.status.show("info", message);
    }

    /// A save finished (after a success, call it AFTER the list reload; a failure
    /// calls it at once). Success closes the editor, failure shows `error` in it —
    /// only if it is still the editor the save started from ([`save_is_current`]:
    /// same open generation AND same target). Returns whether it was, so the card
    /// can toast an error nobody saw. Save is re-enabled last, so a repeat submit
    /// cannot slip in before the editor closes.
    pub(super) fn finish_save(self, ticket: &SaveTicket, error: Option<&str>) -> bool {
        let generation = self.generation.get_untracked();
        let current = self
            .editing
            .with_untracked(|editing| save_is_current(ticket, generation, editing));
        if current {
            match error {
                None => self.close(),
                Some(message) => self.status.show("error", message),
            }
        }
        self.status.busy.set(false);
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

    /// `true` once per open ([`takes_focus`]).
    fn take_first_focus(self) -> bool {
        let generation = self.generation.get_untracked();
        let first = takes_focus(self.focused.get_value(), generation);
        if first {
            self.focused.set_value(generation);
        }
        first
    }
}

/// Is the editor a finished save started from still the one on screen? Not when it
/// was closed, moved to another item, or re-opened since (even on the same row: the
/// open generation moved on), so a late save never closes or overwrites a newer editor.
fn save_is_current(ticket: &SaveTicket, generation: u64, editing: &EditTarget) -> bool {
    ticket.generation == generation && ticket.target == *editing
}

/// Give `button` focus when the editor for `target` closes, so a keyboard user lands
/// back on the Edit / "+ Add" button they started from instead of on `<body>`.
///
/// The request is taken only once the button is unlocked: a disabled button ignores
/// `focus()`, and a row deleted elsewhere during a save closes its editor while
/// "+ Add" is still locked (`Saving`). The Effect tracks the lock, so it re-runs when
/// the save settles. The focus itself runs one task later, after the render effects
/// already queued by the same change (the button's own `disabled`, the editor's
/// removal) — a newly woken Effect is not ordered against them otherwise.
pub(super) fn focus_on_close<F: EditorForm>(
    editor: ListEditor<F>,
    button: NodeRef<leptos::html::Button>,
    target: EditTarget,
) {
    Effect::new(move || {
        let Some(el) = button.get() else {
            return;
        };
        if editor.trigger_lock(&target).is_some() || !editor.take_focus_return(&target) {
            return;
        }
        // Only while nothing has focus (it fell back to `<body>` when the editor
        // holding it was removed). A save closes its editor one request after the
        // click; if the operator has clicked into another field meanwhile, focus
        // stays there.
        leptos::task::spawn_local(async move {
            if focus_is_free() {
                let _ = el.focus();
            }
        });
    });
}

/// What differs between two cards' editors besides their fields.
#[derive(Clone, Copy)]
pub(super) struct EditorSpec {
    /// The `data-role` prefix: `host` → `host-label`, `host-submit`, `host-cancel`, …
    pub(super) role: &'static str,
    /// `id` of the message line every field's `aria-describedby` points at (#459).
    pub(super) message_id: &'static str,
    /// `data-role` of that message line.
    pub(super) message_role: &'static str,
    /// The editor heading and the Save button text for a NEW item.
    pub(super) new_title: &'static str,
    pub(super) new_submit: &'static str,
    /// The editor heading when editing an existing item in place.
    pub(super) edit_title: &'static str,
    /// The Save button of an existing item, and Cancel.
    pub(super) save_text: &'static str,
    pub(super) cancel_text: &'static str,
    /// The tooltips of a locked Edit / "+ Add" ([`TriggerLock::title`]).
    pub(super) unsaved_title: &'static str,
    pub(super) saving_title: &'static str,
}

/// The inline editor. `creating` = the "+ Add …" item (Save creates); otherwise an
/// existing row (Save updates). `on_save` validates and saves; Cancel and Escape
/// close the editor without saving. Only one editor exists in a card at a time, so
/// the field `data-role`s and the `id`s stay unique on the page.
pub(super) fn render_editor<F, S>(
    spec: EditorSpec,
    editor: ListEditor<F>,
    creating: bool,
    on_save: S,
) -> AnyView
where
    F: EditorForm,
    S: Fn() + Copy + Send + 'static,
{
    let status = editor.status;
    let role = move |field: &str| format!("{}-{field}", spec.role);
    let invalid = Signal::derive(move || status.state.get() == "error");
    let (mode, title, submit_text) = if creating {
        ("create", spec.new_title, spec.new_submit)
    } else {
        ("edit", spec.edit_title, spec.save_text)
    };

    // The operator clicked Edit / "+ Add" to type: put the caret in the first field —
    // once per open, never again when this editor is remounted.
    let first = NodeRef::<leptos::html::Input>::new();
    Effect::new(move || {
        if let Some(el) = first.get() {
            if editor.take_first_focus() {
                let _ = el.focus();
            }
        }
    });

    let on_submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        on_save();
    };
    // Escape cancels here and stops (ignored while saving): the operator page's
    // window-level Escape (close the global search) must not fire as well.
    let on_keydown = move |ev: web_sys::KeyboardEvent| {
        if ev.key() == KEY_ESCAPE {
            ev.prevent_default();
            ev.stop_propagation();
            editor.cancel();
        }
    };
    let fields = editor.form.render_fields(spec, first, invalid);
    let lead = editor.form.render_action_lead(spec);

    view! {
        <form class="settings__form settings__form--inline-editor" data-role=role("editor")
            data-mode=mode aria-labelledby=role("editor-title")
            autocomplete="off" on:submit=on_submit on:keydown=on_keydown>
            <h3 class="settings__editor-title" id=role("editor-title")
                data-role=role("editor-title")>{title}</h3>
            {fields}
            <div class="settings__form-row settings__form-row--inline">
                {lead}
                <div class="settings__form-actions">
                    <button type="submit" class="settings__button settings__button--primary"
                        data-role=role("submit") prop:disabled=move || status.busy.get()>
                        {submit_text}
                    </button>
                    <button type="button" class="settings__button settings__button--ghost"
                        data-role=role("cancel") prop:disabled=move || status.busy.get()
                        on:click=move |_| editor.cancel()>{spec.cancel_text}</button>
                </div>
            </div>
            <p id=spec.message_id class="settings__form-status" data-role=spec.message_role
                data-state=move || status.state.get()>
                {move || status.message.get()}
            </p>
        </form>
    }
    .into_any()
}

#[cfg(test)]
mod tests {
    use super::super::connection_form::{DraftValues, SAVING_TITLE, UNSAVED_TITLE};
    use super::{
        is_dirty, save_is_current, takes_focus, trigger_lock, EditTarget, EditorSpec, SaveTicket,
        TriggerLock,
    };

    #[test]
    fn a_late_save_only_settles_the_editor_it_started_from() {
        let ticket = SaveTicket {
            target: EditTarget::Item("host-1".into()),
            generation: 4,
        };
        // Still the same editor: close it / show the error in it.
        assert!(save_is_current(
            &ticket,
            4,
            &EditTarget::Item("host-1".into())
        ));
        // The operator re-opened the SAME row meanwhile (generation moved on).
        assert!(!save_is_current(
            &ticket,
            5,
            &EditTarget::Item("host-1".into())
        ));
        // …closed it, or opened another row / the new item.
        assert!(!save_is_current(&ticket, 4, &EditTarget::Closed));
        assert!(!save_is_current(
            &ticket,
            5,
            &EditTarget::Item("host-2".into())
        ));
        assert!(!save_is_current(&ticket, 5, &EditTarget::New));

        let new_item = SaveTicket {
            target: EditTarget::New,
            generation: 7,
        };
        assert!(save_is_current(&new_item, 7, &EditTarget::New));
        assert!(!save_is_current(&new_item, 8, &EditTarget::New));
    }

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
    fn a_save_in_flight_locks_every_trigger() {
        // Round 2, item 4: no editor opens or switches until the save settles.
        let open = EditTarget::Item("a".into());
        for dirty in [false, true] {
            for trigger in [EditTarget::Item("b".into()), EditTarget::New] {
                assert_eq!(
                    trigger_lock(&open, true, dirty, &trigger),
                    Some(TriggerLock::Saving)
                );
            }
        }
        // Even once the editor is gone (its row was deleted elsewhere mid-save).
        assert_eq!(
            trigger_lock(&EditTarget::Closed, true, false, &EditTarget::New),
            Some(TriggerLock::Saving)
        );
    }

    #[test]
    fn unsaved_changes_lock_every_other_trigger() {
        // Round 2, item 5: one click must not throw typed changes away.
        let open = EditTarget::Item("a".into());
        for trigger in [EditTarget::Item("b".into()), EditTarget::New] {
            assert_eq!(
                trigger_lock(&open, false, true, &trigger),
                Some(TriggerLock::Unsaved)
            );
        }
        // A clean editor locks nothing: Edit on another row switches in one click.
        assert_eq!(
            trigger_lock(&open, false, false, &EditTarget::Item("b".into())),
            None
        );
        assert_eq!(trigger_lock(&open, false, false, &EditTarget::New), None);
        assert_eq!(
            trigger_lock(&EditTarget::Closed, false, false, &EditTarget::New),
            None
        );
    }

    #[test]
    fn the_open_editors_own_trigger_is_locked() {
        // "+ Add" while the new item is open, clean or not.
        for dirty in [false, true] {
            assert_eq!(
                trigger_lock(&EditTarget::New, false, dirty, &EditTarget::New),
                Some(TriggerLock::Open)
            );
        }
    }

    const SPEC: EditorSpec = EditorSpec {
        role: "host",
        message_id: "resolume-form-status",
        message_role: "form-status",
        new_title: "New Resolume connection",
        new_submit: "Add connection",
        edit_title: "Edit Resolume connection",
        save_text: "Save",
        cancel_text: "Cancel",
        unsaved_title: UNSAVED_TITLE,
        saving_title: SAVING_TITLE,
    };

    #[test]
    fn a_lock_says_why_on_the_button() {
        assert_eq!(TriggerLock::Unsaved.title(&SPEC), Some(UNSAVED_TITLE));
        assert_eq!(UNSAVED_TITLE, "Save or cancel the open editor first");
        assert_eq!(TriggerLock::Saving.title(&SPEC), Some(SAVING_TITLE));
        assert_eq!(TriggerLock::Open.title(&SPEC), None);
        assert_eq!(TriggerLock::Saving.data_lock(), "saving");
        assert_eq!(TriggerLock::Unsaved.data_lock(), "unsaved");
        assert_eq!(TriggerLock::Open.data_lock(), "open");
    }

    fn values(label: &str, port: &str, extra: &str) -> DraftValues {
        DraftValues {
            label: label.into(),
            host: "sd1l.lan".into(),
            port: port.into(),
            enabled: false,
            extra: extra.into(),
        }
    }

    #[test]
    fn the_draft_is_dirty_only_when_it_differs_from_what_was_loaded() {
        let loaded = values("Stage", "5555", "com.tcl.browser");
        let open = EditTarget::Item("d1".into());
        assert!(!is_dirty(&open, &loaded, &loaded));
        // A closed editor is never dirty, whatever its stale signals hold.
        assert!(!is_dirty(
            &EditTarget::Closed,
            &values("x", "1", ""),
            &loaded
        ));
        // Every field counts, the extra field included.
        let changed = [
            DraftValues {
                label: "Other".into(),
                ..loaded.clone()
            },
            DraftValues {
                host: "other.lan".into(),
                ..loaded.clone()
            },
            DraftValues {
                port: "5556".into(),
                ..loaded.clone()
            },
            DraftValues {
                enabled: true,
                ..loaded.clone()
            },
            DraftValues {
                extra: "com.example/.Main".into(),
                ..loaded.clone()
            },
        ];
        for current in &changed {
            assert!(is_dirty(&open, current, &loaded), "{current:?}");
            assert!(is_dirty(&EditTarget::New, current, &loaded), "{current:?}");
        }
    }

    #[test]
    fn only_the_first_mount_of_an_open_takes_focus() {
        // Round 2, item 2: generation 0 = nothing opened yet, nothing focused yet.
        assert!(takes_focus(0, 1));
        // The same open remounted (its row was re-keyed): leave the caret alone.
        assert!(!takes_focus(1, 1));
        // The next real open focuses again.
        assert!(takes_focus(1, 2));
    }
}
