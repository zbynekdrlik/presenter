//! Countdown element for the stream output page (#709; timer selection + box #785).
//!
//! Binds to one of Presenter's two timers, chosen by `timer_id` (#785): `1` =
//! `countdown_to_start` (a count-DOWN to a target), `2` = `preach_timer` (the
//! count-UP "časomiera kázne"); any other id renders nothing. The chosen value
//! is formatted with the SHARED core formatters (`format_countdown` for the
//! count-down, `format_elapsed` for the count-up — the same ones the stage
//! surfaces use) and ticks smoothly BETWEEN server `Timers` pushes off the
//! page's 250 ms `now_ms` interval, anchored to the last server value at its
//! receipt time.
//!
//! Visibility (AC "missing/inactive timer ⇒ render nothing"): renders empty when
//! there is no snapshot, when the chosen timer is `Idle` (the un-started default
//! placeholder), when `timer_id` is unknown, or when `format_countdown` returns
//! "" (> 10 s past zero).
//!
//! An optional background box (#785) draws a translucent card behind the text
//! (`TextBox`) so the timer reads over a busy stream; when absent the text node
//! is rendered directly (preserving the #709 DOM the E2E asserts on).

use leptos::prelude::*;
use presenter_core::{format_countdown, format_elapsed, Frame, TextBox, TextStyle, TimerState};

use super::style::{css_justify, frame_css, text_box_css, text_style_css};
use super::StreamContext;

/// Build the (static) container style from the element's `Frame` + `TextStyle`.
/// The countdown IS its own flex line box (single text line), so the frame,
/// flex centering, and typography all live on one element.
fn container_style(frame: &Frame, style: &TextStyle, z: i32) -> String {
    format!(
        "{}display:flex;align-items:center;justify-content:{};{}",
        frame_css(frame, z),
        css_justify(style.align),
        text_style_css(style),
    )
}

#[component]
pub fn ElementCountdown(
    /// `stream_elements.id` — for E2E targeting + stable DOM identity.
    id: i64,
    /// Timer selector (#785): 1 = `countdown_to_start`, 2 = `preach_timer`; any
    /// other id renders nothing.
    timer_id: i64,
    style: TextStyle,
    frame: Frame,
    /// Optional background box behind the text (#785). `None` = no box.
    text_box: Option<TextBox>,
    /// `z_order` mirrored to `z-index`.
    z: i32,
) -> impl IntoView {
    let ctx = use_context::<StreamContext>().expect("StreamContext not provided");
    let style_attr = container_style(&frame, &style, z);

    // A `Memo` so the text node updates only on the per-second value change, not
    // on every 250 ms `now_ms` tick that re-derives the same MM:SS string.
    let text = Memo::new(move |_| {
        let Some(receipt) = ctx.timers.get() else {
            return String::new();
        };
        let received_at_ms = receipt.received_at_ms;
        let overview = receipt.overview;
        // Smoothly-interpolated whole seconds elapsed since the server push (only
        // while a timer is RUNNING — a Paused/Completed timer holds its value).
        let tick = || {
            let elapsed = ((ctx.now_ms.get() - received_at_ms) / 1000.0).floor();
            if elapsed < 0.0 {
                0
            } else {
                elapsed as i64
            }
        };
        match timer_id {
            1 => {
                let snap = overview.countdown_to_start;
                // Idle = the un-started default placeholder — treat as inactive.
                if snap.state == TimerState::Idle {
                    return String::new();
                }
                let remaining = if snap.state == TimerState::Running {
                    snap.seconds_remaining - tick()
                } else {
                    snap.seconds_remaining
                };
                // `format_countdown` yields "" beyond 10 s past zero (cleared).
                format_countdown(remaining)
            }
            2 => {
                let snap = overview.preach_timer;
                if snap.state == TimerState::Idle {
                    return String::new();
                }
                let elapsed = if snap.state == TimerState::Running {
                    snap.seconds_elapsed + tick()
                } else {
                    snap.seconds_elapsed
                };
                format_elapsed(elapsed)
            }
            // Unknown timer id ⇒ render nothing (forward-compatible, per #785).
            _ => String::new(),
        }
    });

    // A countdown MUST swap digits with NO transition — a hard cut per tick (the
    // owner's report #776: "nema tam byt ziaden prechod zatial"). So it renders
    // ONE stable text node (never `CrossfadeText`): a tick is an in-place text
    // update of the SAME `<span>`, never a re-mount and never a fade. The
    // `content_transition` field stays on `StreamElementProps::Countdown` (no
    // wire/API/schema change) but is deliberately ignored for the countdown kind;
    // the editor hides the content-transition control for it. Scene-level
    // (base/overlay) transitions are unaffected. `font-variant-numeric:
    // tabular-nums` (in `stream_output.css`) keeps the width from jittering
    // between glyphs.
    let content = view! {
        <span data-role="stream-countdown-content">{move || text.get()}</span>
    };

    // Optional background box (#785): wrap the text in a translucent card. When
    // absent the span is rendered directly, preserving the #709 DOM. `text_box`
    // is fixed per element mount (a draft change remounts via `scene_render`'s
    // per-element `Memo`), so no reactivity is needed here.
    let body = match text_box {
        Some(text_box) => view! {
            <div
                class="stream-countdown-box"
                data-role="stream-countdown-box"
                style=text_box_css(&text_box)
            >
                {content}
            </div>
        }
        .into_any(),
        None => content.into_any(),
    };

    view! {
        <div
            class="stream-element stream-element--countdown"
            data-role="stream-element-countdown"
            data-element-id=id.to_string()
            data-timer-id=timer_id.to_string()
            style=style_attr
        >
            {body}
        </div>
    }
}
