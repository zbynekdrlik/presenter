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
use super::ndi_clock_offset::ClockOffsetEstimator;
use super::ndi_profile::persist_proven_profile_mode;
use super::ndi_watchdog::{now_ms, ReloadEscalation, Watchdog};

/// Callback that publishes the smoothed stage-side video latency (ms) to the
/// on-screen readout signal (`StageContext::video_latency_ms`). `Some(ms)`
/// updates the "video · N ms" readout; `None` clears it. Boxed behind `Rc` so
/// it threads cheaply from `NdiVideo` through `Watchdog::install` into the rVFC
/// observer. `None` (no setter) disables the readout entirely (e.g. a video
/// element with no StageContext).
pub(crate) type VideoLatencySetter = Rc<dyn Fn(Option<f64>)>;

/// Callback that publishes whether NDI frames are CURRENTLY presenting to the
/// shared `StageContext::ndi_frames_live` signal (#500). Called with `true` on
/// the first presented frame after a gap (so the neutral cover drops the instant
/// video is on screen) and `false` once frames go stale. Boxed behind `Rc` so it
/// threads cheaply from `NdiVideo` through `Watchdog::install` into both the rVFC
/// observer and the health ticker. `None` (no setter) disables the live-frame
/// gate entirely (e.g. a video element with no StageContext).
pub(crate) type FramesLiveSetter = Rc<dyn Fn(bool)>;

/// How long after the last presented frame the video is considered no longer
/// "live" (#500). Once `now - last_frame_at` exceeds this, the 1s health ticker
/// flips `ndi_frames_live` back to `false` so a genuinely silent/stopped source
/// restores the neutral covering placeholder. Comfortably above one 30fps frame
/// interval (~33ms) and the ticker's own 1s cadence, but far below the 10s
/// real-freeze reconnect horizon — so a brief render hitch never drops the cover,
/// yet a truly stopped source restores it within ~1–2 ticks.
pub(crate) const FRAMES_LIVE_STALENESS_MS: f64 = 1500.0;

/// Pure: are presented frames stale (no frame for longer than `window_ms`)?
/// Strictly-greater so a frame exactly at the window edge still counts as live.
/// Host-unit-tested; the reactive wiring (`refresh_frames_live_staleness`) reads
/// the live `now_ms()` / `last_frame_at` and delegates here.
pub(crate) fn frames_are_stale(now_ms: f64, last_frame_at: f64, window_ms: f64) -> bool {
    now_ms - last_frame_at > window_ms
}

/// Mark frames as LIVE (#500). Idempotent within a session: the per-session
/// `stats.frames_live` cell tracks the last-emitted state, so the reactive
/// `setter(true)` fires only on the false→true transition — never ~30×/s from
/// the rVFC callback. Called from the rVFC frame path and the currentTime proxy.
pub(crate) fn mark_frames_live(stats: &FrameStats, setter: &Option<FramesLiveSetter>) {
    if !stats.frames_live.get() {
        stats.frames_live.set(true);
        if let Some(setter) = setter {
            setter(true);
        }
    }
}

/// Flip frames back to NOT-live (#500) when they have gone stale
/// (`frames_are_stale`). Idempotent: only acts on the true→false transition, so
/// the reactive `setter(false)` fires once when a live source stops. Called once
/// per 1s health tick. The neutral cover then reappears if the status is still
/// neutral (a genuinely silent/stopped source).
pub(crate) fn refresh_frames_live_staleness(stats: &FrameStats, setter: &Option<FramesLiveSetter>) {
    if stats.frames_live.get()
        && frames_are_stale(
            now_ms(),
            stats.last_frame_at.get(),
            FRAMES_LIVE_STALENESS_MS,
        )
    {
        stats.frames_live.set(false);
        if let Some(setter) = setter {
            setter(false);
        }
    }
}

/// Cadence (ms) at which the smoothed video latency is pushed to the on-screen
/// signal. rVFC fires ~30×/s; writing the reactive signal that often would
/// re-run the StatusBar autofit every frame. ~1 update/s keeps the readout
/// live without churning the render.
const VIDEO_LATENCY_EMIT_INTERVAL_MS: f64 = 1000.0;

/// EMA responsiveness for the on-screen video-latency figure. Lower = smoother
/// (rides out per-frame jitter so the displayed number is stable and readable),
/// higher = more responsive. 0.2 gives a calm, legible stage readout.
const VIDEO_LATENCY_SMOOTHING_ALPHA: f64 = 0.2;

