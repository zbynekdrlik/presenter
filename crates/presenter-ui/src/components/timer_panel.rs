use crate::state::AppContext;
use leptos::prelude::*;
use presenter_core::TimerCommand;

fn format_seconds(seconds: i64) -> String {
    let abs = seconds.unsigned_abs();
    let h = abs / 3600;
    let m = (abs % 3600) / 60;
    let s = abs % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Timer panel for the timers view.
#[component]
pub fn TimerPanel() -> impl IntoView {
    let ctx = use_context::<AppContext>().expect("AppContext");

    let send_timer_cmd = move |cmd: TimerCommand| {
        let timers = ctx.timers;
        leptos::task::spawn_local(async move {
            if let Ok(overview) = crate::api::timers::send_command(&cmd).await {
                timers.set(Some(overview));
            }
        });
    };

    let on_countdown_start = move |_| {
        send_timer_cmd(TimerCommand::StartCountdown);
    };

    let on_countdown_target = move |ev: web_sys::KeyboardEvent| {
        if ev.key() != "Enter" {
            return;
        }
        let input = ev.target().and_then(|t| {
            use wasm_bindgen::JsCast;
            t.dyn_into::<web_sys::HtmlInputElement>().ok()
        });
        if let Some(el) = input {
            let val = el.value();
            if let Some(target) = parse_time_input(&val) {
                send_timer_cmd(TimerCommand::SetCountdownTarget { target });
            }
        }
    };

    let on_offset_minus = move |_| {
        let timers = ctx.timers.get_untracked();
        if let Some(overview) = timers {
            let current_target = overview.countdown_to_start.target;
            let new_target = current_target - chrono::Duration::minutes(5);
            send_timer_cmd(TimerCommand::SetCountdownTarget { target: new_target });
        }
    };

    let on_offset_plus = move |_| {
        let timers = ctx.timers.get_untracked();
        if let Some(overview) = timers {
            let current_target = overview.countdown_to_start.target;
            let new_target = current_target + chrono::Duration::minutes(5);
            send_timer_cmd(TimerCommand::SetCountdownTarget { target: new_target });
        }
    };

    let on_preach_start = move |_| {
        send_timer_cmd(TimerCommand::StartPreach);
    };

    let on_preach_reset = move |_| {
        send_timer_cmd(TimerCommand::ResetPreach);
    };

    let on_overlay_open = move |_| {
        let window = crate::utils::window::window();
        let _ = window.open_with_url("/overlays/timer");
    };

    let toast_message = ctx.toast_message;
    let toast_variant = ctx.toast_variant;
    let on_overlay_copy = move |_| {
        let origin = crate::utils::window::window()
            .location()
            .origin()
            .unwrap_or_default();
        let url = format!("{origin}/overlays/timer");
        let window = crate::utils::window::window();
        let clipboard = window.navigator().clipboard();
        let _ = clipboard.write_text(&url);
        toast_variant.set("success".to_string());
        toast_message.set(Some("Timer overlay URL copied".to_string()));
    };

    view! {
        <div class="operator__timers" data-view-panel="timers">
            // Countdown timer
            <div data-role="timer-countdown" class="operator__timer-card">
                <h3 class="operator__timer-title">"Countdown"</h3>
                <div class="operator__timer-value">
                    {move || {
                        ctx.timers.get()
                            .map(|t| format_seconds(t.countdown_to_start.seconds_remaining))
                            .unwrap_or_else(|| "0:00".to_string())
                    }}
                </div>
                <div class="operator__timer-state">
                    {move || {
                        ctx.timers.get()
                            .map(|t| format!("{:?}", t.countdown_to_start.state))
                            .unwrap_or_else(|| "Idle".to_string())
                    }}
                </div>
                <div class="operator__timer-target">
                    {move || {
                        ctx.timers.get()
                            .map(|t| format!("Target: {}", t.countdown_to_start.target.format("%H:%M:%S")))
                            .unwrap_or_default()
                    }}
                </div>
                <div class="operator__timer-controls">
                    <input
                        type="text"
                        data-role="countdown-target-input"
                        class="operator__timer-input"
                        inputmode="numeric"
                        placeholder="HH:MM"
                        aria-label="Countdown target time (HH:MM)"
                        on:keydown=on_countdown_target
                    />
                    <button data-role="countdown-start" class="operator__timer-btn" on:click=on_countdown_start>
                        "Start"
                    </button>
                    <button data-role="countdown-offset-minus" class="operator__timer-btn" on:click=on_offset_minus>
                        "-5 min"
                    </button>
                    <button data-role="countdown-offset-plus" class="operator__timer-btn" on:click=on_offset_plus>
                        "+5 min"
                    </button>
                </div>
            </div>

            // Preach timer
            <div data-role="timer-preach" class="operator__timer-card">
                <h3 class="operator__timer-title">"Preach"</h3>
                <div class="operator__timer-value">
                    {move || {
                        ctx.timers.get()
                            .map(|t| format_seconds(t.preach_timer.seconds_elapsed))
                            .unwrap_or_else(|| "0:00".to_string())
                    }}
                </div>
                <div class="operator__timer-state">
                    {move || {
                        ctx.timers.get()
                            .map(|t| format!("{:?}", t.preach_timer.state))
                            .unwrap_or_else(|| "Idle".to_string())
                    }}
                </div>
                <div class="operator__timer-controls">
                    <button data-command="start_preach" class="operator__timer-btn" on:click=on_preach_start>
                        "Start"
                    </button>
                    <button data-command="reset_preach" class="operator__timer-btn" on:click=on_preach_reset>
                        "Reset"
                    </button>
                </div>
            </div>

            // Overlay controls
            <div class="operator__timer-overlay-controls">
                <button data-role="timer-overlay-open" class="operator__timer-btn" on:click=on_overlay_open>
                    "Open Countdown Overlay"
                </button>
                <button data-role="timer-overlay-copy" class="operator__timer-btn" on:click=on_overlay_copy>
                    "Copy Overlay URL"
                </button>
            </div>
        </div>
    }
}

fn parse_time_input(input: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let trimmed = input.trim();
    let parts: Vec<&str> = trimmed.split(':').collect();

    let (hours, minutes) = match parts.len() {
        1 => {
            // Just minutes - interpret as minutes from now
            let mins: u32 = parts[0].parse().ok()?;
            let target = chrono::Utc::now() + chrono::Duration::minutes(i64::from(mins));
            return Some(target);
        }
        2 => {
            let h: u32 = parts[0].trim().parse().ok()?;
            let m: u32 = parts[1].trim().parse().ok()?;
            (h, m)
        }
        _ => return None,
    };

    if hours > 23 || minutes > 59 {
        return None;
    }

    let now = chrono::Utc::now();
    let today = now.date_naive();
    let time = chrono::NaiveTime::from_hms_opt(hours, minutes, 0)?;
    let mut target = today.and_time(time).and_utc();

    // If the target is in the past, set it for tomorrow
    if target <= now {
        target += chrono::Duration::days(1);
    }

    Some(target)
}
