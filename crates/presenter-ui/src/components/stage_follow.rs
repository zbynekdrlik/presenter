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
//! makes the open stale (a newer open, the stage leaving that song, the
//! operator picking a song, a failure of its latest request, Follow off or
//! Edit mode by then) just drops it — there is nothing to undo. In Live mode the next stage
//! change opens the stage song again; after an Edit-mode drop the switch back
//! to Live does. songplayer pushes to an API layout (#799: a stage frame with
//! no song) are not stage changes for Follow.

use std::sync::atomic::{AtomicU64, Ordering};

use leptos::prelude::*;
use presenter_core::{is_api_stage_layout, AbleSetStatusSnapshot, StageDisplaySnapshot};

use crate::state::{is_still_selected, AppContext, FollowPending};

static NEXT_OPEN_TOKEN: AtomicU64 = AtomicU64::new(1);

/// An open still waiting this long for its song is requested again by the
/// next stage change: a hung request never settles (fetch has no timeout).
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

/// A songplayer push to an API layout (#799): a stage frame with no song,
/// interleaved with the frames that carry the real stage song.
fn is_songless_api_frame(snapshot: &StageDisplaySnapshot) -> bool {
    songless_api_frame(snapshot.presentation_id.is_some(), &snapshot.layout.code)
}

fn songless_api_frame(has_song: bool, layout_code: &str) -> bool {
    !has_song && is_api_stage_layout(layout_code)
}

/// Whether the stage has left the song a pending open is loading.
fn stage_left_open(open: Option<&FollowPending>, stage_song: Option<&str>) -> bool {
    open.is_some_and(|open| stage_song != Some(open.presentation_id.as_str()))
}

/// Whether an answer for request `request` of open `token` ends that open: a
/// success of any of its requests opens the song; a failure ends it only when
/// it is the latest request (an older one may fail while a newer one is still
/// on its way).
fn answer_ends_open(open: &FollowPending, token: u64, request: u32, succeeded: bool) -> bool {
    open.token == token && (succeeded || open.request == request)
}

/// Remember `snapshot` as the stage Follow acts on (unless it is a songless
/// API push). Also used for the snapshot loaded with the page.
pub(crate) fn remember_stage(ctx: &AppContext, snapshot: &StageDisplaySnapshot) {
    if !is_songless_api_frame(snapshot) {
        ctx.follow_stage.set_value(Some(snapshot.clone()));
    }
}

/// Follow's stage until the first live frame, at page load: the camera-crew
/// snapshot always carries the stage song (an API layout's own snapshot may
/// be a songless songplayer push); a live frame that came first wins.
pub(crate) fn seed_follow_stage(ctx: &AppContext) {
    let ctx = ctx.clone();
    leptos::task::spawn_local(async move {
        let layout = crate::pages::stage_events::CAMERA_CREW_LAYOUT;
        if let Ok(snapshot) = crate::api::stage::get_snapshot_for(layout).await {
            if ctx.follow_stage.with_value(Option::is_none) {
                remember_stage(&ctx, &snapshot);
            }
        }
    });
}

/// What a stage snapshot on `stage_song` asks of Follow.
#[derive(Clone, Copy, Debug, PartialEq)]
enum SyncStep {
    /// The stage is on the selected song.
    OnSelectedSong,
    /// An open of this song, started from this selection, is still loading.
    AlreadyOpening,
    /// That open has waited past `STALE_OPEN_MS`: request the song again
    /// under the same open, so the first successful answer opens it (an older
    /// request failing meanwhile does not end it).
    Retry,
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
    match pending {
        Some(open)
            if open.presentation_id == stage_song
                && open.selected_at_start.as_deref() == selected =>
        {
            if now_ms - open.started_ms < STALE_OPEN_MS {
                SyncStep::AlreadyOpening
            } else {
                SyncStep::Retry
            }
        }
        _ => SyncStep::Open,
    }
}

/// A stage snapshot reached the operator page. A pending open whose song the
/// stage has left is stale in any mode — in Edit mode no later stage event
/// replaces a Follow ON open, so without this it could still replace the
/// song being edited. In Live mode with Follow on, the stage song is opened.
pub(crate) fn on_stage_snapshot(ctx: &AppContext, snapshot: &StageDisplaySnapshot) {
    if is_songless_api_frame(snapshot) {
        return;
    }
    remember_stage(ctx, snapshot);
    let stage_song = snapshot.presentation_id.map(|id| id.to_string());
    ctx.follow_pending.update_value(|open| {
        if stage_left_open(open.as_ref(), stage_song.as_deref()) {
            *open = None;
        }
    });
    if follow_navigation(ctx.ableset_status, ctx.mode) {
        sync_to_stage(ctx, snapshot, false);
    }
}

