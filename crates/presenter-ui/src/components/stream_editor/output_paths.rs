//! Output SWITCHER + slug-aware output-scoped REST path builders (#785).
//!
//! Before #785 the editor was hard-wired to the single `stream` output
//! (`DEFAULT_OUTPUT_SLUG`); the timer overlay becoming its own stream output
//! (`timer`) means the editor must be able to DESIGN any output. So the
//! output-scoped path builders now take the CURRENTLY-SELECTED slug
//! (`StreamEditorCtx.output_slug`) instead of the constant, and a `<select>` in
//! the editor header lets the operator switch between outputs (list from
//! `GET /stream/api/outputs`). The selected slug is persisted in `localStorage`
//! and mirrored to the `?output=` URL param so a reload / bookmark reopens the
//! same output. The raw-`localStorage` helpers here are shared with the editor
//! tab memory (`editor_tabs.rs`, #829).
//!
//! These live in a sibling module (not `mod.rs`) purely to keep `mod.rs` under
//! the file-size gate. The id-scoped path builders (`/stream/api/scenes/{id}`,
//! `/stream/api/elements/{id}`, …) stay in `mod.rs`: they are not output-scoped.

use leptos::prelude::*;
use presenter_core::StreamOutputSummary;

use super::{StreamEditorCtx, DEFAULT_OUTPUT_SLUG};

// ---- Output-scoped REST paths (slug from the ctx) --------------------------

pub(super) fn def_path(slug: &str) -> String {
    format!("/stream/api/outputs/{slug}/def")
}

/// The output resource itself (PATCH target for rename / transitions, #752).
pub(super) fn output_path(slug: &str) -> String {
    format!("/stream/api/outputs/{slug}")
}

pub(super) fn scenes_path(slug: &str) -> String {
    format!("/stream/api/outputs/{slug}/scenes")
}

pub(super) fn scenes_order_path(slug: &str) -> String {
    format!("/stream/api/outputs/{slug}/scenes/order")
}

pub(super) fn active_scene_path(slug: &str) -> String {
    format!("/stream/api/outputs/{slug}/active-scene")
}

pub(super) fn overlay_path(slug: &str, id: i64) -> String {
    format!("/stream/api/outputs/{slug}/overlays/{id}")
}

pub(super) fn nameplates_path(slug: &str) -> String {
    format!("/stream/api/outputs/{slug}/nameplates")
}

pub(super) fn nameplates_order_path(slug: &str) -> String {
    format!("/stream/api/outputs/{slug}/nameplates/order")
}

pub(super) fn nameplates_active_path(slug: &str) -> String {
    format!("/stream/api/outputs/{slug}/nameplates/active")
}

// ---- Selected-output persistence (localStorage + ?output=) -----------------

/// `localStorage` key for the last-selected editor output slug.
const OUTPUT_STORAGE_KEY: &str = "stream-editor-output";

fn local_storage() -> Option<leptos::web_sys::Storage> {
    leptos::web_sys::window().and_then(|w| w.local_storage().ok().flatten())
}

/// A non-empty raw `localStorage` value of the editor (`None` when absent,
/// empty, or storage is unavailable). Shared with the tab memory (#829).
pub(super) fn read_stored(key: &str) -> Option<String> {
    local_storage()
        .and_then(|s| s.get_item(key).ok().flatten())
        .filter(|v| !v.is_empty())
}

/// Store a raw `localStorage` value (best-effort; no-op when storage is
/// unavailable, e.g. a sandboxed context).
pub(super) fn write_stored(key: &str, value: &str) {
    if let Some(storage) = local_storage() {
        let _ = storage.set_item(key, value);
    }
}

fn stored_output_slug() -> Option<String> {
    read_stored(OUTPUT_STORAGE_KEY)
}

/// Persist the selected output slug to `localStorage`.
pub(super) fn persist_output_slug(slug: &str) {
    write_stored(OUTPUT_STORAGE_KEY, slug);
}

