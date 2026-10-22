//! A row's two texts and how they reach its plate's saves (#871).
//!
//! The row owns only its input DRAFTS. It COMMITS them to the store (on
//! leaving a field, the window or the page, on Zobraziť, when an editor
//! output switch takes the row away) and drops a draft once the store
//! settles that text. Everything else about a save (the PATCH, the order,
//! a failure, the retries, a Zobraziť waiting for it) belongs to the
//! store's controller (`save_control.rs`).

use leptos::prelude::*;
use web_sys::Element;

use super::focus::{field_blur, rows_moving, Blur};
use super::save_control::{Settle, Texts, DELETED_UNSAVED};
use super::{NameplateStore, PlateSave};

/// One editable text of a row (name or role) with its draft.
///
/// #871 review: the field shows the DRAFT while the operator has typed and not
/// saved yet, else the saved text. A list refresh (an own save of the other
/// field, its `StreamNameplatesChanged` echo, another operator, Companion, a
/// reconnect) can never overwrite half-typed text: `saved` is a `Memo` (it
/// notifies only when THIS plate's text changes), and while a draft exists the
/// shown value stays the draft even when the saved text does change. A draft
/// ends when the store settles its text, or on a commit of the saved texts.
#[derive(Clone, Copy)]
pub(super) struct PlateText {
    pub(super) saved: Memo<String>,
    pub(super) draft: RwSignal<Option<String>>,
    pub(super) shown: Memo<String>,
}

impl PlateText {
    pub(super) fn new(saved: Memo<String>) -> Self {
        let draft = RwSignal::new(None::<String>);
        let shown = Memo::new(move |_| draft.get().unwrap_or_else(|| saved.get()));
        Self {
            saved,
            draft,
            shown,
        }
    }

    /// The text a commit sends: the draft, else the saved text, trimmed.
    pub(super) fn value(self) -> String {
        self.draft
            .get_untracked()
            .unwrap_or_else(|| self.saved.get_untracked())
            .trim()
            .to_string()
    }

    /// Drop the draft when it is `text` (the store settled that text; a
    /// draft typed on since stays).
    fn settle(self, text: &str) {
        self.draft.maybe_update(|draft| {
            let done = draft.as_deref().is_some_and(|d| d.trim() == text);
            if done {
                *draft = None;
            }
            done
        });
    }
}

/// Why a row commits its drafts.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Why {
    /// The operator left the field inside the page (Tab, a click elsewhere,
    /// Enter, Zobraziť): an empty name is refused with a toast.
    FieldLeft,
    /// The focus went away without the operator leaving the field (the
    /// window lost it: alt-tab, OBS, Companion, another tab), or an editor
    /// output switch took the row away: an empty name is kept as typed.
    FocusAway,
    /// The page went hidden or is being closed (`pagehide`): as
    /// `FocusAway`, and the commit is urgent (the page may never run again).
    PageHidden,
}

impl Why {
    /// For the log lines.
    fn reason(self) -> &'static str {
        match self {
            Self::FieldLeft => "the field was left",
            Self::FocusAway => "the focus went away",
            Self::PageHidden => "the page went hidden",
        }
    }
}

/// One row's drafts and their way into the store.
#[derive(Clone, Copy)]
pub(super) struct RowSave {
    store: NameplateStore,
    id: i64,
    pub(super) name: PlateText,
    pub(super) role: PlateText,
    /// What the store says about the plate's saves.
    state: Memo<PlateSave>,
    /// The output the row was listed for (the editor can switch outputs).
    slug: StoredValue<String>,
}

impl RowSave {
    pub(super) fn new(store: NameplateStore, id: i64, name: PlateText, role: PlateText) -> Self {
        let save = Self {
            store,
            id,
            name,
            role,
            state: Memo::new(move |_| store.save_state(id)),
            slug: StoredValue::new(store.output_slug.get_untracked()),
        };
        save.follow_settles();
        save
    }

    /// A save of the plate failed and its newest texts are not saved yet.
    pub(super) fn unsaved(self) -> bool {
        self.state.with(|state| state.unsaved)
    }

    /// Drop each draft whose text the store settled, once per settle.
    fn follow_settles(self) {
        let state = self.state;
        Effect::new(move |seen: Option<u64>| {
            let Some((number, settle)) = state.with(|s| s.settled.clone()) else {
                return seen.unwrap_or(0);
            };
            if seen != Some(number) {
                match settle {
                    Settle::Gone => {
                        self.name.draft.set(None);
                        self.role.draft.set(None);
                    }
                    Settle::Fields { name, role } => {
                        if let Some(name) = name {
                            self.name.settle(&name);
                        }
                        if let Some(role) = role {
                            self.role.settle(&role);
                        }
                    }
                }
            }
            number
        });
    }

    fn has_drafts(self) -> bool {
        self.name.draft.with_untracked(Option::is_some)
            || self.role.draft.with_untracked(Option::is_some)
    }

