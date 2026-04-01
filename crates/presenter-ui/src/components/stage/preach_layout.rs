use leptos::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::HtmlElement;

use crate::state::stage::StageContext;
use crate::utils::autofit::autofit_text;
use crate::ws::stage::StageWsState;

const PREACH_MAX_FONT: f64 = 300.0;
/// Overtime threshold in seconds (default 30 min).
const OVERTIME_THRESHOLD_SECS: i64 = 30 * 60;

#[component]
pub fn PreachLayout(
    ws_state: ReadSignal<StageWsState>,
    latency_ms: ReadSignal<Option<f64>>,
) -> impl IntoView {
    let ctx = use_context::<StageContext>().expect("StageContext not provided");

    let preach_ref = NodeRef::<leptos::html::Div>::new();

    let preach_data = move || {
        ctx.snapshot
            .get()
            .map(|s| {
                let elapsed = s.timers.preach_timer.seconds_elapsed;
                let text = format_seconds(elapsed);
                let overtime = elapsed > OVERTIME_THRESHOLD_SECS;
                (text, overtime)
            })
            .unwrap_or_else(|| ("00:00".to_string(), false))
    };

    {
        let r = preach_ref;
        Effect::new(move |_| {
            let _d = preach_data();
            if let Some(el) = r.get() {
                let html_el: &HtmlElement = &el;
                let el_clone = html_el.clone();
                let cb = wasm_bindgen::closure::Closure::once_into_js(move || {
                    autofit_text(&el_clone, PREACH_MAX_FONT);
                });
                let _ = web_sys::window()
                    .expect("window")
                    .request_animation_frame(cb.as_ref().unchecked_ref());
            }
        });
    }

    view! {
        <div class="stage-container" data-layout="preach">
            <div class="stage-preach__display">
                <span class="stage__debug-label">"preach-display"</span>
                <div
                    node_ref=preach_ref
                    class=move || {
                        let (_, overtime) = preach_data();
                        if overtime {
                            "stage-preach__text stage-preach__text--overtime"
                        } else {
                            "stage-preach__text stage-preach__text--normal"
                        }
                    }
                >
                    {move || preach_data().0}
                </div>
            </div>
            <super::status_bar::StatusBar ws_state=ws_state latency_ms=latency_ms />
            <super::bible_overlay::BibleOverlay overlay=ctx.bible_overlay />
        </div>
    }
}

fn format_seconds(seconds: i64) -> String {
    let secs = seconds.max(0);
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h:02}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}
