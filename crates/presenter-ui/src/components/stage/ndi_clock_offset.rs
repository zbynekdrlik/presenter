//! Browser↔server pipeline-clock offset estimator (#510, T3 of the NDI
//! true-latency rework —
//! `docs/superpowers/specs/2026-06-30-ndi-true-latency-design.md` §3).
//!
//! Periodic NTP-style round trip against `/ndi/time` (the server's
//! GStreamer pipeline-clock time — the SAME clock domain the RTCP Sender
//! Reports encode), so a later ticket (#512, T4) can convert a getStats
//! `report.timestamp` reading into that domain and subtract
//! `estimatedPlayoutTimestamp` directly, without needing an absolute
//! wall-clock sync (dantesync) on the critical path.
//!
//! Driven by the rVFC callback, NOT `setInterval` — TV WebViews throttle
//! background/idle timers (the codebase already abandoned `setInterval` for
//! this reason in `ndi_frame_stats.rs`). `t0`/`t2` use `now_ms()`
//! (`performance.now()`), the SAME domain WebRTC's `RTCStats.timestamp`
//! (read as `report.timestamp` in `ndi_beacon.rs`) lives in — so the offset
//! this estimator produces is directly addable to a `report.timestamp`
//! reading.
//!
//! Split into its own module (not folded into `ndi_frame_stats.rs`, already
//! 570 lines) per the design's quality-gate note.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use leptos::wasm_bindgen::{closure::Closure, JsCast, JsValue};
use leptos::web_sys::{HtmlVideoElement, Response};
use wasm_bindgen_futures::{spawn_local, JsFuture};

use super::ndi_frame_stats::{
    schedule_video_frame_callback, video_supports_rvfc, SharedRvfcClosure,
};
use super::ndi_watchdog::now_ms;

/// How often to attempt a fresh NTP-style round trip (ms). Evaluated on the
/// rVFC tick, so actual cadence drifts a little with the presented fps —
/// close enough for a metric measured in single-digit-to-tens of ms.
const HANDSHAKE_PERIOD_MS: f64 = 2000.0;

/// Reject a round-trip sample whose RTT exceeds this. A queued/slow round
/// trip biases the offset by up to `RTT-asymmetry / 2` (design §3) — a
/// multi-second RTT is not a network we should trust for a ms-scale offset.
const MAX_ACCEPTABLE_RTT_MS: f64 = 1000.0;

/// Bounded sample history — old samples are stale anyway once a fresher,
/// lower-RTT one has landed.
const MAX_SAMPLES: usize = 8;

/// An offset estimate is STALE (report `n/a` via `current()`) once this long
/// has passed since the last ACCEPTED sample — a confidently-wrong number
/// during a WAN/Tailscale hiccup is worse than no number (design §3 trust
/// predicate).
const STALE_AFTER_MS: f64 = 15_000.0;

/// Callback that publishes the current `(offset_ms, rtt_ms)` estimate (or
/// `None` when no fresh sample exists) to a shared signal. Boxed behind `Rc`
/// so it threads cheaply from `NdiVideo` through `Watchdog::install`, mirroring
/// `VideoLatencySetter` / `FramesLiveSetter` in `ndi_frame_stats.rs`.
pub(crate) type ClockOffsetSetter = Rc<dyn Fn(Option<(f64, f64)>)>;

/// One NTP-style round-trip sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct OffsetSample {
    /// `server_time_ms − midpoint(t0, t2)` — the browser→server-pipeline-clock
    /// offset this sample implies.
    pub(crate) offset_ms: f64,
    /// `t2 − t0` — total round-trip time for this sample.
    pub(crate) rtt_ms: f64,
    /// `now_ms()` this sample was accepted (== `t2`), for staleness/age-out.
    pub(crate) at_ms: f64,
}

/// Compute one NTP-style sample from a round trip: `t0_ms` = browser clock
/// just before sending, `server_ms` = the server's pipeline-clock time
/// returned by `/ndi/time`, `t2_ms` = browser clock just after the response
/// arrived. Assumes symmetric transit (the classic SNTP simplification) —
/// the design's §3 documents the resulting `RTT-asymmetry/2` bias bound.
pub(crate) fn compute_sample(t0_ms: f64, server_ms: f64, t2_ms: f64) -> OffsetSample {
    OffsetSample {
        offset_ms: server_ms - (t0_ms + t2_ms) / 2.0,
        rtt_ms: t2_ms - t0_ms,
        at_ms: t2_ms,
    }
}

