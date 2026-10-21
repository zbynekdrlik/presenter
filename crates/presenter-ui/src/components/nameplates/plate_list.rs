//! The shared plate-list views (#871): the virtual "Pieseň" row, the person
//! rows and the add form, over a [`NameplateStore`].
//!
//! Used by the stream editor's Menovky tab and the operator's Menovky view.
//! The `data-role` contract is the editor's (#779): `stream-nameplate-song`,
//! `stream-nameplate` rows with `stream-nameplate-name` / `-role` inputs and
//! `-show` / `-preview` / `-hide` / `-up` / `-down` / `-delete` buttons, and the
//! `stream-nameplate-add` form. #871 adds the drag handle
//! (`stream-nameplate-drag`). [`RowOptions`] carries the per-page extras.

use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use presenter_core::{ActiveNameplate, NameplateSource};
use wasm_bindgen::JsCast;
use web_sys::{Element, KeyboardEvent, PointerEvent};

use super::drag::{capture, slot_under_pointer, PlateDrag, PRIMARY_BUTTON};
use super::focus::{field_blur, page_hidden, rows_moving, Blur};
use super::numbering::{beyond_grid, button_title, person_button, SONG_BUTTON};
use super::order::drop_marker;
use super::save_queue::{retry_delay_ms, TextLeft, DELETED_UNSAVED};
use super::{song_line, NameplateStore, SaveOutcome};
use crate::utils::dom_attr::bool_attr;

/// What one page adds to the shared rows. `fn` pointers keep it `Copy` and
/// free of reactive state.
#[derive(Clone, Copy, Default)]
pub struct RowOptions {
    /// The editor's „Prehrať": plays the plate in its own preview frame,
    /// never on air. `None` (the operator view) = no Prehrať button.
    pub preview: Option<fn(NameplateSource, Option<i64>, String, String)>,
    /// Runs after „Skryť" (the editor clears its preview frame too).
    pub after_hide: Option<fn()>,
    /// Show each row's Stream Deck button number (#870 grid order).
    pub numbered: bool,
}

/// „Skryť": take the plate off air, then the page's extra.
fn hide(store: NameplateStore, options: RowOptions) {
    store.hide();
    if let Some(after) = options.after_hide {
        after();
    }
}

/// The virtual "Pieseň" row: the live song (read-only), Zobraziť / Skryť.
#[component]
pub fn NameplateSongRow(store: NameplateStore, options: RowOptions) -> impl IntoView {
    let song_active = move || {
        matches!(
            store.active.get(),
            Some(ActiveNameplate {
                source: NameplateSource::Song,
                ..
            })
        )
    };
    view! {
        <div
            class="nameplates__row nameplates__row--song"
            data-role="stream-nameplate-song"
            data-active=move || bool_attr(song_active())
        >
            {options.numbered.then(move || view! {
                <span
                    class="nameplates__button"
                    data-role="stream-nameplate-button"
                    title=button_title(SONG_BUTTON)
                >
                    {SONG_BUTTON.to_string()}
                </span>
            })}
            <div class="nameplates__texts">
                <strong>"Pieseň (aktuálna)"</strong>
                <span class="nameplates__song-live" data-role="stream-nameplate-song-live">
                    {move || store.song.with(|(name, library)| song_line(name, library))}
                </span>
            </div>
            <div class="nameplates__actions">
                <button type="button" class="nameplates__btn" data-role="stream-nameplate-song-show"
                    on:click=move |_| store.show_song()>"Zobraziť"</button>
                {options.preview.map(move |preview| view! {
                    <button type="button" class="nameplates__btn nameplates__btn--ghost" data-role="stream-nameplate-song-preview"
                        on:click=move |_| {
                            let (name, library) = store.song.get_untracked();
                            let name = if name.is_empty() { "Názov piesne".to_string() } else { name };
                            preview(NameplateSource::Song, None, name, library);
                        }>"Prehrať"</button>
                })}
                <button type="button" class="nameplates__btn nameplates__btn--ghost" data-role="stream-nameplate-hide"
                    on:click=move |_| hide(store, options)>"Skryť"</button>
            </div>
        </div>
    }
}

