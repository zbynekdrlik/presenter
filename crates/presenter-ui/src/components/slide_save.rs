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
use presenter_core::{Presentation, Slide, SlideGroup, SlideText};
use wasm_bindgen::JsCast;

use crate::api;
use crate::state::operator::{OperatorState, SaveStatus};

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
#[allow(clippy::too_many_arguments)]
pub(super) fn save_with_status(
    pres_id: String,
    slide_id: String,
    main: String,
    translation: String,
    stage: String,
    group: Option<String>,
    save_status: SaveStatusMap,
    selected_pres: RwSignal<Option<Presentation>>,
    groups_version: RwSignal<u64>,
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
                // #556 F2: bump the SEPARATE, cheap `groups_version` counter
                // right after the group-cascade-affecting content lands in
                // local state. `apply_saved_content_locally` above uses
                // `update_untracked` deliberately (to avoid a full, unkeyed
                // slide-list rebuild on every save — see its own doc
                // comment), so nothing else would tell the header
                // badge/placeholder fine-grained closures to recompute.
                // This counter is what does — see `groups_version` on
                // `OperatorState`.
                groups_version.update(|v| *v += 1);
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

/// Get a textarea/input value from the DOM by slide_id and field name.
fn get_field_value(doc: &web_sys::Document, slide_id: &str, field: &str) -> String {
    let selector = format!(
        "[data-slide-id=\"{}\"] [data-field=\"{}\"]",
        slide_id, field
    );
    if let Ok(Some(el)) = doc.query_selector(&selector) {
        if let Ok(ta) = el.clone().dyn_into::<web_sys::HtmlTextAreaElement>() {
            return ta.value();
        }
        if let Ok(inp) = el.dyn_into::<web_sys::HtmlInputElement>() {
            return inp.value();
        }
    }
    String::new()
}

/// Unified save function that saves ALL fields from DOM atomically.
/// This prevents data loss when editing multiple fields before blur.
/// Takes `RwSignal` (which is `Copy`) instead of `&AppContext` to allow
/// use in multiple `move` closures without moving the entire context.
pub(super) fn save_all_fields_from_dom(
    pres_id: &str,
    slide_id: &str,
    _current_field: &str,
    _sel_start: u32,
    _sel_end: u32,
    selected_pres: RwSignal<Option<Presentation>>,
    op: &OperatorState,
) {
    let doc = crate::utils::window::document();

    let main = get_field_value(&doc, slide_id, "main");
    let translation = get_field_value(&doc, slide_id, "translation");
    let stage = get_field_value(&doc, slide_id, "stage");
    let group_val = get_field_value(&doc, slide_id, "group");
    let group = if group_val.trim().is_empty() {
        None
    } else {
        Some(group_val.trim().to_string())
    };

    // Skip no-op saves.
    let pres = selected_pres.get_untracked();
    if let Some(p) = &pres {
        if let Some(slide) = p.slides.iter().find(|s| s.id.to_string() == slide_id) {
            let orig = &slide.content;
            let orig_group = orig.group.as_ref().map(|g| g.name().to_string());
            if orig.main.value() == main
                && orig.translation.value() == translation
                && orig.stage.value() == stage
                && orig_group == group
            {
                return;
            }
        }
    }

    // #515: bump the edit generation SYNCHRONOUSLY, the moment a genuine
    // (non-no-op) edit is committed to saving — NOT when the save's network
    // round-trip later completes. A `get_presentation` refetch (opening /
    // re-opening the presentation) captures this counter before it fires;
    // bumping here, before the async save even starts, guarantees that ANY
    // such refetch still in flight at this point is provably stale no
    // matter which of the two async calls happens to resolve first.
    op.slide_edit_seq.update(|seq| *seq += 1);

    save_with_status(
        pres_id.to_string(),
        slide_id.to_string(),
        main,
        translation,
        stage,
        group,
        op.save_status,
        selected_pres,
        op.groups_version,
    );
}

/// Capture current selection range from a textarea event.
pub(super) fn capture_selection(ev: &web_sys::Event) -> (u32, u32) {
    if let Some(target) = ev.target() {
        if let Ok(ta) = target.clone().dyn_into::<web_sys::HtmlTextAreaElement>() {
            let start = ta.selection_start().ok().flatten().unwrap_or(0);
            let end = ta.selection_end().ok().flatten().unwrap_or(0);
            return (start, end);
        }
        if let Ok(inp) = target.dyn_into::<web_sys::HtmlInputElement>() {
            let start = inp.selection_start().ok().flatten().unwrap_or(0);
            let end = inp.selection_end().ok().flatten().unwrap_or(0);
            return (start, end);
        }
    }
    (0, 0)
}

/// The slide id + field name currently holding focus, per an actual DOM
/// check (`document.activeElement`) — not `OperatorState`'s
/// `focused_slide_id`/`focused_field`, which only record the LAST field
/// that was focused and are never cleared on blur. Used by the reorder
/// drop handler (#556 F4) to know whether there is genuinely in-flight
/// typing to protect before a tracked `selected_pres` update tears down
/// and rebuilds the (unkeyed) slide list.
pub(super) fn focused_slide_field_in_dom() -> Option<(String, String)> {
    let doc = crate::utils::window::document();
    let active = doc.active_element()?;
    let field = active.get_attribute("data-field")?;
    let card = active.closest("[data-slide-id]").ok().flatten()?;
    let slide_id = card.get_attribute("data-slide-id")?;
    Some((slide_id, field))
}

/// Re-focus the given slide's field after a tracked slide-list rebuild
/// (#556 F4). Best-effort: if the slide (or the field) no longer exists
/// post-rebuild — e.g. it was concurrently deleted — this is a silent
/// no-op, there is nothing left to focus.
pub(super) fn restore_pending_focus(slide_id: &str, field: &str) {
    let doc = crate::utils::window::document();
    let selector = format!("[data-slide-id=\"{slide_id}\"] [data-field=\"{field}\"]");
    if let Ok(Some(el)) = doc.query_selector(&selector) {
        if let Ok(html_el) = el.dyn_into::<web_sys::HtmlElement>() {
            let _ = html_el.focus();
        }
    }
}

/// Reconcile an editing response (reorder / insert / duplicate / delete)
/// against a possible `slide_edit_seq` mismatch (#556 F1/F3): some OTHER
/// edit landed while this request was in flight. Never apply a response
/// outright once it's known-stale, but never silently keep showing
/// "Saved ✓" over a client that displays structure/content the server
/// doesn't have either.
///
/// - If the response's slide-id SET still matches what's currently
///   displayed, the concurrent edit only changed CONTENT (e.g. a
///   content-field save bumped the seq) — apply the server's ORDER while
///   keeping the (newer) local CONTENT for each slide.
/// - If the id sets differ, a structural change (insert/duplicate/delete)
///   raced this call and the merge above can't be done meaningfully — issue
///   ONE guarded refetch (after in-flight saves have committed, so its
///   server-side read can't predate them — the #515 race) and apply it
///   only if nothing else has landed by the time it resolves.
pub(super) async fn reconcile_after_seq_mismatch(
    pres_id: String,
    server_slides: Vec<Slide>,
    selected_pres: RwSignal<Option<Presentation>>,
    slide_edit_seq: RwSignal<u64>,
    save_status: SaveStatusMap,
) {
    let local_ids: Option<Vec<String>> = selected_pres
        .get_untracked()
        .map(|p| p.slides.iter().map(|s| s.id.to_string()).collect());
    let server_ids: Vec<String> = server_slides.iter().map(|s| s.id.to_string()).collect();

    let same_id_set = local_ids
        .map(|mut local| {
            let mut server_sorted = server_ids.clone();
            local.sort();
            server_sorted.sort();
            local == server_sorted
        })
        .unwrap_or(false);

    if same_id_set {
        selected_pres.update(|p| {
            if let Some(pres) = p.as_mut() {
                let mut by_id: HashMap<String, Slide> = pres
                    .slides
                    .drain(..)
                    .map(|s| (s.id.to_string(), s))
                    .collect();
                pres.slides = server_ids
                    .iter()
                    .filter_map(|id| by_id.remove(id))
                    .collect();
            }
        });
        return;
    }

    // A structural change raced this request: one guarded refetch, applied
    // only if nothing newer has landed by the time it resolves. Wait for
    // in-flight saves (e.g. the F4 pre-reorder flush) to commit first —
    // a refetch issued immediately can be served from pre-save state and
    // its tracked apply would blank the just-saved edit (the #515 race).
    if !crate::state::operator::await_saves_quiescent(save_status, 3_000).await {
        // Still saving after the bounded wait — keep the always-safe
        // drop-the-response behavior.
        return;
    }
    let seq_at_refetch = slide_edit_seq.get_untracked();
    if let Ok(detail) = api::presentations::get_presentation(&pres_id).await {
        if slide_edit_seq.get_untracked() == seq_at_refetch {
            selected_pres.set(Some(detail.presentation));
        }
    }
}
