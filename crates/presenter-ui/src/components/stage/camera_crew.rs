use std::collections::HashMap;

use leptos::prelude::*;
use presenter_core::UpcomingGroup;

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

    let upcoming = move || {
        ctx.snapshot
            .get()
            .map(|s| s.upcoming_groups)
            .unwrap_or_default()
    };

    let next_group = move || upcoming().into_iter().next();
    let future_groups = move || -> Vec<UpcomingGroup> {
        upcoming().into_iter().skip(1).take(3).collect()
    };

    let song_label = move || {
        let snap = ctx.snapshot.get();
        let song = snap
            .as_ref()
            .and_then(|s| s.song_name.clone())
            .unwrap_or_default();
        let library = snap
            .as_ref()
            .and_then(|s| s.library_name.clone())
            .unwrap_or_default();
        match (song.is_empty(), library.is_empty()) {
            (false, false) => format!("{song} · {library}"),
            (false, true) => song,
            (true, false) => library,
            _ => String::new(),
        }
    };

    let preach_label = move || {
        ctx.snapshot
            .get()
            .map(|s| format_elapsed(s.timers.preach_timer.seconds_elapsed))
            .unwrap_or_else(|| "--:--".to_string())
    };

    let countdown_label = move || {
        ctx.snapshot
            .get()
            .map(|s| {
                presenter_core::format_countdown(s.timers.countdown_to_start.seconds_remaining)
            })
            .unwrap_or_else(|| "--:--".to_string())
    };

    let on_air = move || ctx.broadcast_live.get();

    let latency_label = move || {
        latency_ms
            .get()
            .map(|ms| format!("{:.0}ms", ms))
            .unwrap_or_else(|| "—".to_string())
    };

    let connection_class = move || match ws_state.get() {
        StageWsState::Connected => "stage__camera-crew__conn stage__camera-crew__conn--ok",
        _ => "stage__camera-crew__conn stage__camera-crew__conn--bad",
    };

    view! {
        <div class="stage__camera-crew">
            <div class="stage__camera-crew__current stage__group-pill" style=current_group_style>
                {current_group_label}
            </div>

            <div class="stage__camera-crew__next">
                <span class="stage__camera-crew__next-label">"Next:"</span>
                {move || {
                    next_group().map(|g| {
                        let name = g.name.clone();
                        let style = color_for(&name)
                            .map(|c| format!("background-color: {c};"))
                            .unwrap_or_default();
                        view! {
                            <span
                                class="stage__group-pill stage__camera-crew__next-pill"
                                style=style
                            >
                                {name}
                            </span>
                        }
                    })
                }}
            </div>

            <div class="stage__camera-crew__future">
                {move || {
                    future_groups()
                        .into_iter()
                        .map(|g| {
                            let name = g.name.clone();
                            let style = color_for(&name)
                                .map(|c| format!("background-color: {c};"))
                                .unwrap_or_default();
                            view! {
                                <span
                                    class="stage__group-pill stage__camera-crew__future-pill"
                                    style=style
                                >
                                    {name}
                                </span>
                            }
                                .into_any()
                        })
                        .collect::<Vec<_>>()
                }}
            </div>

            <div class="stage__camera-crew__footer">
                <span
                    class="stage__camera-crew__song"
                    data-testid="camera-crew-song"
                >
                    {song_label}
                </span>
                <span
                    class="stage__camera-crew__preach"
                    data-testid="camera-crew-preach"
                >
                    "PREACH "
                    {preach_label}
                </span>
                <span
                    class="stage__camera-crew__countdown"
                    data-testid="camera-crew-countdown"
                >
                    "COUNTDOWN "
                    {countdown_label}
                </span>
                <span
                    class="stage__camera-crew__on-air"
                    class:is-on=on_air
                    data-testid="camera-crew-on-air"
                >
                    "● ON AIR"
                </span>
                <span class=connection_class>
                    <VersionLabel />
                    " · "
                    {latency_label}
                </span>
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
