use gloo_timers::callback::Interval;
use leptos::prelude::*;

use crate::state::stage::StageContext;
use crate::ws::stage::StageWsState;

#[component]
pub fn StatusBar(
    ws_state: ReadSignal<StageWsState>,
    latency_ms: ReadSignal<Option<f64>>,
) -> impl IntoView {
    let ctx = use_context::<StageContext>().expect("StageContext not provided");

    let (clock_text, set_clock_text) = signal(current_time_string());
    let _clock_interval = Interval::new(1_000, move || {
        set_clock_text.set(current_time_string());
    });
    _clock_interval.forget();

    let connection_label = move || match ws_state.get() {
        StageWsState::Connecting => "CONNECTING\u{2026}",
        StageWsState::Connected => "CONNECTED",
        StageWsState::Reconnecting => "RECONNECTING\u{2026}",
        StageWsState::Disconnected => "DISCONNECTED",
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

    let latency_text = move || {
        latency_ms
            .get()
            .map(|ms| format!("\u{00b7} {:03} ms", ms as u32))
    };

    let broadcast_live = ctx.broadcast_live;

    view! {
        <div class="stage__status-bar">
            <span class="stage__clock">{clock_text}</span>

            {move || {
                let is_live = broadcast_live.get();
                let (class, text) = if is_live {
                    ("stage__live-pill stage__live-pill--on", "LIVE")
                } else {
                    ("stage__live-pill stage__live-pill--off", "VYSIELANIE JE VYPNUTE")
                };
                view! { <span class=class>{text}</span> }
            }}

            <span class=connection_class>
                {connection_label}
                {move || {
                    latency_text()
                        .map(|t| {
                            view! { <span class="stage__connection-latency">{" "}{t}</span> }
                        })
                }}
            </span>
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