/// Select the best offset from a set of samples: prefer the LOWEST-RTT
/// samples (least-queued round trip ≈ most symmetric, design §3), then take
/// the MEDIAN of their offsets to remove jitter within that low-RTT set.
/// Returns `None` when `samples` is empty or every sample exceeds
/// `MAX_ACCEPTABLE_RTT_MS` (reject high-RTT — a degraded link must not
/// produce a confident-looking number).
pub(crate) fn select_offset(samples: &[OffsetSample]) -> Option<f64> {
    let mut usable: Vec<&OffsetSample> = samples
        .iter()
        .filter(|s| s.rtt_ms.is_finite() && s.rtt_ms >= 0.0 && s.rtt_ms <= MAX_ACCEPTABLE_RTT_MS)
        .collect();
    if usable.is_empty() {
        return None;
    }
    usable.sort_by(|a, b| a.rtt_ms.total_cmp(&b.rtt_ms));
    let min_rtt = usable[0].rtt_ms;
    // "low-RTT samples" = within 2x the best RTT seen this window — tight
    // enough to reject a queued outlier, loose enough that a healthy LAN
    // link (where every sample is already near-minimal) keeps most samples.
    let low_rtt_cutoff = (min_rtt * 2.0).max(min_rtt + 1.0);
    let mut low_rtt: Vec<f64> = usable
        .iter()
        .filter(|s| s.rtt_ms <= low_rtt_cutoff)
        .map(|s| s.offset_ms)
        .collect();
    low_rtt.sort_by(f64::total_cmp);
    let mid = low_rtt.len() / 2;
    Some(if low_rtt.len() % 2 == 0 {
        (low_rtt[mid - 1] + low_rtt[mid]) / 2.0
    } else {
        low_rtt[mid]
    })
}

/// Is the most recent accepted sample too old to trust?
pub(crate) fn is_stale(last_sample_at_ms: f64, now_ms: f64) -> bool {
    now_ms - last_sample_at_ms > STALE_AFTER_MS
}

/// Shared, `Rc`-cloneable estimator state. One instance per active NDI WHEP
/// connection, owned alongside `FrameStats` in `Watchdog::install`.
pub(crate) struct ClockOffsetEstimator {
    samples: RefCell<Vec<OffsetSample>>,
    last_attempt_at_ms: Cell<f64>,
    in_flight: Cell<bool>,
}

impl ClockOffsetEstimator {
    pub(crate) fn new() -> Rc<Self> {
        Rc::new(Self {
            samples: RefCell::new(Vec::with_capacity(MAX_SAMPLES)),
            last_attempt_at_ms: Cell::new(f64::NEG_INFINITY),
            in_flight: Cell::new(false),
        })
    }

    /// Current best `(offset_ms, rtt_ms)` estimate at `now_ms`, or `None`
    /// when no sample has ever landed OR the freshest one has aged out.
    /// Takes `now_ms` explicitly (rather than reading the DOM clock itself)
    /// so this stays a pure, directly unit-testable function.
    pub(crate) fn current_at(&self, now_ms: f64) -> Option<(f64, f64)> {
        let samples = self.samples.borrow();
        let newest_at = samples.last()?.at_ms;
        if is_stale(newest_at, now_ms) {
            return None;
        }
        let offset = select_offset(&samples)?;
        // The reported RTT is the best (lowest) sample's own RTT — the most
        // representative "how good is this measurement" figure.
        let min_rtt = samples
            .iter()
            .map(|s| s.rtt_ms)
            .fold(f64::INFINITY, f64::min);
        Some((offset, min_rtt))
    }

    /// `current_at(now_ms())` — the DOM-clock-reading convenience wrapper
    /// used by the live handshake loop.
    pub(crate) fn current(&self) -> Option<(f64, f64)> {
        self.current_at(now_ms())
    }

