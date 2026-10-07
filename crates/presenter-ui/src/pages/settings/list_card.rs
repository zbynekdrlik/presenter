//! #819: the list card shared by the Resolume connections and the Android stage
//! displays.
//!
//! [`ListCard`] owns everything the two cards used to copy between them, so its
//! guards live in one place:
//! - the list fetch, numbered by `list_sync::ResponseOrder` (a poll sent before a
//!   save and answered after it is dropped) and followed by `forget_missing` (an
//!   editor on a row deleted elsewhere closes, with a toast);
//! - the 5 s status poll;
//! - save → reload → `finish_save`, and delete;
//! - every keyed row: its live status / timestamp `Memo`s, the inline editor or the
//!   summary, the Edit / Delete buttons with their trigger locks and focus return.
//!
//! A card supplies its item type and API ([`CardItem`]), its copy ([`CardText`]) and
//! the card-specific parts of a row ([`RowParts`]).

use std::hash::Hash;

use leptos::prelude::*;

use super::host_editor::{
    focus_on_close, render_connection_editor, DraftValues, EditTarget, EditorSpec, ListEditor,
    Submission, TriggerLock,
};
use super::list_sync::ResponseOrder;
use super::row_status::updated_created;
use super::{ToastHandle, STATUS_REFRESH_MS};
use crate::api::ApiError;
use crate::components::modal::confirm;

/// One item of a list card, plus the API calls that list, save and delete it.
// `async fn` in a crate-private trait: the futures run on `spawn_local` and never
// need to be `Send`.
#[allow(async_fn_in_trait)]
pub(super) trait CardItem: Clone + Send + Sync + 'static {
    /// The live status the row reads through a `Memo`.
    type Status: Clone + PartialEq + Send + Sync + 'static;
    /// The row's `<For>` key: the id plus every field the row shows or edits. Never
    /// the live status, so a poll never rebuilds a row (or touches its editor).
    type Key: Eq + Hash + 'static;

    fn id(&self) -> &str;
    fn key(&self) -> Self::Key;
    fn label(&self) -> &str;
    fn host(&self) -> &str;
    fn port(&self) -> u16;
    fn is_enabled(&self) -> bool;
    /// The card's extra editor field (Android's launch package); empty without one.
    fn extra(&self) -> &str {
        ""
    }
    fn status(&self) -> Option<Self::Status>;
    fn created_at(&self) -> &str;
    fn updated_at(&self) -> &str;

    async fn list() -> Result<Vec<Self>, ApiError>;
    /// Create (`id` is `None`) or update an item from a validated editor submission.
    async fn save(id: Option<String>, submission: Submission) -> Result<(), ApiError>;
    async fn delete(id: String) -> Result<(), ApiError>;
}

/// A list card's copy and roles. `editor.role` prefixes the shared `data-role`s:
/// `<role>-add`, `<role>-new-item`, `<role>-empty`, `<role>-edit`, `<role>-delete`.
pub(super) struct CardText {
    pub(super) editor: EditorSpec,
    /// The port a new item starts with.
    pub(super) default_port: u16,
    /// `data-role` of the `<ul>`.
    pub(super) list_role: &'static str,
    pub(super) add_text: &'static str,
    pub(super) empty_text: &'static str,
    /// Names the item in the delete confirmation when its label is unknown.
    pub(super) fallback_name: &'static str,
    /// The editor's message line while an update / a create is in flight.
    pub(super) saving: &'static str,
    pub(super) creating: &'static str,
    /// Toasts after a successful update / create / delete.
    pub(super) updated: &'static str,
    pub(super) added: &'static str,
    pub(super) deleted: &'static str,
    /// Prefixes of the failure messages (the API error follows).
    pub(super) save_failed: &'static str,
    pub(super) delete_failed: &'static str,
    /// The toast when the row being edited was deleted elsewhere.
    pub(super) removed_elsewhere: &'static str,
}

/// The card-specific parts of a row's summary.
pub(super) struct RowParts {
    /// After `host:port` on the address line: the status badge, then an aside.
    pub(super) line: AnyView,
    /// Muted lines above "Updated … · Created …".
    pub(super) meta: Option<AnyView>,
    /// The (reactive) warning under "Updated … · Created …".
    pub(super) warning: AnyView,
    /// Buttons before Edit / Delete.
    pub(super) actions: AnyView,
}

/// The editor values of an item.
fn draft_values<T: CardItem>(item: &T) -> DraftValues {
    DraftValues {
        label: item.label().to_string(),
        host: item.host().to_string(),
        port: item.port().to_string(),
        enabled: item.is_enabled(),
        extra: item.extra().to_string(),
    }
}

