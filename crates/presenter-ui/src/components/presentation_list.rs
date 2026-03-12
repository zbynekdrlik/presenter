use leptos::prelude::*;

use crate::components::modal;
use crate::state::operator::OperatorState;
use crate::state::AppContext;

#[component]
pub fn PresentationList() -> impl IntoView {
    let ctx = use_context::<AppContext>().expect("AppContext");
    let _op = use_context::<OperatorState>().expect("OperatorState");

    let select_presentation = move |id: String| {
        ctx.selected_presentation_id.set(Some(id.clone()));
        crate::state::session::set("currentPresentationId", &id);

        // Check slides cache first
        let cached = ctx.slides_cache.get_untracked().get(&id).cloned();
        if let Some(slides) = cached {
            ctx.selected_presentation.update(|p| {
                if let Some(pres) = p.as_mut() {
                    if pres.id.to_string() == id {
                        pres.slides = slides;
                    }
                }
            });
        }

        let id_clone = id.clone();
        leptos::task::spawn_local(async move {
            if let Ok(detail) = crate::api::presentations::get_presentation(&id_clone).await {
                let ctx = use_context::<AppContext>().expect("AppContext");
                // Cache slides
                ctx.slides_cache.update(|cache| {
                    cache.insert(id_clone.clone(), detail.presentation.slides.clone());
                });
                ctx.selected_presentation.set(Some(detail.presentation));
            }
        });
    };

    let on_create = move |_| {
        let op = use_context::<OperatorState>().expect("OperatorState");
        let ctx = use_context::<AppContext>().expect("AppContext");
        let has_playlist = ctx.selected_playlist_id.get_untracked().is_some();

        if has_playlist {
            // When playlist is active, prompt for separator name
            let name = crate::utils::window::window()
                .prompt_with_message("Separator name:")
                .ok()
                .flatten()
                .unwrap_or_default();
            let name = name.trim().to_string();
            if name.is_empty() {
                return;
            }
            let playlist_id = ctx.selected_playlist_id.get_untracked().unwrap_or_default();
            let selected_playlist = ctx.selected_playlist;
            let playlists = ctx.playlists;
            leptos::task::spawn_local(async move {
                // Build current entries + new separator
                let current = selected_playlist.get_untracked();
                let mut entries: Vec<crate::api::playlists::PlaylistEntryPayload> = current
                    .as_ref()
                    .map(|pl| {
                        pl.entries
                            .iter()
                            .map(|e| match &e.kind {
                                presenter_core::playlist::PlaylistEntryKind::Presentation {
                                    presentation_id,
                                    ..
                                } => crate::api::playlists::PlaylistEntryPayload::Presentation {
                                    entry_id: Some(e.id.to_string()),
                                    presentation_id: presentation_id.to_string(),
                                },
                                presenter_core::playlist::PlaylistEntryKind::Separator { name } => {
                                    crate::api::playlists::PlaylistEntryPayload::Separator {
                                        entry_id: Some(e.id.to_string()),
                                        name: name.clone(),
                                    }
                                }
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                entries.push(crate::api::playlists::PlaylistEntryPayload::Separator {
                    entry_id: None,
                    name,
                });
                if let Ok(updated) =
                    crate::api::playlists::replace_entries(&playlist_id, entries).await
                {
                    selected_playlist.set(Some(updated.clone()));
                    // Refresh presentation list from playlist entries
                    let ctx = use_context::<AppContext>().expect("AppContext");
                    rebuild_playlist_presentations(&ctx, &updated);
                }
                if let Ok(pls) = crate::api::playlists::list_playlists().await {
                    playlists.set(pls);
                }
            });
        } else {
            op.modal_mode.set("create".to_string());
            modal::open_modal(&op, "presentation-create");
        }
    };

    view! {
        <div class="operator__catalog-bottom" data-role="catalog-bottom" data-dropzone-target="presentations">
            <header class="operator__group-header operator__presentations-header">
                <h2 data-role="context-title">
                    {move || ctx.context_title.get()}
                </h2>
                <div class="operator__group-controls">
                    <span class="operator__group-count operator__group-count--static" data-role="presentation-count">
                        {move || {
                            let count = ctx.presentations.get().len();
                            if count > 0 { count.to_string() } else { "\u{2014}".to_string() }
                        }}
                    </span>
                    <button
                        type="button"
                        data-role="presentation-create"
                        aria-label="Add presentation or separator"
                        title=move || {
                            if ctx.selected_playlist_id.get().is_some() {
                                "Add separator to playlist"
                            } else {
                                "Add presentation"
                            }
                        }
                        on:click=on_create
                    >
                        "+"
                    </button>
                </div>
            </header>
            <ul class="operator__presentation-list" data-role="presentation-list">
                {move || {
                    let active_id = ctx.selected_presentation_id.get();
                    let mode = ctx.mode.get();
                    let is_edit = mode == "edit";
                    let stage_pres_id = ctx.stage_snapshot.get()
                        .and_then(|s| s.presentation_id.map(|id| id.to_string()));
                    let has_playlist = ctx.selected_playlist_id.get().is_some();
                    let selected_playlist = ctx.selected_playlist.get();
                    let pres_index = ctx.presentation_index.get();

                    // If a playlist is active, render from playlist entries (including separators)
                    if has_playlist {
                        if let Some(playlist) = &selected_playlist {
                            if playlist.entries.is_empty() {
                                return view! {
                                    <li class="empty">"Playlist is empty. Drag songs from a library or add a separator."</li>
                                }.into_any();
                            }
                            return playlist.entries.iter().enumerate().map(|(idx, entry)| {
                                let entry_id = entry.id.to_string();
                                match &entry.kind {
                                    presenter_core::playlist::PlaylistEntryKind::Separator { name } => {
                                        let sep_name = name.clone();
                                        let entry_id_rename = entry_id.clone();
                                        let entry_id_remove = entry_id.clone();
                                        let entry_id_li = entry_id.clone();
                                        view! {
                                            <li
                                                class="operator__presentation-item operator__presentation-item--separator"
                                                data-role="presentation-item"
                                                data-type="separator"
                                                data-entry-id=entry_id_li
                                                data-entry-index=idx
                                            >
                                                <span>{sep_name}</span>
                                                <span class="operator__presentation-meta">"Separator"</span>
                                                {is_edit.then(|| {
                                                    let playlist_id = ctx.selected_playlist_id.get_untracked().unwrap_or_default();
                                                    let playlist_id_remove = playlist_id.clone();
                                                    view! {
                                                        <div class="operator__presentation-actions">
                                                            <button
                                                                type="button"
                                                                class="operator__presentation-action"
                                                                data-action="separator-rename"
                                                                data-entry-id=entry_id_rename.clone()
                                                                on:click=move |ev: leptos::ev::MouseEvent| {
                                                                    ev.stop_propagation();
                                                                    // Rename separator via prompt
                                                                    let new_name = crate::utils::window::window()
                                                                        .prompt_with_message("Rename separator:")
                                                                        .ok().flatten().unwrap_or_default();
                                                                    if new_name.trim().is_empty() { return; }
                                                                    let entry_id = entry_id_rename.clone();
                                                                    let pl_id = playlist_id.clone();
                                                                    let ctx = use_context::<AppContext>().expect("AppContext");
                                                                    let selected_playlist = ctx.selected_playlist;
                                                                    let playlists = ctx.playlists;
                                                                    leptos::task::spawn_local(async move {
                                                                        let current = selected_playlist.get_untracked();
                                                                        if let Some(pl) = current {
                                                                            let entries: Vec<_> = pl.entries.iter().map(|e| {
                                                                                if e.id.to_string() == entry_id {
                                                                                    crate::api::playlists::PlaylistEntryPayload::Separator {
                                                                                        entry_id: Some(e.id.to_string()),
                                                                                        name: new_name.trim().to_string(),
                                                                                    }
                                                                                } else {
                                                                                    entry_to_payload(e)
                                                                                }
                                                                            }).collect();
                                                                            if let Ok(updated) = crate::api::playlists::replace_entries(&pl_id, entries).await {
                                                                                selected_playlist.set(Some(updated.clone()));
                                                                                let ctx = use_context::<AppContext>().expect("AppContext");
                                                                                rebuild_playlist_presentations(&ctx, &updated);
                                                                            }
                                                                            if let Ok(pls) = crate::api::playlists::list_playlists().await {
                                                                                playlists.set(pls);
                                                                            }
                                                                        }
                                                                    });
                                                                }
                                                            >
                                                                "\u{270e}"
                                                            </button>
                                                            <button
                                                                type="button"
                                                                class="operator__presentation-action operator__presentation-action--remove"
                                                                data-action="entry-remove"
                                                                data-entry-id=entry_id_remove.clone()
                                                                on:click=move |ev: leptos::ev::MouseEvent| {
                                                                    ev.stop_propagation();
                                                                    let entry_id = entry_id_remove.clone();
                                                                    let pl_id = playlist_id_remove.clone();
                                                                    let ctx = use_context::<AppContext>().expect("AppContext");
                                                                    let selected_playlist = ctx.selected_playlist;
                                                                    let playlists = ctx.playlists;
                                                                    leptos::task::spawn_local(async move {
                                                                        let current = selected_playlist.get_untracked();
                                                                        if let Some(pl) = current {
                                                                            let entries: Vec<_> = pl.entries.iter()
                                                                                .filter(|e| e.id.to_string() != entry_id)
                                                                                .map(entry_to_payload)
                                                                                .collect();
                                                                            if let Ok(updated) = crate::api::playlists::replace_entries(&pl_id, entries).await {
                                                                                selected_playlist.set(Some(updated.clone()));
                                                                                let ctx = use_context::<AppContext>().expect("AppContext");
                                                                                rebuild_playlist_presentations(&ctx, &updated);
                                                                            }
                                                                            if let Ok(pls) = crate::api::playlists::list_playlists().await {
                                                                                playlists.set(pls);
                                                                            }
                                                                        }
                                                                    });
                                                                }
                                                            >
                                                                "\u{00d7}"
                                                            </button>
                                                        </div>
                                                    }
                                                })}
                                            </li>
                                        }.into_any()
                                    }
                                    presenter_core::playlist::PlaylistEntryKind::Presentation { presentation_id, .. } => {
                                        let id = presentation_id.to_string();
                                        let is_active = active_id.as_deref() == Some(&id);
                                        let is_stage_active = stage_pres_id.as_deref() == Some(&id);
                                        let id_for_click = id.clone();
                                        let id_for_li = id.clone();
                                        let entry_id_li = entry_id.clone();
                                        let entry_id_remove = entry_id.clone();
                                        let id_for_drag = id.clone();

                                        // Look up presentation name from presentations list or index
                                        let presentations = ctx.presentations.get();
                                        let pres_name = presentations.iter()
                                            .find(|p| p.id.to_string() == id)
                                            .map(|p| p.name.clone())
                                            .unwrap_or_default();

                                        // Look up library name from index
                                        let lib_name = pres_index.get(&id).cloned().unwrap_or_default();

                                        view! {
                                            <li
                                                class=move || {
                                                    let mut c = "operator__presentation-item".to_string();
                                                    if is_active { c.push_str(" is-active"); }
                                                    if is_stage_active { c.push_str(" is-stage-active"); }
                                                    c
                                                }
                                                data-role="presentation-item"
                                                data-type="presentation"
                                                data-presentation-id=id_for_li
                                                data-entry-id=entry_id_li
                                                data-entry-index=idx
                                                attr:data-active=move || if is_active { "true" } else { "false" }
                                                draggable="true"
                                                on:click={
                                                    let id = id_for_click.clone();
                                                    move |_| select_presentation(id.clone())
                                                }
                                                on:dragstart=move |ev: web_sys::DragEvent| {
                                                    if let Some(dt) = ev.data_transfer() {
                                                        let _ = dt.set_data("text/plain", &id_for_drag);
                                                        let _ = dt.set_data("application/x-presentation-id", &id_for_drag);
                                                    }
                                                }
                                            >
                                                <span>{pres_name}</span>
                                                <span class="operator__presentation-meta">{lib_name}</span>
                                                {is_edit.then(|| {
                                                    let id_for_rename = id.clone();
                                                    let playlist_id = ctx.selected_playlist_id.get_untracked().unwrap_or_default();
                                                    view! {
                                                        <div class="operator__presentation-actions">
                                                            <button
                                                                type="button"
                                                                class="operator__presentation-action"
                                                                data-action="presentation-rename"
                                                                data-presentation-id=id_for_rename.clone()
                                                                on:click=move |ev: leptos::ev::MouseEvent| {
                                                                    ev.stop_propagation();
                                                                    let op = use_context::<OperatorState>().expect("OperatorState");
                                                                    op.modal_mode.set("edit".to_string());
                                                                    op.modal_target_id.set(Some(id_for_rename.clone()));
                                                                    modal::open_modal(&op, "presentation-edit");
                                                                }
                                                            >
                                                                "\u{270e}"
                                                            </button>
                                                            <button
                                                                type="button"
                                                                class="operator__presentation-action operator__presentation-action--remove"
                                                                data-action="entry-remove"
                                                                data-entry-id=entry_id_remove.clone()
                                                                on:click=move |ev: leptos::ev::MouseEvent| {
                                                                    ev.stop_propagation();
                                                                    let entry_id = entry_id_remove.clone();
                                                                    let pl_id = playlist_id.clone();
                                                                    let ctx = use_context::<AppContext>().expect("AppContext");
                                                                    let selected_playlist = ctx.selected_playlist;
                                                                    let playlists = ctx.playlists;
                                                                    leptos::task::spawn_local(async move {
                                                                        let current = selected_playlist.get_untracked();
                                                                        if let Some(pl) = current {
                                                                            let entries: Vec<_> = pl.entries.iter()
                                                                                .filter(|e| e.id.to_string() != entry_id)
                                                                                .map(entry_to_payload)
                                                                                .collect();
                                                                            if let Ok(updated) = crate::api::playlists::replace_entries(&pl_id, entries).await {
                                                                                selected_playlist.set(Some(updated.clone()));
                                                                                let ctx = use_context::<AppContext>().expect("AppContext");
                                                                                rebuild_playlist_presentations(&ctx, &updated);
                                                                            }
                                                                            if let Ok(pls) = crate::api::playlists::list_playlists().await {
                                                                                playlists.set(pls);
                                                                            }
                                                                        }
                                                                    });
                                                                }
                                                            >
                                                                "\u{00d7}"
                                                            </button>
                                                        </div>
                                                    }
                                                })}
                                            </li>
                                        }.into_any()
                                    }
                                }
                            }).collect_view().into_any();
                        }
                    }

                    // Library context: show presentations normally
                    let presentations = ctx.presentations.get();

                    if presentations.is_empty() {
                        let msg = if ctx.selected_library_id.get().is_some() {
                            "No presentations in this library."
                        } else {
                            "Select a library or playlist to view presentations."
                        };
                        return view! {
                            <li class="empty">{msg}</li>
                        }.into_any();
                    }

                    presentations.into_iter().map(|pres| {
                        let id = pres.id.to_string();
                        let name = pres.name.clone();
                        let is_active = active_id.as_deref() == Some(&id);
                        let is_stage_active = stage_pres_id.as_deref() == Some(&id);
                        let id_for_click = id.clone();
                        let id_for_rename = id.clone();
                        let id_for_drag = id.clone();
                        let id_for_li = id.clone();

                        let lib_name = ctx.context_title.get();

                        view! {
                            <li
                                class=move || {
                                    let mut c = "operator__presentation-item".to_string();
                                    if is_active { c.push_str(" is-active"); }
                                    if is_stage_active { c.push_str(" is-stage-active"); }
                                    c
                                }
                                data-role="presentation-item"
                                data-type="presentation"
                                data-presentation-id=id_for_li
                                attr:data-active=move || if is_active { "true" } else { "false" }
                                draggable="true"
                                on:click={
                                    let id = id_for_click.clone();
                                    move |_| select_presentation(id.clone())
                                }
                                on:dragstart=move |ev: web_sys::DragEvent| {
                                    if let Some(dt) = ev.data_transfer() {
                                        let _ = dt.set_data("text/plain", &id_for_drag);
                                        let _ = dt.set_data("application/x-presentation-id", &id_for_drag);
                                    }
                                }
                            >
                                <span>{name}</span>
                                <span class="operator__presentation-meta">{lib_name}</span>
                                {is_edit.then(|| view! {
                                    <div class="operator__presentation-actions">
                                        <button
                                            type="button"
                                            class="operator__presentation-action"
                                            data-action="presentation-rename"
                                            data-presentation-id=id_for_rename.clone()
                                            on:click=move |ev: leptos::ev::MouseEvent| {
                                                ev.stop_propagation();
                                                let op = use_context::<OperatorState>().expect("OperatorState");
                                                op.modal_mode.set("edit".to_string());
                                                op.modal_target_id.set(Some(id_for_rename.clone()));
                                                modal::open_modal(&op, "presentation-edit");
                                            }
                                        >
                                            "\u{270e}"
                                        </button>
                                    </div>
                                })}
                            </li>
                        }
                    }).collect_view().into_any()
                }}
            </ul>
        </div>
    }
}

/// Convert a playlist entry to an API payload for replace_entries calls.
fn entry_to_payload(
    e: &presenter_core::playlist::PlaylistEntry,
) -> crate::api::playlists::PlaylistEntryPayload {
    match &e.kind {
        presenter_core::playlist::PlaylistEntryKind::Presentation {
            presentation_id, ..
        } => crate::api::playlists::PlaylistEntryPayload::Presentation {
            entry_id: Some(e.id.to_string()),
            presentation_id: presentation_id.to_string(),
        },
        presenter_core::playlist::PlaylistEntryKind::Separator { name } => {
            crate::api::playlists::PlaylistEntryPayload::Separator {
                entry_id: Some(e.id.to_string()),
                name: name.clone(),
            }
        }
    }
}

/// Rebuild the presentations signal from a playlist's entries.
fn rebuild_playlist_presentations(ctx: &AppContext, playlist: &presenter_core::Playlist) {
    let summaries: Vec<presenter_core::PresentationSummary> = playlist
        .entries
        .iter()
        .filter_map(|e| match &e.kind {
            presenter_core::playlist::PlaylistEntryKind::Presentation {
                presentation_id, ..
            } => Some(presenter_core::PresentationSummary::new(
                *presentation_id,
                String::new(),
            )),
            _ => None,
        })
        .collect();
    ctx.presentations.set(summaries);
}