/// The person plates in list order, keyed by plate id (a reorder moves rows).
/// Owns the one drag in progress, so every row can show the drop line.
#[component]
pub fn NameplateRows(store: NameplateStore, options: RowOptions) -> impl IntoView {
    let drag = RwSignal::new(None::<PlateDrag>);
    // #871 review: a plate deleted elsewhere while it is dragged ends its
    // drag. Its handle is gone, so no release or cancel would ever reach it.
    Effect::new(move || {
        let gone = store.plates.with(|list| {
            drag.with_untracked(|d| d.is_some_and(|d| list.iter().all(|p| p.id != d.id)))
        });
        if gone {
            leptos::logging::log!("nameplates: the dragged plate was deleted; drag ended");
            drag.set(None);
        }
    });
    let plate_ids = move || {
        store
            .plates
            .with(|list| list.iter().map(|p| p.id).collect::<Vec<_>>())
    };
    view! {
        <ul class="nameplates__list" data-role="stream-nameplate-list">
            <For
                each=plate_ids
                key=|id| *id
                children=move |id| view! { <NameplateRow store=store id=id options=options drag=drag /> }
            />
        </ul>
    }
}

/// One editable text of a row (name or role) with its draft.
///
/// #871 review: the field shows the DRAFT while the operator has typed and not
/// saved yet, else the saved text. A list refresh (an own save of the other
/// field, its `StreamNameplatesChanged` echo, another operator, Companion, a
/// reconnect) can never overwrite half-typed text: `saved` is a `Memo` (it
/// notifies only when THIS plate's text changes), and while a draft exists the
/// shown value stays the draft even when the saved text does change. A draft
/// ends when its save is done (saved, or refused: the field then shows the
/// saved text again), or on a blur that changed nothing.
#[derive(Clone, Copy)]
struct PlateText {
    saved: Memo<String>,
    draft: RwSignal<Option<String>>,
    shown: Memo<String>,
}

impl PlateText {
    fn new(saved: Memo<String>) -> Self {
        let draft = RwSignal::new(None::<String>);
        let shown = Memo::new(move |_| draft.get().unwrap_or_else(|| saved.get()));
        Self {
            saved,
            draft,
            shown,
        }
    }

    /// The text a save sends: the draft, else the saved text.
    fn value(self) -> String {
        self.draft
            .get_untracked()
            .unwrap_or_else(|| self.saved.get_untracked())
    }

    /// Whether the draft differs from the saved text.
    fn is_dirty(self) -> bool {
        self.draft
            .get_untracked()
            .is_some_and(|draft| draft != self.saved.get_untracked())
    }

    /// Drop the draft if it is still exactly `sent` (text typed since stays).
    fn settle(self, sent: &str) {
        self.draft.maybe_update(|draft| {
            let done = draft.as_deref() == Some(sent);
            if done {
                *draft = None;
            }
            done
        });
    }
}

/// Why a row saves its drafts (#871 review).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Why {
    /// The operator left the field inside the page (Tab, a click elsewhere,
    /// Enter): an empty name is refused with a toast.
    FieldLeft,
    /// The focus went away without the operator leaving the field (the
    /// window lost it: alt-tab, OBS, Companion, another tab): an empty name
    /// is kept as typed, the operator is retyping it.
    FocusAway,
    /// The page went hidden or is being closed (`pagehide`): as `FocusAway`,
    /// and a save that would wait for the one on its way goes out NOW.
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

