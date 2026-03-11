use leptos::prelude::*;

use crate::components::modal;
use crate::state::operator::OperatorState;
use crate::state::AppContext;

#[component]
pub fn PlaylistList() -> impl IntoView {
    let ctx = use_context::<AppContext>().expect("AppContext");
    let _op = use_context::<OperatorState>().expect("OperatorState");

    let select_playlist = move |id: String, name: String| {
        ctx.selected_playlist_id.set(Some(id.clone()));
        ctx.selected_library_id.set(None);
        ctx.context_title.set(name);
        crate::state::session::set("activePlaylistId", &id);
        crate::state::session::remove("activeLibraryId");

        let id_clone = id.clone();
        leptos::task::spawn_local(async move {
            if let Ok(playlist) = crate::api::playlists::get_playlist(&id_clone).await {
                let ctx = use_context::<AppContext>().expect("AppContext");
                let summaries: Vec<presenter_core::PresentationSummary> = playlist
                    .entries
                    .iter()
                    .filter_map(|e| match &e.kind {
                        presenter_core::playlist::PlaylistEntryKind::Presentation {
                            presentation_id,
                            ..
                        } => Some(presenter_core::PresentationSummary::new(
                            *presentation_id,
                            String::new(),
                        )),
                        _ => None,
                    })
                    .collect();
                ctx.presentations.set(summaries);
            }
        });
    };

    let on_create = move |_| {
        let op = use_context::<OperatorState>().expect("OperatorState");
        op.modal_mode.set("create".to_string());
        op.modal_target_id.set(None);
        modal::open_modal(&op, "playlist-edit");
    };

    let on_more = move |_| {
        let op = use_context::<OperatorState>().expect("OperatorState");
        modal::open_modal(&op, "playlist");
    };

    let visible_count: usize = 5;

    view! {
        <div class="operator__list-section">
            <div class="operator__list-header">
                <h3 class="operator__list-title">"Playlists"</h3>
                <button data-role="playlist-more" class="operator__list-more" on:click=on_more>
                    {move || {
                        let total = ctx.playlists.get().len();
                        if total > visible_count { format!("{total}") } else { String::new() }
                    }}
                </button>
                <button data-role="playlist-create" class="operator__list-create" on:click=on_create>"+"</button>
            </div>
            <ul data-role="playlist-list" class="operator__list">
                {move || {
                    let playlists = ctx.playlists.get();
                    let active_id = ctx.selected_playlist_id.get();

                    let visible: Vec<_> = playlists.into_iter().take(visible_count).collect();

                    visible.into_iter().map(|pl| {
                        let id = pl.id.to_string();
                        let name = pl.name.clone();
                        let count = pl.entries.len();
                        let is_active = active_id.as_deref() == Some(&id);
                        let id_for_click = id.clone();
                        let name_for_click = name.clone();
                        let id_for_edit = id.clone();
                        let id_for_row = id.clone();
                        let id_for_btn = id.clone();
                        let id_for_modal = id.clone();

                        view! {
                            <li data-role="playlist-row" data-playlist-id=id_for_row class="operator__list-item">
                                <button
                                    data-role="playlist-item"
                                    data-playlist-id=id_for_btn
                                    attr:data-active=move || if is_active { "true" } else { "false" }
                                    class="operator__list-btn"
                                    on:click=move |_| {
                                        select_playlist(id_for_click.clone(), name_for_click.clone());
                                    }
                                >
                                    <span class="operator__list-label">{name}</span>
                                    <span class="operator__list-meta" data-role="playlist-count">{count}</span>
                                </button>
                                <button
                                    data-action="playlist-edit"
                                    data-playlist-id=id_for_edit
                                    class="operator__list-edit"
                                    on:click=move |ev: leptos::ev::MouseEvent| {
                                        ev.stop_propagation();
                                        let op = use_context::<OperatorState>().expect("OperatorState");
                                        op.modal_mode.set("edit".to_string());
                                        op.modal_target_id.set(Some(id_for_modal.clone()));
                                        modal::open_modal(&op, "playlist-edit");
                                    }
                                >
                                    "\u{270e}"
                                </button>
                            </li>
                        }
                    }).collect_view()
                }}
            </ul>
        </div>
    }
}
