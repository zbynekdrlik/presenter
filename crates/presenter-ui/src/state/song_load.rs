//! Showing a song the operator opened (a song-list click, a search result,
//! the session restore) once its content arrives (#515, #556 F6, #853).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use presenter_core::{Presentation, Slide};

use super::operator::{await_saves_quiescent, SaveStatus};
use super::{is_still_selected, session, FollowPending};

/// A load still unanswered after this long (a hung request — fetch has no
/// timeout) no longer holds back another load of the same song from giving up.
const STALE_LOAD_MS: f64 = 5_000.0;

thread_local! {
    /// Loads in flight per song id (`(load number, start time)`): a
    /// double-click loads a song twice, and one failing must not undo the
    /// other, whichever answers first.
    static IN_FLIGHT: RefCell<HashMap<String, Vec<(u64, f64)>>> = RefCell::new(HashMap::new());
    static NEXT_LOAD: Cell<u64> = const { Cell::new(0) };
}

/// One load of a song counted in `IN_FLIGHT` until it is dropped.
struct InFlight {
    id: String,
    number: u64,
}

impl InFlight {
    fn start(id: &str) -> Self {
        let number = NEXT_LOAD.with(|next| {
            next.set(next.get() + 1);
            next.get()
        });
        let started = js_sys::Date::now();
        IN_FLIGHT.with(|loads| {
            loads
                .borrow_mut()
                .entry(id.to_string())
                .or_default()
                .push((number, started));
        });
        Self {
            id: id.to_string(),
            number,
        }
    }

    /// How long until no OTHER load of the same song counts any more
    /// (`None` when none counts now): each counts until it answers or for
    /// `STALE_LOAD_MS` (a hung request).
    fn others_count_for_ms(&self) -> Option<f64> {
        let now = js_sys::Date::now();
        IN_FLIGHT.with(|loads| {
            loads.borrow().get(&self.id).and_then(|list| {
                list.iter()
                    .filter(|(number, _)| *number != self.number)
                    .map(|(_, started)| started + STALE_LOAD_MS - now)
                    .filter(|left| *left > 0.0)
                    .reduce(f64::max)
            })
        })
    }
}

/// Loads of `id` on their way that still count (younger than `STALE_LOAD_MS`).
fn counting_loads(id: &str) -> usize {
    let now = js_sys::Date::now();
    IN_FLIGHT.with(|loads| {
        loads.borrow().get(id).map_or(0, |list| {
            list.iter()
                .filter(|(_, started)| now - started < STALE_LOAD_MS)
                .count()
        })
    })
}

impl Drop for InFlight {
    fn drop(&mut self) {
        IN_FLIGHT.with(|loads| {
            let mut loads = loads.borrow_mut();
            if let Some(list) = loads.get_mut(&self.id) {
                list.retain(|(number, _)| *number != self.number);
                if list.is_empty() {
                    loads.remove(&self.id);
                }
            }
        });
    }
}

/// One song being opened. The caller sets `selected_id` first.
#[derive(Clone)]
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
/// Either way the shown song (if any) becomes the selection again, with the
/// playlist occurrence it is shown as (`shown_entry_index`, #496).
#[derive(Clone, Copy)]
pub enum OnFailure {
    /// A click or a search result: with nothing shown, the selection and the
    /// session key clear.
    SelectShown,
    /// The session restore: with nothing shown yet, the selection clears but
    /// the session key stays for the next load.
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

/// The song could not be shown. While another load of it still counts, that
/// load decides (it shows the song or gives up itself); if it hangs, the
/// give-up is checked again once it stops counting (`STALE_LOAD_MS`). Then
/// `give_up_now`.
fn give_up(load: &SongLoad, flight: &InFlight) {
    if let Some(wait_ms) = flight.others_count_for_ms() {
        // Another load of this song is on its way: it shows the song or gives
        // up itself — and if it hangs, check again once it no longer counts.
        let load = load.clone();
        leptos::task::spawn_local(async move {
            TimeoutFuture::new(wait_ms.ceil() as u32 + 50).await;
            if counting_loads(&load.id) == 0 {
                give_up_now(&load);
            }
        });
        return;
    }
    give_up_now(load);
}

/// If the song is still selected, the shown song becomes the selection again
/// (`OnFailure`). A load of the song that is already shown (a re-click of its
/// other occurrence) gives up nothing: the clicked occurrence stays and is
/// recorded as the shown one.
fn give_up_now(load: &SongLoad) {
    if !is_still_selected(load.selected_id, &load.id) {
        return;
    }
    let shown = load
        .shown
        .with_untracked(|p| p.as_ref().map(|p| p.id.to_string()));
    if shown.as_deref() == Some(load.id.as_str()) {
        load.shown_entry_index
            .set_value(load.selected_entry_index.get_untracked());
        return;
    }
    // A Follow open that started while this song was selected follows the
    // selection (the move is no operator pick). If the selection becomes the
    // open's own song, its answer moves the selection, the occurrence and the
    // library context but never the content (`stage_follow`, #515).
    load.follow_pending.update_value(|pending| {
        if let Some(open) = pending {
            if open.selected_at_start.as_deref() == Some(load.id.as_str()) {
                open.selected_at_start = shown.clone();
            }
        }
    });
    match (&shown, &load.on_failure) {
        (None, OnFailure::ClearSelection) => {}
        (Some(id), _) => {
            session::set("currentPresentationId", id);
            load.selected_entry_index
                .set(load.shown_entry_index.get_value());
        }
        (None, OnFailure::SelectShown) => {
            session::remove("currentPresentationId");
            load.selected_entry_index
                .set(load.shown_entry_index.get_value());
        }
    }
    load.selected_id.set(shown);
}