/// How one row saves its two texts (#871): on leaving a field, when the
/// window loses the focus, when the page goes hidden or is closed, again
/// after a failed save, what Zobraziť waits for, and what becomes of text
/// typed and not saved when the row leaves the list.
#[derive(Clone, Copy)]
struct RowSave {
    store: NameplateStore,
    id: i64,
    name: PlateText,
    role: PlateText,
    /// The output the row was listed for (the editor can switch outputs).
    slug: StoredValue<String>,
    /// The texts of the row's newest save, while it is on its way (or waits
    /// for an older one). The same texts are never sent twice: leaving the
    /// window blurs the field, and a phone also hides the page right after.
    sending: StoredValue<Option<(String, String)>>,
    /// The texts of the row's newest save, which failed (a network error, a
    /// server fault, an expired login). They stay in the fields, the row is
    /// marked „neuložené", and they are sent again: after a pause
    /// ([`retry_delay_ms`]), when the network or the page comes back, on
    /// Zobraziť.
    failed: RwSignal<Option<(String, String)>>,
    /// Failed tries in a row (the pause before the next retry).
    failures: StoredValue<u32>,
    /// The ticket of a Zobraziť waiting for the row's save
    /// ([`NameplateStore::defer_show`]).
    show_after: StoredValue<Option<u64>>,
}

impl RowSave {
    fn new(store: NameplateStore, id: i64, name: PlateText, role: PlateText) -> Self {
        Self {
            store,
            id,
            name,
            role,
            slug: StoredValue::new(store.output_slug.get_untracked()),
            sending: StoredValue::new(None),
            failed: RwSignal::new(None),
            failures: StoredValue::new(0),
            show_after: StoredValue::new(None),
        }
    }

    /// The texts a save would send now, when they differ from the saved ones.
    fn unsaved(self) -> Option<(String, String)> {
        (self.name.is_dirty() || self.role.is_dirty())
            .then(|| (self.name.value(), self.role.value()))
    }

    /// Whether a save of exactly `texts` is already on its way.
    fn is_sending(self, texts: &(String, String)) -> bool {
        self.sending
            .with_value(|sending| sending.as_ref() == Some(texts))
    }

    /// Save the row's drafts. One save carries both values, so committing
    /// the role never sends a stale name. Each branch logs its decision.
    fn commit(self, why: Why) {
        let id = self.id;
        let Some(texts) = self.unsaved() else {
            // Nothing changed (typed and typed back): drop the drafts, and a
            // failed text the operator gave up.
            self.name.draft.set(None);
            self.role.draft.set(None);
            self.forget_failed();
            return;
        };
        // #871 review: the operator cleared the name to retype it and
        // alt-tabbed or locked the phone. Not a refusal: the name stays as
        // typed, and a role typed in the row goes with the SAVED name.
        let name_kept = why != Why::FieldLeft && texts.0.trim().is_empty();
        let texts = if !name_kept {
            texts
        } else if self.role.is_dirty() {
            (self.name.saved.get_untracked(), texts.1)
        } else {
            leptos::logging::log!(
                "nameplates: plate {id} name is empty; kept as typed, not saved ({})",
                why.reason()
            );
            return;
        };
        if self.is_sending(&texts) {
            if why == Why::PageHidden && self.store.flush_save(id) {
                return;
            }
            leptos::logging::log!(
                "nameplates: plate {id} save of these texts is on its way ({})",
                why.reason()
            );
            return;
        }
        self.send(texts, why == Why::PageHidden);
        if name_kept {
            leptos::logging::log!(
                "nameplates: plate {id} name is empty; kept as typed, the role sent with the saved name ({})",
                why.reason()
            );
        }
    }

    /// Send `texts` as the row's newest save (`urgent`: the page is being
    /// hidden or closed). Texts other than the failed ones replace them.
    fn send(self, texts: (String, String), urgent: bool) {
        if self
            .failed
            .with_untracked(|failed| failed.as_ref() != Some(&texts))
        {
            self.forget_failed();
        }
        self.sending.set_value(Some(texts.clone()));
        let (primary, secondary) = texts.clone();
        self.store
            .update(self.id, primary, secondary, urgent, move |outcome| {
                self.settle(&texts, outcome)
            });
    }

    /// The failed text is given up or replaced: the mark goes, and the retry
    /// pauses start again.
    fn forget_failed(self) {
        if self.failed.with_untracked(Option::is_some) {
            self.failed.set(None);
        }
        self.failures.set_value(0);
    }

