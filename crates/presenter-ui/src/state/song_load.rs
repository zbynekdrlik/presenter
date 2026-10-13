//! Showing a song the operator opened (a song-list click, a search result,
//! the session restore) once its content arrives (#515, #556 F6, #853).

use std::cell::RefCell;
use std::collections::HashMap;

use leptos::prelude::*;
use presenter_core::{Presentation, Slide};

use super::operator::{await_saves_quiescent, SaveStatus};
use super::{is_still_selected, session, FollowPending};

thread_local! {
    /// Loads in flight per song id: a double-click loads a song twice, and
    /// one failing must not undo the other, whichever answers first.
    static IN_FLIGHT: RefCell<HashMap<String, u32>> = RefCell::new(HashMap::new());
}

/// One load of a song counted in `IN_FLIGHT` until it is dropped.
struct InFlight(String);

impl InFlight {
    fn start(id: &str) -> Self {
        IN_FLIGHT.with(|loads| *loads.borrow_mut().entry(id.to_string()).or_default() += 1);
        Self(id.to_string())
    }

    /// Other loads of the same song still on their way.
    fn others(&self) -> u32 {
        IN_FLIGHT.with(|loads| {
            loads
                .borrow()
                .get(&self.0)
                .copied()
                .unwrap_or(1)
                .saturating_sub(1)
        })
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        IN_FLIGHT.with(|loads| {
            let mut loads = loads.borrow_mut();
            if let Some(count) = loads.get_mut(&self.0) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    loads.remove(&self.0);
                }
            }
        });
    }
}

/// One song being opened. The caller sets `selected_id` first.
pub struct SongLoad {
    pub id: String,
    pub selected_id: RwSignal<Option<String>>,
    pub shown: RwSignal<Option<Presentation>>,
    /// Filled with the song's slides when given (the click path).
    pub slides_cache: Option<RwSignal<HashMap<String, Vec<Slide>>>>,
    pub slide_edit_seq: RwSignal<u64>,
    pub save_status: RwSignal<HashMap<String, (SaveStatus, u64)>>,
    /// `slide_edit_seq` captured SYNCHRONOUSLY by the caller, before its
    /// `spawn_local` (#515): a `spawn_local` body runs as a microtask after
    /// the rest of the event handler, so a blur-save fired in the same script
    /// would bump the counter first and make a stale pre-save answer look
    /// fresh, blanking the edit.
    pub seq_at_select: u64,
    /// What to do when the song cannot be shown (failed fetch, saves that
    /// never settle, a second edit race) while it is still selected.
    pub on_failure: OnFailure,
    /// `AppContext::follow_pending`: a Follow open that started while this
    /// song was selected follows the selection when it moves on failure (the
    /// move is not an operator pick).
    pub follow_pending: StoredValue<Option<FollowPending>>,
    /// `AppContext::selected_entry_index` / `shown_entry_index`: when the
    /// song is shown, the occurrence selected at that moment (the click's, or
    /// a stage mirror that came during the load) is recorded as the shown
    /// one (#496).
    pub selected_entry_index: RwSignal<Option<u32>>,
    pub shown_entry_index: StoredValue<Option<u32>>,
}

/// A song that could not be shown must not stay selected: an arrow key would
/// send the shown song's slide, and Follow would think the stage song is open.
pub enum OnFailure {
    /// A click: select the shown song again, with the playlist occurrence it
    /// is shown as (`shown_entry_index`, #496) — not the one of a click that
    /// is still loading.
    SelectShown,
    /// The session restore: nothing is shown yet, so clear the selection but
    /// keep the session key for the next load.
    ClearSelection,
}

/// Fetch the song and show it — only while it is still the selected song (a
/// later click or a Follow open wins, #853) and only if no slide edit landed
/// since the fetch began (#515: the answer would carry pre-edit content).
///
/// #556 F6: reorder/insert/duplicate/delete also bump `slide_edit_seq`, so an
/// edit landing during the fetch is likely; a bare drop would leave the song
/// never loading. Retry ONCE with a fresh seq, but only after in-flight saves
/// have committed (`await_saves_quiescent`): a retry issued at once can be
/// served from pre-save state and blank the just-typed edit.
pub async fn show_when_loaded(load: SongLoad) {
    let flight = InFlight::start(&load.id);
    let mut seq_at_fetch = load.seq_at_select;
    for attempt in 0..2 {
        let Ok(detail) = crate::api::presentations::get_presentation(&load.id).await else {
            give_up(&load, &flight);
            return;
        };
        if !is_still_selected(load.selected_id, &load.id) {
            return;
        }
        if load.slide_edit_seq.get_untracked() == seq_at_fetch {
            if let Some(cache) = load.slides_cache {
                cache.update(|cache| {
                    cache.insert(load.id.clone(), detail.presentation.slides.clone());
                });
            }
            load.shown_entry_index
                .set_value(load.selected_entry_index.get_untracked());
            load.shown.set(Some(detail.presentation));
            return;
        }
        // Still saving after the bounded wait: keep the always-safe drop.
        if attempt == 1 || !await_saves_quiescent(load.save_status, 3_000).await {
            give_up(&load, &flight);
            return;
        }
        seq_at_fetch = load.slide_edit_seq.get_untracked();
    }
}

/// The song could not be shown: if it is still selected, not shown already
/// (a re-click of the shown song: nothing to undo, its clicked occurrence
/// stays) and no other load of it is on its way, apply `on_failure`.
fn give_up(load: &SongLoad, flight: &InFlight) {
    let already_shown = load
        .shown
        .with_untracked(|p| p.as_ref().is_some_and(|p| p.id.to_string() == load.id));
    if already_shown || flight.others() > 0 || !is_still_selected(load.selected_id, &load.id) {
        return;
    }
    let selection = match &load.on_failure {
        OnFailure::SelectShown => load
            .shown
            .with_untracked(|p| p.as_ref().map(|p| p.id.to_string())),
        OnFailure::ClearSelection => None,
    };
    // A Follow open that started while this song was selected follows the
    // selection (the move is no operator pick). If the selection becomes the
    // open's own song, its answer only moves the selection and occurrence —
    // `stage_follow` never replaces the content of a song already shown.
    load.follow_pending.update_value(|pending| {
        if let Some(open) = pending {
            if open.selected_at_start.as_deref() == Some(load.id.as_str()) {
                open.selected_at_start = selection.clone();
            }
        }
    });
    if let OnFailure::SelectShown = load.on_failure {
        match &selection {
            Some(id) => session::set("currentPresentationId", id),
            None => session::remove("currentPresentationId"),
        }
        load.selected_entry_index
            .set(load.shown_entry_index.get_value());
    }
    load.selected_id.set(selection);
}
