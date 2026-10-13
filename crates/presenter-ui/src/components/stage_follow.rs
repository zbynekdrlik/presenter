//! AbleSet Follow: opening the song the stage is on, in the operator.
//!
//! #853: stage events navigate only in Live mode. In Edit mode a stage song
//! change would replace the open song, scroll its slide list to the top and
//! destroy the textarea being typed into, so they leave the selection alone; on
//! the switch back to Live the operator catches up with the stage at once, so an
//! arrow key never sends a slide of the song that was being edited. An explicit
//! Follow ON click (`stage_preview.rs`) syncs in Edit mode too (`forced`).
//!
//! An open is recorded in `AppContext::follow_pending` while its song loads;
//! Follow never moves the selection ahead of the song's content. Anything that
//! makes the open stale (a newer open, the stage returning to the selected song
//! or clearing, the operator picking a song, a failed fetch, Follow off or Edit
//! mode by then) just drops it — there is nothing to undo. A stage change opens
//! the stage song again; after an Edit-mode drop the switch back to Live does.

use std::sync::atomic::{AtomicU64, Ordering};

use leptos::prelude::*;
use presenter_core::{AbleSetStatusSnapshot, StageDisplaySnapshot};

use crate::state::{is_still_selected, AppContext, FollowPending};

static NEXT_OPEN_TOKEN: AtomicU64 = AtomicU64::new(1);

/// An open still waiting this long for its song is retried by the next stage
/// change: a hung request never settles (fetch has no timeout).
const STALE_OPEN_MS: f64 = 5_000.0;

/// Whether a stage change may navigate the operator to the stage's song:
/// AbleSet Follow is on and the operator is not editing. Until the Follow
/// status has loaded it counts as off, like the Follow button and the server
/// default.
pub(crate) fn follow_navigation(
    ableset_status: RwSignal<Option<AbleSetStatusSnapshot>>,
    mode: RwSignal<String>,
) -> bool {
    may_navigate(follow_enabled(ableset_status), &mode.get_untracked(), false)
}

fn follow_enabled(ableset_status: RwSignal<Option<AbleSetStatusSnapshot>>) -> bool {
    ableset_status
        .get_untracked()
        .map(|s| s.follow_enabled)
        .unwrap_or(false)
}

/// Follow may navigate when it is on and the operator is not editing; the
/// Follow ON click (`forced`) may navigate in Edit mode too.
fn may_navigate(follow_enabled: bool, mode: &str, forced: bool) -> bool {
    follow_enabled && (forced || mode != "edit")
}

/// What a stage snapshot on `stage_song` asks of Follow.
#[derive(Debug, PartialEq)]
enum SyncStep {
    /// The stage is on the selected song.
    OnSelectedSong,
    /// An open of this song, started from this selection, is still loading.
    AlreadyOpening,
    /// Open the stage song.
    Open,
}

fn sync_step(
    stage_song: &str,
    selected: Option<&str>,
    pending: Option<&FollowPending>,
    now_ms: f64,
) -> SyncStep {
    if selected == Some(stage_song) {
        return SyncStep::OnSelectedSong;
    }
    let opening = pending.is_some_and(|open| {
        open.presentation_id == stage_song
            && open.selected_at_start.as_deref() == selected
            && now_ms - open.started_ms < STALE_OPEN_MS
    });
    if opening {
        SyncStep::AlreadyOpening
    } else {
        SyncStep::Open
    }
}

/// Follow the stage: open its song when it differs from the selected one, or
/// mirror its playlist occurrence (#496) when it is the selected song, so a
/// later keyboard advance targets the repeated-song row the stage is on.
/// Stage events check `follow_navigation` first; `forced` (the Follow ON
/// click) opens the song in Edit mode too.
pub(crate) fn sync_to_stage(ctx: &AppContext, snapshot: &StageDisplaySnapshot, forced: bool) {
    let entry_index = snapshot.active_entry_index;
    let Some(pres_id) = snapshot.presentation_id.map(|id| id.to_string()) else {
        // Nothing on stage: an open still loading is stale.
        ctx.follow_pending.set_value(None);
        ctx.selected_entry_index.set(entry_index);
        return;
    };
    let selected = ctx.selected_presentation_id.get_untracked();
    let now_ms = js_sys::Date::now();
    let step = ctx
        .follow_pending
        .with_value(|open| sync_step(&pres_id, selected.as_deref(), open.as_ref(), now_ms));
    match step {
        SyncStep::OnSelectedSong => {
            // An open of another song still loading is stale (stage A → S → A).
            ctx.follow_pending.set_value(None);
            ctx.selected_entry_index.set(entry_index);
        }
        SyncStep::AlreadyOpening => ctx.follow_pending.update_value(|open| {
            if let Some(open) = open {
                open.forced |= forced;
                open.entry_index = entry_index;
            }
        }),
        SyncStep::Open => open_stage_song(
            ctx,
            FollowPending {
                token: NEXT_OPEN_TOKEN.fetch_add(1, Ordering::Relaxed),
                presentation_id: pres_id,
                selected_at_start: selected,
                forced,
                entry_index,
                started_ms: now_ms,
            },
        ),
    }
}