/// Derive the TRUE server→display video latency in milliseconds (#512, T4).
///
/// The number is the time from the frame leaving the server to it being painted
/// on the stage — the sum of two independently-measured, physically-real hops:
///
/// 1. **render residual** = `expected_display_time - receive_time` (rVFC).
///    `receiveTime` is when the frame's LAST RTP packet arrived in the browser;
///    `expectedDisplayTime` is when the UA will paint it. The span between them
///    is jitter-buffer + decode + present — i.e. everything AFTER arrival. (When
///    the receive/display pair is absent, decode duration `processing_duration_s`
///    is the closest residual proxy.)
/// 2. **network one-way** = `network_one_way_ms`, ≈ RTT/2 from the `/ndi/time`
///    offset handshake (#510). This is the server→browser transit that happens
///    BEFORE `receiveTime` — the hop the old #479 residual-only number MISSED
///    entirely (why Tailscale-from-home read the LOWEST latency: its WAN transit
///    was invisible, leaving only local paint delay).
///
/// `network_one_way_ms = None` (no fresh `/ndi/time` round-trip landed, or the
/// last one aged out) → returns `None` (**n/a**), NOT the residual alone: the
/// residual-only figure is exactly the misleading number #512 replaces, so it is
/// never shown as if it were the full latency. Honest n/a beats a plausible lie.
///
/// Negatives (clock skew) clamp to 0 so the stage never shows a nonsensical
/// negative latency; the sum is therefore always ≥ 0, and for a given residual a
/// higher RTT (e.g. Tailscale) always yields a higher — never lower — number.
pub(crate) fn derive_video_latency_ms(
    receive_time: Option<f64>,
    expected_display_time: Option<f64>,
    processing_duration_s: Option<f64>,
    network_one_way_ms: Option<f64>,
) -> Option<f64> {
    let residual = if let (Some(recv), Some(disp)) = (receive_time, expected_display_time) {
        (disp - recv).max(0.0)
    } else {
        (processing_duration_s? * 1000.0).max(0.0)
    };
    // Truthful server→display REQUIRES the network hop. Without it the residual
    // alone is the old misleading number → report n/a (design §3 trust predicate).
    Some(residual + network_one_way_ms?.max(0.0))
}

/// Fold one latency `sample` (ms) into the running EMA. `prev = None` (no
/// sample yet) passes the first sample through unchanged. `alpha` in (0, 1].
pub(crate) fn smooth_latency(prev: Option<f64>, sample: f64, alpha: f64) -> f64 {
    match prev {
        Some(p) => p + alpha * (sample - p),
        None => sample,
    }
}

/// Read the rVFC metadata object's timing fields and run them through
/// `derive_video_latency_ms`. The metadata is a plain JS object; web_sys has no
/// typed binding for it, so the fields are read via `Reflect` (missing/non-numeric
/// → `None`, never an error/log). Returns `None` when the frame carries no
/// usable timing metadata.
fn video_latency_from_meta(meta: &JsValue, network_one_way_ms: Option<f64>) -> Option<f64> {
    let get = |k: &str| {
        js_sys::Reflect::get(meta, &JsValue::from_str(k))
            .ok()
            .and_then(|v| v.as_f64())
    };
    derive_video_latency_ms(
        get("receiveTime"),
        get("expectedDisplayTime"),
        get("processingDuration"),
        network_one_way_ms,
    )
}

