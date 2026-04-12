use leptos::prelude::*;

use crate::components::stage::status_bar::StatusBar;
use crate::state::stage::StageContext;
use crate::ws::stage::StageWsState;

#[component]
pub fn BibleLayout(
    ws_state: ReadSignal<StageWsState>,
    latency_ms: ReadSignal<Option<f64>>,
) -> impl IntoView {
    let ctx = use_context::<StageContext>().expect("StageContext not provided");
    let bible_overlay = ctx.bible_overlay;

    view! {
        <div class="stage-container" data-layout="bible">
            {move || {
                if let Some(output) = bible_overlay.get() {
                    let has_secondary = !output.secondary_text.is_empty();
                    let secondary_visible = if has_secondary { "true" } else { "false" };

                    view! {
                        <div class="stage__bible-content">
                            <div class="stage__bible-text">{output.main_text.clone()}</div>
                            <div class="stage__bible-reference">{output.main_reference.clone()}</div>

                            <div class="stage__bible-secondary" data-visible=secondary_visible>
                                <div class="stage__bible-secondary-text">
                                    {output.secondary_text.clone()}
                                </div>
                                <div class="stage__bible-secondary-ref">
                                    {output.secondary_reference.clone()}
                                </div>
                            </div>
                        </div>
                    }
                    .into_any()
                } else {
                    view! {
                        <div class="stage__bible-waiting">"Waiting for Bible passage\u{2026}"</div>
                    }
                    .into_any()
                }
            }}
            <StatusBar ws_state=ws_state latency_ms=latency_ms />
        </div>
    }
}