    fn push_sample(&self, sample: OffsetSample) {
        let mut samples = self.samples.borrow_mut();
        if samples.len() == MAX_SAMPLES {
            samples.remove(0);
        }
        samples.push(sample);
    }
}

/// One NTP-style round trip against `/ndi/time`. Fire-and-forget on failure —
/// a dropped fetch just means no new sample this round; the estimator will
/// retry on the next due handshake tick, and `current()` reports `n/a` once
/// the existing samples age out.
async fn round_trip(estimator: &Rc<ClockOffsetEstimator>) {
    let t0 = now_ms();
    let Some(window) = leptos::web_sys::window() else {
        return;
    };
    let Ok(resp_value) = JsFuture::from(window.fetch_with_str("/ndi/time")).await else {
        return;
    };
    let t2 = now_ms();
    let Ok(response) = resp_value.dyn_into::<Response>() else {
        return;
    };
    if !response.ok() {
        return;
    }
    let Ok(json_promise) = response.json() else {
        return;
    };
    let Ok(json) = JsFuture::from(json_promise).await else {
        return;
    };
    let server_ms = js_sys::Reflect::get(&json, &JsValue::from_str("serverTimeMs"))
        .ok()
        .and_then(|v| v.as_f64());
    let Some(server_ms) = server_ms else {
        return;
    };
    estimator.push_sample(compute_sample(t0, server_ms, t2));
}

/// If a handshake is due and none is already in flight, fire one — and once
/// it completes (success OR failure), publish the resulting `current()`
/// estimate to `setter` so a run of failures eventually ages the reading out
/// to `n/a` rather than leaving a stale number displayed forever.
fn maybe_fire_handshake(estimator: &Rc<ClockOffsetEstimator>, setter: Option<ClockOffsetSetter>) {
    if estimator.in_flight.get() {
        return;
    }
    let now = now_ms();
    if now - estimator.last_attempt_at_ms.get() < HANDSHAKE_PERIOD_MS {
        return;
    }
    estimator.last_attempt_at_ms.set(now);
    estimator.in_flight.set(true);
    let estimator = Rc::clone(estimator);
    spawn_local(async move {
        round_trip(&estimator).await;
        estimator.in_flight.set(false);
        if let Some(setter) = setter {
            setter(estimator.current());
        }
    });
}

