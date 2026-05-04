use crate::state::operator::OperatorState;
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
    let ctx = use_ctx!(AppContext);
    let op = use_ctx!(OperatorState);

    let send_timer_cmd = move |cmd: TimerCommand| {
        let timers = ctx.timers;
        leptos::task::spawn_local(async move {
            if let Ok(overview) = crate::api::timers::send_command(&cmd).await {
                timers.set(Some(overview));
            }
        });
    };

    // Focus/blur tracking for countdown input
    let on_countdown_focus = {
        let countdown_input_active = op.countdown_input_active;
        move |_| {
            countdown_input_active.set(true);
        }
    };

    let on_countdown_blur = {
        let countdown_input_active = op.countdown_input_active;
        let countdown_input_dirty = op.countdown_input_dirty;
        move |ev: web_sys::FocusEvent| {
            countdown_input_active.set(false);
            // If dirty, save on blur
            if countdown_input_dirty.get_untracked() {
                if let Some(input) = ev.target().and_then(|t| {
                    use wasm_bindgen::JsCast;
                    t.dyn_into::<web_sys::HtmlInputElement>().ok()
                }) {
                    let val = input.value();
                    if let Some((hours, minutes)) = parse_time_input(&val) {
                        send_timer_cmd(TimerCommand::SetCountdownTargetLocal { hours, minutes });
                    }
                }
                countdown_input_dirty.set(false);
            }
        }
    };

    let on_countdown_input = {
        let countdown_input_dirty = op.countdown_input_dirty;
        move |_| {
            countdown_input_dirty.set(true);
        }
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
            if let Some((hours, minutes)) = parse_time_input(&val) {
                send_timer_cmd(TimerCommand::SetCountdownTargetLocal { hours, minutes });
                op.countdown_input_dirty.set(false);
            }
        }
    };

    let on_offset_minus = move |_| {
        send_timer_cmd(TimerCommand::AdjustCountdownTarget { offset_minutes: -5 });
    };

    let on_offset_plus = move |_| {
        send_timer_cmd(TimerCommand::AdjustCountdownTarget { offset_minutes: 5 });
    };

    let on_preach_start = move |_| {
        send_timer_cmd(TimerCommand::StartPreach);
    };

    let on_preach_pause = move |_| {
        send_timer_cmd(TimerCommand::PausePreach);
    };

    let on_preach_reset = move |_| {
        send_timer_cmd(TimerCommand::ResetPreach);
    };

    let on_preach_limit_focus = {
        let active = op.preach_limit_input_active;
        move |_| {
            active.set(true);
        }
    };

    let on_preach_limit_blur = {
        let active = op.preach_limit_input_active;
        let dirty = op.preach_limit_input_dirty;
        move |ev: web_sys::FocusEvent| {
            active.set(false);
            if dirty.get_untracked() {
                if let Some(input) = ev.target().and_then(|t| {
                    use wasm_bindgen::JsCast;
                    t.dyn_into::<web_sys::HtmlInputElement>().ok()
                }) {
                    let val = input.value();
                    if let Some(seconds) = parse_limit_input(&val) {
                        send_timer_cmd(TimerCommand::SetPreachLimit { seconds });
                    }
                }
                dirty.set(false);
            }
        }
    };

    let on_preach_limit_input = {
        let dirty = op.preach_limit_input_dirty;
        move |_| {
            dirty.set(true);
        }
    };

    let on_preach_limit_keydown = move |ev: web_sys::KeyboardEvent| {
        if ev.key() != "Enter" {
            return;
        }
        let input = ev.target().and_then(|t| {
            use wasm_bindgen::JsCast;
            t.dyn_into::<web_sys::HtmlInputElement>().ok()
        });
        if let Some(el) = input {
            let val = el.value();
            if let Some(seconds) = parse_limit_input(&val) {
                send_timer_cmd(TimerCommand::SetPreachLimit { seconds });
                op.preach_limit_input_dirty.set(false);
            }
        }
    };

    let on_preach_limit_clear = move |_| {
        send_timer_cmd(TimerCommand::ClearPreachLimit);
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
        let copied = copy_to_clipboard(&url);
        if copied {
            toast_variant.set("success".to_string());
            toast_message.set(Some("Timer overlay URL copied".to_string()));
        } else {
            toast_variant.set("error".to_string());
            toast_message.set(Some(format!("Could not copy. URL: {url}")));
        }
    };

    view! {
        <div class="operator__timers" data-role="timer-cards">
            <article class="operator__timer-card" data-role="timer-countdown">
                <header>
                    <strong>"Countdown"</strong>
                </header>
                <p class="operator__timer-primary" id="countdown-value">
                    {move || {
                        ctx.timers.get()
                            .map(|t| presenter_core::format_countdown(t.countdown_to_start.seconds_remaining))
                            .unwrap_or_else(|| "0:00".to_string())
                    }}
                </p>
                <small id="countdown-target">
                    {move || {
                        ctx.timers.get()
                            .map(|t| format!("Target {}", t.countdown_to_start.target_local))
                            .unwrap_or_default()
                    }}
                </small>
            </article>

            <article class="operator__timer-card" data-role="timer-preach">
                <header>
                    <strong>"Preach"</strong>
                    <span class="operator__timer-state" id="preach-state">
                        {move || {
                            ctx.timers.get()
                                .map(|t| format!("{:?}", t.preach_timer.state))
                                .unwrap_or_else(|| "Idle".to_string())
                        }}
                    </span>
                </header>
                <p class="operator__timer-primary" id="preach-value">
                    {move || {
                        ctx.timers.get()
                            .map(|t| format_seconds(t.preach_timer.seconds_elapsed))
                            .unwrap_or_else(|| "0:00".to_string())
                    }}
                </p>
                <small id="preach-limit">
                    {move || {
                        ctx.timers.get()
                            .and_then(|t| t.preach_timer.limit_seconds)
                            .map(|s| format!("Limit: {}", format_seconds(s as i64)))
                            .unwrap_or_else(|| "No limit".to_string())
                    }}
                </small>
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
                        on:focus=on_countdown_focus
                        on:blur=on_countdown_blur
                        on:input=on_countdown_input
                        on:keydown=on_countdown_target
                    />
                </label>
                <p class="operator__timer-help">
                    "Type the service start time and press Enter. Examples: "
                    <code>"18"</code>" → 18:00, "
                    <code>"830"</code>" → 8:30, "
                    <code>"1915"</code>" → 19:15, "
                    <code>"18:30"</code>" → 18:30. Setting a target starts the countdown automatically."
                </p>
                <div class="operator__timer-buttons">
                    <button type="button" data-role="countdown-offset-minus" on:click=on_offset_minus>"-5 min"</button>
                    <button type="button" data-role="countdown-offset-plus" on:click=on_offset_plus>"+5 min"</button>
                </div>
                <div class="operator__timer-links">
                    <button type="button" data-role="timer-overlay-open" on:click=on_overlay_open>"Open Countdown Overlay"</button>
                    <button type="button" data-role="timer-overlay-copy" on:click=on_overlay_copy>"Copy Overlay URL"</button>
                </div>
            </div>
            <div class="operator__timer-group">
                <h3>"Preach"</h3>
                <label class="operator__timer-field">
                    <span>"Preach limit"</span>
                    <input
                        type="text"
                        inputmode="numeric"
                        placeholder="5"
                        data-role="preach-limit-input"
                        aria-label="Preach limit (HH:MM or minutes)"
                        on:focus=on_preach_limit_focus
                        on:blur=on_preach_limit_blur
                        on:input=on_preach_limit_input
                        on:keydown=on_preach_limit_keydown
                    />
                </label>
                <p class="operator__timer-help">
                    "Type minutes (or HH:MM) and press Enter. "
                    <button type="button" data-role="preach-limit-clear" on:click=on_preach_limit_clear>
                        "Clear limit"
                    </button>
                </p>
                <div class="operator__timer-buttons">
                    <button type="button" data-command="start_preach" on:click=on_preach_start>"Start"</button>
                    <button type="button" data-command="pause_preach" on:click=on_preach_pause>"Pause"</button>
                    <button type="button" data-command="reset_preach" on:click=on_preach_reset>"Reset"</button>
                </div>
            </div>
        </div>
    }
}