/// A list card's state. `Copy` (signals and a `'static` reference only).
pub(super) struct ListCard<T: CardItem> {
    /// The last applied list. Written only by [`Self::fetch`]; cards read it through
    /// [`Self::items`].
    items: RwSignal<Vec<T>>,
    editor: ListEditor,
    order: ResponseOrder,
    toast: ToastHandle,
    text: &'static CardText,
}

impl<T: CardItem> Clone for ListCard<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: CardItem> Copy for ListCard<T> {}

impl<T: CardItem> ListCard<T> {
    /// The card's state; loads the list and starts the 5 s status poll.
    pub(super) fn new(toast: ToastHandle, text: &'static CardText) -> Self {
        let card = Self {
            items: RwSignal::new(Vec::new()),
            editor: ListEditor::new(text.editor.extra),
            order: ResponseOrder::default(),
            toast,
            text,
        };
        card.reload();
        gloo_timers::callback::Interval::new(STATUS_REFRESH_MS, move || card.reload()).forget();
        card
    }

    fn reload(self) {
        leptos::task::spawn_local(self.fetch());
    }

    /// The list as last applied, read-only (the card header counts it): only
    /// [`Self::fetch`] writes it, so no card can bypass the response order.
    pub(super) fn items(self) -> ReadSignal<Vec<T>> {
        self.items.read_only()
    }

    /// Every list fetch goes through here (poll, save, delete, a card's own Test):
    /// numbered, so a slow older response never overwrites a newer one, and an editor
    /// left open on an item deleted elsewhere closes, saying so.
    pub(super) async fn fetch(self) {
        let seq = self.order.begin();
        let Ok(list) = T::list().await else {
            return;
        };
        if !self.order.accept(seq) {
            return;
        }
        if self.editor.forget_missing(list.iter().map(T::id)) {
            self.toast.show(self.text.removed_elsewhere, "info");
        }
        self.items.set(list);
    }

    fn open_new(self) {
        self.editor.open_new(self.text.default_port);
    }

    fn open_edit(self, id: &str) {
        let values = self
            .items
            .with_untracked(|list| list.iter().find(|item| item.id() == id).map(draft_values));
        if let Some(values) = values {
            self.editor.open_item(id.to_string(), values);
        }
    }

    /// Save the open editor: request, reload the list, THEN settle the editor
    /// (`finish_save` re-enables Save last and closes only the editor it started from).
    fn save(self) {
        let Some((ticket, submission)) = self.editor.begin_save() else {
            return;
        };
        let updating = ticket.updating();
        let creating = updating.is_none();
        let text = self.text;
        self.editor
            .mark_saving(if creating { text.creating } else { text.saving });
        leptos::task::spawn_local(async move {
            match T::save(updating, submission).await {
                Ok(()) => {
                    self.fetch().await;
                    self.toast
                        .show(if creating { text.added } else { text.updated }, "success");
                    self.editor.finish_save(&ticket, None);
                }
                Err(err) => {
                    let message = format!("{} {err}", text.save_failed);
                    if !self.editor.finish_save(&ticket, Some(&message)) {
                        self.toast.show(&message, "error");
                    }
                }
            }
        });
    }

    fn delete(self, id: String) {
        let name = self
            .items
            .with_untracked(|list| {
                list.iter()
                    .find(|item| item.id() == id)
                    .map(|item| item.label().to_string())
            })
            .unwrap_or_else(|| self.text.fallback_name.to_string());
        if !confirm(&format!("Remove {name}? Presenter will stop reconnecting.")) {
            return;
        }
        leptos::task::spawn_local(async move {
            match T::delete(id.clone()).await {
                Ok(()) => {
                    self.editor.discard_if_open_on(&id);
                    self.fetch().await;
                    self.toast.show(self.text.deleted, "success");
                }
                Err(err) => self
                    .toast
                    .show(&format!("{} {err}", self.text.delete_failed), "error"),
            }
        });
    }

    /// The "+ Add …" toolbar and the list: the new-item editor first, then the rows,
    /// keyed by [`CardItem::key`]. `row` builds each row's card-specific parts.
    pub(super) fn render_list<R>(self, row: R) -> impl IntoView
    where
        R: Fn(&T, Memo<Option<T::Status>>) -> RowParts + Copy + Send + Sync + 'static,
    {
        let editor = self.editor;
        let text = self.text;
        let spec = text.editor;
        let role = spec.role;
        let list_role = text.list_role;
        let add_text = text.add_text;
        let empty_text = text.empty_text;
        let items = self.items;
        let save = move || self.save();
        let adding = move || editor.is_new();
        let add_lock = Memo::new(move |_| editor.trigger_lock(&EditTarget::New));
        let add_ref = NodeRef::<leptos::html::Button>::new();
        focus_on_close(editor, add_ref, EditTarget::New);
        let each_item = move || items.get();
        let row_view = move |item: T| self.render_row(item, row);

        view! {
            <>
                <div class="settings__list-toolbar">
                    <button type="button" class="settings__button settings__button--primary"
                        node_ref=add_ref data-role=format!("{role}-add")
                        data-lock=move || add_lock.get().map(TriggerLock::data_lock)
                        title=move || add_lock.get().and_then(TriggerLock::title)
                        prop:disabled=move || add_lock.get().is_some()
                        on:click=move |_| self.open_new()>{add_text}</button>
                </div>
                <ul class="settings__list" data-role=list_role>
                    <Show when=adding>
                        <li class="settings__list-item" data-role=format!("{role}-new-item")
                            data-editing="true">
                            {render_connection_editor(spec, editor, true, save)}
                        </li>
                    </Show>
                    <Show when=move || items.with(Vec::is_empty) && !adding()>
                        <li class="settings__list-empty" data-role=format!("{role}-empty")>
                            {empty_text}
                        </li>
                    </Show>
                    <For each=each_item key=T::key children=row_view />
                </ul>
            </>
        }
    }

