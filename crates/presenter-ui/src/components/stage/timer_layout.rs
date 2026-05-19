use leptos::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::HtmlElement;

use crate::components::stage::ndi_status_text;
use crate::components::stage::ndi_video::NdiVideo;
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
    let ndi_active = ctx.ndi_active;
    let ndi_active_source_id = ctx.ndi_active_source_id;
    let ndi_status = ctx.ndi_status;

    let timer_ref = NodeRef::<leptos::html::Div>::new();

    let timer_text = move || {
        ctx.snapshot
            .get()
            .map(|s| presenter_core::format_countdown(s.timers.countdown_to_start.seconds_remaining))
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
            <Show when=move || ndi_active.get()>
                {move || {
                    ndi_active_source_id.get().map(|source_id| view! {
                        <NdiVideo
                            source_id=source_id
                            class="stage-timer__ndi"
                        />
                    })
                }}
            </Show>

            <Show when=move || {
                let status = ndi_status.get();
                status == "disconnected"
                    || status == "connecting"
                    || status.starts_with("failed")
            }>
                <div class="stage-timer__overlay">
                    {move || ndi_status_text(&ndi_status.get())}
                </div>
            </Show>

            <div class="stage-timer__display">
                <span class="stage__debug-label">"timer-display"</span>
                <div node_ref=timer_ref class="stage-timer__text">
                    {timer_text}
                </div>
            </div>
            <super::status_bar::StatusBar ws_state=ws_state latency_ms=latency_ms />
        </div>
    }
}
