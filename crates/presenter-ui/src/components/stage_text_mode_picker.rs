use leptos::prelude::*;
use presenter_core::{is_api_stage_layout, StageTextMode};

use crate::state::AppContext;

/// Operator-facing label of a text mode.
pub fn text_mode_label(mode: StageTextMode) -> &'static str {
    match mode {
        StageTextMode::Original => "Original",
        StageTextMode::Translation => "Translation",
        StageTextMode::Both => "Both",
    }
}

/// Operator control for the API layouts' text mode (#799), rendered next to
/// the "Stage Output" layout picker. Visible only while an API layout (`api`
/// / `api-ambient`) is selected — the mode affects nothing else. The change
/// applies optimistically here; the server persists it and pushes it live to
/// every display (and every other operator via `LiveEvent::StageTextMode`).
#[component]
pub fn StageTextModePicker() -> impl IntoView {
    let ctx = use_ctx!(AppContext);
    let text_mode = ctx.stage_text_mode;
    let layout_code = ctx.stage_layout_code;

    let on_change = move |ev| {
        let value: String = event_target_value(&ev);
        let Ok(mode) = value.parse::<StageTextMode>() else {
            return;
        };
        let previous = text_mode.get_untracked();
        text_mode.set(mode);
        leptos::task::spawn_local(async move {
            match crate::api::stage::set_text_mode(mode).await {
                Ok(resp) => text_mode.set(resp.mode),
                Err(_) => {
                    // Revert: the server never took the change, so the picker
                    // must not claim a mode the displays are not showing.
                    leptos::logging::warn!("failed to set stage text mode {mode}");
                    text_mode.set(previous);
                }
            }
        });
    };

    view! {
        <Show when=move || is_api_stage_layout(&layout_code.get())>
            <div class="operator__stage-layout" aria-label="Stage text mode">
                <label class="operator__stage-layout-label" for="stage-text-mode-select">
                    "Stage Text"
                </label>
                <select
                    id="stage-text-mode-select"
                    data-role="stage-text-mode-select"
                    on:change=on_change
                >
                    {StageTextMode::ALL.into_iter().map(|mode| {
                        let selected = move || text_mode.get() == mode;
                        view! {
                            <option value=mode.as_str() prop:selected=selected>
                                {text_mode_label(mode)}
                            </option>
                        }
                    }).collect_view()}
                </select>
            </div>
        </Show>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mode_has_a_distinct_label() {
        let labels: std::collections::HashSet<_> = StageTextMode::ALL
            .into_iter()
            .map(text_mode_label)
            .collect();
        assert_eq!(labels.len(), StageTextMode::ALL.len());
    }
}
