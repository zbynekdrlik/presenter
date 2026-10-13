//! AbleSet Follow: opening the song the stage is on, in the operator.
//!
//! #853: stage events navigate only in Live mode. In Edit mode a stage song
//! change would replace the open song, scroll its slide list to the top and
//! destroy the textarea being typed into, so they leave the selection alone; on
//! the switch back to Live the operator catches up with the stage at once, so an
//! arrow key never sends a slide of the song that was being edited. An explicit
//! Follow ON click (`stage_preview.rs`) syncs in any mode (`forced`).

use leptos::prelude::*;
use presenter_core::{AbleSetStatusSnapshot, StageDisplaySnapshot};

use crate::state::AppContext;

/// Whether a stage change may navigate the operator to the stage's song:
/// AbleSet Follow is on and the operator is not editing. Until the Follow
/// status has loaded it counts as off, like the Follow button and the server
/// default.
pub(crate) fn follow_navigation(
    ableset_status: RwSignal<Option<AbleSetStatusSnapshot>>,
    mode: RwSignal<String>,
) -> bool {
    let follow_enabled = ableset_status
        .get_untracked()
        .map(|s| s.follow_enabled)
        .unwrap_or(false);
    follow_enabled && mode.get_untracked() != "edit"
}

/// Follow the stage: mirror its playlist occurrence (#496) and open its song
/// when it differs from the selected one. Stage events check
/// `follow_navigation` first; `forced` (the Follow ON click) opens the song in
/// any mode.
pub(crate) fn sync_to_stage(ctx: &AppContext, snapshot: &StageDisplaySnapshot, forced: bool) {
    // #496: mirror the active playlist occurrence so a later keyboard advance
    // targets the same repeated-song row the stage is on (e.g. after an
    // AbleSet-driven change).
    ctx.selected_entry_index.set(snapshot.active_entry_index);
    let Some(pres_id) = snapshot.presentation_id.map(|id| id.to_string()) else {
        return;
    };
    if ctx.selected_presentation_id.get_untracked().as_deref() != Some(pres_id.as_str()) {
        open_stage_song(ctx, pres_id, forced);
    }
}

fn is_selected(ctx: &AppContext, pres_id: &str) -> bool {
    ctx.selected_presentation_id.get_untracked().as_deref() == Some(pres_id)
}

/// Open `pres_id`: the song first, then its library and song list. Each async
/// step applies only while `pres_id` is still the selected song, so a song the
/// operator picks meanwhile wins. If Follow may no longer navigate when the
/// song arrives (Edit mode, Follow off), the selection goes back to the song
/// actually shown, so the two never disagree and the next catch-up reopens the
/// stage song.
fn open_stage_song(ctx: &AppContext, pres_id: String, forced: bool) {
    ctx.selected_presentation_id.set(Some(pres_id.clone()));
    crate::state::session::set("currentPresentationId", &pres_id);
    let ctx = ctx.clone();
    leptos::task::spawn_local(async move {
        let Ok(detail) = crate::api::presentations::get_presentation(&pres_id).await else {
            return;
        };
        if !is_selected(&ctx, &pres_id) {
            return;
        }
        if !forced && !follow_navigation(ctx.ableset_status, ctx.mode) {
            restore_shown_selection(&ctx);
            return;
        }
        let lib_id = detail.library_id.to_string();
        ctx.slides_cache.update(|cache| {
            cache.insert(pres_id.clone(), detail.presentation.slides.clone());
        });
        ctx.selected_presentation.set(Some(detail.presentation));
        // Switch to the library containing this presentation.
        ctx.selected_library_id.set(Some(lib_id.clone()));
        ctx.selected_playlist_id.set(None);
        ctx.selected_playlist.set(None);
        crate::state::session::set("activeLibraryId", &lib_id);
        if let Ok(pres_list) = crate::api::libraries::list_presentations(&lib_id).await {
            if is_selected(&ctx, &pres_id) {
                ctx.presentations.set(pres_list);
            }
        }
    });
}

/// Point the selection back at the song the slide list shows.
fn restore_shown_selection(ctx: &AppContext) {
    let shown = ctx
        .selected_presentation
        .with_untracked(|p| p.as_ref().map(|p| p.id.to_string()));
    match &shown {
        Some(id) => crate::state::session::set("currentPresentationId", id),
        None => crate::state::session::remove("currentPresentationId"),
    }
    ctx.selected_presentation_id.set(shown);
}

/// On the switch from Edit back to Live, catch up with the stage once (no
/// further stage event needed).
pub(crate) fn setup_follow_on_live(ctx: &AppContext) {
    let ctx = ctx.clone();
    let mode = ctx.mode;
    Effect::new(move |prev: Option<String>| {
        let current = mode.get();
        let back_to_live = prev.as_deref() == Some("edit") && current == "live";
        if back_to_live && follow_navigation(ctx.ableset_status, ctx.mode) {
            if let Some(snapshot) = ctx.stage_snapshot.get_untracked() {
                sync_to_stage(&ctx, &snapshot, false);
            }
        }
        current
    });
}
