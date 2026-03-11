use leptos::prelude::*;
use presenter_core::playlist::PlaylistEntryKind;

use crate::state::operator::OperatorState;
use crate::state::AppContext;

/// Playlist sidebar list component.
#[component]
pub fn PlaylistList(ctx: AppContext, op: OperatorState) -> impl IntoView {
    let playlists = ctx.playlists;
    let selected_pl = ctx.selected_playlist_id;
    let selected_lib = ctx.selected_library_id;
    let presentations = ctx.presentations;
    let context_title = ctx.context_title;
    let selected_pres_id = ctx.selected_presentation_id;
    let selected_pres = ctx.selected_presentation;

    let on_select = move |id: String, name: String, entries: Vec<presenter_core::PlaylistEntry>| {
        // Selecting a playlist clears library selection
        selected_lib.set(None);
        crate::state::session::remove("activeLibraryId");
        selected_pl.set(Some(id.clone()));
        crate::state::session::set("activePlaylistId", &id);
        context_title.set(name);
        // Clear presentation selection
        selected_pres_id.set(None);
        selected_pres.set(None);
        // Convert playlist entries to presentation summaries
        let pres: Vec<presenter_core::PresentationSummary> = entries
            .into_iter()
            .filter_map(|entry| match entry.kind {
                PlaylistEntryKind::Presentation {
                    presentation_id, ..
                } => {
                    Some(presenter_core::PresentationSummary::new(
                        presentation_id,
                        String::new(), // Name will be resolved client-side
                    ))
                }
                PlaylistEntryKind::Separator { .. } => None,
            })
            .collect();
        presentations.set(pres);
    };

    let pl_count = move || playlists.get().len();
    let op_clone = op.clone();

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
                        on:click={
                            let op = op_clone.clone();
                            move |_| crate::components::modal::open_modal(&op, "playlist-list")
                        }
                    >{move || pl_count().to_string()}</button>
                    <button
                        type="button"
                        data-role="playlist-create"
                        aria-label="Create playlist"
                        title="Create playlist"
                        on:click={
                            let op = op_clone.clone();
                            move |_| crate::components::modal::open_modal(&op, "playlist-create")
                        }
                    >"+"</button>
                </div>
            </header>
            <ul class="operator__list" data-role="playlist-list">
                {move || {
                    playlists.get().into_iter().map(|pl| {
                        let id = pl.id.to_string();
                        let name = pl.name.clone();
                        let entry_count = pl.entries.len();
                        let entries = pl.entries.clone();
                        let id_click = id.clone();
                        let name_click = name.clone();
                        let id_cmp = id.clone();
                        view! {
                            <li class="operator__list-item">
                                <button
                                    type="button"
                                    class="operator__list-button"
                                    data-role="playlist-item"
                                    data-playlist-id={id.clone()}
                                    data-active=move || if selected_pl.get().as_deref() == Some(&id_cmp) { "true" } else { "false" }
                                    on:click={
                                        let id = id_click.clone();
                                        let name = name_click.clone();
                                        let entries = entries.clone();
                                        move |_| on_select(id.clone(), name.clone(), entries.clone())
                                    }
                                >
                                    <span class="operator__list-label">{name}</span>
                                    <span class="operator__list-meta" data-role="playlist-count">{entry_count}</span>
                                </button>
                                <div class="operator__list-actions">
                                    <button
                                        type="button"
                                        class="operator__list-action operator__list-action--icon operator__list-action--menu"
                                        data-action="playlist-edit"
                                        data-playlist-id={id.clone()}
                                        aria-label="Edit playlist"
                                        on:click={
                                            let op = op_clone.clone();
                                            let id = id.clone();
                                            move |_| {
                                                op.modal_target_id.set(Some(id.clone()));
                                                crate::components::modal::open_modal(&op, "playlist-edit");
                                            }
                                        }
                                    >{"\u{22EE}"}</button>
                                </div>
                            </li>
                        }
                    }).collect::<Vec<_>>()
                }}
            </ul>
        </section>
    }
}
