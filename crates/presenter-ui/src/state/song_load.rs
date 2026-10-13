//! Showing a song the operator opened (a song-list click, the session
//! restore) once its content arrives (#515, #556 F6, #853).

use std::collections::HashMap;

use leptos::prelude::*;
use presenter_core::{Presentation, Slide};

use super::operator::{await_saves_quiescent, SaveStatus};
use super::{is_still_selected, session};

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
}

/// A song that could not be shown must not stay selected: an arrow key would
/// send the shown song's slide, and Follow would think the stage song is open.
pub enum OnFailure {
    /// A click: select the shown song again, with the playlist occurrence it
    /// had (#496).
    SelectShown {
        entry_index: RwSignal<Option<u32>>,
        previous_entry_index: Option<u32>,
    },
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
    let mut seq_at_fetch = load.seq_at_select;
    for attempt in 0..2 {
        let Ok(detail) = crate::api::presentations::get_presentation(&load.id).await else {
            give_up(&load);
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
            load.shown.set(Some(detail.presentation));
            return;
        }
        // Still saving after the bounded wait: keep the always-safe drop.
        if attempt == 1 || !await_saves_quiescent(load.save_status, 3_000).await {
            give_up(&load);
            return;
        }
        seq_at_fetch = load.slide_edit_seq.get_untracked();
    }
}

/// The song could not be shown: if it is still selected, apply `on_failure`.
fn give_up(load: &SongLoad) {
    if !is_still_selected(load.selected_id, &load.id) {
        return;
    }
    match &load.on_failure {
        OnFailure::SelectShown {
            entry_index,
            previous_entry_index,
        } => {
            let shown = load
                .shown
                .with_untracked(|p| p.as_ref().map(|p| p.id.to_string()));
            match &shown {
                Some(id) => session::set("currentPresentationId", id),
                None => session::remove("currentPresentationId"),
            }
            load.selected_id.set(shown);
            entry_index.set(*previous_entry_index);
        }
        OnFailure::ClearSelection => load.selected_id.set(None),
    }
}