/// Per presented frame: derive this frame's stage-side video latency from its
/// rVFC `meta`, fold it into the smoothed value on `stats`, and push the
/// smoothed figure to the on-screen `setter` at most once per
/// `VIDEO_LATENCY_EMIT_INTERVAL_MS` (#479). No-op when the frame carries no
/// usable timing metadata, so a browser whose rVFC omits the timing fields
/// simply never shows the readout (rather than showing a wrong value).
fn update_video_latency(
    meta: &JsValue,
    stats: &FrameStats,
    setter: &Option<VideoLatencySetter>,
    network_one_way_ms: Option<f64>,
) {
    let now = now_ms();
    let due = now - stats.last_latency_emit_at.get() >= VIDEO_LATENCY_EMIT_INTERVAL_MS;
    match video_latency_from_meta(meta, network_one_way_ms) {
        Some(sample) => {
            let smoothed = smooth_latency(
                stats.video_latency_ms.get(),
                sample,
                VIDEO_LATENCY_SMOOTHING_ALPHA,
            );
            stats.video_latency_ms.set(Some(smoothed));
            if due {
                stats.last_latency_emit_at.set(now);
                if let Some(setter) = setter {
                    setter(Some(smoothed));
                }
            }
        }
        // No trustworthy figure this frame (no fresh /ndi/time offset yet, or it
        // aged out): show n/a rather than a stale-but-confident number. Reset the
        // EMA so a resumed reading starts fresh instead of dragging an old value.
        None => {
            stats.video_latency_ms.set(None);
            if due {
                stats.last_latency_emit_at.set(now);
                if let Some(setter) = setter {
                    setter(None);
                }
            }
        }
    }
}

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
    /// Smoothed stage-side VIDEO latency in ms (#479) — the "received →
    /// displayed" decode+present lag derived from each frame's rVFC metadata
    /// (`derive_video_latency_ms`) and folded through an EMA. `None` until the
    /// first frame carries usable timing metadata. This is the figure shown in
    /// the stage's separate "video · N ms" readout (distinct from the WS
    /// connection round-trip in the "CONNECTED · N ms" readout).
    pub(crate) video_latency_ms: Cell<Option<f64>>,
    /// `now_ms()` of the last push of `video_latency_ms` to the on-screen
    /// signal. The rVFC callback fires ~30×/s; the readout is updated at most
    /// once per `VIDEO_LATENCY_EMIT_INTERVAL_MS` so the StatusBar autofit does
    /// not re-run every frame. Seeded to 0.0 so the first sample emits at once.
    pub(crate) last_latency_emit_at: Cell<f64>,
    /// Last-EMITTED frames-live state for THIS session (#500). Tracks what was
    /// last pushed to `StageContext::ndi_frames_live` via the `FramesLiveSetter`
    /// so the reactive signal is written only on transitions (frame path sets
    /// true once; the health ticker sets false once on staleness) — not ~30×/s.
    /// Per-session (reset to `false` each `FrameStats::new`), so a fresh session
    /// re-emits `true` on its first presented frame.
    pub(crate) frames_live: Cell<bool>,
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
            video_latency_ms: Cell::new(None),
            last_latency_emit_at: Cell::new(0.0),
            frames_live: Cell::new(false),
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
/// handle to itself to re-register for the next presented frame). `pub(crate)`
/// so `ndi_clock_offset`'s independent rVFC-driven handshake loop (#510) can
/// reuse the SAME scheduling idiom instead of duplicating it.
pub(crate) type SharedRvfcClosure = Rc<RefCell<Option<Closure<dyn FnMut(JsValue, JsValue)>>>>;

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
#[allow(clippy::too_many_arguments)]
pub(crate) fn start_rvfc_frame_observer(
    video: &HtmlVideoElement,
    pc: &RtcPeerConnection,
    source_id: &str,
    active: &Rc<Cell<bool>>,
    stats: &Rc<FrameStats>,
    escalation: &Rc<ReloadEscalation>,
    clock_offset: &Rc<ClockOffsetEstimator>,
    video_latency_setter: Option<VideoLatencySetter>,
    frames_live_setter: Option<FramesLiveSetter>,
) -> bool {
    if !video_supports_rvfc(video) {
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
        let clock_offset = Rc::clone(clock_offset);
        Closure::<dyn FnMut(JsValue, JsValue)>::new(move |_now: JsValue, meta: JsValue| {
            if !active.get() {
                return;
            }
            // A frame reached the screen: clear the page-level reload timer so
            // the last-resort reload (#401) only ever fires while video is
            // genuinely dead across reconnects, never during healthy playback.
            escalation.note_decoded_frame();
            // #500: frames are presenting → drop the neutral covering placeholder
            // (idempotent: only the false→true transition writes the signal).
            mark_frames_live(&stats, &frames_live_setter);
            // #512: TRUE server→display latency = network one-way (RTT/2 from the
            // /ndi/time offset handshake, #510) + this frame's render residual.
            // `None` RTT (no fresh offset yet) → update_video_latency shows n/a.
            let network_one_way_ms = clock_offset.current().map(|(_offset, rtt)| rtt / 2.0);
            update_video_latency(&meta, &stats, &video_latency_setter, network_one_way_ms);
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

/// Does `video` support `requestVideoFrameCallback`? `pub(crate)` so
/// `ndi_clock_offset`'s independent rVFC-driven handshake loop (#510) shares
/// this same feature check instead of duplicating it.
pub(crate) fn video_supports_rvfc(video: &HtmlVideoElement) -> bool {
    js_sys::Reflect::get(
        video.as_ref(),
        &JsValue::from_str("requestVideoFrameCallback"),
    )
    .map(|f| f.is_function())
    .unwrap_or(false)
}

/// Invoke `video.requestVideoFrameCallback(cb)` via Reflect (web_sys has no
/// stable binding for rVFC). Silent no-op if the method is missing.
/// `pub(crate)` so `ndi_clock_offset`'s independent rVFC-driven handshake loop
/// (#510) can reuse the SAME scheduling idiom instead of duplicating it.
pub(crate) fn schedule_video_frame_callback(video: &HtmlVideoElement, holder: &SharedRvfcClosure) {
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

#[cfg(test)]
mod tests {
    use super::{
        derive_video_latency_ms, frames_are_stale, mark_frames_live, smooth_latency, FrameStats,
        FramesLiveSetter, FRAMES_LIVE_STALENESS_MS, VIDEO_LATENCY_SMOOTHING_ALPHA,
    };
    use std::cell::Cell;
    use std::rc::Rc;

    // ─────────────────────────────────────────────────────────────────────
    // #500 frames-live gate: the neutral covering placeholder must reflect
    // whether frames are ACTUALLY presenting. `frames_are_stale` is the pure
    // staleness decision; `mark_frames_live` is the transition-guarded setter.
    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn frames_are_stale_only_past_the_window() {
        // Just decoded → not stale.
        assert!(!frames_are_stale(1000.0, 1000.0, FRAMES_LIVE_STALENESS_MS));
        // Within the window → not stale.
        assert!(!frames_are_stale(
            1000.0 + FRAMES_LIVE_STALENESS_MS - 1.0,
            1000.0,
            FRAMES_LIVE_STALENESS_MS
        ));
        // Exactly at the window edge → still live (strictly-greater boundary).
        assert!(!frames_are_stale(
            1000.0 + FRAMES_LIVE_STALENESS_MS,
            1000.0,
            FRAMES_LIVE_STALENESS_MS
        ));
        // Past the window → stale.
        assert!(frames_are_stale(
            1000.0 + FRAMES_LIVE_STALENESS_MS + 1.0,
            1000.0,
            FRAMES_LIVE_STALENESS_MS
        ));
    }

    #[test]
    fn mark_frames_live_emits_only_on_the_false_to_true_transition() {
        let stats = FrameStats::new(0.0);
        assert!(!stats.frames_live.get());
        let calls = Rc::new(Cell::new(0u32));
        let last = Rc::new(Cell::new(false));
        let setter: FramesLiveSetter = {
            let calls = Rc::clone(&calls);
            let last = Rc::clone(&last);
            Rc::new(move |v: bool| {
                calls.set(calls.get() + 1);
                last.set(v);
            })
        };
        // First call flips the cell and emits true once.
        mark_frames_live(&stats, &Some(Rc::clone(&setter)));
        assert!(stats.frames_live.get());
        assert_eq!(calls.get(), 1);
        assert!(last.get());
        // Subsequent calls while already live do NOT re-emit (no ~30×/s churn).
        mark_frames_live(&stats, &Some(Rc::clone(&setter)));
        mark_frames_live(&stats, &Some(setter));
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn mark_frames_live_without_setter_still_updates_the_cell() {
        // No StageContext → no setter, but the per-session cell still flips so
        // the staleness logic remains consistent.
        let stats = FrameStats::new(0.0);
        mark_frames_live(&stats, &None);
        assert!(stats.frames_live.get());
    }

    // ─────────────────────────────────────────────────────────────────────
    // #479 stage-side VIDEO latency derivation (received → displayed).
    //
    // The figure shown in the stage's separate "video · N ms" readout is the
    // decode+present lag of each frame, derived from rVFC metadata. These
    // tests pin the derivation math (the JsValue field-reading shim is a thin
    // wrapper over `derive_video_latency_ms`, covered by the E2E render test).
    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn sums_render_residual_and_network_one_way() {
        // residual = expectedDisplayTime(1080) - receiveTime(1000) = 80ms;
        // network one-way = 20ms → true server→display = 100ms. processingDuration
        // is IGNORED when the receive/display pair exists.
        assert_eq!(
            derive_video_latency_ms(Some(1000.0), Some(1080.0), Some(0.005), Some(20.0)),
            Some(100.0)
        );
    }

    #[test]
    fn falls_back_to_processing_duration_then_adds_network() {
        // No receiveTime → decode duration residual (0.012s = 12ms) + 10ms network.
        assert_eq!(
            derive_video_latency_ms(None, Some(1080.0), Some(0.012), Some(10.0)),
            Some(22.0)
        );
        // processingDuration alone (no receive/display pair) + network.
        assert_eq!(
            derive_video_latency_ms(None, None, Some(0.02), Some(5.0)),
            Some(25.0)
        );
    }

    #[test]
    fn n_a_without_a_network_sample() {
        // #512 trust predicate: no fresh /ndi/time offset → n/a, NOT the
        // residual-only figure (that is exactly the old misleading number).
        assert_eq!(
            derive_video_latency_ms(Some(1000.0), Some(1080.0), Some(0.005), None),
            None
        );
        assert_eq!(derive_video_latency_ms(None, None, Some(0.02), None), None);
    }

    #[test]
    fn none_when_no_residual_source() {
        // No residual source at all → n/a regardless of a network sample.
        assert_eq!(derive_video_latency_ms(None, None, None, Some(10.0)), None);
        // receiveTime without expectedDisplayTime and no processingDuration.
        assert_eq!(
            derive_video_latency_ms(Some(1000.0), None, None, Some(10.0)),
            None
        );
    }

    #[test]
    fn clamps_negative_residual_but_keeps_network() {
        // Clock skew making display appear BEFORE receive clamps the residual to
        // 0; the (real) network term still contributes. Never negative.
        assert_eq!(
            derive_video_latency_ms(Some(1080.0), Some(1000.0), None, Some(10.0)),
            Some(10.0)
        );
        // Negative processingDuration (defensive) also clamps to 0 residual.
        assert_eq!(
            derive_video_latency_ms(None, None, Some(-0.01), Some(5.0)),
            Some(5.0)
        );
    }

    #[test]
    fn tailscale_reading_is_never_lower_than_lan() {
        // The core #512 regression invariant: for the SAME stream/residual, a
        // higher network one-way (Tailscale/WAN) must yield a HIGHER — never
        // lower — latency than LAN. This is the exact bug (Tailscale read the
        // LOWEST) the redefinition fixes: the old residual-only number ignored
        // network transit entirely.
        let lan = derive_video_latency_ms(Some(1000.0), Some(1080.0), None, Some(3.0))
            .expect("lan reading");
        let tailscale = derive_video_latency_ms(Some(1000.0), Some(1080.0), None, Some(60.0))
            .expect("tailscale reading");
        assert!(
            tailscale > lan,
            "tailscale ({tailscale}) must exceed lan ({lan}) for the same residual"
        );
    }

    #[test]
    fn latency_is_always_non_negative() {
        for (recv, disp, proc, net) in [
            (Some(1000.0), Some(1080.0), None, Some(0.0)),
            (Some(1080.0), Some(1000.0), None, Some(0.0)),
            (None, None, Some(0.0), Some(0.0)),
            (Some(1000.0), Some(1080.0), None, Some(1000.0)),
        ] {
            if let Some(v) = derive_video_latency_ms(recv, disp, proc, net) {
                assert!(v >= 0.0, "latency must never be negative, got {v}");
            }
        }
    }

    #[test]
    fn ema_passes_first_sample_through_then_smooths() {
        // First sample (prev=None) passes through unchanged.
        let first = smooth_latency(None, 100.0, VIDEO_LATENCY_SMOOTHING_ALPHA);
        assert_eq!(first, 100.0);
        // Next sample moves the average toward it by alpha (0.2): 100 + 0.2*(200-100) = 120.
        let second = smooth_latency(Some(first), 200.0, VIDEO_LATENCY_SMOOTHING_ALPHA);
        assert!((second - 120.0).abs() < 1e-9, "EMA second = {second}");
        // A steady stream of equal samples converges to that value.
        let mut v = smooth_latency(None, 50.0, VIDEO_LATENCY_SMOOTHING_ALPHA);
        for _ in 0..100 {
            v = smooth_latency(Some(v), 50.0, VIDEO_LATENCY_SMOOTHING_ALPHA);
        }
        assert!((v - 50.0).abs() < 1e-6, "EMA converged = {v}");
    }
}
