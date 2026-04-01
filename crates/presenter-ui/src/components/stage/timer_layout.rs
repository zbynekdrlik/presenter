use leptos::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::HtmlElement;

use crate::state::stage::StageContext;
use crate::utils::autofit::autofit_text;
use crate::ws::stage::StageWsState;

const TIMER_MAX_FONT: f64 = 300.0;

#[component]
pub fn TimerLayout(
    ws_state: ReadSignal<StageWsState>,
    latency_ms: ReadSignal<Option<f64>>,
) -> impl IntoView {
    let ctx = use_context::<StageContext>().expect("StageContext not provided");

    let timer_ref = NodeRef::<leptos::html::Div>::new();

    let timer_text = move || {
        ctx.snapshot
            .get()
            .map(|s| format_seconds(s.timers.countdown_to_start.seconds_remaining))
            .unwrap_or_else(|| "00:00".to_string())
    };

    {
        let r = timer_ref;
        Effect::new(move |_| {
            let _t = timer_text();
            if let Some(el) = r.get() {
                let html_el: &HtmlElement = &el;
                let el_clone = html_el.clone();
                let cb = wasm_bindgen::closure::Closure::once_into_js(move || {
                    autofit_text(&el_clone, TIMER_MAX_FONT);
                });
                let _ = web_sys::window()
                    .expect("window")
                    .request_animation_frame(cb.as_ref().unchecked_ref());
            }
        });
    }

    view! {
        <div class="stage-container" data-layout="timer">
            <div class="stage-timer__display">
                <span class="stage__debug-label">"timer-display"</span>
                <div node_ref=timer_ref class="stage-timer__text">
                    {timer_text}
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
