//! Layout render functions for stage displays.

use leptos::prelude::*;
use presenter_core::{
    StageDisplaySlide, StageDisplaySnapshot, StagePlaylistEntry, TimerState,
    DEFAULT_STAGE_LAYOUT_CODE,
};

pub(super) fn render_layout(snapshot: &StageDisplaySnapshot) -> AnyView {
    match snapshot.layout.code.as_str() {
        DEFAULT_STAGE_LAYOUT_CODE => render_worship_snv(snapshot),
        "worship-pp" => render_worship_pp(snapshot),
        "timer" => render_timer(snapshot),
        "preach" => render_preach(snapshot),
        _ => view! { <p class="stage__empty">"Unsupported layout."</p> }.into_any(),
    }
}

fn render_worship_snv(snapshot: &StageDisplaySnapshot) -> AnyView {
    let current_text = snapshot
        .current
        .as_ref()
        .map(primary_text)
        .unwrap_or_default();
    let current_group = snapshot
        .current
        .as_ref()
        .and_then(|slide| slide.group.clone())
        .unwrap_or_default();
    let next_text = snapshot.next.as_ref().map(primary_text).unwrap_or_default();
    let next_group = snapshot
        .next
        .as_ref()
        .and_then(|slide| slide.group.clone())
        .unwrap_or_default();

    view! {
        <>
            <div class="stage__box stage__box--current-group" data-hidden={(current_group.is_empty()).to_string()}>
                <span id="current-group" class="stage__group">{current_group.clone()}</span>
            </div>
            <div class="stage__box stage__box--current-slide">
                <p id="current-text">{current_text}</p>
            </div>
            <div class="stage__box stage__box--next-group" data-hidden={(next_group.is_empty()).to_string()}>
                <span id="next-group" class="stage__group">{next_group.clone()}</span>
            </div>
            <div class="stage__box stage__box--next-slide">
                <p id="next-text">{next_text}</p>
            </div>
            <div class="stage__box stage__box--clock">
                <span id="stage-clock">"00:00:00"</span>
            </div>
            <div class="stage__box stage__box--live-indicator">
                <span id="stage-live" class="stage__live" data-active="false">"VYSIELANIE JE VYPNUTE"</span>
            </div>
            <div class="stage__box stage__box--connection-status">
                <span id="stage-status-connection">"Connecting..."</span>
                <span id="stage-status-latency" class="stage__status-latency" data-visible="false"></span>
            </div>
        </>
    }
    .into_any()
}

fn render_worship_pp(snapshot: &StageDisplaySnapshot) -> AnyView {
    let current_main = snapshot
        .current
        .as_ref()
        .map(primary_text)
        .unwrap_or_default();
    let current_group = snapshot
        .current
        .as_ref()
        .and_then(|slide| slide.group.clone())
        .unwrap_or_default();
    let next_main = snapshot.next.as_ref().map(primary_text).unwrap_or_default();
    let next_group = snapshot
        .next
        .as_ref()
        .and_then(|slide| slide.group.clone())
        .unwrap_or_default();
    let playlist_name = snapshot.playlist_name.clone().unwrap_or_default();
    let entries = snapshot.playlist_entries.clone().unwrap_or_default();
    let has_playlist = !entries.is_empty();

    view! {
        <section class="stage__worship-pp" data-has-playlist={has_playlist.to_string()}>
            <div class="stage__worship-pp-slides">
                <div class="stage__worship-pp-current">
                    <div class="stage__group-slot">
                        <span
                            id="current-group"
                            class="stage__group"
                            data-hidden={(current_group.is_empty()).to_string()}
                        >
                            {current_group.clone()}
                        </span>
                    </div>
                    <p id="current-main">{current_main}</p>
                </div>
                <div class="stage__worship-pp-next">
                    <div class="stage__group-slot stage__group-slot--next">
                        <span
                            id="next-group"
                            class="stage__group stage__group--next"
                            data-hidden={(next_group.is_empty()).to_string()}
                        >
                            {next_group.clone()}
                        </span>
                    </div>
                    <p id="next-main">{next_main}</p>
                </div>
            </div>
            <aside class="stage__worship-pp-playlist" id="playlist-sidebar">
                <h3 id="playlist-name">{playlist_name}</h3>
                <ul class="stage__worship-pp-playlist-list" id="playlist-list">
                    {render_playlist_entries(&entries)}
                </ul>
            </aside>
        </section>
    }
    .into_any()
}