/// Follow the stage: open its song when it differs from the selected one, or
/// mirror its playlist occurrence (#496) when it is the selected song, so a
/// later keyboard advance targets the repeated-song row the stage is on.
/// Stage events check `follow_navigation` first; `forced` (the Follow ON
/// click) opens the song in Edit mode too.
pub(crate) fn sync_to_stage(ctx: &AppContext, snapshot: &StageDisplaySnapshot, forced: bool) {
    if is_songless_api_frame(snapshot) {
        return;
    }
    let entry_index = snapshot.active_entry_index;
    // No song on stage: nothing to follow, and the occurrence the operator
    // picked stays (#496).
    let Some(pres_id) = snapshot.presentation_id.map(|id| id.to_string()) else {
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
            ctx.shown_entry_index.set_value(entry_index);
        }
        SyncStep::AlreadyOpening | SyncStep::Retry => {
            let mut retry = None;
            ctx.follow_pending.update_value(|open| {
                if let Some(open) = open {
                    open.forced |= forced;
                    open.entry_index = entry_index;
                    if step == SyncStep::Retry {
                        open.started_ms = now_ms;
                        open.request += 1;
                        retry = Some((open.token, open.request));
                    }
                }
            });
            if let Some((token, request)) = retry {
                fetch_stage_song(ctx, token, request, pres_id);
            }
        }
        SyncStep::Open => {
            let token = NEXT_OPEN_TOKEN.fetch_add(1, Ordering::Relaxed);
            ctx.follow_pending.set_value(Some(FollowPending {
                token,
                presentation_id: pres_id.clone(),
                selected_at_start: selected,
                forced,
                entry_index,
                started_ms: now_ms,
                request: 0,
                context_at_start: (
                    ctx.selected_library_id.get_untracked(),
                    ctx.selected_playlist_id.get_untracked(),
                ),
            }));
            fetch_stage_song(ctx, token, 0, pres_id);
        }
    }
}

/// Fetch the stage song (request `request` of the open `token`) and show it if
/// that open is still current; then its library context.
fn fetch_stage_song(ctx: &AppContext, token: u64, request: u32, pres_id: String) {
    let ctx = ctx.clone();
    leptos::task::spawn_local(async move {
        let fetched = crate::api::presentations::get_presentation(&pres_id).await;
        let Some(open) = take_current_open(&ctx, token, request, fetched.is_ok()) else {
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
        ctx.shown_entry_index.set_value(open.entry_index);
        ctx.slides_cache.update(|cache| {
            cache.insert(pres_id.clone(), detail.presentation.slides.clone());
        });
        ctx.follow_landing.set_value(Some(detail.presentation.id));
        ctx.selected_presentation.set(Some(detail.presentation));
        show_song_library(&ctx, &open, lib_id).await;
    });
}

/// The library containing the song Follow just showed: its context and song
/// list together, unless the operator has since picked another song, library
/// or playlist (since the open started — a pick while the song loaded counts).
/// If the library cannot be read, the old context stays (accepted: the next
/// Follow open or a click fixes it).
async fn show_song_library(ctx: &AppContext, open: &FollowPending, lib_id: String) {
    let Ok(library) = crate::api::libraries::get_library(&lib_id).await else {
        return;
    };
    let context_now = (
        ctx.selected_library_id.get_untracked(),
        ctx.selected_playlist_id.get_untracked(),
    );
    if !is_still_selected(ctx.selected_presentation_id, &open.presentation_id)
        || context_now != open.context_at_start
    {
        return;
    }
    ctx.selected_library_id.set(Some(lib_id.clone()));
    ctx.selected_playlist_id.set(None);
    ctx.selected_playlist.set(None);
    crate::state::session::set("activeLibraryId", &lib_id);
    crate::state::session::remove("activePlaylistId");
    ctx.context_title.set(library.name);
    ctx.presentations.set(library.presentations);
    // A Follow open already loading compares against this context now, so
    // Follow's own switch is never taken for an operator pick.
    ctx.follow_pending.update_value(|pending| {
        if let Some(pending) = pending {
            pending.context_at_start = (Some(lib_id.clone()), None);
        }
    });
}

/// The open `token` if this answer ends it (`answer_ends_open`) and the
/// operator has not picked another song since it started. An ended open is no
/// longer pending either way.
fn take_current_open(
    ctx: &AppContext,
    token: u64,
    request: u32,
    succeeded: bool,
) -> Option<FollowPending> {
    let open = ctx
        .follow_pending
        .with_value(|open| open.clone())
        .filter(|open| answer_ends_open(open, token, request, succeeded))?;
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
            if let Some(snapshot) = ctx.follow_stage.get_value() {
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
            request: 0,
            context_at_start: (None, None),
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
    fn a_stale_open_is_requested_again_under_the_same_open() {
        let open = pending("S", Some("A"), 1_000.0);
        assert_eq!(
            sync_step("S", Some("A"), Some(&open), 6_000.0),
            SyncStep::Retry
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
    fn a_songless_frame_is_a_songplayer_push_only_on_an_api_layout() {
        assert!(songless_api_frame(false, "api"));
        assert!(songless_api_frame(false, "api-ambient"));
        assert!(!songless_api_frame(true, "api"));
        assert!(!songless_api_frame(false, "worship"));
    }

    #[test]
    fn a_pending_open_is_left_when_the_stage_is_on_another_song_or_none() {
        let open = pending("S", Some("A"), 0.0);
        assert!(!stage_left_open(Some(&open), Some("S")));
        assert!(stage_left_open(Some(&open), Some("T")));
        assert!(stage_left_open(Some(&open), None));
        assert!(!stage_left_open(None, Some("T")));
    }

    #[test]
    fn only_a_success_or_the_latest_request_failing_ends_an_open() {
        let mut open = pending("S", Some("A"), 0.0);
        open.request = 1;
        assert!(answer_ends_open(&open, 1, 0, true));
        assert!(answer_ends_open(&open, 1, 1, true));
        assert!(answer_ends_open(&open, 1, 1, false));
        assert!(!answer_ends_open(&open, 1, 0, false));
        assert!(!answer_ends_open(&open, 2, 1, true));
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