/// Copy `text` to the system clipboard. Returns `true` on success.
///
/// Uses the legacy `document.execCommand('copy')` path because
/// `navigator.clipboard.writeText()` is only available in secure
/// contexts (HTTPS or localhost). The operator UI is served over plain
/// HTTP on the LAN, so the modern API is `undefined` there.
///
/// `execCommand` is deprecated in web-sys (gated behind an unstable
/// feature flag), so we call it dynamically through js_sys::Reflect.
fn copy_to_clipboard(text: &str) -> bool {
    use js_sys::{Function, Reflect};
    use wasm_bindgen::{JsCast, JsValue};

    let Some(document) = crate::utils::window::window().document() else {
        return false;
    };
    let Ok(element) = document.create_element("textarea") else {
        return false;
    };
    let Ok(textarea) = element.dyn_into::<web_sys::HtmlTextAreaElement>() else {
        return false;
    };
    textarea.set_value(text);
    // Position off-screen so the page doesn't scroll or flash visibly.
    let _ = textarea.set_attribute(
        "style",
        "position:fixed;left:-9999px;top:0;opacity:0;pointer-events:none;",
    );
    let _ = textarea.set_attribute("readonly", "");
    let Some(body) = document.body() else {
        return false;
    };
    if body.append_child(&textarea).is_err() {
        return false;
    }
    let _ = textarea.focus();
    textarea.select();

    // document.execCommand("copy") via reflection (web-sys gates the
    // direct binding behind an unstable_apis feature flag).
    let copied = (|| -> Option<bool> {
        let exec_command = Reflect::get(&document, &"execCommand".into()).ok()?;
        let func = exec_command.dyn_into::<Function>().ok()?;
        let result = func.call1(&document, &JsValue::from_str("copy")).ok()?;
        result.as_bool()
    })()
    .unwrap_or(false);

    let _ = body.remove_child(&textarea);
    copied
}

