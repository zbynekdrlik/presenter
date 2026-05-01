//! Drag-drop helpers for the playlist entry list.
//!
//! Extracted from `presentation_list.rs` to keep both files under the
//! 1000-line hard cap. The PresentationList component imports these
//! via `use super::presentation_list_drag::{...};`.

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::state::operator::OperatorState;

// ----- thread-local drag state -----

// Signal for tracking dragged entry ID during playlist reordering
thread_local! {
    static DRAGGING_ENTRY_ID: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

pub(super) fn set_dragging_entry(id: Option<String>) {
    DRAGGING_ENTRY_ID.with(|cell| *cell.borrow_mut() = id);
}

pub(super) fn get_dragging_entry() -> Option<String> {
    DRAGGING_ENTRY_ID.with(|cell| cell.borrow().clone())
}

// ----- drop-position helpers -----

/// Compute "before"/"after" insertion side from the cursor's Y position
/// relative to the target entry's bounding-box midline. Returns `"before"`
/// if the cursor is in the top half, `"after"` if in the bottom half.
pub(super) fn drop_side_for_event(
    ev: &web_sys::DragEvent,
    target: &web_sys::Element,
) -> &'static str {
    let rect = target.get_bounding_client_rect();
    let midline = rect.top() + rect.height() / 2.0;
    if (ev.client_y() as f64) < midline {
        "before"
    } else {
        "after"
    }
}

/// Read the `data-drop-position` attribute set by the dragover handler,
/// then clear it. Returns the position string or "after" as a safe
/// default if the attribute is missing.
pub(super) fn take_drop_position(target: &web_sys::Element) -> String {
    let pos = target
        .get_attribute("data-drop-position")
        .unwrap_or_else(|| "after".to_string());
    let _ = target.remove_attribute("data-drop-position");
    pos
}

// ----- search-drop handlers -----

/// Shared async body: insert `presentation_id` at `insert_idx` in the
/// playlist, call `replace_entries`, and show a success or error toast.
/// Both `handle_search_drop` and `handle_search_drop_at_fixed` delegate
/// here after computing their respective `insert_idx`.
#[allow(clippy::too_many_arguments)]
fn run_search_insert(
    insert_idx: usize,
    presentation_id: String,
    playlist_id: String,
    selected_playlist: RwSignal<Option<presenter_core::Playlist>>,
    playlists: RwSignal<Vec<presenter_core::Playlist>>,
    toast_message: RwSignal<Option<String>>,
    toast_variant: RwSignal<String>,
) {
    leptos::task::spawn_local(async move {
        let current = selected_playlist.get_untracked();
        if let Some(pl) = current {
            let mut entries: Vec<_> = pl.entries.iter().map(entry_to_payload).collect();
            let insert_idx = insert_idx.min(entries.len());
            entries.insert(
                insert_idx,
                crate::api::playlists::PlaylistEntryPayload::Presentation {
                    entry_id: None,
                    presentation_id,
                },
            );
            match crate::api::playlists::replace_entries(&playlist_id, entries).await {
                Ok(updated) => {
                    selected_playlist.set(Some(updated));
                    toast_variant.set("success".to_string());
                    toast_message.set(Some("Added presentation to playlist".to_string()));
                    if let Ok(pls) = crate::api::playlists::list_playlists().await {
                        playlists.set(pls);
                    }
                }
                Err(e) => {
                    toast_variant.set("error".to_string());
                    toast_message.set(Some(format!("Error: {e}")));
                }
            }
        }
    });
}

/// Handle a search-drag → playlist-insert drop.
///
/// Reads `data-drop-position` from the target element, extracts the dragged
/// presentation id from the dataTransfer, and delegates to `run_search_insert`
/// with the computed insert index. The caller is responsible for resetting
/// `dragging_from_search` / `search_dragging` signals after this returns
/// (those resets stay outside so they're always visible alongside the early
/// `return`).
#[allow(clippy::too_many_arguments)]
pub(super) fn handle_search_drop(
    ev: &web_sys::DragEvent,
    target_index: usize,
    playlist_id: String,
    selected_playlist: RwSignal<Option<presenter_core::Playlist>>,
    playlists: RwSignal<Vec<presenter_core::Playlist>>,
    toast_message: RwSignal<Option<String>>,
    toast_variant: RwSignal<String>,
) {
    let drop_position = ev
        .current_target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
        .map(|target| take_drop_position(&target))
        .unwrap_or_else(|| "after".to_string());

    let presentation_id = ev
        .data_transfer()
        .and_then(|dt| dt.get_data("application/x-presentation-id").ok())
        .filter(|s| !s.is_empty());

    let Some(presentation_id) = presentation_id else {
        toast_variant.set("error".to_string());
        toast_message.set(Some("Drag payload missing presentation id".to_string()));
        return;
    };

    let insert_idx = if drop_position == "before" {
        target_index
    } else {
        target_index + 1
    };

    run_search_insert(
        insert_idx,
        presentation_id,
        playlist_id,
        selected_playlist,
        playlists,
        toast_message,
        toast_variant,
    );
}

