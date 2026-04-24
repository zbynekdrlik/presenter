use gloo_timers::callback::Interval;
use leptos::prelude::*;

use crate::state::stage::StageContext;
use crate::utils::autofit::autofit_effect;
use crate::ws::stage::StageWsState;

const STATUS_MAX_FONT: f64 = 200.0;

#[component]
pub fn StatusBar(
    ws_state: ReadSignal<StageWsState>,
    latency_ms: ReadSignal<Option<f64>>,
    /// Hide the live/broadcast pill (used by NDI fullscreen layout)
    #[prop(default = false)]
    hide_live: bool,
) -> impl IntoView {
    let ctx = use_context::<StageContext>().expect("StageContext not provided");

    let clock_ref = NodeRef::<leptos::html::Div>::new();
    let live_ref = NodeRef::<leptos::html::Div>::new();
    let connection_ref = NodeRef::<leptos::html::Div>::new();
    let song_number_ref = NodeRef::<leptos::html::Div>::new();

    let (clock_text, set_clock_text) = signal(current_time_string());
    let _clock_interval = Interval::new(1_000, move || {
        set_clock_text.set(current_time_string());
    });
    _clock_interval.forget();

    let broadcast_live = ctx.broadcast_live;

    let song_number = move || {
        ctx.snapshot
            .get()
            .and_then(|s| s.song_number)
            .map(|n| format!("#{n}"))
            .unwrap_or_default()
    };

    let has_song_number = move || ctx.snapshot.get().and_then(|s| s.song_number).is_some();

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
            .map(|ms| format!(" \u{00b7} {} ms", ms as u32))
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

    autofit_effect(clock_ref, STATUS_MAX_FONT, move || clock_text.get());
    if !hide_live {
        autofit_effect(live_ref, STATUS_MAX_FONT, live_text);
    }
    autofit_effect(connection_ref, STATUS_MAX_FONT, connection_text);
    autofit_effect(song_number_ref, STATUS_MAX_FONT, song_number);

    view! {
        <div node_ref=clock_ref class="stage__clock">
            <span class="stage__debug-label">"clock"</span>
            {clock_text}
        </div>
        {move || has_song_number().then(|| view! {
            <div node_ref=song_number_ref class="stage__song-number" data-role="song-number">
                <span class="stage__debug-label">"song-number"</span>
                {song_number}
            </div>
        })}
        {(!hide_live).then(|| view! {
            <div node_ref=live_ref class=live_class>
                <span class="stage__debug-label">"live"</span>
                {live_text}
            </div>
        })}
        <div node_ref=connection_ref class=connection_class>
            <span class="stage__debug-label">"connection"</span>
            {connection_text}
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