/// Parse a time-of-day input written by the operator.
///
/// Accepted forms (alarm-clock convention):
/// - `"18"`        → 18:00 (1–2 digits = hour-of-day, minutes = 0)
/// - `"8"`         → 08:00
/// - `"830"`       → 08:30 (3 digits = H:MM)
/// - `"1915"`      → 19:15 (4 digits = HH:MM)
/// - `"18:30"`     → 18:30 (with colon)
/// - `"8:05"`      → 08:05
///
/// Returns `None` for invalid input. Hours must be 0–23, minutes 0–59.
fn parse_time_input(input: &str) -> Option<(u32, u32)> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Form with explicit colon: "H:MM" or "HH:MM"
    if let Some((h_str, m_str)) = trimmed.split_once(':') {
        let h: u32 = h_str.trim().parse().ok()?;
        let m: u32 = m_str.trim().parse().ok()?;
        if h > 23 || m > 59 {
            return None;
        }
        return Some((h, m));
    }

    // Compact digit-only form: "H", "HH", "HMM", "HHMM"
    if !trimmed.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let (h, m) = match trimmed.len() {
        1 | 2 => (trimmed.parse::<u32>().ok()?, 0),
        3 => (
            trimmed[..1].parse::<u32>().ok()?,
            trimmed[1..].parse::<u32>().ok()?,
        ),
        4 => (
            trimmed[..2].parse::<u32>().ok()?,
            trimmed[2..].parse::<u32>().ok()?,
        ),
        _ => return None,
    };
    if h > 23 || m > 59 {
        return None;
    }
    Some((h, m))
}

