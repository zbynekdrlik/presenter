use std::collections::HashMap;

use gloo_timers::callback::Interval;
use leptos::prelude::*;

use crate::api;
use crate::components::version_label::VersionLabel;
use crate::state::stage::StageContext;
use crate::ws::stage::StageWsState;

#[component]
pub fn CameraCrew(
    ws_state: ReadSignal<StageWsState>,
    latency_ms: ReadSignal<Option<f64>>,
) -> impl IntoView {
    let ctx = use_context::<StageContext>().expect("StageContext provided by CameraPage");

    let group_colors = RwSignal::new(HashMap::<String, String>::new());
    {
        leptos::task::spawn_local(async move {
            if let Ok(colors) = api::presentations::fetch_group_colors().await {
                group_colors.set(colors);
            }
        });
    }

    let color_for = move |name: &str| -> Option<String> {
        group_colors.with(|map| map.get(name).cloned())
    };

    // ── current group ──────────────────────────────────────────────────────────
    let current_group_label = move || {
        ctx.snapshot
            .get()
            .and_then(|s| s.current.and_then(|sl| sl.group))
            .unwrap_or_default()
    };

    let current_group_style = move || {
        let name = current_group_label();
        if name.is_empty() {
            return String::new();
        }
        color_for(&name)
            .map(|c| format!("background-color: {c};"))
            .unwrap_or_default()
    };

    // ── upcoming helper ────────────────────────────────────────────────────────
    let upcoming = move || {
        ctx.snapshot
            .get()
            .map(|s| s.upcoming_groups)
            .unwrap_or_default()
    };

    // ── next 1 ─────────────────────────────────────────────────────────────────
    let next_1_label = move || {
        upcoming()
            .into_iter()
            .next()
            .map(|g| g.name)
            .unwrap_or_default()
    };
    let next_1_style = move || {
        let name = next_1_label();
        if name.is_empty() {
            return String::new();
        }
        color_for(&name)
            .map(|c| format!("background-color: {c};"))
            .unwrap_or_default()
    };

    // ── next 2 ─────────────────────────────────────────────────────────────────
    let next_2_label = move || {
        upcoming()
            .into_iter()
            .nth(1)
            .map(|g| g.name)
            .unwrap_or_default()
    };
    let next_2_style = move || {
        let name = next_2_label();
        if name.is_empty() {
            return String::new();
        }
        color_for(&name)
            .map(|c| format!("background-color: {c};"))
            .unwrap_or_default()
    };

    // ── next 3 ─────────────────────────────────────────────────────────────────
    let next_3_label = move || {
        upcoming()
            .into_iter()
            .nth(2)
            .map(|g| g.name)
            .unwrap_or_default()
    };
    let next_3_style = move || {
        let name = next_3_label();
        if name.is_empty() {
            return String::new();
        }
        color_for(&name)
            .map(|c| format!("background-color: {c};"))
            .unwrap_or_default()
    };

    // ── next 4 ─────────────────────────────────────────────────────────────────
    let next_4_label = move || {
        upcoming()
            .into_iter()
            .nth(3)
            .map(|g| g.name)
            .unwrap_or_default()
    };
    let next_4_style = move || {
        let name = next_4_label();
        if name.is_empty() {
            return String::new();
        }
        color_for(&name)
            .map(|c| format!("background-color: {c};"))
            .unwrap_or_default()
    };

    // ── timers ─────────────────────────────────────────────────────────────────
    let preach_label = move || {
        ctx.snapshot
            .get()
            .map(|s| format_elapsed(s.timers.preach_timer.seconds_elapsed))
            .unwrap_or_else(|| "--:--".to_string())
    };

    let countdown_label = move || {
        let raw = ctx
            .snapshot
            .get()
            .map(|s| {
                presenter_core::format_countdown(s.timers.countdown_to_start.seconds_remaining)
            })
            .unwrap_or_default();
        if raw.is_empty() {
            "--:--".to_string()
        } else {
            raw
        }
    };

    // ── wall clock (updates every second) ─────────────────────────────────────
    let clock_label = RwSignal::new(String::new());
    {
        let update = move || {
            let date = js_sys::Date::new_0();
            clock_label.set(format!(
                "{:02}:{:02}",
                date.get_hours(),
                date.get_minutes()
            ));
        };
        update();
        let interval = Interval::new(1_000, update);
        interval.forget();
    }

    // ── on-air + latency ───────────────────────────────────────────────────────
    let on_air = move || ctx.broadcast_live.get();
    let latency_text = move || {
        latency_ms
            .get()
            .map(|ms| format!("{:.0}ms", ms))
            .unwrap_or_else(|| "—".to_string())
    };

    // Connection state class (colour hint on the on-air strip border).
    let _ws_state = ws_state; // retained so the prop is used; may drive future styling

    view! {
        <div class="stage-container" data-layout="camera-crew">
            // ── Left column: 5 stacked group boxes ───────────────────────────
            <div class="stage__camera-crew__column-left">
                <div class="stage__camera-crew__current-group">
                    <span class="stage__debug-label">"now"</span>
                    <div class="stage__group-pill" style=current_group_style>
                        {current_group_label}
                    </div>
                </div>
                <div class="stage__camera-crew__next-1">
                    <span class="stage__debug-label">"next 1"</span>
                    <div class="stage__group-pill" style=next_1_style>
                        {next_1_label}
                    </div>
                </div>
                <div class="stage__camera-crew__next-2">
                    <span class="stage__debug-label">"next 2"</span>
                    <div class="stage__group-pill" style=next_2_style>
                        {next_2_label}
                    </div>
                </div>
                <div class="stage__camera-crew__next-3">
                    <span class="stage__debug-label">"next 3"</span>
                    <div class="stage__group-pill" style=next_3_style>
                        {next_3_label}
                    </div>
                </div>
                <div class="stage__camera-crew__next-4">
                    <span class="stage__debug-label">"next 4"</span>
                    <div class="stage__group-pill" style=next_4_style>
                        {next_4_label}
                    </div>
                </div>
            </div>

            // ── Right column: preach | time | countdown | on-air+latency ─────
            <div class="stage__camera-crew__column-right">
                <div class="stage__camera-crew__preach">
                    <span class="stage__debug-label">"preach"</span>
                    <span class="stage__camera-crew__caption">"PREACH"</span>
                    <div
                        class="stage__camera-crew__timer-text"
                        data-testid="camera-crew-preach"
                    >
                        {preach_label}
                    </div>
                </div>
                <div class="stage__camera-crew__time">
                    <span class="stage__debug-label">"time"</span>
                    <span class="stage__camera-crew__caption">"TIME"</span>
                    <div
                        class="stage__camera-crew__timer-text"
                        data-testid="camera-crew-time"
                    >
                        {move || clock_label.get()}
                    </div>
                </div>
                <div class="stage__camera-crew__countdown">
                    <span class="stage__debug-label">"countdown"</span>
                    <span class="stage__camera-crew__caption">"COUNTDOWN"</span>
                    <div
                        class="stage__camera-crew__timer-text"
                        data-testid="camera-crew-countdown"
                    >
                        {countdown_label}
                    </div>
                </div>
                <div class="stage__camera-crew__on-air-strip">
                    <span class="stage__debug-label">"on-air"</span>
                    <div class="stage__camera-crew__on-air-row">
                        <span
                            class="stage__camera-crew__on-air-indicator"
                            class:is-on=on_air
                        >
                            "● ON AIR"
                        </span>
                        <span
                            class="stage__camera-crew__latency"
                            data-testid="camera-crew-latency"
                        >
                            {latency_text}
                        </span>
                    </div>
                </div>
            </div>

            // ── Version corner ────────────────────────────────────────────────
            <div class="stage__camera-crew__version">
                <span class="stage__debug-label">"version"</span>
                <VersionLabel />
            </div>
        </div>
    }
}

/// Format elapsed seconds as MM:SS (or HH:MM:SS when ≥ 1 hour).
fn format_elapsed(seconds: i64) -> String {
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
