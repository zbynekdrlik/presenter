//! Per-slide save-status state machine for the worship slide editor (#313).
//!
//! The operator's worship slide editor saves on blur and shows a transient
//! "Saved ✓ / Saving… / Save failed" badge per slide. This module owns the
//! state machine for that badge:
//!
//! - `start_save_status` marks the slide as `Saving` and returns a token
//! - `finish_save_status_ok` marks `Saved` and schedules the 2s fade
//! - `finish_save_status_err` marks `Failed` (sticky)
//! - `save_with_status` is the fire-and-forget wrapper used by textarea blur
//!
//! The monotonic `SAVE_TOKEN` guards against a stale fade timer clearing a
//! newer save's entry — the fade only removes the entry when the token still
//! matches the one captured at save start.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use presenter_core::{Presentation, SlideGroup, SlideText};

use crate::api;
use crate::state::operator::SaveStatus;

/// Monotonic token used to invalidate stale fade timers.
static SAVE_TOKEN: AtomicU64 = AtomicU64::new(0);

/// Status-map signal alias used by the save-status helpers.
pub(super) type SaveStatusMap = RwSignal<HashMap<String, (SaveStatus, u64)>>;

/// Mark a slide as `Saving` and return its monotonic token. Callers pass the
/// token to `finish_save_status_ok` / `finish_save_status_err` so a stale
/// completion can't overwrite a newer save's entry.
pub(super) fn start_save_status(slide_id: &str, save_status: SaveStatusMap) -> u64 {
    let token = SAVE_TOKEN.fetch_add(1, Ordering::Relaxed) + 1;
    let key = slide_id.to_string();
    save_status.update(|map| {
        map.insert(key, (SaveStatus::Saving, token));
    });
    token
}

/// Mark a slide save as `Saved` and schedule the 2s fade-removal. The fade
/// only clears the entry when the token still matches.
pub(super) async fn finish_save_status_ok(
    slide_id: String,
    save_status: SaveStatusMap,
    token: u64,
) {
    let key_for_saved = slide_id.clone();
    save_status.update(|map| {
        map.insert(key_for_saved, (SaveStatus::Saved, token));
    });
    TimeoutFuture::new(2_000).await;
    save_status.update(|map| {
        if map.get(&slide_id).map(|(_, t)| *t) == Some(token) {
            map.remove(&slide_id);
        }
    });
}

/// Mark a slide save as `Failed` (sticky until the next save attempt).
pub(super) fn finish_save_status_err(slide_id: String, save_status: SaveStatusMap, token: u64) {
    save_status.update(|map| {
        map.insert(slide_id, (SaveStatus::Failed, token));
    });
}

/// Fire-and-forget save with indicator wiring. Used by the textarea blur
/// handlers where no refetch is needed.
///
/// On success this ALSO patches `selected_pres`'s in-memory slide content to
/// match what was just saved (#515), so the edit is visible everywhere
/// `selected_pres` is read (e.g. the operator's own Live-mode preview), not
/// only in the textarea's own DOM value. The `slide_edit_seq` bump that
/// guards against a stale `get_presentation` refetch clobbering a newer
/// edit happens SYNCHRONOUSLY at save-START, in the caller
/// (`save_all_fields_from_dom`) — not here — so the guard holds no matter
/// which of the two async calls (this save, or the refetch) resolves first.
pub(super) fn save_with_status(
    pres_id: String,
    slide_id: String,
    main: String,
    translation: String,
    stage: String,
    group: Option<String>,
    save_status: SaveStatusMap,
    selected_pres: RwSignal<Option<Presentation>>,
) {
    let token = start_save_status(&slide_id, save_status);
    leptos::task::spawn_local(async move {
        let result = api::presentations::update_slide_with_group(
            &pres_id,
            &slide_id,
            &main,
            &translation,
            &stage,
            group.clone(),
        )
        .await;

        match result {
            Ok(_) => {
                apply_saved_content_locally(
                    selected_pres,
                    &slide_id,
                    &main,
                    &translation,
                    &stage,
                    group.as_deref(),
                );
                finish_save_status_ok(slide_id, save_status, token).await
            }
            Err(_) => finish_save_status_err(slide_id, save_status, token),
        }
    });
}

/// Patch the in-memory presentation's slide content to match a just-saved
/// edit (#515). Best-effort: if the values can no longer construct a valid
/// `SlideText` (e.g. over the hard character cap), the local cache is left
/// untouched — the save already round-tripped through server-side
/// validation, so this only guards the local reconstruction.
///
/// Uses `update_untracked` DELIBERATELY (not `update`): a save fires on
/// EVERY field's blur, so a tracked `.update()` here would force a full
/// slide-list re-render — recreating every slide's local reactive signals
/// (`main_warn_sig` & co. in `slide_list.rs`) — every time ANY field on ANY
/// slide is saved. That re-render can stomp an in-progress edit to a
/// DIFFERENT field of the same slide that hasn't been blurred/saved yet
/// (its freshly-typed warning state gets reset from the stale pre-edit
/// snapshot). `selected_pres` still ends up with the correct data for the
/// next NATURAL re-render (mode toggle, re-opening the presentation, …) —
/// it just doesn't force one of its own.
fn apply_saved_content_locally(
    selected_pres: RwSignal<Option<Presentation>>,
    slide_id: &str,
    main: &str,
    translation: &str,
    stage: &str,
    group: Option<&str>,
) {
    let (Ok(main_text), Ok(translation_text), Ok(stage_text)) = (
        SlideText::new(main),
        SlideText::new(translation),
        SlideText::new(stage),
    ) else {
        return;
    };
    let group = group
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(SlideGroup::new);

    selected_pres.update_untracked(|maybe_pres| {
        let Some(pres) = maybe_pres.as_mut() else {
            return;
        };
        let Some(slide) = pres
            .slides
            .iter_mut()
            .find(|slide| slide.id.to_string() == slide_id)
        else {
            return;
        };
        slide.content.main = main_text;
        slide.content.translation = translation_text;
        slide.content.stage = stage_text;
        slide.content.group = group;
    });
}
