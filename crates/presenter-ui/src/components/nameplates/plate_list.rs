//! The shared plate-list views (#871): the virtual "Pieseň" row, the person
//! rows and the add form, over a [`NameplateStore`].
//!
//! Used by the stream editor's Menovky tab and the operator's Menovky view.
//! The `data-role` contract is the editor's (#779): `stream-nameplate-song`,
//! `stream-nameplate` rows with `stream-nameplate-name` / `-role` inputs and
//! `-show` / `-preview` / `-hide` / `-up` / `-down` / `-delete` buttons, and the
//! `stream-nameplate-add` form. #871 adds the drag handle
//! (`stream-nameplate-drag`). [`RowOptions`] carries the per-page extras.

use leptos::prelude::*;
use presenter_core::{ActiveNameplate, NameplateSource};
use wasm_bindgen::JsCast;
use web_sys::{KeyboardEvent, PointerEvent};

use super::drag::{capture, slot_under_pointer, PlateDrag, PRIMARY_BUTTON};
use super::focus::page_hidden;
use super::numbering::{beyond_grid, button_title, person_button, SONG_BUTTON};
use super::order::drop_marker;
use super::row_save::{PlateText, RowSave, Why};
use super::{song_line, NameplateStore};
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
/// Owns the one drag in progress, so every row can show the drop line, and
/// the list's retry triggers: back online or back in view, every failed
/// save goes again (a phone fires `visibilitychange` on unlock before its
/// Wi-Fi is back, so that try alone often fails; the timers follow).
#[component]
pub fn NameplateRows(store: NameplateStore, options: RowOptions) -> impl IntoView {
    let network_back = window_event_listener_untyped("online", move |_| {
        store.retry_failed("the network is back");
    });
    let page_back = window_event_listener_untyped("visibilitychange", move |_| {
        if !page_hidden() {
            store.retry_failed("the page is back in view");
        }
    });
    on_cleanup(move || {
        network_back.remove();
        page_back.remove();
    });
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
    // so both commit what is typed, urgently. The row leaving the list hands
    // its drafts over or reports them (`RowSave::leave`).
    let page_hides = window_event_listener_untyped("visibilitychange", move |_| {
        if page_hidden() {
            save.commit(Why::PageHidden);
        }
    });
    let page_closes = window_event_listener_untyped("pagehide", move |_| {
        save.commit(Why::PageHidden);
    });
    on_cleanup(move || {
        page_hides.remove();
        page_closes.remove();
        save.leave();
    });
    let unsaved = move || save.unsaved();
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
