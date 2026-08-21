//! Countdown element for the stream output page (#709).
//!
//! Binds to Presenter's `countdown_to_start` timer (the only countdown in the
//! timer model — there is no id-addressable registry yet, so `timer_id` is
//! carried as data but currently forward-looking; see the ticket's
//! CONTRACT-ASSUMPTIONS). Formats the remaining time with the SHARED
//! `presenter_core::format_countdown` (the same formatter the stage timer uses)
//! and ticks smoothly BETWEEN server `Timers` pushes off the page's 250 ms
//! `now_ms` interval, anchored to the last server value at its receipt time.
//!
//! Visibility (AC "missing/inactive timer ⇒ render nothing"): renders empty when
//! there is no snapshot, when the countdown is `Idle` (the un-started default
//! placeholder), or when `format_countdown` returns "" (> 10 s past zero).

use leptos::prelude::*;
use presenter_core::{format_countdown, Frame, TextStyle, TimerState};

use super::style::{css_justify, frame_css, text_style_css};
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
    /// Forward-looking timer selector (see module docs); currently every
    /// countdown binds to `countdown_to_start`.
    timer_id: i64,
    style: TextStyle,
    frame: Frame,
    /// `z_order` mirrored to `z-index`.
    z: i32,
) -> impl IntoView {
    let ctx = use_context::<StreamContext>().expect("StreamContext not provided");
    let style_attr = container_style(&frame, &style, z);

    let text = move || {
        let Some(receipt) = ctx.timers.get() else {
            return String::new();
        };
        let received_at_ms = receipt.received_at_ms;
        let snap = receipt.overview.countdown_to_start;
        // Idle = the un-started default placeholder — treat as inactive.
        if snap.state == TimerState::Idle {
            return String::new();
        }
        // Interpolate the local tick only while RUNNING — a Paused/Completed
        // countdown holds the server's value (interpolating a paused timer would
        // drift it downward between server pushes).
        let remaining = if snap.state == TimerState::Running {
            let elapsed = ((ctx.now_ms.get() - received_at_ms) / 1000.0).floor();
            let elapsed = if elapsed < 0.0 { 0 } else { elapsed as i64 };
            snap.seconds_remaining - elapsed
        } else {
            snap.seconds_remaining
        };
        // `format_countdown` yields "" beyond 10 s past zero (cleared).
        format_countdown(remaining)
    };

    view! {
        <div
            class="stream-element stream-element--countdown"
            data-role="stream-element-countdown"
            data-element-id=id.to_string()
            data-timer-id=timer_id.to_string()
            style=style_attr
        >
            {text}
        </div>
    }
}