/// Insert at a fixed position. Used by the head spacer (insert_idx=0),
/// the tail spacer (insert_idx=entries.len()), and the empty-state
/// placeholder (insert_idx=0). Reads the dragged presentation id from
/// the dataTransfer and delegates to `run_search_insert`. Shows
/// success/error toast.
#[allow(clippy::too_many_arguments)]
pub(super) fn handle_search_drop_at_fixed(
    ev: &web_sys::DragEvent,
    insert_idx: usize,
    playlist_id: String,
    selected_playlist: RwSignal<Option<presenter_core::Playlist>>,
    playlists: RwSignal<Vec<presenter_core::Playlist>>,
    toast_message: RwSignal<Option<String>>,
    toast_variant: RwSignal<String>,
) {
    // Clear any data-drop-position attribute we may have set during dragover.
    if let Some(target) = ev
        .current_target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
    {
        let _ = target.remove_attribute("data-drop-position");
    }

    let presentation_id = ev
        .data_transfer()
        .and_then(|dt| dt.get_data("application/x-presentation-id").ok())
        .filter(|s| !s.is_empty());

    let Some(presentation_id) = presentation_id else {
        toast_variant.set("error".to_string());
        toast_message.set(Some("Drag payload missing presentation id".to_string()));
        return;
    };

    run_search_insert(
        insert_idx,
        presentation_id,
        playlist_id,
        selected_playlist,
        playlists,
        toast_message,
        toast_variant,
    );
}

/// Render a transparent ~16px-tall <li> that captures search-drag dragover
/// in the dead zone above the first entry (head) or below the last entry
/// (tail). On drop, inserts at the fixed insert_idx using
/// handle_search_drop_at_fixed.
#[allow(clippy::too_many_arguments)]
pub(super) fn render_list_spacer(
    role: &'static str,
    insert_idx: usize,
    op: OperatorState,
    playlist_id: String,
    selected_playlist: RwSignal<Option<presenter_core::Playlist>>,
    playlists: RwSignal<Vec<presenter_core::Playlist>>,
    toast_message: RwSignal<Option<String>>,
    toast_variant: RwSignal<String>,
) -> impl IntoView {
    let op_for_dragover = op.clone();
    let op_for_dragleave = op.clone();
    let op_for_drop = op.clone();
    let pl_id_for_drop = playlist_id;
    // Head spacer wants the line at its bottom (visually = before entry 0)
    // → "after". Tail spacer wants the line at its top (visually = below
    // last entry) → "before".
    let drop_side = if role == "head-spacer" {
        "after"
    } else {
        "before"
    };
    view! {
        <li
            class="operator__list-spacer"
            data-role=role
            on:dragover=move |ev: web_sys::DragEvent| {
                if !op_for_dragover.dragging_from_search.get_untracked() {
                    return;
                }
                ev.prevent_default();
                if let Some(target) = ev
                    .current_target()
                    .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                {
                    let _ = target.set_attribute("data-drop-position", drop_side);
                }
            }
            on:dragleave=move |ev: web_sys::DragEvent| {
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
            on:drop=move |ev: web_sys::DragEvent| {
                ev.prevent_default();
                if op_for_drop.dragging_from_search.get_untracked() {
                    handle_search_drop_at_fixed(
                        &ev,
                        insert_idx,
                        pl_id_for_drop.clone(),
                        selected_playlist,
                        playlists,
                        toast_message,
                        toast_variant,
                    );
                    op_for_drop.dragging_from_search.set(false);
                    op_for_drop.search_dragging.set(false);
                }
            }
        >
        </li>
    }
}

// ----- entry-payload helpers -----

/// Convert a playlist entry to an API payload for replace_entries calls.
pub(super) fn entry_to_payload(
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

/// Get entry_id from a PlaylistEntryPayload
pub(super) fn get_entry_id(
    payload: &crate::api::playlists::PlaylistEntryPayload,
) -> Option<&String> {
    match payload {
        crate::api::playlists::PlaylistEntryPayload::Presentation { entry_id, .. } => {
            entry_id.as_ref()
        }
        crate::api::playlists::PlaylistEntryPayload::Separator { entry_id, .. } => {
            entry_id.as_ref()
        }
    }
}
