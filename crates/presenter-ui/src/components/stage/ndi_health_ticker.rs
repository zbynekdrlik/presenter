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
    mark_frames_live, record_presented_frame, refresh_frames_live_staleness, FrameStats,
    HealthSetter, StageSignalSetters,
};
use super::ndi_profile::maybe_profile_fallback;
use super::ndi_watchdog::{now_ms, ReloadEscalation, Watchdog};

/// Should this ticker's beacon push a health verdict (#532 review fix)? NO on
/// rVFC-less browsers: `approximate_frame_from_current_time` below counts AT
/// MOST ONE "frame" per 1s tick — a stall-detection proxy, never a real
/// frame-rate measurement — so over this ticker's ~15s beacon window
/// `presented_in_interval` caps at ~15, always computing ≈1 fps regardless of
/// how smoothly the TV is actually playing. Feeding that to `stage_health()`
/// would permanently show 🔴 "výpadky" on a perfectly healthy rVFC-less
/// display — exactly the untrustworthy readout this feature exists to
/// replace. `dropped_frames`/beacon telemetry are unaffected by this gate;
/// only the on-screen verdict is withheld (the readout falls back to the
/// latency figure alone, same as "no beacon yet"). Pure + host-unit-tested.
pub(crate) fn health_setter_for_ticker_beacon(
    rvfc_supported: bool,
    setter: &Option<HealthSetter>,
) -> Option<HealthSetter> {
    if rvfc_supported {
        setter.clone()
    } else {
        None
    }
}

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
    setters: &StageSignalSetters,
    on_failure: Rc<F>,
) -> i32 {
    let active = Rc::clone(active);
    let stats = Rc::clone(stats);
    let video = video.clone();
    let pc = pc.clone();
    let source_id = source_id.to_string();
    let escalation = Rc::clone(escalation);
    let clock_offset = Rc::clone(clock_offset);
    // #527: only the fields this ticker needs, plucked once out of the
    // shared bundle before they're moved into the `move` closure below.
    let frames_live_setter = setters.frames_live.clone();
    let dropped_frames_setter = setters.dropped_frames.clone();
    let stage_health_setter = setters.stage_health.clone();
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
        // starve it during normal playback. See
        // `health_setter_for_ticker_beacon` for why the health verdict is
        // withheld on rVFC-less browsers.
        maybe_post_beacon(
            &tick_count,
            &pc,
            &source_id,
            &stats,
            clock_offset.current(),
            dropped_frames_setter.clone(),
            health_setter_for_ticker_beacon(rvfc_supported, &stage_health_setter),
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

#[cfg(test)]
mod tests {
    use super::health_setter_for_ticker_beacon;
    use crate::components::stage::ndi_frame_stats::HealthSetter;
    use crate::state::stage::StageHealthReading;
    use std::rc::Rc;

    // ─────────────────────────────────────────────────────────────────────
    // #532 review fix: on rVFC-less browsers the ticker's OWN frame count is
    // a stall-detection proxy (≤1 "frame"/tick), never a real fps figure —
    // feeding it to the health verdict would permanently show a false 🔴 on
    // a perfectly smooth TV. The ticker beacon must withhold the health
    // setter entirely on that path.
    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn passes_the_setter_through_when_rvfc_is_supported() {
        let setter: HealthSetter = Rc::new(|_: Option<StageHealthReading>| {});
        assert!(health_setter_for_ticker_beacon(true, &Some(setter)).is_some());
    }

    #[test]
    fn withholds_the_setter_when_rvfc_is_unsupported() {
        let setter: HealthSetter = Rc::new(|_: Option<StageHealthReading>| {});
        assert!(health_setter_for_ticker_beacon(false, &Some(setter)).is_none());
    }

    #[test]
    fn stays_none_when_there_was_no_setter_to_begin_with() {
        assert!(health_setter_for_ticker_beacon(true, &None).is_none());
        assert!(health_setter_for_ticker_beacon(false, &None).is_none());
    }
}
