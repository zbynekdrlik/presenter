//! Per-slide stage-layout marker control for the operator slide grid (#515).
//!
//! Edit mode: a compact selector in the slide-card header assigning/clearing
//! the marker. Live mode: a small badge on slides that carry one, so the
//! operator sees which slide will flip the stage layout when triggered.

use std::collections::HashMap;

use leptos::prelude::*;

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
        view! {
            <select
                class="operator__slide-stage-layout-select"
                data-role="slide-stage-layout-select"
                title="Stage layout for this slide (switches when triggered)"
                prop:value=move || current_code.get().unwrap_or_default()
                on:change=move |ev| {
                    let value = event_target_value(&ev);
                    let code = if value.is_empty() { None } else { Some(value) };
                    let pres_id = pres_id_change.clone();
                    let slide_id = slide_id_change.clone();
                    let code_for_map = code.clone();
                    leptos::task::spawn_local(async move {
                        if api::presentations::set_slide_stage_layout(
                            &pres_id,
                            &slide_id,
                            code.clone(),
                        )
                        .await
                        .is_ok()
                        {
                            markers.update(|map| match code_for_map {
                                Some(code) => {
                                    map.insert(slide_id.clone(), code);
                                }
                                None => {
                                    map.remove(&slide_id);
                                }
                            });
                        }
                    });
                }
            >
                <option value="">"Stage: —"</option>
                {move || {
                    layouts
                        .get()
                        .into_iter()
                        .map(|layout| {
                            view! {
                                <option value=layout.code.clone()>
                                    {format!("Stage: {}", layout.name)}
                                </option>
                            }
                        })
                        .collect_view()
                }}
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
