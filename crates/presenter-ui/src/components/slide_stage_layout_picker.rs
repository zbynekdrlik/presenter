//! Per-slide stage-layout marker control for the operator slide grid (#515).
//!
//! Edit mode: a compact selector in the slide-card header assigning/clearing
//! the marker. Live mode: a small badge on slides that carry one, so the
//! operator sees which slide will flip the stage layout when triggered.

use std::collections::HashMap;

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::api;
use crate::state::AppContext;

/// Badge / option label for a layout code: the layout's display name when the
/// picker list knows it, otherwise the uppercased code (stale marker whose
/// layout disappeared — still shown honestly rather than hidden).
pub fn marker_label(code: &str, layouts: &[presenter_core::StageDisplayLayout]) -> String {
    layouts
        .iter()
        .find(|layout| layout.code == code)
        .map(|layout| layout.name.clone())
        .unwrap_or_else(|| code.to_uppercase())
}

/// Fetch + maintain the stage-layout marker map (`slide_id → layout_code`)
/// of the currently selected presentation.
///
/// The map is cleared IMMEDIATELY on every presentation switch so the
/// previous presentation's badges never show on the new one, and a resolved
/// fetch is applied only when its presentation is STILL the selected one —
/// otherwise a slow response for presentation A could clobber the map after
/// the operator already switched to presentation B (out-of-order responses).
/// A failed fetch leaves the map empty — the honest pessimistic state.
pub fn use_slide_stage_markers(ctx: &AppContext) -> RwSignal<HashMap<String, String>> {
    let markers = RwSignal::new(HashMap::<String, String>::new());
    let selected_presentation_id = ctx.selected_presentation_id;
    Effect::new(move |_| {
        let Some(pres_id) = selected_presentation_id.get() else {
            markers.set(HashMap::new());
            return;
        };
        markers.set(HashMap::new());
        leptos::task::spawn_local(async move {
            let result = api::presentations::fetch_slide_stage_layouts(&pres_id).await;
            // Stale-response guard: only the still-open presentation may
            // populate the map.
            if selected_presentation_id.get_untracked().as_deref() != Some(pres_id.as_str()) {
                return;
            }
            if let Ok(map) = result {
                markers.set(map);
            }
        });
    });
    markers
}

#[component]
pub fn SlideStageLayoutControl(
    pres_id: String,
    slide_id: String,
    is_edit: bool,
    markers: RwSignal<HashMap<String, String>>,
) -> impl IntoView {
    let ctx = use_ctx!(AppContext);
    let layouts = ctx.stage_layouts;

    let sid_for_memo = slide_id.clone();
    let current_code = Memo::new(move |_| markers.get().get(&sid_for_memo).cloned());

    if is_edit {
        let pres_id_change = pres_id.clone();
        let slide_id_change = slide_id.clone();
        // Options carry per-<option> prop:selected (the header layout picker's
        // pattern) instead of prop:value on the <select>: a select's value can
        // only stick when a matching <option> already exists, and this
        // component's options (from /stage-displays) and marker value (per-
        // presentation fetch) arrive from independent async fetches in either
        // order. prop:selected re-renders WITH the options themselves, so the
        // marked layout is selected no matter which response lands first.
        let options = move || {
            let selected = current_code.get().unwrap_or_default();
            let layout_options = layouts
                .get()
                .into_iter()
                .map(|layout| {
                    let code = layout.code.clone();
                    let is_selected = code == selected;
                    view! {
                        <option value=code prop:selected=is_selected>
                            {format!("Stage: {}", layout.name)}
                        </option>
                    }
                })
                .collect_view();
            view! {
                <option value="" prop:selected=selected.is_empty()>"Stage: —"</option>
                {layout_options}
            }
        };
        let on_change = move |ev: leptos::ev::Event| {
            let value = event_target_value(&ev);
            let code = if value.is_empty() { None } else { Some(value) };
            let pres_id = pres_id_change.clone();
            let slide_id = slide_id_change.clone();
            let select = ev
                .target()
                .and_then(|t| t.dyn_into::<web_sys::HtmlSelectElement>().ok());
            let code_for_map = code.clone();
            leptos::task::spawn_local(async move {
                match api::presentations::set_slide_stage_layout(&pres_id, &slide_id, code).await {
                    Ok(()) => {
                        markers.update(|map| match code_for_map {
                            Some(code) => {
                                map.insert(slide_id.clone(), code);
                            }
                            None => {
                                map.remove(&slide_id);
                            }
                        });
                    }
                    Err(err) => {
                        // The browser already applied the pick to the DOM
                        // select — snap it back to the server truth so a
                        // failed save can't silently look saved (the marker
                        // map did not change, so no re-render fixes it).
                        let server_code = markers
                            .get_untracked()
                            .get(&slide_id)
                            .cloned()
                            .unwrap_or_default();
                        if let Some(select) = select {
                            select.set_value(&server_code);
                        }
                        leptos::logging::warn!(
                            "stage-layout marker save failed for slide {slide_id}: {err:?}"
                        );
                    }
                }
            });
        };
        view! {
            <select
                class="operator__slide-stage-layout-select"
                data-role="slide-stage-layout-select"
                title="Stage layout for this slide (switches when triggered)"
                on:change=on_change
            >
                {options}
            </select>
        }
        .into_any()
    } else {
        view! {
            <Show when=move || current_code.get().is_some()>
                <span
                    class="operator__slide-stage-layout-badge"
                    data-role="slide-stage-layout-badge"
                    title="Triggering this slide switches the stage layout"
                >
                    {move || {
                        current_code
                            .get()
                            .map(|code| format!("⤢ {}", marker_label(&code, &layouts.get())))
                            .unwrap_or_default()
                    }}
                </span>
            </Show>
        }
        .into_any()
    }
}

#[cfg(test)]
mod tests {
    use super::marker_label;
    use presenter_core::StageDisplayLayout;

    #[test]
    fn marker_label_uses_layout_name_when_known() {
        let layouts = StageDisplayLayout::built_in();
        assert_eq!(marker_label("fulltext", &layouts), "FULL TEXT");
        assert_eq!(marker_label("timer", &layouts), "TIMER");
    }

    #[test]
    fn marker_label_falls_back_to_uppercased_code() {
        let layouts = StageDisplayLayout::built_in();
        assert_eq!(marker_label("removed-layout", &layouts), "REMOVED-LAYOUT");
    }
}
