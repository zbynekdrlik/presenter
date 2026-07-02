use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::components::modal;
use crate::state::operator::OperatorState;
use crate::state::AppContext;

use super::presentation_list_drag::{
    drop_side_for_event, entry_to_payload, get_dragging_entry, get_entry_id, handle_search_drop,
    make_fixed_drop_handlers, render_list_spacer, set_dragging_entry, FixedDropHandlers,
};

#[component]
pub fn PresentationList() -> impl IntoView {
    let ctx = use_ctx!(AppContext);
    let op = use_ctx!(OperatorState);

    let select_presentation = move |id: String, entry_index: Option<u32>| {
        ctx.selected_presentation_id.set(Some(id.clone()));
        // #496: remember which playlist OCCURRENCE was picked so triggers send
        // the right entry index for a repeated song (None for library picks).
        ctx.selected_entry_index.set(entry_index);
        crate::state::session::set("currentPresentationId", &id);

        // #515: NO synchronous "show cached slides first" step here anymore.
        // As written (guarded by `pres.id.to_string() == id`, checked against
        // the CURRENT pre-click selection), that branch could only ever fire
        // when RE-selecting the presentation ALREADY on screen — it can't
        // help a genuine switch to a different presentation, since `pres`
        // still holds the OLD id at click time. For a re-select it was
        // actively harmful: `ctx.slides_cache` is populated only from GET
        // responses (never updated by a save), so re-clicking an
        // already-open song right after editing it (e.g. typing a stage
        // hand-off message) unconditionally reset the just-edited slide back
        // to its pre-edit cached content, synchronously, with no guard at
        // all. The `get_presentation` fetch below (seq-guarded) is the only
        // source of truth needed here.

        // Capture signals OUTSIDE async block - context may not be available inside spawn_local
        let slides_cache_signal = ctx.slides_cache;
        let selected_presentation_signal = ctx.selected_presentation;
        let id_clone = id.clone();
        // #515: capture the edit generation BEFORE issuing the fetch. If a
        // slide-content save lands while this GET is still in flight (a
        // very plausible race right after opening a song and immediately
        // typing a stage/main/translation edit), the response below reflects
        // pre-edit content — apply it ONLY if no save has landed since,
        // otherwise it would clobber the newer edit with stale data.
        let seq_at_fetch = op.slide_edit_seq.get_untracked();
        let slide_edit_seq = op.slide_edit_seq;
        leptos::task::spawn_local(async move {
            if let Ok(detail) = crate::api::presentations::get_presentation(&id_clone).await {
                if slide_edit_seq.get_untracked() != seq_at_fetch {
                    return;
                }
                // Cache slides
                slides_cache_signal.update(|cache| {
                    cache.insert(id_clone.clone(), detail.presentation.slides.clone());
                });
                selected_presentation_signal.set(Some(detail.presentation));
            }
        });
    };

    let on_create = {
        let op = op.clone();
        move |_| {
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
                                    } => {
                                        crate::api::playlists::PlaylistEntryPayload::Presentation {
                                            entry_id: Some(e.id.to_string()),
                                            presentation_id: presentation_id.to_string(),
                                        }
                                    }
                                    presenter_core::playlist::PlaylistEntryKind::Separator {
                                        name,
                                    } => crate::api::playlists::PlaylistEntryPayload::Separator {
                                        entry_id: Some(e.id.to_string()),
                                        name: name.clone(),
                                    },
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
                    }
                    if let Ok(pls) = crate::api::playlists::list_playlists().await {
                        playlists.set(pls);
                    }
                });
            } else {
                op.modal_mode.set("create".to_string());
                modal::open_modal(&op, "presentation-create");
            }
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
                                let playlist_id = ctx.selected_playlist_id.get_untracked().unwrap_or_default();
                                let FixedDropHandlers {
                                    on_dragover,
                                    on_dragleave,
                                    on_drop,
                                } = make_fixed_drop_handlers(
                                    0,
                                    "before",
                                    op.clone(),
                                    playlist_id,
                                    ctx.selected_playlist,
                                    ctx.playlists,
                                    ctx.toast_message,
                                    ctx.toast_variant,
                                );
                                return view! {
                                    <li
                                        class="empty operator__list-empty-drop"
                                        data-role="presentation-empty-drop"
                                        on:dragover=on_dragover
                                        on:dragleave=on_dragleave
                                        on:drop=on_drop
                                    >
                                        "Playlist is empty. Drag songs from a library or add a separator."
                                    </li>
                                }.into_any();
                            }
                            let entries_view: Vec<_> = playlist.entries.iter().enumerate().map(|(idx, entry)| {
                                let entry_id = entry.id.to_string();
                                match &entry.kind {
                                    presenter_core::playlist::PlaylistEntryKind::Separator { name } => {
                                        let sep_name = name.clone();
                                        let entry_id_rename = entry_id.clone();
                                        let entry_id_remove = entry_id.clone();
                                        let entry_id_li = entry_id.clone();
                                        let entry_id_drag = entry_id.clone();
                                        let entry_id_drop = entry_id.clone();
                                        let playlist_id_reorder = ctx.selected_playlist_id.get_untracked().unwrap_or_default();
                                        view! {
                                            <li
                                                class="operator__presentation-item operator__presentation-item--separator"
                                                data-role="presentation-item"
                                                data-type="separator"
                                                data-entry-id=entry_id_li
                                                data-entry-index=idx
                                                draggable=is_edit.to_string()
                                                on:dragstart=move |ev: web_sys::DragEvent| {
                                                    if !is_edit { return; }
                                                    if let Some(dt) = ev.data_transfer() {
                                                        let _ = dt.set_data("application/x-entry-id", &entry_id_drag);
                                                        dt.set_effect_allowed("move");
                                                    }
                                                    set_dragging_entry(Some(entry_id_drag.clone()));
                                                }
                                                on:dragend=move |_| {
                                                    set_dragging_entry(None);
                                                }
                                                on:dragover={
                                                    let op_for_dragover = op.clone();
                                                    move |ev: web_sys::DragEvent| {
                                                        let is_reorder = get_dragging_entry().is_some();
                                                        let is_search = op_for_dragover.dragging_from_search.get_untracked();
                                                        if !is_reorder && !is_search {
                                                            return;
                                                        }
                                                        ev.prevent_default();
                                                        // Only the search-drag path renders the line indicator.
                                                        // Within-playlist reorder uses slot-replacement (existing behavior).
                                                        if !is_search {
                                                            return;
                                                        }
                                                        if let Some(target) = ev
                                                            .current_target()
                                                            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                                                        {
                                                            let side = drop_side_for_event(&ev, &target);
                                                            let _ = target.set_attribute("data-drop-position", side);
                                                        }
                                                    }
                                                }
                                                on:dragleave={
                                                    let op_for_dragleave = op.clone();
                                                    move |ev: web_sys::DragEvent| {
                                                        if !op_for_dragleave.dragging_from_search.get_untracked() {
                                                            return;
                                                        }
                                                        if let Some(target) = ev
                                                            .current_target()
                                                            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                                                        {
                                                            let _ = target.remove_attribute("data-drop-position");
                                                        }
                                                    }
                                                }
                                                on:drop={
                                                    let target_entry_id = entry_id_drop.clone();
                                                    let playlist_id = playlist_id_reorder.clone();
                                                    let selected_playlist = ctx.selected_playlist;
                                                    let playlists = ctx.playlists;
                                                    let toast_message = ctx.toast_message;
                                                    let toast_variant = ctx.toast_variant;
                                                    let op_for_drop = op.clone();
                                                    let target_index = idx;
                                                    move |ev: web_sys::DragEvent| {
                                                        ev.prevent_default();

                                                        // ----- New: search-drag path (issue #274) -----
                                                        if op_for_drop.dragging_from_search.get_untracked() {
                                                            handle_search_drop(
                                                                &ev,
                                                                target_index,
                                                                playlist_id.clone(),
                                                                selected_playlist,
                                                                playlists,
                                                                toast_message,
                                                                toast_variant,
                                                            );
                                                            op_for_drop.dragging_from_search.set(false);
                                                            op_for_drop.search_dragging.set(false);
                                                            return;
                                                        }

                                                        // ----- Existing: within-playlist reorder path (UNCHANGED) -----
                                                        if let Some(dragged_id) = get_dragging_entry() {
                                                            if dragged_id == target_entry_id { return; }
                                                            let playlist_id = playlist_id.clone();
                                                            let target_entry_id = target_entry_id.clone();
                                                            leptos::task::spawn_local(async move {
                                                                let current = selected_playlist.get_untracked();
                                                                if let Some(pl) = current {
                                                                    let mut entries: Vec<_> = pl.entries.iter().map(entry_to_payload).collect();
                                                                    // Find positions
                                                                    let drag_pos = entries.iter().position(|e| get_entry_id(e) == Some(&dragged_id));
                                                                    let target_pos = entries.iter().position(|e| get_entry_id(e) == Some(&target_entry_id));
                                                                    if let (Some(from), Some(to)) = (drag_pos, target_pos) {
                                                                        let item = entries.remove(from);
                                                                        entries.insert(to, item);
                                                                        if let Ok(updated) = crate::api::playlists::replace_entries(&playlist_id, entries).await {
                                                                            selected_playlist.set(Some(updated.clone()));
                                                                        }
                                                                        if let Ok(pls) = crate::api::playlists::list_playlists().await {
                                                                            playlists.set(pls);
                                                                        }
                                                                    }
                                                                }
                                                            });
                                                        }
                                                        set_dragging_entry(None);
                                                    }
                                                }
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
                                                                    // Capture signals OUTSIDE async block
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
                                                                    // Capture signals OUTSIDE async block
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
                                        let entry_id_drag = entry_id.clone();
                                        let entry_id_drop = entry_id.clone();
                                        let playlist_id_reorder = ctx.selected_playlist_id.get_untracked().unwrap_or_default();

                                        // Read presentation name directly from the playlist entry
                                        // (server enriches it via fetch_presentation_names_for_playlist).
                                        let pres_name = match &entry.kind {
                                            presenter_core::playlist::PlaylistEntryKind::Presentation {
                                                presentation_name, ..
                                            } => presentation_name.clone().unwrap_or_default(),
                                            _ => String::new(),
                                        };

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
                                                data-active=move || if is_active { "true" } else { "false" }
                                                draggable="true"
                                                on:click={
                                                    let id = id_for_click.clone();
                                                    move |_| select_presentation(id.clone(), Some(idx as u32))
                                                }
                                                on:dragstart=move |ev: web_sys::DragEvent| {
                                                    if let Some(dt) = ev.data_transfer() {
                                                        let _ = dt.set_data("text/plain", &id_for_drag);
                                                        let _ = dt.set_data("application/x-presentation-id", &id_for_drag);
                                                        let _ = dt.set_data("application/x-entry-id", &entry_id_drag);
                                                        dt.set_effect_allowed("move");
                                                    }
                                                    set_dragging_entry(Some(entry_id_drag.clone()));
                                                }
                                                on:dragend=move |_| {
                                                    set_dragging_entry(None);
                                                }
                                                on:dragover={
                                                    let op_for_dragover = op.clone();
                                                    move |ev: web_sys::DragEvent| {
                                                        let is_reorder = get_dragging_entry().is_some();
                                                        let is_search = op_for_dragover.dragging_from_search.get_untracked();
                                                        if !is_reorder && !is_search {
                                                            return;
                                                        }
                                                        ev.prevent_default();
                                                        // Only the search-drag path renders the line indicator.
                                                        // Within-playlist reorder uses slot-replacement (existing behavior).
                                                        if !is_search {
                                                            return;
                                                        }
                                                        if let Some(target) = ev
                                                            .current_target()
                                                            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                                                        {
                                                            let side = drop_side_for_event(&ev, &target);
                                                            let _ = target.set_attribute("data-drop-position", side);
                                                        }
                                                    }
                                                }
                                                on:dragleave={
                                                    let op_for_dragleave = op.clone();
                                                    move |ev: web_sys::DragEvent| {
                                                        if !op_for_dragleave.dragging_from_search.get_untracked() {
                                                            return;
                                                        }
                                                        if let Some(target) = ev
                                                            .current_target()
                                                            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                                                        {
                                                            let _ = target.remove_attribute("data-drop-position");
                                                        }
                                                    }
                                                }
                                                on:drop={
                                                    let target_entry_id = entry_id_drop.clone();
                                                    let playlist_id = playlist_id_reorder.clone();
                                                    let selected_playlist = ctx.selected_playlist;
                                                    let playlists = ctx.playlists;
                                                    let toast_message = ctx.toast_message;
                                                    let toast_variant = ctx.toast_variant;
                                                    let op_for_drop = op.clone();
                                                    let target_index = idx;
                                                    move |ev: web_sys::DragEvent| {
                                                        ev.prevent_default();

                                                        // ----- New: search-drag path (issue #274) -----
                                                        if op_for_drop.dragging_from_search.get_untracked() {
                                                            handle_search_drop(
                                                                &ev,
                                                                target_index,
                                                                playlist_id.clone(),
                                                                selected_playlist,
                                                                playlists,
                                                                toast_message,
                                                                toast_variant,
                                                            );
                                                            op_for_drop.dragging_from_search.set(false);
                                                            op_for_drop.search_dragging.set(false);
                                                            return;
                                                        }

                                                        // ----- Existing: within-playlist reorder path (UNCHANGED) -----
                                                        if let Some(dragged_id) = get_dragging_entry() {
                                                            if dragged_id == target_entry_id { return; }
                                                            let playlist_id = playlist_id.clone();
                                                            let target_entry_id = target_entry_id.clone();
                                                            leptos::task::spawn_local(async move {
                                                                let current = selected_playlist.get_untracked();
                                                                if let Some(pl) = current {
                                                                    let mut entries: Vec<_> = pl.entries.iter().map(entry_to_payload).collect();
                                                                    let drag_pos = entries.iter().position(|e| get_entry_id(e) == Some(&dragged_id));
                                                                    let target_pos = entries.iter().position(|e| get_entry_id(e) == Some(&target_entry_id));
                                                                    if let (Some(from), Some(to)) = (drag_pos, target_pos) {
                                                                        let item = entries.remove(from);
                                                                        entries.insert(to, item);
                                                                        if let Ok(updated) = crate::api::playlists::replace_entries(&playlist_id, entries).await {
                                                                            selected_playlist.set(Some(updated.clone()));
                                                                        }
                                                                        if let Ok(pls) = crate::api::playlists::list_playlists().await {
                                                                            playlists.set(pls);
                                                                        }
                                                                    }
                                                                }
                                                            });
                                                        }
                                                        set_dragging_entry(None);
                                                    }
                                                }
                                            >
                                                <span>{pres_name}</span>
                                                <span class="operator__presentation-meta">{lib_name}</span>
                                                {is_edit.then(|| {
                                                    let id_for_rename = id.clone();
                                                    let playlist_id = ctx.selected_playlist_id.get_untracked().unwrap_or_default();
                                                    // Capture op OUTSIDE the click handler closure
                                                    let op_for_rename = op.clone();
                                                    view! {
                                                        <div class="operator__presentation-actions">
                                                            <button
                                                                type="button"
                                                                class="operator__presentation-action"
                                                                data-action="presentation-rename"
                                                                data-presentation-id=id_for_rename.clone()
                                                                on:click=move |ev: leptos::ev::MouseEvent| {
                                                                    ev.stop_propagation();
                                                                    op_for_rename.modal_mode.set("edit".to_string());
                                                                    op_for_rename.modal_target_id.set(Some(id_for_rename.clone()));
                                                                    modal::open_modal(&op_for_rename, "presentation-edit");
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
                                                                    // Capture signals OUTSIDE async block
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
                            }).collect();
                            let entries_len = playlist.entries.len();
                            let playlist_id_spacer = ctx.selected_playlist_id.get_untracked().unwrap_or_default();
                            let selected_playlist = ctx.selected_playlist;
                            let playlists = ctx.playlists;
                            let toast_message = ctx.toast_message;
                            let toast_variant = ctx.toast_variant;
                            let head_view = render_list_spacer(
                                "head-spacer",
                                0,
                                op.clone(),
                                playlist_id_spacer.clone(),
                                selected_playlist,
                                playlists,
                                toast_message,
                                toast_variant,
                            );
                            let tail_view = render_list_spacer(
                                "tail-spacer",
                                entries_len,
                                op.clone(),
                                playlist_id_spacer,
                                selected_playlist,
                                playlists,
                                toast_message,
                                toast_variant,
                            );
                            return view! {
                                {head_view}
                                {entries_view}
                                {tail_view}
                            }.into_any();
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
                                data-active=move || if is_active { "true" } else { "false" }
                                draggable="true"
                                on:click={
                                    let id = id_for_click.clone();
                                    move |_| select_presentation(id.clone(), None)
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
                                {
                                    // Capture op OUTSIDE the click handler closure
                                    let op_for_rename = op.clone();
                                    is_edit.then(|| view! {
                                        <div class="operator__presentation-actions">
                                            <button
                                                type="button"
                                                class="operator__presentation-action"
                                                data-action="presentation-rename"
                                                data-presentation-id=id_for_rename.clone()
                                                on:click=move |ev: leptos::ev::MouseEvent| {
                                                    ev.stop_propagation();
                                                    op_for_rename.modal_mode.set("edit".to_string());
                                                    op_for_rename.modal_target_id.set(Some(id_for_rename.clone()));
                                                    modal::open_modal(&op_for_rename, "presentation-edit");
                                                }
                                            >
                                                "\u{270e}"
                                            </button>
                                        </div>
                                    })
                                }
                            </li>
                        }
                    }).collect_view().into_any()
                }}
            </ul>
        </div>
    }
}