    /// Send the texts of the row's failed save again (`trigger`: the retry
    /// pause ran out, the network or the page is back, Zobraziť). Nothing
    /// when the row is gone, nothing failed, or that save is on its way.
    fn retry(self, trigger: &'static str) {
        if self.failed.is_disposed() {
            return;
        }
        let id = self.id;
        let Some(texts) = self.failed.get_untracked() else {
            return;
        };
        if self.is_sending(&texts) {
            leptos::logging::log!("nameplates: plate {id} failed save is on its way ({trigger})");
            return;
        }
        self.send(texts, false);
        leptos::logging::log!("nameplates: plate {id} failed save sent again ({trigger})");
    }

    /// A save of `texts` ended; returns where its typed text is now (for the
    /// toast of a failed save). The row may be gone by then (its `done` runs
    /// when the answer comes): the first, `try_` read says so, and then
    /// nothing else of the row is touched (a read of a disposed signal
    /// panics). All of a row's signals are disposed together.
    fn settle(self, texts: &(String, String), outcome: SaveOutcome) -> TextLeft {
        let gone = outcome == SaveOutcome::Gone;
        let Some(newest) = self.sending.try_update_value(|sending| {
            let newest = sending.as_ref() == Some(texts);
            if newest || gone {
                *sending = None;
            }
            newest
        }) else {
            // The row left the list: no field holds its text any more.
            return TextLeft::Nowhere;
        };
        match outcome {
            SaveOutcome::Saved | SaveOutcome::Refused => {
                self.name.settle(&texts.0);
                self.role.settle(&texts.1);
                if newest {
                    self.forget_failed();
                    self.show_once_saved();
                }
                TextLeft::InField
            }
            SaveOutcome::Gone => {
                // The store has said the plate was deleted; nothing is left
                // to save, so leaving the list says nothing more.
                self.name.draft.set(None);
                self.role.draft.set(None);
                self.forget_failed();
                self.show_after.set_value(None);
                TextLeft::InField
            }
            SaveOutcome::Failed if newest => self.keep_failed(texts.clone()),
            // A newer save of the row carries the text.
            SaveOutcome::Failed => TextLeft::InField,
        }
    }

    /// The row's newest save failed: keep its texts for another try, marked
    /// „neuložené", retried after a pause; a Zobraziť waiting for it does
    /// not put the plate on air.
    fn keep_failed(self, texts: (String, String)) -> TextLeft {
        let id = self.id;
        self.failed.set(Some(texts));
        let failures = self
            .failures
            .try_update_value(|failures| {
                *failures += 1;
                *failures
            })
            .unwrap_or(0);
        if let Some(delay) = retry_delay_ms(failures) {
            leptos::logging::log!("nameplates: plate {id} failed save retried in {delay} ms");
            leptos::task::spawn_local(async move {
                TimeoutFuture::new(delay).await;
                self.retry("the retry pause ran out");
            });
        }
        match self.show_after.try_update_value(Option::take).flatten() {
            Some(ticket) if self.store.show_is_current(ticket) => TextLeft::InFieldNotShown,
            _ => TextLeft::InField,
        }
    }

    /// The row's newest save went through: a Zobraziť waiting for it puts
    /// the plate on air now, unless another show or hide was asked for
    /// since.
    fn show_once_saved(self) {
        let Some(ticket) = self.show_after.try_update_value(Option::take).flatten() else {
            return;
        };
        let id = self.id;
        if self.store.show_is_current(ticket) {
            leptos::logging::log!("nameplates: plate {id} saved; shown now");
            self.store.show_person(id);
        } else {
            leptos::logging::log!(
                "nameplates: plate {id} saved; not shown, another show or hide came since"
            );
        }
    }

    /// Zobraziť (#871 review). The server puts on air the texts it HAS, so
    /// while the row's save is on its way, or failed (sent again now), the
    /// plate goes on air once that save is through; a failed one says the
    /// plate was not shown. Otherwise it goes on air at once.
    fn show(self) {
        let id = self.id;
        if self.sending.with_value(Option::is_none) && self.failed.with_untracked(Option::is_none) {
            self.store.show_person(id);
            return;
        }
        self.show_after.set_value(Some(self.store.defer_show()));
        leptos::logging::log!("nameplates: plate {id} goes on air once its save is through");
        self.retry("Zobraziť was pressed");
    }

