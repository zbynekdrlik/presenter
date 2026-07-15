//! #555 "Zmazané piesne" (trash) card: soft-deleted songs with restore.

use leptos::prelude::*;

use super::{format_timestamp, ToastHandle};
use crate::api::sync::{self, TrashedSongDto};

#[component]
pub fn TrashCard(toast: ToastHandle) -> impl IntoView {
    let items = RwSignal::new(Vec::<TrashedSongDto>::new());

    let reload = move || {
        leptos::task::spawn_local(async move {
            if let Ok(list) = sync::list_trash().await {
                items.set(list);
            }
        });
    };
    reload();

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
        </section>
    }
}
