use gloo_timers::callback::Interval;
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::HtmlElement;

use crate::state::stage::StageContext;
use crate::utils::autofit::autofit_text;
use crate::ws::stage::StageWsState;

const STATUS_MAX_FONT: f64 = 80.0;

fn autofit_status<T: 'static>(
    node_ref: NodeRef<leptos::html::Div>,
    trigger: impl Fn() -> T + 'static,
) {
    Effect::new(move |_| {
        let _trigger = trigger();
        if let Some(el) = node_ref.get() {
            let html_el: &HtmlElement = &el;
            let el_clone = html_el.clone();
            let cb = wasm_bindgen::closure::Closure::once_into_js(move || {
                autofit_text(&el_clone, STATUS_MAX_FONT);
            });
            let _ = web_sys::window()
                .expect("window")
                .request_animation_frame(cb.as_ref().unchecked_ref());
        }
    });
}

#[component]
pub fn StatusBar(
    ws_state: ReadSignal<StageWsState>,
    latency_ms: ReadSignal<Option<f64>>,
) -> impl IntoView {
    let ctx = use_context::<StageContext>().expect("StageContext not provided");

    let clock_ref = NodeRef::<leptos::html::Div>::new();
    let live_ref = NodeRef::<leptos::html::Div>::new();
    let connection_ref = NodeRef::<leptos::html::Div>::new();

    let (clock_text, set_clock_text) = signal(current_time_string());
    let _clock_interval = Interval::new(1_000, move || {
        set_clock_text.set(current_time_string());
    });
    _clock_interval.forget();

    let broadcast_live = ctx.broadcast_live;

    let live_text = move || {
        if broadcast_live.get() {
            "LIVE".to_string()
        } else {
            "VYSIELANIE JE VYPNUTE".to_string()
        }
    };

    let live_class = move || {
        if broadcast_live.get() {
            "stage__live-pill stage__live-pill--on"
        } else {
            "stage__live-pill stage__live-pill--off"
        }
    };

    let connection_text = move || {
        let label = match ws_state.get() {
            StageWsState::Connecting => "CONNECTING\u{2026}",
            StageWsState::Connected => "CONNECTED",
            StageWsState::Reconnecting => "RECONNECTING\u{2026}",
            StageWsState::Disconnected => "DISCONNECTED",
        };
        let latency = latency_ms
            .get()
            .map(|ms| format!(" \u{00b7} {:03} ms", ms as u32))
            .unwrap_or_default();
        format!("{label}{latency}")
    };

    let connection_class = move || {
        let base = "stage__connection";
        match ws_state.get() {
            StageWsState::Connecting => format!("{base} {base}--connecting"),
            StageWsState::Connected => format!("{base} {base}--connected"),
            StageWsState::Reconnecting => format!("{base} {base}--reconnecting"),
            StageWsState::Disconnected => format!("{base} {base}--disconnected"),
        }
    };

    autofit_status(clock_ref, move || clock_text.get());
    autofit_status(live_ref, live_text.clone());
    autofit_status(connection_ref, connection_text.clone());

    view! {
        <div class="stage__status-bar">
            <span class="stage__debug-label">"status-bar"</span>
            <div node_ref=clock_ref class="stage__clock">{clock_text}</div>
            <div node_ref=live_ref class=live_class>{live_text}</div>
            <div node_ref=connection_ref class=connection_class>{connection_text}</div>
        </div>
    }
}

fn current_time_string() -> String {
    let now = js_sys::Date::new_0();
    format!(
        "{:02}:{:02}:{:02}",
        now.get_hours(),
        now.get_minutes(),
        now.get_seconds()
    )
}