/// Parse a duration input written by the operator for the preach limit.
///
/// Accepted forms:
/// - `"5"`     → 300 seconds (bare number = minutes)
/// - `"45"`    → 2700 seconds
/// - `"1:30"`  → 5400 seconds (hours:minutes)
/// - `"1:00"`  → 3600 seconds
///
/// Note: single numbers mean MINUTES here (not hour-of-day like
/// `parse_time_input`), because preach-limit is a duration, not a
/// wall-clock time.
///
/// Returns `None` for invalid input. Minutes must be 0–59.
fn parse_limit_input(input: &str) -> Option<u64> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some((h_str, m_str)) = trimmed.split_once(':') {
        let h: u64 = h_str.trim().parse().ok()?;
        let m: u64 = m_str.trim().parse().ok()?;
        if m > 59 {
            return None;
        }
        return Some(h * 3600 + m * 60);
    }

    // Bare number = minutes.
    let mins: u64 = trimmed.parse().ok()?;
    Some(mins * 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_time_input_single_digit_hour() {
        assert_eq!(parse_time_input("8"), Some((8, 0)));
        assert_eq!(parse_time_input("0"), Some((0, 0)));
    }

    #[test]
    fn parse_time_input_two_digit_hour() {
        assert_eq!(parse_time_input("18"), Some((18, 0)));
        assert_eq!(parse_time_input("23"), Some((23, 0)));
        assert_eq!(parse_time_input("08"), Some((8, 0)));
    }

    #[test]
    fn parse_time_input_three_digit_compact() {
        // 3-digit form: H + MM
        assert_eq!(parse_time_input("830"), Some((8, 30)));
        assert_eq!(parse_time_input("905"), Some((9, 5)));
        assert_eq!(parse_time_input("100"), Some((1, 0)));
    }

    #[test]
    fn parse_time_input_four_digit_compact() {
        // 4-digit form: HH + MM
        assert_eq!(parse_time_input("1915"), Some((19, 15)));
        assert_eq!(parse_time_input("1830"), Some((18, 30)));
        assert_eq!(parse_time_input("0800"), Some((8, 0)));
        assert_eq!(parse_time_input("2359"), Some((23, 59)));
    }

    #[test]
    fn parse_time_input_with_colon() {
        assert_eq!(parse_time_input("18:30"), Some((18, 30)));
        assert_eq!(parse_time_input("8:05"), Some((8, 5)));
        assert_eq!(parse_time_input("0:00"), Some((0, 0)));
    }

    #[test]
    fn parse_time_input_rejects_invalid() {
        assert_eq!(parse_time_input(""), None);
        assert_eq!(parse_time_input("   "), None);
        assert_eq!(parse_time_input("abc"), None);
        assert_eq!(parse_time_input("25"), None); // hour > 23
        assert_eq!(parse_time_input("24:00"), None);
        assert_eq!(parse_time_input("1860"), None); // 18:60 invalid minutes
        assert_eq!(parse_time_input("12:60"), None);
        assert_eq!(parse_time_input("99999"), None); // 5 digits
        assert_eq!(parse_time_input("18:"), None);
        assert_eq!(parse_time_input("18:ab"), None);
    }

    #[test]
    fn parse_time_input_trims_whitespace() {
        assert_eq!(parse_time_input("  18  "), Some((18, 0)));
        assert_eq!(parse_time_input(" 1915 "), Some((19, 15)));
    }

    #[test]
    fn parse_limit_input_bare_number_is_minutes() {
        assert_eq!(parse_limit_input("5"), Some(300));
        assert_eq!(parse_limit_input("45"), Some(2700));
        assert_eq!(parse_limit_input("0"), Some(0));
        // Not capped at 59 because bare number is minutes, not minutes-of-hour.
        assert_eq!(parse_limit_input("90"), Some(5400));
    }

    #[test]
    fn parse_limit_input_with_colon_is_hours_minutes() {
        assert_eq!(parse_limit_input("1:30"), Some(5400));
        assert_eq!(parse_limit_input("0:45"), Some(2700));
        assert_eq!(parse_limit_input("2:00"), Some(7200));
    }

    #[test]
    fn parse_limit_input_rejects_invalid() {
        assert_eq!(parse_limit_input(""), None);
        assert_eq!(parse_limit_input("   "), None);
        assert_eq!(parse_limit_input("abc"), None);
        assert_eq!(parse_limit_input("1:60"), None); // minutes > 59
        assert_eq!(parse_limit_input("1:ab"), None);
        assert_eq!(parse_limit_input("1:"), None);
    }

    #[test]
    fn parse_limit_input_trims_whitespace() {
        assert_eq!(parse_limit_input("  5  "), Some(300));
        assert_eq!(parse_limit_input(" 1:30 "), Some(5400));
    }
}
