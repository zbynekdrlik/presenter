//! Frame-presentation counters + the `requestVideoFrameCallback` observer.
//!
//! Frame observation is driven by `requestVideoFrameCallback` (fires once per
//! frame actually PRESENTED to the compositor, NOT throttled like setInterval
//! on TV WebViews). `FrameStats` holds the counters shared between the rVFC
//! observer (writer) and the health ticker (reader): presented-frame counts,
//! the proven-mode rate-gate anchors, and the per-interval presentation-gap
//! accumulators. Split out of `ndi_watchdog.rs` to keep that file under the
//! size cap (#418).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use leptos::wasm_bindgen::{closure::Closure, JsCast, JsValue};
use leptos::web_sys::{HtmlVideoElement, RtcPeerConnection};

use super::ndi_beacon::post_stats_beacon;
use super::ndi_profile::persist_proven_profile_mode;
use super::ndi_watchdog::{now_ms, ReloadEscalation, Watchdog};

/// Frame-presentation counters shared between the rVFC observer (writer)
/// and the health ticker (reader). All timestamps are `now_ms()` values.
pub(crate) struct FrameStats {
    /// Frames PRESENTED to the compositor this session (rVFC count, or the
    /// coarse currentTime proxy on rVFC-less browsers).
    pub(crate) frames_presented: Cell<u32>,
    /// Timestamp of the FIRST presented frame (0.0 until one arrives) —
    /// anchors the proven-mode rate gate (`PROVEN_MODE_WINDOW_MS`).
    pub(crate) first_frame_at: Cell<f64>,
    /// Timestamp of the most recently presented frame.
    pub(crate) last_frame_at: Cell<f64>,
    /// When this session's watchdog was installed (≈ connect time).
    pub(crate) started_at: Cell<f64>,
    /// PRESENTATION-GAP accumulators for the CURRENT beacon interval. These
    /// measure render-side cadence (how evenly frames reach the screen) —
    /// distinct from getStats' decode-side `framesDecoded`/`framesPerSecond`.
    /// A frame can decode on time yet be PRESENTED late (WebView compositor
    /// or main-thread hitch from the WASM page's periodic work); that late
    /// presentation is the user-visible "lag every ~20s" the decode metrics
    /// cannot see. All three reset on each beacon (see `snapshot_present_gaps`).
    ///
    /// Largest inter-present gap (ms) observed this interval.
    pub(crate) max_present_gap_ms: Cell<f64>,
    /// Count of inter-present gaps > 100ms this interval (perceptible hitches).
    pub(crate) present_gaps_over100: Cell<u32>,
    /// Frames presented this interval (rVFC callback count) — the numerator
    /// of the render-side fps, paired with the interval wall-clock duration.
    pub(crate) presented_in_interval: Cell<u32>,
    /// `now_ms()` when the current beacon interval started (last reset).
    pub(crate) interval_started_at: Cell<f64>,
}

impl FrameStats {
    /// Create a fresh per-session `FrameStats`, anchoring every timestamp
    /// (except `first_frame_at`, which stays 0.0 until the first presented
    /// frame) to `now` (≈ connect time).
    pub(crate) fn new(now: f64) -> Rc<Self> {
        Rc::new(Self {
            frames_presented: Cell::new(0),
            first_frame_at: Cell::new(0.0),
            last_frame_at: Cell::new(now),
            started_at: Cell::new(now),
            max_present_gap_ms: Cell::new(0.0),
            present_gaps_over100: Cell::new(0),
            presented_in_interval: Cell::new(0),
            interval_started_at: Cell::new(now),
        })
    }
}

/// Record ONE presented frame into `stats` and run the proven-mode check.
/// Shared by the rVFC observer and the currentTime proxy so both enforce
/// the SAME rate gate: when the `PROVEN_MODE_FRAMES`th frame lands, the
/// current profile mode is persisted ONLY if that frame arrived within
/// `PROVEN_MODE_WINDOW_MS` of the FIRST presented frame (≥~10fps
/// sustained). 100 frames dribbling in at 1fps over 100s must NOT prove a
/// mode — that's a broken decode, not a working one. Missing the window
/// persists nothing (the stored mode, if any, is left untouched).
///
/// Returns the new presented-frame count. Note the proxy path counts at
/// most one frame per 1s tick, so it can never reach 100 frames in 10s —
/// rVFC-less browsers therefore never persist a proven mode, which is the
/// honest outcome of a measurement too coarse to prove a frame RATE.
pub(crate) fn record_presented_frame(stats: &FrameStats) -> u32 {
    let n = stats.frames_presented.get().saturating_add(1);
    stats.frames_presented.set(n);
    let now = now_ms();
    // Inter-present gap = wall-clock since the PREVIOUS presented frame. Skip
    // the first frame of the session (no predecessor) so a long pre-roll wait
    // is never charged as a presentation hitch. Update the present-gap
    // accumulators BEFORE overwriting last_frame_at so the delta is correct.
    if n > 1 {
        record_present_gap(stats, now - stats.last_frame_at.get());
    }
    stats
        .presented_in_interval
        .set(stats.presented_in_interval.get().saturating_add(1));
    stats.last_frame_at.set(now);
    if n == 1 {
        stats.first_frame_at.set(now);
    }
    if n == Watchdog::PROVEN_MODE_FRAMES
        && now - stats.first_frame_at.get() <= Watchdog::PROVEN_MODE_WINDOW_MS
    {
        persist_proven_profile_mode();
    }
    n
}

/// Threshold (ms) above which an inter-present gap is a perceptible render-side
/// hitch. At 30fps the nominal gap is ~33ms; >100ms means ≥3 frame-times'
/// worth of stall reached the screen even if the decoder kept up.
const PRESENT_GAP_HITCH_MS: f64 = 100.0;