    /// One keyed row: the inline editor while it is open on this item, its summary
    /// otherwise. Status and timestamps are re-read on every poll through `Memo`s (ui
    /// skill: key on identity, read the changing state through a Memo).
    fn render_row<R>(self, item: T, row: R) -> impl IntoView
    where
        R: Fn(&T, Memo<Option<T::Status>>) -> RowParts + Copy + Send + Sync + 'static,
    {
        let editor = self.editor;
        let spec = self.text.editor;
        let items = self.items;
        let id = item.id().to_string();
        let editing_this = {
            let id = id.clone();
            Memo::new(move |_| editor.is_open_on(&id))
        };
        let status = {
            let id = id.clone();
            Memo::new(move |_| {
                items.with(|list| list.iter().find(|x| x.id() == id).and_then(T::status))
            })
        };
        let timestamps = {
            let id = id.clone();
            Memo::new(move |_| {
                items.with(|list| {
                    list.iter()
                        .find(|x| x.id() == id)
                        .map(|x| updated_created(x.updated_at(), x.created_at()))
                        .unwrap_or_default()
                })
            })
        };
        let enabled = item.is_enabled().to_string();
        let save = move || self.save();
        let summary = move || self.render_summary(&item, status, timestamps, row);

        view! {
            <li class="settings__list-item" data-id=id data-enabled=enabled
                data-editing=move || editing_this.get().to_string()>
                {move || if editing_this.get() {
                    render_connection_editor(spec, editor, false, save)
                } else {
                    summary()
                }}
            </li>
        }
    }

    /// A row's summary: label, `host:port` + the card's line parts, the card's meta,
    /// "Updated … · Created …", the warning, then the card's buttons + Edit / Delete.
    fn render_summary<R>(
        self,
        item: &T,
        status: Memo<Option<T::Status>>,
        timestamps: Memo<String>,
        row: R,
    ) -> AnyView
    where
        R: Fn(&T, Memo<Option<T::Status>>) -> RowParts,
    {
        let editor = self.editor;
        let role = self.text.editor.role;
        let id = item.id().to_string();
        let edit_ref = NodeRef::<leptos::html::Button>::new();
        focus_on_close(editor, edit_ref, EditTarget::Item(id.clone()));
        let lock = {
            let target = EditTarget::Item(id.clone());
            Memo::new(move |_| editor.trigger_lock(&target))
        };
        let parts = row(item, status);
        let label = item.label().to_string();
        let host = item.host().to_string();
        let port = format!(":{}", item.port());
        let (id_edit, id_delete) = (id.clone(), id);

        view! {
            <div class="settings__list-summary">
                <div class="settings__list-primary">
                    <div class="settings__list-title">
                        <span class="settings__host-label">{label}</span>
                    </div>
                    <p class="settings__list-line">
                        <span class="settings__host-addr">
                            <code>{host}</code>
                            {port}
                        </span>
                        {parts.line}
                    </p>
                    {parts.meta}
                    <p class="settings__list-meta settings__list-meta--muted">
                        {move || timestamps.get()}
                    </p>
                    {parts.warning}
                </div>
                <div class="settings__list-actions">
                    {parts.actions}
                    <button type="button" class="settings__button settings__button--ghost settings__button--small"
                        node_ref=edit_ref data-role=format!("{role}-edit") data-id=id_edit.clone()
                        data-lock=move || lock.get().map(TriggerLock::data_lock)
                        title=move || lock.get().and_then(TriggerLock::title)
                        prop:disabled=move || lock.get().is_some()
                        on:click=move |_| self.open_edit(&id_edit)>"Edit"</button>
                    <button type="button" class="settings__button settings__button--danger settings__button--small"
                        data-role=format!("{role}-delete") data-id=id_delete.clone()
                        on:click=move |_| self.delete(id_delete.clone())>"Delete"</button>
                </div>
            </div>
        }
        .into_any()
    }
}
