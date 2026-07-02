//! The 1s health ticker: evaluates the frame-based health rules and paces the
//! tick-driven stats beacons.
//!
//! A `setInterval` 1s tick evaluates the stall / profile-fallback rules against
//! the rVFC frame counters and drives the last-resort page reload (#401). The
//! tick deliberately fires AFTER the `escalation.maybe_reload()` check and
//! BEFORE the `active` gate so the page-session reload keeps evaluating across
//! reconnect cycles. Split out of `ndi_watchdog.rs` to keep that file under the
//! size cap (#418).

use std::cell::Cell;
use std::rc::Rc;

use leptos::wasm_bindgen::{closure::Closure, JsCast};
use leptos::web_sys::{HtmlVideoElement, RtcPeerConnection};

use super::ndi_beacon::maybe_post_beacon;
use super::ndi_clock_offset::ClockOffsetEstimator;
use super::ndi_frame_stats::{
    mark_frames_live, record_presented_frame, refresh_frames_live_staleness, DroppedFramesSetter,
    FrameStats, FramesLiveSetter,
};
use super::ndi_profile::maybe_profile_fallback;
use super::ndi_watchdog::{now_ms, ReloadEscalation, Watchdog};

/// 1s interval driving (a) the beacon cadence and (b) evaluation of the
/// FRAME-BASED health rules:
///
/// - STALL: playback started (`frames_presented > 0`) AND no frame presented
///   for `STALL_NO_FRAME_MS` → a real freeze (render hiccups never span
///   10s) → reconnect.
/// - PROFILE FALLBACK: ICE connected AND zero frames presented for
///   `NO_DECODE_FALLBACK_MS` after connect (true no-decode) → switch the
///   profile mode (at most once per page load) and reconnect.
/// - No first frame yet otherwise: WAIT — a connected frameless consumer
///   must not reconnect (multi-consumer churn spiral, see Watchdog doc).
#[allow(clippy::too_many_arguments)]
pub(crate) fn start_health_ticker<F: Fn() + 'static>(
    video: &HtmlVideoElement,
    pc: &RtcPeerConnection,
    source_id: &str,
    active: &Rc<Cell<bool>>,
    stats: &Rc<FrameStats>,
    rvfc_supported: bool,
    escalation: &Rc<ReloadEscalation>,
    clock_offset: &Rc<ClockOffsetEstimator>,
    frames_live_setter: Option<FramesLiveSetter>,
    dropped_frames_setter: Option<DroppedFramesSetter>,
    on_failure: Rc<F>,
) -> i32 {
    let active = Rc::clone(active);
    let stats = Rc::clone(stats);
    let video = video.clone();
    let pc = pc.clone();
    let source_id = source_id.to_string();
    let escalation = Rc::clone(escalation);
    let clock_offset = Rc::clone(clock_offset);
    let tick_count = Cell::new(0u32);
    let last_current_time = Cell::new(0.0f64);
    let cb = Closure::<dyn FnMut()>::new(move || {
        // LAST-RESORT page reload (#401) — checked BEFORE the `active` gate so
        // it keeps evaluating across reconnect cycles (the page-session timer
        // is reset on every decoded frame; reaching the horizon means
        // reconnect+backoff has failed to produce a single frame for the whole
        // window). One-shot internally, so multiple tickers can't double-fire.
        // If it reloads, the page is tearing down — stop all further work.
        if escalation.maybe_reload() {
            return;
        }
        if !active.get() {
            return;
        }
        // Beacon first: the healthy-path early returns below must not
        // starve it during normal playback.
        maybe_post_beacon(
            &tick_count,
            &pc,
            &source_id,
            &stats,
            clock_offset.current(),
            dropped_frames_setter.clone(),
        );
        if !rvfc_supported
            && approximate_frame_from_current_time(&video, &stats, &last_current_time)
        {
            // The currentTime proxy is the rVFC-less browser's only frame
            // signal — reset the page-level reload timer ONLY when it actually
            // advanced (rVFC path resets in its own callback).
            escalation.note_decoded_frame();
            // #500: proxy frame advanced → frames are live (drop the cover) on
            // rVFC-less browsers too. Idempotent (transition-guarded).
            mark_frames_live(&stats, &frames_live_setter);
        }
        // #500: restore the neutral cover when frames have gone stale — runs on
        // BOTH rVFC and proxy browsers (the rVFC path only marks live; this is
        // the single place that marks NOT-live). Transition-guarded.
        refresh_frames_live_staleness(&stats, &frames_live_setter);
        let now = now_ms();
        let frames = stats.frames_presented.get();
        if frames == 0 {
            // Pre-first-frame: only the bounded profile fallback may act.
            maybe_profile_fallback(now, &stats, &pc, &active, &on_failure);
            return;
        }
        let since_last_frame = now - stats.last_frame_at.get();
        if since_last_frame > Watchdog::STALL_NO_FRAME_MS {
            leptos::logging::warn!(
                "watchdog: no frame presented for {since_last_frame:.0}ms (frames_presented={frames}) — real freeze, reconnecting"
            );
            active.set(false);
            (on_failure)();
        }
    });
    let handle = leptos::web_sys::window()
        .and_then(|window| {
            window
                .set_interval_with_callback_and_timeout_and_arguments_0(
                    cb.as_ref().unchecked_ref(),
                    Watchdog::TICK_INTERVAL_MS,
                )
                .ok()
        })
        .unwrap_or(-1);
    cb.forget();
    handle
}

/// rVFC-less fallback (non-Chromium browsers): treat currentTime advancing
/// between ticks as one presented frame. Coarse (≤1 "frame" per tick) but
/// keeps the stall and no-decode rules functional with identical semantics.
/// Returns true iff a new frame was recorded this tick (currentTime advanced)
/// — the caller uses this to reset the page-level reload timer ONLY on real
/// advance, never on a stalled tick.
fn approximate_frame_from_current_time(
    video: &HtmlVideoElement,
    stats: &FrameStats,
    last_current_time: &Cell<f64>,
) -> bool {
    let t = video.current_time();
    if t > 0.0 && (t - last_current_time.get()).abs() > 0.001 {
        last_current_time.set(t);
        record_presented_frame(stats);
        return true;
    }
    false
}
