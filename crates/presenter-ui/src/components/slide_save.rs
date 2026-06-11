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
pub(super) fn save_with_status(
    pres_id: String,
    slide_id: String,
    main: String,
    translation: String,
    stage: String,
    group: Option<String>,
    save_status: SaveStatusMap,
) {
    let token = start_save_status(&slide_id, save_status);
    leptos::task::spawn_local(async move {
        let result = api::presentations::update_slide_with_group(
            &pres_id,
            &slide_id,
            &main,
            &translation,
            &stage,
            group,
        )
        .await;

        match result {
            Ok(_) => finish_save_status_ok(slide_id, save_status, token).await,
            Err(_) => finish_save_status_err(slide_id, save_status, token),
        }
    });
}
