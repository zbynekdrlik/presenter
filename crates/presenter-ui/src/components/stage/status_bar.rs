use gloo_timers::callback::Interval;
use leptos::prelude::*;

use crate::components::version_label::VersionLabel;
use crate::state::stage::StageContext;
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
    // #523: per-display dropped-frame + freeze count from the getStats beacon,
    // shown appended to the latency figure — see `format_video_latency_line`.
    let dropped_frames = ctx.dropped_frames;
    let video_latency_text =
        move || format_video_latency_line(video_latency.get(), dropped_frames.get());

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

/// Pure format helper for the `.stage__video-latency` readout text (#523):
/// appends the per-display dropped-frame (+ freeze, when nonzero) count from
/// the getStats beacon to the existing "server→displej · N ms" figure.
/// Extracted so the formatting is host-unit-testable without a live
/// Leptos/WASM render — the reactive closure in `StatusBar` is a thin wrapper
/// over this.
fn format_video_latency_line(
    latency_ms: Option<f64>,
    dropped_frames: Option<(u32, u32)>,
) -> String {
    let base = match latency_ms {
        Some(ms) => format!("server\u{2192}displej \u{00b7} {} ms", ms as u32),
        None => "server\u{2192}displej \u{00b7} n/a".to_string(),
    };
    match dropped_frames {
        // Freeze count is shown too, but only when it's actually nonzero —
        // keeps the common case (dropped frames with no freeze) to ONE short
        // extra token, per the issue's "ONE short line" format.
        Some((dropped, freeze)) if freeze > 0 => {
            format!("{base} \u{00b7} \u{2b07}{dropped} \u{2744}{freeze}")
        }
        Some((dropped, _)) => format!("{base} \u{00b7} \u{2b07}{dropped}"),
        None => base,
    }
}

#[cfg(test)]
mod tests {
    use super::format_video_latency_line;

    #[test]
    fn shows_latency_alone_when_no_drop_data_yet() {
        // No beacon has landed yet (fresh session / reconnect) — the readout
        // must not show a stale/fabricated drop count.
        assert_eq!(
            format_video_latency_line(Some(112.0), None),
            "server\u{2192}displej \u{00b7} 112 ms"
        );
        assert_eq!(
            format_video_latency_line(None, None),
            "server\u{2192}displej \u{00b7} n/a"
        );
    }

    #[test]
    fn appends_dropped_count_with_no_freeze() {
        assert_eq!(
            format_video_latency_line(Some(112.0), Some((0, 0))),
            "server\u{2192}displej \u{00b7} 112 ms \u{00b7} \u{2b07}0"
        );
        assert_eq!(
            format_video_latency_line(Some(84.0), Some((128, 0))),
            "server\u{2192}displej \u{00b7} 84 ms \u{00b7} \u{2b07}128"
        );
    }

    #[test]
    fn appends_freeze_count_only_when_nonzero() {
        assert_eq!(
            format_video_latency_line(Some(84.0), Some((128, 2))),
            "server\u{2192}displej \u{00b7} 84 ms \u{00b7} \u{2b07}128 \u{2744}2"
        );
        // Freezes with zero dropped frames is a real combination too (a
        // display can freeze without the decoder ever reporting a drop) —
        // both figures show together, symmetric with the dropped-only case.
        assert_eq!(
            format_video_latency_line(Some(84.0), Some((0, 3))),
            "server\u{2192}displej \u{00b7} 84 ms \u{00b7} \u{2b07}0 \u{2744}3"
        );
    }

    #[test]
    fn n_a_latency_still_shows_drop_count() {
        // A dropped-frame count is meaningful even without a trustworthy
        // latency reading — never suppress it just because latency is n/a.
        assert_eq!(
            format_video_latency_line(None, Some((5, 0))),
            "server\u{2192}displej \u{00b7} n/a \u{00b7} \u{2b07}5"
        );
    }
}