/// Fold one inter-present gap (ms) into the current-interval accumulators:
/// track the maximum and count the ones over the perceptible-hitch threshold.
fn record_present_gap(stats: &FrameStats, gap_ms: f64) {
    if gap_ms > stats.max_present_gap_ms.get() {
        stats.max_present_gap_ms.set(gap_ms);
    }
    if gap_ms > PRESENT_GAP_HITCH_MS {
        stats
            .present_gaps_over100
            .set(stats.present_gaps_over100.get().saturating_add(1));
    }
}

/// Snapshot the present-gap accumulators for a beacon and RESET them so the
/// NEXT beacon reports only the next interval (last-interval semantics, not
/// cumulative). Returns `(maxPresentGapMs, presentGapsOver100, presentedFps)`:
/// - `max_present_gap_ms` — largest inter-present gap this interval (ms).
/// - `present_gaps_over100` — count of >100ms gaps this interval.
/// - `presented_fps` — frames presented / interval-seconds (render-side fps,
///   distinct from getStats' decode-side fps). `None` when the interval is too
///   short to be meaningful (no elapsed time yet).
pub(crate) fn snapshot_present_gaps(stats: &FrameStats) -> (f64, u32, Option<f64>) {
    let now = now_ms();
    let max_gap = stats.max_present_gap_ms.get();
    let over100 = stats.present_gaps_over100.get();
    let presented = stats.presented_in_interval.get();
    let elapsed_ms = now - stats.interval_started_at.get();
    let fps = if elapsed_ms > 0.0 {
        Some(f64::from(presented) / (elapsed_ms / 1000.0))
    } else {
        None
    };
    stats.max_present_gap_ms.set(0.0);
    stats.present_gaps_over100.set(0);
    stats.presented_in_interval.set(0);
    stats.interval_started_at.set(now);
    (max_gap, over100, fps)
}

/// Shared holder for the self-rescheduling rVFC closure (the closure needs a
/// handle to itself to re-register for the next presented frame).
type SharedRvfcClosure = Rc<RefCell<Option<Closure<dyn FnMut(JsValue, JsValue)>>>>;

/// Start a self-rescheduling `requestVideoFrameCallback` loop on `video`,
/// maintaining `stats.frames_presented` / `stats.last_frame_at`. rVFC fires
/// once per frame PRESENTED to the compositor and — unlike setInterval — is
/// NOT throttled by TV power-saving timer policies, so the counters stay
/// truthful exactly where the wall-clock heuristics lied.
///
/// Side effects driven from the frame path (see `record_presented_frame`):
/// - at `Watchdog::PROVEN_MODE_FRAMES` presented frames — IF they arrived
///   within `Watchdog::PROVEN_MODE_WINDOW_MS` of the first frame — the
///   current profile mode is persisted to localStorage (proven-mode
///   stickiness, rate-gated);
/// - every `Watchdog::RVFC_BEACON_FRAME_PERIOD` frames (~15s at 30fps) a
///   stats beacon posts — reliable on throttled displays where the 1s-tick
///   beacons can become sparse.
///
/// Returns false when the browser lacks rVFC (non-Chromium): the health
/// ticker then approximates frames from currentTime advance instead.
///
/// The closure is gated by `active`: once cleared it returns WITHOUT
/// rescheduling, ending the chain (the leaked holder cycle goes inert —
/// same bounded-leak idiom as the rest of this file).
pub(crate) fn start_rvfc_frame_observer(
    video: &HtmlVideoElement,
    pc: &RtcPeerConnection,
    source_id: &str,
    active: &Rc<Cell<bool>>,
    stats: &Rc<FrameStats>,
    escalation: &Rc<ReloadEscalation>,
) -> bool {
    let supported = js_sys::Reflect::get(
        video.as_ref(),
        &JsValue::from_str("requestVideoFrameCallback"),
    )
    .map(|f| f.is_function())
    .unwrap_or(false);
    if !supported {
        return false;
    }

    let holder: SharedRvfcClosure = Rc::new(RefCell::new(None));
    let cb = {
        let active = Rc::clone(active);
        let stats = Rc::clone(stats);
        let video = video.clone();
        let pc = pc.clone();
        let source_id = source_id.to_string();
        let holder = Rc::clone(&holder);
        let escalation = Rc::clone(escalation);
        Closure::<dyn FnMut(JsValue, JsValue)>::new(move |_now: JsValue, _meta: JsValue| {
            if !active.get() {
                return;
            }
            // A frame reached the screen: clear the page-level reload timer so
            // the last-resort reload (#401) only ever fires while video is
            // genuinely dead across reconnects, never during healthy playback.
            escalation.note_decoded_frame();
            let n = record_presented_frame(&stats);
            if n % Watchdog::RVFC_BEACON_FRAME_PERIOD == 0 {
                post_stats_beacon(&pc, &source_id, &stats);
            }
            schedule_video_frame_callback(&video, &holder);
        })
    };
    *holder.borrow_mut() = Some(cb);
    schedule_video_frame_callback(video, &holder);
    true
}

/// Invoke `video.requestVideoFrameCallback(cb)` via Reflect (web_sys has no
/// stable binding for rVFC). Silent no-op if the method is missing.
fn schedule_video_frame_callback(video: &HtmlVideoElement, holder: &SharedRvfcClosure) {
    let Ok(f) = js_sys::Reflect::get(
        video.as_ref(),
        &JsValue::from_str("requestVideoFrameCallback"),
    ) else {
        return;
    };
    let Some(f) = f.dyn_ref::<js_sys::Function>() else {
        return;
    };
    if let Some(cb) = holder.borrow().as_ref() {
        let _ = f.call1(video.as_ref(), cb.as_ref().unchecked_ref());
    }
}