    /// The texts the row shows as saved.
    fn saved(self) -> Texts {
        (
            self.name.saved.get_untracked(),
            self.role.saved.get_untracked(),
        )
    }

    /// Commit the row's drafts: one save carries both texts, so committing
    /// the role never sends a stale name. An empty name is handled here,
    /// before anything is sent: leaving the field refuses it (a toast, the
    /// old name stays); otherwise the operator is retyping it, so it stays
    /// as typed and a role typed in the row goes with the newest name.
    pub(super) fn commit(self, why: Why) {
        if !self.has_drafts() {
            return;
        }
        let id = self.id;
        let saved = self.saved();
        let mut name = self.name.value();
        let role = self.role.value();
        let mut name_kept = false;
        if name.is_empty() {
            // The old name: the newest one committed, else the saved one.
            let newest = self
                .store
                .desired(id)
                .map_or_else(|| saved.0.clone(), |(name, _)| name);
            match why {
                Why::FieldLeft => {
                    self.store
                        .notify("Meno nesmie byť prázdne.".to_string(), "error");
                    let busy = self.store.is_saving(id);
                    self.name.draft.set(busy.then(|| newest.clone()));
                    leptos::logging::log!(
                        "nameplates: plate {id} name is empty; the old name stays ({})",
                        why.reason()
                    );
                }
                _ if self.role.draft.with_untracked(Option::is_some) => name_kept = true,
                _ => {
                    leptos::logging::log!(
                        "nameplates: plate {id} name is empty; kept as typed, not saved ({})",
                        why.reason()
                    );
                    return;
                }
            }
            name = newest;
        }
        let urgent = why == Why::PageHidden;
        self.store
            .save(id, (name, role), saved, urgent, why.reason());
        if name_kept {
            leptos::logging::log!(
                "nameplates: plate {id} name is empty; kept as typed, the role sent with the newest name ({})",
                why.reason()
            );
        }
    }

    /// Zobraziť. A phone may not blur the field on a button tap (iOS
    /// Safari), so the row commits its drafts first; the store then puts
    /// the plate on air at once, or once its newest texts are saved.
    pub(super) fn show(self) {
        self.commit(Why::FieldLeft);
        self.store.show_saved(self.id);
    }

    /// A row field lost the focus: commit the row, unless the field was only
    /// MOVED.
    ///
    /// #871 review: a reorder (another operator, Companion, this page's ↑ / ↓
    /// or drag) moves the row with `insertBefore`, which drops the focus of
    /// the field being typed in, and the browser fires `blur` on the way.
    /// Whether rows are moving is read NOW, in the blur handler
    /// ([`rows_moving`]); the decision waits for the focus restore of
    /// `focus::keep_focus` (this task is queued after it). A blur during a
    /// move whose field holds the focus again was only a move and commits
    /// nothing (the text stays a draft). Any other blur commits: a field still
    /// active outside a move lost the WINDOW's focus (alt-tab, OBS, another
    /// tab), whatever the browser's `hasFocus()` says at that moment; a field
    /// that is no longer active was left (Tab, a click elsewhere, Enter). A
    /// field whose row is gone commits nothing ([`Self::leave`] did).
    pub(super) fn commit_on_leave(self, field: Element) {
        let id = self.id;
        let moving = rows_moving();
        leptos::task::spawn_local(async move {
            if !field.is_connected() || self.name.draft.is_disposed() {
                leptos::logging::log!("nameplates: plate {id} row is gone; nothing to save");
                return;
            }
            match field_blur(moving, &field) {
                Blur::Moved => leptos::logging::log!(
                    "nameplates: plate {id} field was moved, not left; not saved"
                ),
                Blur::Away => self.commit(Why::FocusAway),
                Blur::Left => self.commit(Why::FieldLeft),
            }
        });
    }

    /// The row leaves the list (runs in `on_cleanup`, before the row's
    /// signals are disposed). An editor output switch (the store's slug is
    /// not the row's any more): the plate still exists and the PATCH is
    /// id-scoped, so the drafts are committed and the store carries on
    /// without the row (a commit waiting behind the save on its way goes at
    /// once if the page hides: `NameplateStore::send_waiting`, from the
    /// list's own listeners). Otherwise the plate was deleted elsewhere: typed
    /// text the server does not have is reported, and the plate's saves are
    /// forgotten (they could only be answered 404).
    pub(super) fn leave(self) {
        let id = self.id;
        if self.store.output_slug.get_untracked() != self.slug.get_value() {
            self.commit(Why::FocusAway);
            return;
        }
        if self.has_drafts() && (self.name.value(), self.role.value()) != self.saved() {
            leptos::logging::log!("nameplates: plate {id} deleted with unsaved text");
            self.store.notify(DELETED_UNSAVED.to_string(), "error");
        }
        self.store.drop_saves(id);
    }
}