/// Fetch the stage song and show it if the open is still current; then its
/// library context.
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
        if !may_navigate(
            follow_enabled(ctx.ableset_status),
            &ctx.mode.get_untracked(),
            open.forced,
        ) {
            return;
        }
        let lib_id = detail.library_id.to_string();
        ctx.selected_presentation_id.set(Some(pres_id.clone()));
        crate::state::session::set("currentPresentationId", &pres_id);
        ctx.selected_entry_index.set(open.entry_index);
        ctx.slides_cache.update(|cache| {
            cache.insert(pres_id.clone(), detail.presentation.slides.clone());
        });
        ctx.follow_landing.set_value(Some(detail.presentation.id));
        ctx.selected_presentation.set(Some(detail.presentation));
        show_song_library(&ctx, &pres_id, lib_id).await;
    });
}

/// The library containing the song Follow just showed: its context and song
/// list together, unless the operator has since picked another song, library
/// or playlist.
async fn show_song_library(ctx: &AppContext, pres_id: &str, lib_id: String) {
    let context = (
        ctx.selected_library_id.get_untracked(),
        ctx.selected_playlist_id.get_untracked(),
    );
    let Ok(library) = crate::api::libraries::get_library(&lib_id).await else {
        return;
    };
    let context_now = (
        ctx.selected_library_id.get_untracked(),
        ctx.selected_playlist_id.get_untracked(),
    );
    if !is_still_selected(ctx.selected_presentation_id, pres_id) || context_now != context {
        return;
    }
    ctx.selected_library_id.set(Some(lib_id.clone()));
    ctx.selected_playlist_id.set(None);
    ctx.selected_playlist.set(None);
    crate::state::session::set("activeLibraryId", &lib_id);
    crate::state::session::remove("activePlaylistId");
    ctx.context_title.set(library.name);
    ctx.presentations.set(library.presentations);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(song: &str, selected_at_start: Option<&str>, started_ms: f64) -> FollowPending {
        FollowPending {
            token: 1,
            presentation_id: song.to_string(),
            selected_at_start: selected_at_start.map(str::to_string),
            forced: false,
            entry_index: None,
            started_ms,
        }
    }

    #[test]
    fn stage_on_the_selected_song_is_not_an_open() {
        let other = pending("S", Some("A"), 0.0);
        assert_eq!(
            sync_step("A", Some("A"), Some(&other), 10.0),
            SyncStep::OnSelectedSong
        );
        assert_eq!(
            sync_step("A", Some("A"), None, 10.0),
            SyncStep::OnSelectedSong
        );
    }

    #[test]
    fn a_fresh_open_of_the_same_song_from_the_same_selection_is_reused() {
        let open = pending("S", Some("A"), 1_000.0);
        assert_eq!(
            sync_step("S", Some("A"), Some(&open), 1_100.0),
            SyncStep::AlreadyOpening
        );
        assert_eq!(
            sync_step("S", Some("A"), Some(&open), 5_999.0),
            SyncStep::AlreadyOpening
        );
    }

    #[test]
    fn a_stale_open_is_opened_again() {
        let open = pending("S", Some("A"), 1_000.0);
        assert_eq!(
            sync_step("S", Some("A"), Some(&open), 6_000.0),
            SyncStep::Open
        );
    }

    #[test]
    fn another_song_or_a_changed_selection_starts_a_new_open() {
        let open = pending("S", Some("A"), 0.0);
        assert_eq!(sync_step("T", Some("A"), Some(&open), 10.0), SyncStep::Open);
        assert_eq!(sync_step("S", Some("X"), Some(&open), 10.0), SyncStep::Open);
        assert_eq!(sync_step("S", None, Some(&open), 10.0), SyncStep::Open);
        assert_eq!(sync_step("S", Some("A"), None, 10.0), SyncStep::Open);
    }

    #[test]
    fn follow_navigates_only_when_on_and_edit_only_when_forced() {
        assert!(may_navigate(true, "live", false));
        assert!(!may_navigate(true, "edit", false));
        assert!(may_navigate(true, "edit", true));
        assert!(!may_navigate(false, "live", false));
        assert!(!may_navigate(false, "live", true));
        assert!(!may_navigate(false, "edit", true));
    }
}
