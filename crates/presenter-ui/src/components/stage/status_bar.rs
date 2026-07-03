use gloo_timers::callback::Interval;
use leptos::prelude::*;

use crate::components::version_label::VersionLabel;
use crate::state::stage::{StageContext, StageHealth};
use crate::utils::autofit::autofit_effect_tabular;
use crate::ws::stage::StageWsState;

const STATUS_MAX_FONT: f64 = 200.0;

#[component]
pub fn StatusBar(
    ws_state: ReadSignal<StageWsState>,
    latency_ms: ReadSignal<Option<f64>>,
    /// Hide the live/broadcast pill (used by NDI fullscreen layout)
    #[prop(default = false)]
    hide_live: bool,
    /// Hide the song number (used by NDI fullscreen layout — #436)
    #[prop(default = false)]
    hide_song_number: bool,
) -> impl IntoView {
    let ctx = use_context::<StageContext>().expect("StageContext not provided");

    let clock_ref = NodeRef::<leptos::html::Div>::new();
    let live_ref = NodeRef::<leptos::html::Div>::new();
    let connection_ref = NodeRef::<leptos::html::Div>::new();
    let song_number_ref = NodeRef::<leptos::html::Div>::new();
    let video_latency_ref = NodeRef::<leptos::html::Div>::new();

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

    let has_song_number =
        move || !hide_song_number && ctx.snapshot.get().and_then(|s| s.song_number).is_some();

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

    // #512: the TRUE server→display video latency — network transit (RTT/2 via
    // /ndi/time) + render residual (buffer+decode+present). A SEPARATE readout
    // next to the connection one. Sourced from the shared StageContext signal
    // written by `NdiVideo`'s frame observer. Shown whenever NDI is the ACTIVE
    // source (`ndi_active` — a stable per-layout flag, NOT the flaky per-frame
    // `frames_live` which throttles on idle/headless and would wrongly hide the
    // readout); the value is the number, or "n/a" when there is no trustworthy
    // measurement (no fresh /ndi/time offset / it aged out) — never a misleading
    // residual. Non-NDI layouts leave `ndi_active` false so the readout is absent.
    let video_latency = ctx.video_latency_ms;
    let ndi_active = ctx.ndi_active;
    let has_video_latency = move || ndi_active.get();
    // #532: recent-window health verdict (🟢/🟡/🔴), shown appended to the
    // latency figure — see `format_video_latency_line`. Replaces the
    // cumulative ⬇N/❄N suffix #523 drove from `ctx.dropped_frames` (that
    // signal is still populated by the beacon, just no longer read here).
    let stage_health = ctx.stage_health;
    let video_latency_text = move || format_video_latency_line(video_latency.get());

    autofit_effect_tabular(clock_ref, STATUS_MAX_FONT, move || clock_text.get());
    if !hide_live {
        autofit_effect_tabular(live_ref, STATUS_MAX_FONT, live_text);
    }
    if !hide_song_number {
        autofit_effect_tabular(song_number_ref, STATUS_MAX_FONT, song_number);
    }
    // #524: `.stage__connection` and `.stage__video-latency` are diagnostic-only
    // readouts (close-up info for the operator, not primary content) — they
    // deliberately do NOT autofit to fill their box (that's why they used to
    // look too prominent). `stage.css` gives them a small fixed font-size +
    // low opacity instead; the clock/live/song-number readouts above keep
    // autofit since they ARE primary content.

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
        {move || has_video_latency().then(|| view! {
            <div node_ref=video_latency_ref class="stage__video-latency" data-role="video-latency">
                <span class="stage__debug-label">"video-latency"</span>
                {video_latency_text}
                // #532/#536: recent-window health verdict as a SMALL colored
                // dot (green/amber/red) + fps — replaces the wide emoji+word
                // suffix that overflowed the narrow readout box off-screen.
                {move || stage_health.get().map(|r| view! {
                    " \u{00b7} "
                    <span class=stage_health_dot_class(r.state) data-role="health-dot"></span>
                    {format!(" {} fps", r.fps.round() as u32)}
                })}
            </div>
        })}
        <div class="stage__version">
            <span class="stage__debug-label">"version"</span>
            <VersionLabel />
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

/// Pure format helper for the `.stage__video-latency` readout text (#532):
/// appends the recent-window health verdict (🟢/🟡/🔴 + Slovak label + the
/// recent fps figure it was derived from) to the existing "server→displej ·
/// N ms" figure. Replaces the CUMULATIVE ⬇N/❄N suffix #523 put here — that
/// count never recovered from one old network blip, which the user found
/// meaningless; the underlying `dropped_frames` plumbing that fed it is left
/// running (per the issue), only this on-screen text moved to the new
/// signal. Extracted so the formatting is host-unit-testable without a live
/// Leptos/WASM render — the reactive closure in `StatusBar` is a thin wrapper
/// over this. #536: this now returns ONLY the latency figure; the health
/// verdict is rendered separately as a small colored DOT (+ fps) span in the
/// view, because the old emoji+word suffix (`🟢 plynulé · 28 fps`) overflowed
/// the narrow (24%-wide) readout box and was truncated / pushed off-screen on
/// the real stage TVs, and the emoji glyph could not be sized down.
fn format_video_latency_line(latency_ms: Option<f64>) -> String {
    match latency_ms {
        Some(ms) => format!("server\u{2192}displej \u{00b7} {} ms", ms as u32),
        None => "server\u{2192}displej \u{00b7} n/a".to_string(),
    }
}

/// CSS class for the recent-window health DOT (#536): a small colored circle
/// (green/amber/red) whose SIZE is controlled by `stage.css` — unlike the old
/// emoji glyph, which rendered oversized and could not be shrunk. The color
/// alone conveys "usable right now?", so the Slovak word is dropped (it was
/// what overflowed the box). Pure + host-unit-tested.
fn stage_health_dot_class(state: StageHealth) -> &'static str {
    match state {
        StageHealth::Good => "stage__health-dot stage__health-dot--good",
        StageHealth::Degraded => "stage__health-dot stage__health-dot--degraded",
        StageHealth::Bad => "stage__health-dot stage__health-dot--bad",
    }
}

#[cfg(test)]
mod tests {
    use super::{format_video_latency_line, stage_health_dot_class};
    use crate::state::stage::StageHealth;

    #[test]
    fn shows_latency_number_when_measured() {
        assert_eq!(
            format_video_latency_line(Some(112.0)),
            "server\u{2192}displej \u{00b7} 112 ms"
        );
    }

    #[test]
    fn shows_n_a_when_no_trustworthy_latency() {
        assert_eq!(
            format_video_latency_line(None),
            "server\u{2192}displej \u{00b7} n/a"
        );
    }

    #[test]
    fn health_dot_class_is_distinct_and_color_coded_per_state() {
        // Each state maps to its own modifier so stage.css can colour + SIZE
        // the dot (the whole point of #536 — a small, controllable dot instead
        // of the oversized emoji). Base class shared; modifier distinct.
        let good = stage_health_dot_class(StageHealth::Good);
        let degraded = stage_health_dot_class(StageHealth::Degraded);
        let bad = stage_health_dot_class(StageHealth::Bad);
        assert_eq!(good, "stage__health-dot stage__health-dot--good");
        assert_eq!(degraded, "stage__health-dot stage__health-dot--degraded");
        assert_eq!(bad, "stage__health-dot stage__health-dot--bad");
        assert_ne!(good, degraded);
        assert_ne!(degraded, bad);
        assert_ne!(good, bad);
        for c in [good, degraded, bad] {
            assert!(c.starts_with("stage__health-dot "));
        }
    }
}