/// Start a self-rescheduling rVFC-driven handshake loop on `video`. Ticks
/// once per PRESENTED frame (never throttled like `setInterval` on TV
/// WebViews — design §3) but only actually fires a round trip once per
/// `HANDSHAKE_PERIOD_MS`; every other tick is a cheap no-op check.
///
/// Returns `false` when the browser lacks rVFC — same fallback boundary as
/// `start_rvfc_frame_observer`; the estimator simply never runs there and
/// `current()` stays `None` (`n/a`), which is honest per the trust predicate.
pub(crate) fn start(
    video: &HtmlVideoElement,
    active: &Rc<Cell<bool>>,
    estimator: &Rc<ClockOffsetEstimator>,
    setter: Option<ClockOffsetSetter>,
) -> bool {
    if !video_supports_rvfc(video) {
        return false;
    }

    let holder: SharedRvfcClosure = Rc::new(RefCell::new(None));
    let cb = {
        let active = Rc::clone(active);
        let estimator = Rc::clone(estimator);
        let video = video.clone();
        let holder = Rc::clone(&holder);
        Closure::<dyn FnMut(JsValue, JsValue)>::new(move |_now: JsValue, _meta: JsValue| {
            if !active.get() {
                return;
            }
            maybe_fire_handshake(&estimator, setter.clone());
            schedule_video_frame_callback(&video, &holder);
        })
    };
    *holder.borrow_mut() = Some(cb);
    schedule_video_frame_callback(video, &holder);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_sample_symmetric_round_trip() {
        // t0=1000, server=1050, t2=1010 -> offset = 1050 - (1000+1010)/2 = 45; rtt=10.
        let s = compute_sample(1000.0, 1050.0, 1010.0);
        assert_eq!(s.rtt_ms, 10.0);
        assert!((s.offset_ms - 45.0).abs() < 1e-9);
        assert_eq!(s.at_ms, 1010.0);
    }

    #[test]
    fn select_offset_empty_is_none() {
        assert_eq!(select_offset(&[]), None);
    }

    #[test]
    fn select_offset_rejects_all_high_rtt() {
        let samples = [OffsetSample {
            offset_ms: 10.0,
            rtt_ms: 5000.0,
            at_ms: 0.0,
        }];
        assert_eq!(
            select_offset(&samples),
            None,
            "every sample exceeds MAX_ACCEPTABLE_RTT_MS and must be rejected"
        );
    }

    #[test]
    fn select_offset_even_count_averages_middle_two() {
        let samples = [
            OffsetSample {
                offset_ms: 10.0,
                rtt_ms: 5.0,
                at_ms: 0.0,
            },
            OffsetSample {
                offset_ms: 20.0,
                rtt_ms: 5.0,
                at_ms: 0.0,
            },
        ];
        assert_eq!(select_offset(&samples), Some(15.0));
    }

    #[test]
    fn select_offset_prefers_min_rtt_median_and_excludes_far_outlier() {
        // Three low-RTT samples (median wins) plus one high-RTT outlier whose
        // wildly different offset must NOT pollute the result: low_rtt_cutoff
        // = max(min_rtt*2, min_rtt+1) = max(10*2, 11) = 20, so the 40ms-RTT
        // sample is excluded from the median even though it's under the
        // absolute MAX_ACCEPTABLE_RTT_MS reject bound.
        let samples = [
            OffsetSample {
                offset_ms: 100.0,
                rtt_ms: 10.0,
                at_ms: 0.0,
            },
            OffsetSample {
                offset_ms: 105.0,
                rtt_ms: 12.0,
                at_ms: 0.0,
            },
            OffsetSample {
                offset_ms: 102.0,
                rtt_ms: 11.0,
                at_ms: 0.0,
            },
            OffsetSample {
                offset_ms: 500.0,
                rtt_ms: 40.0,
                at_ms: 0.0,
            },
        ];
        assert_eq!(
            select_offset(&samples),
            Some(102.0),
            "median of the 3 low-RTT samples; the high-RTT outlier must be excluded"
        );
    }

    #[test]
    fn is_stale_within_bound_is_fresh() {
        assert!(!is_stale(1000.0, 1000.0 + STALE_AFTER_MS - 1.0));
    }

    #[test]
    fn is_stale_past_bound_is_stale() {
        assert!(is_stale(1000.0, 1000.0 + STALE_AFTER_MS + 1.0));
    }

    #[test]
    fn estimator_current_at_none_before_any_sample() {
        let estimator = ClockOffsetEstimator::new();
        assert_eq!(estimator.current_at(0.0), None);
    }

    #[test]
    fn estimator_current_at_reports_fresh_sample() {
        let estimator = ClockOffsetEstimator::new();
        estimator.push_sample(compute_sample(1000.0, 1050.0, 1010.0));
        let (offset, rtt) = estimator
            .current_at(1010.0)
            .expect("a single fresh sample must be reported");
        assert!((offset - 45.0).abs() < 1e-9);
        assert_eq!(rtt, 10.0);
    }

    #[test]
    fn estimator_current_at_none_once_stale() {
        let estimator = ClockOffsetEstimator::new();
        estimator.push_sample(compute_sample(1000.0, 1050.0, 1010.0));
        // Ask "now" far enough past the sample's at_ms (1010.0) to be stale.
        let far_future = 1010.0 + STALE_AFTER_MS + 1.0;
        assert_eq!(
            estimator.current_at(far_future),
            None,
            "an aged-out sample must report n/a, never a stale-but-confident number"
        );
    }

    #[test]
    fn estimator_push_sample_bounds_history_to_max_samples() {
        let estimator = ClockOffsetEstimator::new();
        for i in 0..(MAX_SAMPLES + 5) {
            let t = i as f64 * 100.0;
            estimator.push_sample(compute_sample(t, t + 10.0, t + 5.0));
        }
        assert_eq!(estimator.samples.borrow().len(), MAX_SAMPLES);
    }
}