/// Mirror the selected output into the `?output=` URL param so a reload / share
/// reopens the same output (no new history entry per switch). Only that one
/// param changes — `?tab=` (#829) and anything else in the query stay. The slug
/// is `^[a-z0-9-]+`, so it needs no URL-encoding.
pub(super) fn mirror_output_to_url(slug: &str) {
    crate::utils::window::replace_url_param("output", slug);
}

/// The output slug to open on load: `?output=` URL param → last `localStorage`
/// value → the built-in default (`stream`).
pub fn initial_output_slug() -> String {
    crate::utils::window::url_param("output")
        .filter(|s| !s.is_empty())
        .or_else(stored_output_slug)
        .unwrap_or_else(|| DEFAULT_OUTPUT_SLUG.to_string())
}

/// #827: the output to switch to once the outputs list is known — the FIRST
/// listed output when `current` is not in the list (deleted / mistyped /
/// remembered from another server), else `None` (keep editing `current`).
pub(super) fn fallback_output(current: &str, outputs: &[StreamOutputSummary]) -> Option<String> {
    if outputs.iter().any(|o| o.slug == current) {
        return None;
    }
    outputs.first().map(|o| o.slug.clone())
}

// ---- The output switcher ---------------------------------------------------

/// Header `<select>` listing every output (`GET /stream/api/outputs`); changing
/// it switches the whole editor to that output (`StreamEditorCtx::switch_output`).
///
/// #827: the selection lives on the OPTIONS (`prop:selected`), not on the
/// select's `value`. The list arrives after the def, and a `prop:value` applied
/// to a still-empty select matches nothing; when the options render later the
/// browser then picks the FIRST one while the editor edits another output.
/// Each option is created already in the right state and follows every later
/// slug change, whatever order the list and the def land in.
#[component]
pub fn OutputSelect(ctx: StreamEditorCtx) -> impl IntoView {
    let options = move || {
        ctx.outputs
            .get()
            .into_iter()
            .map(|o: StreamOutputSummary| {
                let slug = o.slug.clone();
                let selected = move || ctx.output_slug.with(|s| *s == slug);
                view! { <option value=o.slug prop:selected=selected>{o.name}</option> }
            })
            .collect_view()
    };
    view! {
        <label class="stream-editor__output-select" data-role="stream-output-select-label">
            <span>"Výstup"</span>
            <select
                data-role="stream-output-select"
                on:change=move |ev| on_output_change(ctx, &ev)
            >
                {options}
            </select>
        </label>
    }
}

/// Switch to the picked output. When the operator declines to discard unsaved
/// element edits (#787 reopen) the slug does not change, so no option's
/// `prop:selected` re-fires — put the `<select>` back on the current output by
/// hand.
fn on_output_change(ctx: StreamEditorCtx, ev: &web_sys::Event) {
    use wasm_bindgen::JsCast;
    let Some(select) = ev
        .target()
        .and_then(|t| t.dyn_into::<web_sys::HtmlSelectElement>().ok())
    else {
        return;
    };
    if !ctx.switch_output(select.value()) {
        select.set_value(&ctx.slug());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(slug: &str) -> StreamOutputSummary {
        StreamOutputSummary {
            id: 1,
            slug: slug.to_string(),
            name: slug.to_uppercase(),
            default_transition_ms: 300,
            base_transition_ms: None,
            overlay_transition_ms: None,
            active_scene_id: None,
            config_revision: 0,
        }
    }

    #[test]
    fn a_listed_output_stays_selected() {
        let list = [output("moderator"), output("stream"), output("timer")];
        assert_eq!(fallback_output("timer", &list), None);
        assert_eq!(fallback_output("moderator", &list), None);
    }

    #[test]
    fn a_missing_output_falls_back_to_the_first_listed() {
        let list = [output("moderator"), output("stream")];
        assert_eq!(
            fallback_output("gone", &list),
            Some("moderator".to_string())
        );
    }

    #[test]
    fn an_empty_list_has_nothing_to_fall_back_to() {
        assert_eq!(fallback_output("stream", &[]), None);
    }
}
