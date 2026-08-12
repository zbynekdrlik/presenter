//! #555 "Zmazané piesne" (trash) card: soft-deleted songs with restore.
//! #644: extended with trashed LIBRARIES in the SAME card (settled: no new
//! surface/tab) — restoring a library is cascade-scoped (see
//! `Repository::restore_library`'s doc comment): it brings back exactly the
//! songs ITS OWN deletion cascaded, leaving an independently-trashed song in
//! the song list below untouched — so a library restore also refreshes the
//! song list, in case a cascaded song needs to disappear from it too.

use leptos::prelude::*;

use super::{format_timestamp, ToastHandle};
use crate::api::sync::{self, TrashedLibraryDto, TrashedSongDto};

#[component]
pub fn TrashCard(toast: ToastHandle) -> impl IntoView {
    let items = RwSignal::new(Vec::<TrashedSongDto>::new());
    let libraries = RwSignal::new(Vec::<TrashedLibraryDto>::new());

    let reload_songs = move || {
        leptos::task::spawn_local(async move {
            if let Ok(list) = sync::list_trash().await {
                items.set(list);
            }
        });
    };
    let reload_libraries = move || {
        leptos::task::spawn_local(async move {
            if let Ok(list) = sync::list_trashed_libraries().await {
                libraries.set(list);
            }
        });
    };
    reload_songs();
    reload_libraries();

    let restore = move |id: String| {
        leptos::task::spawn_local(async move {
            match sync::restore_song(&id).await {
                Ok(()) => {
                    toast.show("Pieseň obnovená", "success");
                    if let Ok(list) = sync::list_trash().await {
                        items.set(list);
                    }
                }
                Err(err) => toast.show(&format!("Obnovenie zlyhalo: {err}"), "error"),
            }
        });
    };

    let restore_library = move |id: String| {
        leptos::task::spawn_local(async move {
            match sync::restore_library(&id).await {
                Ok(()) => {
                    toast.show("Knižnica obnovená", "success");
                    if let Ok(list) = sync::list_trashed_libraries().await {
                        libraries.set(list);
                    }
                    // A library restore can cascade its own songs back too —
                    // refresh the song list so a just-restored song
                    // disappears from "Zmazané piesne" as well.
                    if let Ok(list) = sync::list_trash().await {
                        items.set(list);
                    }
                }
                Err(err) => toast.show(&format!("Obnovenie zlyhalo: {err}"), "error"),
            }
        });
    };

    view! {
        <section class="settings__card" data-role="trash-card">
            <header class="settings__card-header">
                <div>
                    <h2>"Zmazané piesne"</h2>
                    <p>"Obnov omylom zmazanú pieseň (uchované 30 dní)."</p>
                </div>
            </header>
            <div class="settings__list" data-role="trash-list">
                <Show
                    when=move || !items.get().is_empty()
                    fallback=|| view! { <p class="settings__empty">"Kôš je prázdny."</p> }
                >
                    <For
                        each=move || items.get()
                        key=|item| item.id.clone()
                        children=move |item: TrashedSongDto| {
                            let id = item.id.clone();
                            view! {
                                <div
                                    class="settings__row"
                                    data-role="trash-row"
                                    data-song-name=item.name.clone()
                                >
                                    <div>
                                        <span class="settings__row-title">{item.name.clone()}</span>
                                        <span class="settings__row-sub">
                                            {item.library_name.clone()}
                                            " · "
                                            {format_timestamp(&item.deleted_at)}
                                        </span>
                                    </div>
                                    <button
                                        class="settings__btn"
                                        data-role="restore-btn"
                                        on:click=move |_| restore(id.clone())
                                    >
                                        "Obnoviť"
                                    </button>
                                </div>
                            }
                        }
                    />
                </Show>
            </div>
            <header class="settings__card-header" data-role="trash-libraries-header">
                <div>
                    <h2>"Zmazané knižnice"</h2>
                    <p>"Obnov omylom zmazanú knižnicu — vráti sa aj s piesňami, ktoré zmazala spolu s ňou."</p>
                </div>
            </header>
            <div class="settings__list" data-role="trash-libraries-list">
                <Show
                    when=move || !libraries.get().is_empty()
                    fallback=|| {
                        view! {
                            <p class="settings__empty" data-role="trash-libraries-empty">
                                "Kôš knižníc je prázdny."
                            </p>
                        }
                    }
                >
                    <For
                        each=move || libraries.get()
                        key=|item| item.id.clone()
                        children=move |item: TrashedLibraryDto| {
                            let id = item.id.clone();
                            view! {
                                <div
                                    class="settings__row"
                                    data-role="trash-library-row"
                                    data-library-name=item.name.clone()
                                >
                                    <div>
                                        <span class="settings__row-title">{item.name.clone()}</span>
                                        <span class="settings__row-sub">
                                            {format_timestamp(&item.deleted_at)}
                                        </span>
                                    </div>
                                    <button
                                        class="settings__btn"
                                        data-role="restore-library-btn"
                                        on:click=move |_| restore_library(id.clone())
                                    >
                                        "Obnoviť"
                                    </button>
                                </div>
                            }
                        }
                    />
                </Show>
            </div>
        </section>
    }
}
