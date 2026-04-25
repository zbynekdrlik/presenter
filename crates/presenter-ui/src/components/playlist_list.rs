use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::components::modal;
use crate::state::operator::OperatorState;
use crate::state::AppContext;

#[component]
pub fn PlaylistList() -> impl IntoView {
    let ctx = use_ctx!(AppContext);
    let op = use_ctx!(OperatorState);

    let select_playlist = move |id: String, name: String| {
        ctx.selected_playlist_id.set(Some(id.clone()));
        ctx.selected_library_id.set(None);
        ctx.context_title.set(name);
        crate::state::session::set("activePlaylistId", &id);
        crate::state::session::remove("activeLibraryId");

        // Capture signals OUTSIDE async block - context may not be available inside spawn_local
        let presentations_signal = ctx.presentations;
        let selected_playlist_signal = ctx.selected_playlist;
        let id_clone = id.clone();
        leptos::task::spawn_local(async move {
            if let Ok(playlist) = crate::api::playlists::get_playlist(&id_clone).await {
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
                presentations_signal.set(summaries);
                selected_playlist_signal.set(Some(playlist));
            }
        });
    };

    let on_create = {
        let op = op.clone();
        move |_| {
            op.modal_mode.set("create".to_string());
            op.modal_target_id.set(None);
            // Use "playlist-create" so edit_mode() returns "create" in playlist_modal.rs
            modal::open_modal(&op, "playlist-create");
        }
    };

    let on_more = {
        let op = op.clone();
        move |_| {
            modal::open_modal(&op, "playlist-list");
        }
    };

    view! {
        <section class="operator__group operator__group--playlists">
            <header class="operator__group-header">
                <h2>"Playlists"</h2>
                <div class="operator__group-controls">
                    <button
                        type="button"
                        class="operator__group-count"
                        data-role="playlist-more"
                        aria-label="Show all playlists"
                        on:click=on_more
                    >
                        {move || {
                            let total = ctx.playlists.get().len();
                            total.to_string()
                        }}
                    </button>
                    <button
                        type="button"
                        data-role="playlist-create"
                        aria-label="Create playlist"
                        title="Create playlist"
                        on:click=on_create
                    >"+"</button>
                </div>
            </header>
            <ul class="operator__list" data-role="playlist-list">
                {move || {
                    let playlists = ctx.playlists.get();
                    let active_id = ctx.selected_playlist_id.get();

                    if playlists.is_empty() {
                        return view! {
                            <li class="operator__favorites-empty">
                                "No playlists yet. Create one to build a run sheet."
                            </li>
                        }.into_any();
                    }

                    // Filter to dashboard + active playlist only
                    let dashboard_visible: Vec<_> = playlists.iter().filter(|pl| {
                        let id = pl.id.to_string();
                        pl.show_in_dashboard || active_id.as_deref() == Some(&id)
                    }).cloned().collect();

                    // If no dashboard items, show full sorted list
                    let visible = if dashboard_visible.is_empty() {
                        let mut sorted = playlists;
                        sorted.sort_by_key(|a| a.name.to_lowercase());
                        sorted
                    } else {
                        dashboard_visible
                    };

                    view! {
                        <div class="operator__favorites">
                            {visible.into_iter().map(|pl| {
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
                                let op_for_edit = op.clone();

                                let id_for_drop = id.clone();
                                view! {
                                    <li
                                        class="operator__list-item operator__list-row"
                                        data-playlist-id=id_for_row
                                        on:dragover=move |ev: web_sys::DragEvent| {
                                            // Accept presentation drops
                                            if let Some(dt) = ev.data_transfer() {
                                                let types = dt.types();
                                                let accepts = (0..types.length())
                                                    .any(|i| {
                                                        let t = types.get(i).as_string().unwrap_or_default();
                                                        t == "application/x-presentation-id" || t == "application/x-presenter-search"
                                                    });
                                                if accepts {
                                                    ev.prevent_default();
                                                    if let Some(target) = ev.target() {
                                                        if let Ok(el) = target.dyn_into::<web_sys::Element>() {
                                                            let _ = el.closest(".operator__list-item")
                                                                .ok()
                                                                .flatten()
                                                                .map(|li| li.class_list().add_1("drag-over"));
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        on:dragleave=move |ev: web_sys::DragEvent| {
                                            if let Some(target) = ev.target() {
                                                if let Ok(el) = target.dyn_into::<web_sys::Element>() {
                                                    let _ = el.closest(".operator__list-item")
                                                        .ok()
                                                        .flatten()
                                                        .map(|li| li.class_list().remove_1("drag-over"));
                                                }
                                            }
                                        }
                                        on:drop={
                                            let playlist_id = id_for_drop.clone();
                                            let playlists = ctx.playlists;
                                            let selected_playlist = ctx.selected_playlist;
                                            move |ev: web_sys::DragEvent| {
                                                ev.prevent_default();
                                                // Remove drag-over class
                                                if let Some(target) = ev.target() {
                                                    if let Ok(el) = target.dyn_into::<web_sys::Element>() {
                                                        let _ = el.closest(".operator__list-item")
                                                            .ok()
                                                            .flatten()
                                                            .map(|li| li.class_list().remove_1("drag-over"));
                                                    }
                                                }
                                                // Get presentation ID from drag data
                                                if let Some(dt) = ev.data_transfer() {
                                                    let pres_id = dt.get_data("application/x-presentation-id")
                                                        .ok()
                                                        .filter(|s| !s.is_empty())
                                                        .or_else(|| dt.get_data("application/x-presenter-search").ok().filter(|s| !s.is_empty()));

                                                    if let Some(pres_id) = pres_id {
                                                        let playlist_id = playlist_id.clone();
                                                        leptos::task::spawn_local(async move {
                                                            // Get current playlist entries
                                                            if let Ok(pl) = crate::api::playlists::get_playlist(&playlist_id).await {
                                                                let mut entries: Vec<crate::api::playlists::PlaylistEntryPayload> = pl.entries.iter().map(|e| {
                                                                    match &e.kind {
                                                                        presenter_core::playlist::PlaylistEntryKind::Presentation { presentation_id, .. } => {
                                                                            crate::api::playlists::PlaylistEntryPayload::Presentation {
                                                                                entry_id: Some(e.id.to_string()),
                                                                                presentation_id: presentation_id.to_string(),
                                                                            }
                                                                        }
                                                                        presenter_core::playlist::PlaylistEntryKind::Separator { name } => {
                                                                            crate::api::playlists::PlaylistEntryPayload::Separator {
                                                                                entry_id: Some(e.id.to_string()),
                                                                                name: name.clone(),
                                                                            }
                                                                        }
                                                                    }
                                                                }).collect();
                                                                // Add new presentation
                                                                entries.push(crate::api::playlists::PlaylistEntryPayload::Presentation {
                                                                    entry_id: None,
                                                                    presentation_id: pres_id,
                                                                });
                                                                if let Ok(updated) = crate::api::playlists::replace_entries(&playlist_id, entries).await {
                                                                    // Update selected playlist if it's the one we modified
                                                                    selected_playlist.update(|sel| {
                                                                        if sel.as_ref().map(|s| s.id.to_string()) == Some(playlist_id.clone()) {
                                                                            *sel = Some(updated.clone());
                                                                        }
                                                                    });
                                                                }
                                                                // Refresh playlists list
                                                                if let Ok(pls) = crate::api::playlists::list_playlists().await {
                                                                    playlists.set(pls);
                                                                }
                                                            }
                                                        });
                                                    }
                                                }
                                            }
                                        }
                                    >
                                        <button
                                            type="button"
                                            class="operator__list-button"
                                            data-role="playlist-item"
                                            data-playlist-id=id_for_btn
                                            data-active=move || if is_active { "true" } else { "false" }
                                            on:click=move |_| {
                                                select_playlist(id_for_click.clone(), name_for_click.clone());
                                            }
                                        >
                                            <span class="operator__list-label">{name}</span>
                                            <span class="operator__list-meta" data-role="playlist-count">{count}</span>
                                        </button>
                                        <div class="operator__list-actions">
                                            <button
                                                type="button"
                                                class="operator__list-action operator__list-action--icon operator__list-action--menu"
                                                data-action="playlist-edit"
                                                data-playlist-id=id_for_edit
                                                aria-label="Edit playlist"
                                                on:click=move |ev: leptos::ev::MouseEvent| {
                                                    ev.stop_propagation();
                                                    op_for_edit.modal_mode.set("edit".to_string());
                                                    op_for_edit.modal_target_id.set(Some(id_for_modal.clone()));
                                                    modal::open_modal(&op_for_edit, "playlist-edit");
                                                }
                                            >
                                                "\u{22ee}"
                                            </button>
                                        </div>
                                    </li>
                                }
                            }).collect_view()}
                        </div>
                    }.into_any()
                }}
            </ul>
        </section>
    }
}
