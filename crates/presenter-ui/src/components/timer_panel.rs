use leptos::prelude::*;

use crate::state::AppContext;

fn format_seconds(seconds: i64) -> String {
    let abs = seconds.unsigned_abs();
    let h = abs / 3600;
    let m = (abs % 3600) / 60;
    let s = abs % 60;
    let sign = if seconds < 0 { "-" } else { "" };
    if h > 0 {
        format!("{sign}{h}:{m:02}:{s:02}")
    } else {
        format!("{sign}{m}:{s:02}")
    }
}

fn format_timer_state(state: presenter_core::timer::TimerState) -> &'static str {
    match state {
        presenter_core::timer::TimerState::Idle => "idle",
        presenter_core::timer::TimerState::Running => "running",
        presenter_core::timer::TimerState::Paused => "paused",
        presenter_core::timer::TimerState::Completed => "completed",
    }
}

/// Timer panel for the timers view.
#[component]
pub fn TimerPanel(ctx: AppContext) -> impl IntoView {
    let timers = ctx.timers;

    let countdown_value = move || {
        timers
            .get()
            .map(|t| format_seconds(t.countdown_to_start.seconds_remaining))
            .unwrap_or_else(|| "0:00".to_string())
    };

    let countdown_target_str = move || {
        timers
            .get()
            .map(|t| format!("Target {}", t.countdown_to_start.target.format("%H:%M:%S")))
            .unwrap_or_default()
    };

    let preach_value = move || {
        timers
            .get()
            .map(|t| format_seconds(t.preach_timer.seconds_elapsed))
            .unwrap_or_else(|| "0:00".to_string())
    };

    let preach_state = move || {
        timers
            .get()
            .map(|t| format_timer_state(t.preach_timer.state))
            .unwrap_or("idle")
    };

    let preach_elapsed = move || {
        timers
            .get()
            .map(|t| format!("Elapsed {}", format_seconds(t.preach_timer.seconds_elapsed)))
            .unwrap_or_default()
    };

    view! {
        <section class="operator__panel operator__panel--timers" data-view-panel="timers">
            <div class="operator__timers" data-role="timer-cards">
                <article class="operator__timer-card" data-role="timer-countdown">
                    <header><strong>"Countdown"</strong></header>
                    <p class="operator__timer-primary" id="countdown-value">{countdown_value}</p>
                    <small id="countdown-target">{countdown_target_str}</small>
                </article>
                <article class="operator__timer-card" data-role="timer-preach">
                    <header>
                        <strong>"Preach"</strong>
                        <span class="operator__timer-state" id="preach-state">{preach_state}</span>
                    </header>
                    <p class="operator__timer-primary" id="preach-value">{preach_value}</p>
                    <small id="preach-elapsed">{preach_elapsed}</small>
                </article>
            </div>
            <div class="operator__timer-actions" data-role="timer-actions">
                <div class="operator__timer-group">
                    <h3>"Countdown"</h3>
                    <label class="operator__timer-field">
                        <span>"Service start"</span>
                        <input
                            type="text"
                            inputmode="numeric"
                            placeholder="18:00"
                            data-role="countdown-target-input"
                            aria-label="Countdown target time (HH:MM)"
                        />
                    </label>
                    <p class="operator__timer-help">
                        "Type HH:MM (or minutes only) and press Enter or Set to update while the timer runs."
                    </p>
                    <div class="operator__timer-buttons">
                        <button type="button" data-role="countdown-start"
                            on:click=move |_| {
                                leptos::task::spawn_local(async move {
                                    let _ = crate::api::timers::send_command("countdown", "start").await;
                                });
                            }
                        >"Start"</button>
                        <button type="button" data-role="countdown-offset-minus"
                            on:click=move |_| {
                                leptos::task::spawn_local(async move {
                                    let _ = crate::api::timers::send_command("countdown", "offset_minus_5").await;
                                });
                            }
                        >"-5 min"</button>
                        <button type="button" data-role="countdown-offset-plus"
                            on:click=move |_| {
                                leptos::task::spawn_local(async move {
                                    let _ = crate::api::timers::send_command("countdown", "offset_plus_5").await;
                                });
                            }
                        >"+5 min"</button>
                    </div>
                    <div class="operator__timer-links">
                        <button type="button" data-role="timer-overlay-open">"Open Countdown Overlay"</button>
                        <button type="button" data-role="timer-overlay-copy">"Copy Overlay URL"</button>
                    </div>
                </div>
                <div class="operator__timer-group">
                    <h3>"Preach"</h3>
                    <div class="operator__timer-buttons">
                        <button type="button" data-command="start_preach"
                            on:click=move |_| {
                                leptos::task::spawn_local(async move {
                                    let _ = crate::api::timers::send_command("preach", "start").await;
                                });
                            }
                        >"Start"</button>
                        <button type="button" data-command="reset_preach"
                            on:click=move |_| {
                                leptos::task::spawn_local(async move {
                                    let _ = crate::api::timers::send_command("preach", "reset").await;
                                });
                            }
                        >"Reset"</button>
                    </div>
                </div>
            </div>
        </section>
    }
}