fn render_playlist_entries(entries: &[StagePlaylistEntry]) -> String {
    let mut pres_num = 0u32;
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let active = if entry.is_active { "true" } else { "false" };
            let entry_type = &entry.entry_type;
            let name = html_escape(&entry.name);
            let label = if entry_type == "presentation" {
                pres_num += 1;
                format!("{pres_num}. {name}")
            } else {
                name
            };
            format!(
                "<li class=\"stage__worship-pp-playlist-entry\" \
                 id=\"playlist-entry-{index}\" \
                 data-active=\"{active}\" \
                 data-type=\"{entry_type}\">{label}</li>"
            )
        })
        .collect::<Vec<_>>()
        .join("")
}

fn html_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn render_timer(snapshot: &StageDisplaySnapshot) -> AnyView {
    let countdown = snapshot.timers.countdown_to_start.seconds_remaining;
    let formatted = format_hms(countdown);

    view! {
        <>
            <div class="stage__box stage__box--countdown-timer">
                <div>
                    <span id="countdown-value">{formatted}</span>
                    <p class="stage__timer-label">"Service Countdown"</p>
                </div>
            </div>
            <div class="stage__box stage__box--clock">
                <span id="stage-clock">"00:00:00"</span>
            </div>
            <div class="stage__box stage__box--live-indicator">
                <span id="stage-live" class="stage__live" data-active="false">"VYSIELANIE JE VYPNUTE"</span>
            </div>
            <div class="stage__box stage__box--connection-status">
                <span id="stage-status-connection">"Connecting..."</span>
                <span id="stage-status-latency" class="stage__status-latency" data-visible="false"></span>
            </div>
        </>
    }
    .into_any()
}

fn render_preach(snapshot: &StageDisplaySnapshot) -> AnyView {
    let elapsed = snapshot.timers.preach_timer.seconds_elapsed;
    let formatted = format_hms(elapsed);
    let status = match snapshot.timers.preach_timer.state {
        TimerState::Running => "Running",
        TimerState::Paused => "Paused",
        TimerState::Idle => "Idle",
        TimerState::Completed => "Completed",
    };

    view! {
        <>
            <div class="stage__box stage__box--preach-timer">
                <div>
                    <span id="preach-value">{formatted}</span>
                    <p class="stage__timer-label">"Preach Timer ("<span id="preach-status">{status}</span>")"</p>
                </div>
            </div>
            <div class="stage__box stage__box--clock">
                <span id="stage-clock">"00:00:00"</span>
            </div>
            <div class="stage__box stage__box--live-indicator">
                <span id="stage-live" class="stage__live" data-active="false">"VYSIELANIE JE VYPNUTE"</span>
            </div>
            <div class="stage__box stage__box--connection-status">
                <span id="stage-status-connection">"Connecting..."</span>
                <span id="stage-status-latency" class="stage__status-latency" data-visible="false"></span>
            </div>
        </>
    }
    .into_any()
}

fn primary_text(slide: &StageDisplaySlide) -> String {
    if slide.stage.trim().is_empty() {
        slide.main.clone()
    } else {
        slide.stage.clone()
    }
}

fn format_hms(seconds: i64) -> String {
    let t = seconds.max(0);
    let (h, m, s) = (t / 3600, (t % 3600) / 60, t % 60);
    if h > 0 {
        format!("{h:02}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}
