//! AbleSet Follow: opening the song the stage is on, in the operator.
//!
//! #853: stage events navigate only in Live mode. In Edit mode a stage song
//! change would replace the open song, scroll its slide list to the top and
//! destroy the textarea being typed into, so they leave the selection alone; on
//! the switch back to Live the operator catches up with the stage at once, so an
//! arrow key never sends a slide of the song that was being edited. An explicit
//! Follow ON click (`stage_preview.rs`) syncs in any mode (`forced`).
//!
//! An open is recorded in `AppContext::follow_pending` while its song loads;
//! the selection moves only when the song is shown, so the selected song and
//! the shown song never disagree. Anything that makes the open stale (a newer
//! open, the stage returning to the selected song, the operator picking a
//! song, a failed fetch, Edit mode or Follow off by then) just drops it — there
//! is nothing to undo, and the next stage change opens the stage song again.

use std::sync::atomic::{AtomicU64, Ordering};

use leptos::prelude::*;
use presenter_core::{AbleSetStatusSnapshot, StageDisplaySnapshot};

use crate::state::{AppContext, FollowPending};

static NEXT_OPEN_TOKEN: AtomicU64 = AtomicU64::new(1);

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

/// Follow the stage: open its song when it differs from the selected one, or
/// mirror its playlist occurrence (#496) when it is the selected song, so a
/// later keyboard advance targets the repeated-song row the stage is on.
/// Stage events check `follow_navigation` first; `forced` (the Follow ON
/// click) opens the song in any mode.
pub(crate) fn sync_to_stage(ctx: &AppContext, snapshot: &StageDisplaySnapshot, forced: bool) {
    let entry_index = snapshot.active_entry_index;
    let Some(pres_id) = snapshot.presentation_id.map(|id| id.to_string()) else {
        ctx.selected_entry_index.set(entry_index);
        return;
    };
    let selected = ctx.selected_presentation_id.get_untracked();
    if selected.as_deref() == Some(pres_id.as_str()) {
        // Back on the selected song: an open of another song still loading
        // is stale (stage A → S → A).
        ctx.follow_pending.set_value(None);
        ctx.selected_entry_index.set(entry_index);
        return;
    }
    let already_opening = ctx.follow_pending.with_value(|open| {
        open.as_ref().is_some_and(|open| {
            open.presentation_id == pres_id && open.selected_at_start == selected
        })
    });
    if already_opening {
        ctx.follow_pending.update_value(|open| {
            if let Some(open) = open {
                open.forced |= forced;
                open.entry_index = entry_index;
            }
        });
        return;
    }
    open_stage_song(
        ctx,
        FollowPending {
            token: NEXT_OPEN_TOKEN.fetch_add(1, Ordering::Relaxed),
            presentation_id: pres_id,
            selected_at_start: selected,
            forced,
            entry_index,
        },
    );
}

fn is_selected(ctx: &AppContext, pres_id: &str) -> bool {
    ctx.selected_presentation_id.get_untracked().as_deref() == Some(pres_id)
}

/// Fetch the stage song and show it if the open is still current; then its
/// library and song list, while that song is still selected.
fn open_stage_song(ctx: &AppContext, open: FollowPending) {
    let token = open.token;
    let pres_id = open.presentation_id.clone();
    ctx.follow_pending.set_value(Some(open));
    let ctx = ctx.clone();
    leptos::task::spawn_local(async move {
        let fetched = crate::api::presentations::get_presentation(&pres_id).await;
        let Some(open) = take_current_open(&ctx, token) else {
            return;
        };
        let Ok(detail) = fetched else {
            return;
        };
        if !open.forced && !follow_navigation(ctx.ableset_status, ctx.mode) {
            return;
        }
        let lib_id = detail.library_id.to_string();
        ctx.selected_presentation_id.set(Some(pres_id.clone()));
        crate::state::session::set("currentPresentationId", &pres_id);
        ctx.selected_entry_index.set(open.entry_index);
        ctx.slides_cache.update(|cache| {
            cache.insert(pres_id.clone(), detail.presentation.slides.clone());
        });
        ctx.selected_presentation.set(Some(detail.presentation));
        // The library containing the song: its context and list together.
        if let Ok(library) = crate::api::libraries::get_library(&lib_id).await {
            if is_selected(&ctx, &pres_id) {
                ctx.selected_library_id.set(Some(lib_id.clone()));
                ctx.selected_playlist_id.set(None);
                ctx.selected_playlist.set(None);
                crate::state::session::set("activeLibraryId", &lib_id);
                ctx.context_title.set(library.name);
                ctx.presentations.set(library.presentations);
            }
        }
    });
}

/// The open `token` if it is still the one in flight and the operator has not
/// picked another song since it started. Either way it is no longer pending.
fn take_current_open(ctx: &AppContext, token: u64) -> Option<FollowPending> {
    let open = ctx
        .follow_pending
        .with_value(|open| open.clone())
        .filter(|open| open.token == token)?;
    ctx.follow_pending.set_value(None);
    (ctx.selected_presentation_id.get_untracked() == open.selected_at_start).then_some(open)
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