    /// A row field lost the focus: save the row, unless the field was only
    /// MOVED.
    ///
    /// #871 review: a reorder (another operator, Companion, this page's ↑ / ↓
    /// or drag) moves the row with `insertBefore`, which drops the focus of
    /// the field being typed in, and the browser fires `blur` on the way.
    /// Whether rows are moving is read NOW, in the blur handler
    /// ([`rows_moving`]); the decision waits for the focus restore of
    /// `focus::keep_focus` (this task is queued after it). A blur during a
    /// move whose field holds the focus again was only a move and saves
    /// nothing (the text stays a draft). Any other blur saves: a field still
    /// active outside a move lost the WINDOW's focus (alt-tab, OBS, another
    /// tab), whatever the browser's `hasFocus()` says at that moment; a field
    /// that is no longer active was left (Tab, a click elsewhere, Enter). A
    /// field whose row is gone saves nothing ([`Self::report_lost`] tells
    /// the operator).
    fn commit_on_leave(self, field: Element) {
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

    /// The page went hidden or is being closed (another tab or app, a locked
    /// phone, a reload, a closed tab): save what is typed NOW, since a hidden
    /// page may be frozen or killed. Back in view: send a failed save again.
    fn page_changed(self, leaving: bool) {
        if leaving {
            self.commit(Why::PageHidden);
        } else {
            self.retry("the page is back in view");
        }
    }

    /// The row left the list with typed text that no save carries (runs in
    /// `on_cleanup`, before the row's signals are disposed). Its plate was
    /// deleted elsewhere: say so, instead of dropping it silently. Or the
    /// editor switched to another output (only kept drafts of a failed save
    /// get here: the select click blurs a field first): the plate still
    /// exists and the PATCH is id-scoped, so the text is sent now (`commit`
    /// logs its decision; the save's `done` then finds the row gone).
    fn report_lost(self) {
        let Some(texts) = self.unsaved() else {
            return;
        };
        if self.is_sending(&texts) {
            return;
        }
        if self.store.output_slug.get_untracked() != self.slug.get_value() {
            self.commit(Why::FocusAway);
            return;
        }
        leptos::logging::log!("nameplates: plate {} deleted with unsaved text", self.id);
        self.store.notify(DELETED_UNSAVED.to_string(), "error");
    }
}

/// Enter in a row field commits it, by leaving the field.
fn blur_on_enter(ev: KeyboardEvent) {
    if ev.key() != "Enter" {
        return;
    }
    ev.prevent_default();
    if let Some(input) = ev
        .target()
        .and_then(|target| target.dyn_into::<web_sys::HtmlElement>().ok())
    {
        let _ = input.blur();
    }
}

/// One person-plate row: drag handle, (button number), name + role inputs
/// (saved on blur), on-air highlight, Zobraziť / (Prehrať) / Skryť / ↑ / ↓ /
/// Zmazať. Under the keyed `<For>` the row is reused across reorders, so its
/// texts and its button number are read from the store by id, never captured.
#[component]
fn NameplateRow(
    store: NameplateStore,
    id: i64,
    options: RowOptions,
    drag: RwSignal<Option<PlateDrag>>,
) -> impl IntoView {
    // A row's saved text. #871 review: it KEEPS its last value once the plate
    // is missing from the list (an output switch clears the list, a delete
    // elsewhere), so a save made as the row leaves still sends the field the
    // operator did not type in, never "".
    let field = move |pick: fn(&presenter_core::Nameplate) -> String| {
        Memo::new(move |previous: Option<&String>| {
            store
                .plates
                .with(|list| list.iter().find(|p| p.id == id).map(pick))
                .or_else(|| previous.cloned())
                .unwrap_or_default()
        })
    };
    let name = PlateText::new(field(|p| p.primary_text.clone()));
    let role = PlateText::new(field(|p| p.secondary_text.clone()));
    let save = RowSave::new(store, id, name, role);
    // A hidden page may be frozen or killed (a phone), and a reload or a
    // closed tab fires `pagehide` (Safari may skip `visibilitychange` then),
    // so both save what is typed; back in view or back online, a failed save
    // goes again. A row whose plate was deleted elsewhere reports text it
    // could not save.
    let page_hides = window_event_listener_untyped("visibilitychange", move |_| {
        save.page_changed(page_hidden());
    });
    let page_closes = window_event_listener_untyped("pagehide", move |_| {
        save.page_changed(true);
    });
    let network_back = window_event_listener_untyped("online", move |_| {
        save.retry("the network is back");
    });
    on_cleanup(move || {
        page_hides.remove();
        page_closes.remove();
        network_back.remove();
        save.report_lost();
    });
    let unsaved = move || save.failed.with(Option::is_some);
    let is_active = move || {
        matches!(
            store.active.get(),
            Some(ActiveNameplate { source: NameplateSource::Person, nameplate_id: Some(a), .. }) if a == id
        )
    };
    // The row's POSITION (its Stream Deck button, the drop line), which a
    // reorder or a delete elsewhere changes while the id (the key) stays.
    let position = Memo::new(move |_| {
        store
            .plates
            .with(|list| list.iter().position(|p| p.id == id))
    });
    let button = move || position.get().map(person_button);
    let beyond = move || button().is_some_and(beyond_grid);
    let dragging = move || drag.with(|d| d.is_some_and(|d| d.id == id));
    // The drop line on this row, measured on the list as it is NOW: the
    // dragged plate's place is looked up, so a plate deleted (or moved)
    // elsewhere mid-drag never leaves a stale line (#871 review).
    let marker = move || {
        let d = drag.get().filter(|d| d.id != id)?;
        let pos = position.get()?;
        store.plates.with(|list| {
            let from = list.iter().position(|p| p.id == d.id)?;
            drop_marker(from, d.slot, pos, list.len())
        })
    };

    // Drag by the handle (pointer events: mouse, finger, pen).
    let on_down = move |ev: PointerEvent| {
        if ev.button() != PRIMARY_BUTTON || !capture(&ev) {
            return;
        }
        ev.prevent_default();
        if let Some(from) = position.get_untracked() {
            drag.set(Some(PlateDrag { id, slot: from }));
        }
    };
    let on_move = move |ev: PointerEvent| {
        let Some(current) = drag.get_untracked().filter(|d| d.id == id) else {
            return;
        };
        // No button held any more (a lost release): the drag must end.
        if ev.buttons() == 0 {
            drag.set(None);
            return;
        }
        if let Some(slot) = slot_under_pointer(&ev) {
            if slot != current.slot {
                drag.set(Some(PlateDrag { slot, ..current }));
            }
        }
    };
    let on_up = move |ev: PointerEvent| {
        let Some(current) = drag.get_untracked().filter(|d| d.id == id) else {
            return;
        };
        drag.set(None);
        let slot = slot_under_pointer(&ev).unwrap_or(current.slot);
        store.drop_at(id, slot);
    };
    let on_cancel = move |_: PointerEvent| {
        if drag.get_untracked().is_some_and(|d| d.id == id) {
            drag.set(None);
        }
    };
    // The handle also moves the plate from the keyboard.
    let on_handle_key = move |ev: KeyboardEvent| match ev.key().as_str() {
        "ArrowUp" => {
            ev.prevent_default();
            store.move_plate(id, true);
        }
        "ArrowDown" => {
            ev.prevent_default();
            store.move_plate(id, false);
        }
        _ => {}
    };

    view! {
        <li
            class="nameplates__row"
            data-role="stream-nameplate"
            data-nameplate-id=id.to_string()
            data-active=move || bool_attr(is_active())
            data-beyond-grid=move || options.numbered.then(|| bool_attr(beyond()))
            data-dragging=move || bool_attr(dragging())
            data-drop=marker
            data-unsaved=move || bool_attr(unsaved())
        >
            <button
                type="button"
                class="nameplates__btn nameplates__btn--ghost nameplates__handle"
                data-role="stream-nameplate-drag"
                title="Ťahaním presuniete menovku (alebo šípkami hore / dole)"
                aria-label="Presunúť menovku"
                on:pointerdown=on_down
                on:pointermove=on_move
                on:pointerup=on_up
                on:pointercancel=on_cancel
                on:lostpointercapture=on_cancel
                on:keydown=on_handle_key
            >
                "⠿"
            </button>
            {options.numbered.then(move || view! {
                <span
                    class="nameplates__button"
                    data-role="stream-nameplate-button"
                    title=move || button().map(button_title).unwrap_or_default()
                >
                    {move || button().map(|b| b.to_string()).unwrap_or_default()}
                </span>
            })}
            <div class="nameplates__texts">
                <input type="text" data-role="stream-nameplate-name" aria-label="Meno"
                    prop:value=move || name.shown.get()
                    on:input=move |ev| name.draft.set(Some(event_target_value(&ev)))
                    on:keydown=blur_on_enter
                    on:blur=move |ev| save.commit_on_leave(event_target(&ev)) />
                <input type="text" data-role="stream-nameplate-role" aria-label="Pozícia"
                    prop:value=move || role.shown.get()
                    on:input=move |ev| role.draft.set(Some(event_target_value(&ev)))
                    on:keydown=blur_on_enter
                    on:blur=move |ev| save.commit_on_leave(event_target(&ev)) />
                <Show when=unsaved>
                    <span class="nameplates__unsaved" data-role="stream-nameplate-unsaved"
                        title="Text sa neuložil. Uloží sa znova sám, alebo keď pole opustíte.">
                        "neuložené"
                    </span>
                </Show>
            </div>
            <div class="nameplates__actions">
                <button type="button" class="nameplates__btn" data-role="stream-nameplate-show"
                    on:click=move |_| save.show()>"Zobraziť"</button>
                {options.preview.map(move |preview| view! {
                    <button type="button" class="nameplates__btn nameplates__btn--ghost" data-role="stream-nameplate-preview"
                        on:click=move |_| preview(NameplateSource::Person, Some(id), name.value(), role.value())>"Prehrať"</button>
                })}
                <button type="button" class="nameplates__btn nameplates__btn--ghost" data-role="stream-nameplate-hide"
                    on:click=move |_| hide(store, options)>"Skryť"</button>
                <button type="button" class="nameplates__btn nameplates__btn--ghost" data-role="stream-nameplate-up"
                    title="Vyššie" aria-label="Vyššie" on:click=move |_| store.move_plate(id, true)>"↑"</button>
                <button type="button" class="nameplates__btn nameplates__btn--ghost" data-role="stream-nameplate-down"
                    title="Nižšie" aria-label="Nižšie" on:click=move |_| store.move_plate(id, false)>"↓"</button>
                <button type="button" class="nameplates__btn nameplates__btn--danger" data-role="stream-nameplate-delete"
                    on:click=move |_| store.delete(id)>"Zmazať"</button>
            </div>
        </li>
    }
}

/// „Pridať menovku": name + role, submitted with the button or Enter.
#[component]
pub fn NameplateAddForm(store: NameplateStore) -> impl IntoView {
    let new_name = RwSignal::new(String::new());
    let new_role = RwSignal::new(String::new());
    view! {
        <form
            class="nameplates__add"
            data-role="stream-nameplate-add"
            on:submit=move |ev| {
                ev.prevent_default();
                store.add(new_name.get_untracked(), new_role.get_untracked());
                new_name.set(String::new());
                new_role.set(String::new());
            }
        >
            <input type="text" placeholder="Meno" aria-label="Meno novej menovky" data-role="stream-nameplate-new-name"
                prop:value=move || new_name.get()
                on:input=move |ev| new_name.set(event_target_value(&ev)) />
            <input type="text" placeholder="Pozícia (napr. pastor)" aria-label="Pozícia novej menovky" data-role="stream-nameplate-new-role"
                prop:value=move || new_role.get()
                on:input=move |ev| new_role.set(event_target_value(&ev)) />
            <button type="submit" class="nameplates__btn nameplates__btn--primary" data-role="stream-nameplate-add-submit">
                "Pridať menovku"
            </button>
        </form>
    }
}
