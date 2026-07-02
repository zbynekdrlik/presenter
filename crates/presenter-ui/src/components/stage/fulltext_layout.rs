//! Full-screen stage-text layout (#515).
//!
//! Renders ONLY the current slide's stage text, auto-scaled up/down to fill
//! the entire stage screen — for handing the speaker a long text to read or a
//! short message. Follows the timer layout's minimal pattern: the text area
//! plus the standard status bar every layout keeps, nothing else.

use leptos::prelude::*;

use crate::state::stage::StageContext;
use crate::utils::autofit::autofit_effect;
use crate::ws::stage::StageWsState;

const FULLTEXT_MAX_FONT: f64 = 800.0;

/// Text the fulltext layout shows for a snapshot's current slide: the stage
/// field when present, falling back to the main lyrics text — the same
/// precedence the worship layouts use for their current-slide line.
pub fn fulltext_display_text(stage: &str, main: &str) -> String {
    if stage.is_empty() {
        main.to_string()
    } else {
        stage.to_string()
    }
}

#[component]
pub fn FulltextLayout(
    ws_state: ReadSignal<StageWsState>,
    latency_ms: ReadSignal<Option<f64>>,
) -> impl IntoView {
    let ctx = use_context::<StageContext>().expect("StageContext not provided");

    let text_ref = NodeRef::<leptos::html::Div>::new();

    let current_text = move || {
        ctx.snapshot
            .get()
            .and_then(|s| {
                s.current
                    .map(|slide| fulltext_display_text(&slide.stage, &slide.main))
            })
            .unwrap_or_default()
    };

    autofit_effect(text_ref, FULLTEXT_MAX_FONT, current_text);

    view! {
        <div class="stage-container" data-layout="fulltext">
            <div class="stage-fulltext__display">
                <span class="stage__debug-label">"fulltext-display"</span>
                <div node_ref=text_ref class="stage-fulltext__text" data-role="fulltext-text">
                    {current_text}
                </div>
            </div>
            <super::status_bar::StatusBar ws_state=ws_state latency_ms=latency_ms />
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::fulltext_display_text;

    #[test]
    fn prefers_stage_field_when_present() {
        assert_eq!(fulltext_display_text("read this", "lyrics"), "read this");
    }

    #[test]
    fn falls_back_to_main_when_stage_empty() {
        assert_eq!(fulltext_display_text("", "lyrics"), "lyrics");
    }

    #[test]
    fn empty_slide_renders_empty_text() {
        assert_eq!(fulltext_display_text("", ""), "");
    }
}
