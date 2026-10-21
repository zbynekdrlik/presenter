//! The shared plate-list views (#871): the virtual "Pieseň" row, the person
//! rows and the add form, over a [`NameplateStore`].
//!
//! Used by the stream editor's Menovky tab and the operator's Menovky view.
//! The `data-role` contract is the editor's (#779): `stream-nameplate-song`,
//! `stream-nameplate` rows with `stream-nameplate-name` / `-role` inputs and
//! `-show` / `-preview` / `-hide` / `-up` / `-down` / `-delete` buttons, and the
//! `stream-nameplate-add` form. [`RowOptions`] carries the per-page extras.

use leptos::prelude::*;
use presenter_core::{ActiveNameplate, NameplateSource};

use super::numbering::{beyond_grid, button_title, person_button, SONG_BUTTON};
use super::{song_line, NameplateStore};
use crate::components::stream_editor::bool_attr;

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
#[component]
pub fn NameplateRows(store: NameplateStore, options: RowOptions) -> impl IntoView {
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
                children=move |id| view! { <NameplateRow store=store id=id options=options /> }
            />
        </ul>
    }
}

/// One person-plate row: name + role inputs (commit on change), on-air
/// highlight, Zobraziť / (Prehrať) / Skryť / ↑ / ↓ / Zmazať. Under the keyed
/// `<For>` the row is reused across reorders, so its texts and its button
/// number are read from the store by id, never captured once.
#[component]
fn NameplateRow(store: NameplateStore, id: i64, options: RowOptions) -> impl IntoView {
    let plate = move || {
        store
            .plates
            .with(|list| list.iter().find(|p| p.id == id).cloned())
    };
    let name = move || plate().map(|p| p.primary_text).unwrap_or_default();
    let role = move || plate().map(|p| p.secondary_text).unwrap_or_default();
    // Handlers read untracked (they are not reactive scopes).
    let texts_now = move || {
        store.plates.with_untracked(|list| {
            list.iter()
                .find(|p| p.id == id)
                .map(|p| (p.primary_text.clone(), p.secondary_text.clone()))
                .unwrap_or_default()
        })
    };
    let is_active = move || {
        matches!(
            store.active.get(),
            Some(ActiveNameplate { source: NameplateSource::Person, nameplate_id: Some(a), .. }) if a == id
        )
    };
    // The Stream Deck button follows the row's POSITION, which a reorder or a
    // delete elsewhere changes while the id (the key) stays.
    let button = Memo::new(move |_| {
        store
            .plates
            .with(|list| list.iter().position(|p| p.id == id))
            .map(person_button)
    });
    let beyond = move || button.get().is_some_and(beyond_grid);
    view! {
        <li
            class="nameplates__row"
            data-role="stream-nameplate"
            data-nameplate-id=id.to_string()
            data-active=move || bool_attr(is_active())
            data-beyond-grid=move || options.numbered.then(|| bool_attr(beyond()))
        >
            {options.numbered.then(move || view! {
                <span
                    class="nameplates__button"
                    data-role="stream-nameplate-button"
                    title=move || button.get().map(button_title).unwrap_or_default()
                >
                    {move || button.get().map(|b| b.to_string()).unwrap_or_default()}
                </span>
            })}
            <div class="nameplates__texts">
                <input type="text" data-role="stream-nameplate-name" aria-label="Meno" prop:value=name
                    on:change=move |ev| store.update(id, event_target_value(&ev), texts_now().1) />
                <input type="text" data-role="stream-nameplate-role" aria-label="Pozícia" prop:value=role
                    on:change=move |ev| store.update(id, texts_now().0, event_target_value(&ev)) />
            </div>
            <div class="nameplates__actions">
                <button type="button" class="nameplates__btn" data-role="stream-nameplate-show"
                    on:click=move |_| store.show_person(id)>"Zobraziť"</button>
                {options.preview.map(move |preview| view! {
                    <button type="button" class="nameplates__btn nameplates__btn--ghost" data-role="stream-nameplate-preview"
                        on:click=move |_| {
                            let (name, role) = texts_now();
                            preview(NameplateSource::Person, Some(id), name, role);
                        }>"Prehrať"</button>
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
