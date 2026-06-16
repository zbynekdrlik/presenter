//! Frame-based health watchdog + profile-mode state for `NdiVideo`.
//!
//! Frame observation is driven by `requestVideoFrameCallback` (fires once
//! per frame actually PRESENTED to the compositor, NOT throttled like
//! setInterval on TV WebViews); a 1s ticker evaluates the stall and
//! profile-fallback rules against those counters and paces the stats
//! beacons. Split out of `ndi_video.rs` to keep both files under the size
//! cap.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use leptos::wasm_bindgen::{closure::Closure, JsCast, JsValue};
use leptos::web_sys::{HtmlVideoElement, RtcIceConnectionState, RtcPeerConnection};
use wasm_bindgen_futures::{spawn_local, JsFuture};

/// localStorage key for the stream-profile fallback mode. Absent or
/// `"default"` = WHEP POST without a profile query; `"compat"` = the WHEP
/// POST URL carries `?profile=compat`.
///
/// NOTE: the server now serves ONE 720p H264 stream regardless of
/// `?profile=` (see `StreamProfile::from_query`), so the compat flip is a
/// no-op server-side — it does NOT switch to any 640×480 / VP8 branch (that
/// branch never shipped). The flip is retained ONLY because changing the URL
/// forces a reconnect, and that reconnect re-establishes a stuck session.
///
/// The KEY deliberately keeps its historical name ("ndiCodecMode") so
/// deployed TVs don't grow a second orphaned entry; the retired "vp8" value
/// some of them still store parses as default mode and self-heals through
/// the normal fallback → proven-mode flow.
const PROFILE_MODE_KEY: &str = "ndiCodecMode";

/// localStorage key for the persistent per-display identity used in stats
/// beacons (per-TV health attribution server-side).
const DISPLAY_ID_KEY: &str = "ndiDisplayId";

/// Access the window's localStorage (None when unavailable, e.g. sandboxed).
fn local_storage() -> Option<leptos::web_sys::Storage> {
    leptos::web_sys::window().and_then(|w| w.local_storage().ok().flatten())
}

thread_local! {
    /// In-memory profile mode for THIS page load, seeded from localStorage on
    /// first use. `None` = not yet seeded. Connect attempts read this, NOT
    /// localStorage directly: a fallback switch flips it in memory only —
    /// the sticky localStorage value is written exclusively by
    /// `persist_proven_profile_mode` once a mode actually decodes (so the
    /// persisted value is always a PROVEN one, never a guess mid-ping-pong).
    static PROFILE_MODE_COMPAT: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
    /// At most ONE profile switch per page load. One Vestel TV alternated
    /// modes repeatedly when its wall-clock-based decode check misfired;
    /// bounding the switch to once-per-pageload kills the ping-pong.
    static PROFILE_SWITCHED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// True when the stream-profile fallback mode is "compat". Any other value
/// (including absent and the retired "vp8") means the default 720p stream.
pub(crate) fn profile_mode_is_compat() -> bool {
    PROFILE_MODE_COMPAT.with(|cell| {
        if let Some(v) = cell.get() {
            return v;
        }
        let stored = local_storage()
            .and_then(|s| s.get_item(PROFILE_MODE_KEY).ok().flatten())
            .as_deref()
            == Some("compat");
        cell.set(Some(stored));
        stored
    })
}

/// Flip the in-memory profile mode (default → compat or compat → default)
/// and return the new mode name — at most ONCE per page load. Returns
/// `None` when the one-shot switch was already spent (no further toggling
/// until reload). Deliberately does NOT touch localStorage: only a mode
/// that goes on to present `PROVEN_MODE_FRAMES` frames within
/// `PROVEN_MODE_WINDOW_MS` of the first frame gets persisted (see
/// `record_presented_frame`).
fn switch_profile_mode_once() -> Option<&'static str> {
    if PROFILE_SWITCHED.with(|c| c.replace(true)) {
        return None;
    }
    let new_compat = !profile_mode_is_compat();
    PROFILE_MODE_COMPAT.with(|c| c.set(Some(new_compat)));
    Some(profile_mode_name(new_compat))
}

/// The wire/storage name of a profile mode: "compat" or "default".
fn profile_mode_name(compat: bool) -> &'static str {
    if compat {
        "compat"
    } else {
        "default"
    }
}

/// Persist the CURRENT profile mode to localStorage. Called once a session
/// presents `PROVEN_MODE_FRAMES` frames WITHIN `PROVEN_MODE_WINDOW_MS` of
/// the first presented frame — the mode demonstrably decodes AT A USABLE
/// RATE on this display, so it is safe to make sticky across reloads.
///
/// The rate gate is load-bearing: 100 frames at <10fps must NOT prove a
/// mode. A Vestel TV limping along at 0.3-1.7 fps (the VP8-era crawl)
/// still reaches 100 presented frames eventually (~100s at 1fps), and
/// persisting then locked the broken mode in forever. Callers
/// (`record_presented_frame`) enforce the window; an unproven mode is
/// simply left unpersisted — the existing stored value is never cleared —
/// so the next page load retries.
fn persist_proven_profile_mode() {
    if let Some(storage) = local_storage() {
        let _ = storage.set_item(
            PROFILE_MODE_KEY,
            profile_mode_name(profile_mode_is_compat()),
        );
    }
}

/// Persistent random display id (16 hex chars) for beacon attribution.
/// Generated once and stored in localStorage; None when storage is
/// unavailable (beacon then sends null — still better than dropping it).
fn display_id() -> Option<String> {
    let storage = local_storage()?;
    if let Ok(Some(id)) = storage.get_item(DISPLAY_ID_KEY) {
        if !id.is_empty() {
            return Some(id);
        }
    }
    let mut id = String::with_capacity(16);
    for _ in 0..16 {
        let digit = (js_sys::Math::random() * 16.0) as u32 % 16;
        id.push(char::from_digit(digit, 16)?);
    }
    let _ = storage.set_item(DISPLAY_ID_KEY, &id);
    Some(id)
}

/// Frame-presentation counters shared between the rVFC observer (writer)
/// and the health ticker (reader). All timestamps are `now_ms()` values.
struct FrameStats {
    /// Frames PRESENTED to the compositor this session (rVFC count, or the
    /// coarse currentTime proxy on rVFC-less browsers).
    frames_presented: Cell<u32>,
    /// Timestamp of the FIRST presented frame (0.0 until one arrives) —
    /// anchors the proven-mode rate gate (`PROVEN_MODE_WINDOW_MS`).
    first_frame_at: Cell<f64>,
    /// Timestamp of the most recently presented frame.
    last_frame_at: Cell<f64>,
    /// When this session's watchdog was installed (≈ connect time).
    started_at: Cell<f64>,
    /// PRESENTATION-GAP accumulators for the CURRENT beacon interval. These
    /// measure render-side cadence (how evenly frames reach the screen) —
    /// distinct from getStats' decode-side `framesDecoded`/`framesPerSecond`.
    /// A frame can decode on time yet be PRESENTED late (WebView compositor
    /// or main-thread hitch from the WASM page's periodic work); that late
    /// presentation is the user-visible "lag every ~20s" the decode metrics
    /// cannot see. All three reset on each beacon (see `snapshot_present_gaps`).
    ///
    /// Largest inter-present gap (ms) observed this interval.
    max_present_gap_ms: Cell<f64>,
    /// Count of inter-present gaps > 100ms this interval (perceptible hitches).
    present_gaps_over100: Cell<u32>,
    /// Frames presented this interval (rVFC callback count) — the numerator
    /// of the render-side fps, paired with the interval wall-clock duration.
    presented_in_interval: Cell<u32>,
    /// `now_ms()` when the current beacon interval started (last reset).
    interval_started_at: Cell<f64>,
}

/// PAGE-SESSION reload escalation state (#401), shared across EVERY reconnect
/// cycle of a single stage page load. Unlike `FrameStats` (recreated per
/// Watchdog / per WHEP session), this lives in `NdiVideo`'s effect for the
/// whole page lifetime, so it can observe "video has been dead for 60s ACROSS
/// all reconnect attempts" — the signal a per-session counter can never see.
///
/// `last_decoded_frame_at` is bumped to `now` on every presented frame (any
/// session) and seeded to page-load time, so the elapsed time only grows
/// while NO session is decoding. When it crosses `RELOAD_NO_FRAME_MS` the
/// health ticker performs a one-shot `window.location.reload()` (guarded by
/// `reloaded` so concurrent ticks / sessions can't double-fire).
pub(crate) struct ReloadEscalation {
    last_decoded_frame_at: Cell<f64>,
    reloaded: Cell<bool>,
    /// The no-frames horizon (ms) for THIS page load. Defaults to
    /// `RELOAD_NO_FRAME_MS`; an explicit `?ndiReloadMs=<n>` URL query lowers
    /// it so the E2E can exercise the full reload path (incl. the real
    /// `window.location.reload()`) deterministically without a 60s wait. The
    /// param is read ONCE here; production pages never carry it, so prod always
    /// uses the conservative 60s.
    threshold_ms: f64,
}

impl ReloadEscalation {
    /// Create the page-session escalation tracker, seeding the
    /// last-decoded-frame timestamp to "now" (the page just loaded, so the
    /// reload horizon is measured from page start until the first frame).
    pub(crate) fn new() -> Rc<Self> {
        Rc::new(Self {
            last_decoded_frame_at: Cell::new(now_ms()),
            reloaded: Cell::new(false),
            threshold_ms: reload_threshold_ms_from_url(),
        })
    }

    /// Record that a frame just decoded — resets the page-level reload timer.
    /// Called from the frame path of EVERY session so a brief reconnect that
    /// resumes decoding clears the escalation well before the threshold.
    fn note_decoded_frame(&self) {
        self.last_decoded_frame_at.set(now_ms());
    }

    /// Milliseconds since any session last decoded a frame (page-session).
    fn ms_since_last_decoded_frame(&self) -> f64 {
        now_ms() - self.last_decoded_frame_at.get()
    }

    /// Evaluate the escalation rule and, if it trips, perform the one-shot
    /// full-page reload. Returns true once it has reloaded (so callers stop).
    /// Idempotent: the `reloaded` flag makes repeated ticks / multiple
    /// sessions fire `window.location.reload()` at most once.
    fn maybe_reload(&self) -> bool {
        if self.reloaded.get() {
            return true;
        }
        if !should_escalate_reload(self.ms_since_last_decoded_frame(), self.threshold_ms) {
            return false;
        }
        self.reloaded.set(true);
        leptos::logging::warn!(
            "watchdog: no decoded frame for {:.0}ms across reconnect attempts — LAST-RESORT full page reload (#401)",
            self.ms_since_last_decoded_frame()
        );
        if let Some(window) = leptos::web_sys::window() {
            let _ = window.location().reload();
        }
        true
    }
}

/// Watchdog that fires `on_failure` when EITHER:
/// - the RTCPeerConnection's iceConnectionState becomes "failed",
///   "disconnected", or "closed" (genuine connection loss), OR
/// - a FRAME-BASED health rule trips (see `start_health_ticker`).
///
/// Frame observation is driven by `requestVideoFrameCallback` (fires once
/// per frame actually PRESENTED to the compositor) — NOT by wall-clock
/// currentTime sampling. The previous wall-clock heuristics misfired on
/// prod TVs whose JS timers throttle (Vestel WebViews): the 3s
/// currentTime-stall check fired during render hiccups although frames
/// decoded at 30fps, and the tick-12 fallback check ping-ponged modes —
/// measured as 94 WHEP add/removes in 3 minutes across 4 TVs.
///
/// It deliberately does NOT reconnect on "connected but no first frame yet"
/// (except the bounded once-per-pageload profile fallback): the server
/// reliably delivers media to a stable consumer, so a frameless healthy
/// connection waits. Reconnecting in that window drove a multi-consumer
/// churn spiral (every reconnect's tee add/remove disrupted the other
/// displays, so they stalled and reconnected too — all black forever).
///
/// The closure handles are leaked via `forget()` because wasm-bindgen
/// `Closure` types are not `Send` and removing them on drop would require
/// keeping the original handles around in a `Send`-bounded `StoredValue` —
/// which doesn't fit. Instead we use an `active: Rc<Cell<bool>>` flag:
/// closures check it first and become no-ops once cleared (the rVFC chain
/// additionally stops rescheduling itself). `Watchdog::stop()` flips the
/// flag. The leaked closures consume only a few `Rc` clones each.
pub(crate) struct Watchdog {
    active: Rc<Cell<bool>>,
}

impl Watchdog {
    /// Real-freeze threshold: after playback has started, this long without
    /// a single PRESENTED frame triggers a reconnect. 10s tolerates render
    /// hiccups and heavy main-thread throttling — an actual freeze (zero
    /// frames at all) is unambiguous at this horizon.
    const STALL_NO_FRAME_MS: f64 = 10_000.0;
    /// True-no-decode horizon: ICE-connected with ZERO presented frames for
    /// this long after connect → the decoder is dead → profile fallback
    /// (bounded to once per page load).
    const NO_DECODE_FALLBACK_MS: f64 = 15_000.0;
    /// Beacon cadence driver tick (ms). Health decisions are frame-based;
    /// the tick only EVALUATES them and paces beacons. May fire late on
    /// throttled TVs — acceptable, the thresholds are 10-15s.
    const TICK_INTERVAL_MS: i32 = 1000;
    /// Presented-frame count at which the current profile mode is PROVEN to
    /// decode on this display and persisted to localStorage — but ONLY when
    /// those frames arrived within `PROVEN_MODE_WINDOW_MS` of the first one.
    const PROVEN_MODE_FRAMES: u32 = 100;
    /// Proven-mode RATE gate: the `PROVEN_MODE_FRAMES`th frame must land
    /// within this window after the FIRST presented frame (100 frames in
    /// ≤10s ≈ ≥10fps sustained). Without it, 100 frames at 1fps over 100s
    /// "proved" a broken mode (the prod Vestel-VP8 freeze-crawl) and made
    /// it sticky. A session that misses the window persists nothing — the
    /// next page load retries profile selection from the prior stored state.
    const PROVEN_MODE_WINDOW_MS: f64 = 10_000.0;
    /// rVFC-path beacon period (~15s at 30fps) — the reliable beacon channel
    /// on displays whose setInterval is throttled to near-silence (rVFC is
    /// compositor-driven and not throttled while video plays).
    const RVFC_BEACON_FRAME_PERIOD: u32 = 450;
    /// LAST-RESORT full-page-reload horizon. After this long with ZERO
    /// decoded frames despite the reconnect+backoff loop continuously retrying
    /// (#369 reconnect, #371 churn guard), the page itself is escalated with
    /// `window.location.reload()` (#401 — Fully Kiosk auto-reload replacement,
    /// adb-independent). This timer spans the WHOLE page session (NOT one
    /// Watchdog instance): it is reset only when a frame actually decodes, so
    /// a normal brief reconnect — which produces frames again within a few
    /// seconds — never approaches it. 60s ≫ STALL_NO_FRAME_MS (10s) +
    /// NO_DECODE_FALLBACK_MS (15s) + the 5s-capped backoff, so the reconnect
    /// path gets many full attempts before the page-level reload fires.
    pub(crate) const RELOAD_NO_FRAME_MS: f64 = 60_000.0;

    /// Install ICE-state listener + rVFC frame observer + health ticker.
    /// `on_failure` is called at most ONCE per Watchdog instance — after
    /// firing, all observers become no-ops (gated by the `active` flag).
    ///
    /// `escalation` is the PAGE-SESSION reload tracker shared across reconnect
    /// cycles (#401): the frame observer resets its timer on each decoded
    /// frame and the health ticker performs the last-resort full-page reload
    /// when video has been dead long enough that reconnect has demonstrably
    /// failed. It is passed in (not created here) precisely so it survives the
    /// Watchdog being recreated on every reconnect.
    pub(crate) fn install<F: Fn() + 'static>(
        video: &HtmlVideoElement,
        pc: &RtcPeerConnection,
        source_id: &str,
        escalation: &Rc<ReloadEscalation>,
        on_failure: F,
    ) -> Self {
        let active: Rc<Cell<bool>> = Rc::new(Cell::new(true));
        let on_failure = Rc::new(on_failure);

        install_ice_failure_listener(pc, Rc::clone(&active), Rc::clone(&on_failure));

        let now = now_ms();
        let stats = Rc::new(FrameStats {
            frames_presented: Cell::new(0),
            first_frame_at: Cell::new(0.0),
            last_frame_at: Cell::new(now),
            started_at: Cell::new(now),
            max_present_gap_ms: Cell::new(0.0),
            present_gaps_over100: Cell::new(0),
            presented_in_interval: Cell::new(0),
            interval_started_at: Cell::new(now),
        });
        let rvfc_supported =
            start_rvfc_frame_observer(video, pc, source_id, &active, &stats, escalation);
        if !rvfc_supported {
            leptos::logging::warn!(
                "watchdog: requestVideoFrameCallback unsupported — using currentTime frame proxy"
            );
        }
        start_health_ticker(
            video,
            pc,
            source_id,
            &active,
            &stats,
            rvfc_supported,
            escalation,
            on_failure,
        );

        Self { active }
    }

    /// Disable all observers. Idempotent. Calling `stop` after `on_failure`
    /// has already fired is a safe no-op.
    pub(crate) fn stop(&self) {
        self.active.set(false);
    }
}

/// LAST-RESORT escalation decision (#401): should the stage page perform a
/// full `window.location.reload()` because video has been dead for too long
/// despite the reconnect loop continuously retrying?
///
/// `ms_since_last_decoded_frame` is measured across the WHOLE page session
/// (it survives individual reconnect cycles — see `ReloadEscalation`), so the
/// only way it grows past `reload_threshold_ms` is a genuinely stuck stream
/// that reconnect alone has NOT recovered. A normal brief reconnect decodes
/// frames again within seconds and resets the timer well before the threshold.
///
/// Pure + side-effect-free so the escalation rule is unit-testable without a
/// browser (the wiring that calls `window.location.reload()` is in the health
/// ticker; this function only decides).
pub(crate) fn should_escalate_reload(
    ms_since_last_decoded_frame: f64,
    reload_threshold_ms: f64,
) -> bool {
    // Strictly greater-than so a tick landing exactly AT the threshold waits
    // one more tick (no off-by-one reload on the boundary). The page-session
    // timer is reset on every decoded frame, so reaching this point at all
    // means reconnect has not produced a single frame for the whole window.
    ms_since_last_decoded_frame > reload_threshold_ms
}

/// Resolve the no-frames reload horizon for THIS page load: the conservative
/// `RELOAD_NO_FRAME_MS` default, unless a `?ndiReloadMs=<n>` URL query lowers
/// it. The override is a read-only query param (no behavior change beyond the
/// timer length) used solely by the deterministic E2E to exercise the full
/// reload path without a real 60s wait — production stage pages never carry it.
/// Only a strictly-positive numeric value is honoured; anything else falls
/// back to the default.
fn reload_threshold_ms_from_url() -> f64 {
    let parsed = leptos::web_sys::window()
        .and_then(|w| w.location().search().ok())
        .and_then(|search| {
            leptos::web_sys::UrlSearchParams::new_with_str(&search)
                .ok()
                .and_then(|p| p.get("ndiReloadMs"))
        })
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| *v > 0.0);
    parsed.unwrap_or(Watchdog::RELOAD_NO_FRAME_MS)
}

/// Monotonic now in milliseconds: `performance.now()`, with a `Date.now()`
/// fallback when the Performance API is unavailable.
fn now_ms() -> f64 {
    leptos::web_sys::window()
        .and_then(|w| w.performance())
        .map(|p| p.now())
        .unwrap_or_else(js_sys::Date::now)
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
fn record_presented_frame(stats: &FrameStats) -> u32 {
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
fn snapshot_present_gaps(stats: &FrameStats) -> (f64, u32, Option<f64>) {
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
fn start_rvfc_frame_observer(
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
fn start_health_ticker<F: Fn() + 'static>(
    video: &HtmlVideoElement,
    pc: &RtcPeerConnection,
    source_id: &str,
    active: &Rc<Cell<bool>>,
    stats: &Rc<FrameStats>,
    rvfc_supported: bool,
    escalation: &Rc<ReloadEscalation>,
    on_failure: Rc<F>,
) {
    let active = Rc::clone(active);
    let stats = Rc::clone(stats);
    let video = video.clone();
    let pc = pc.clone();
    let source_id = source_id.to_string();
    let escalation = Rc::clone(escalation);
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
        maybe_post_beacon(&tick_count, &pc, &source_id, &stats);
        if !rvfc_supported
            && approximate_frame_from_current_time(&video, &stats, &last_current_time)
        {
            // The currentTime proxy is the rVFC-less browser's only frame
            // signal — reset the page-level reload timer ONLY when it actually
            // advanced (rVFC path resets in its own callback).
            escalation.note_decoded_frame();
        }
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
    if let Some(window) = leptos::web_sys::window() {
        let _ = window.set_interval_with_callback_and_timeout_and_arguments_0(
            cb.as_ref().unchecked_ref(),
            Watchdog::TICK_INTERVAL_MS,
        );
    }
    cb.forget();
}

/// Profile-fallback check (frame-based): a session that is ICE-connected with
/// ZERO presented frames `NO_DECODE_FALLBACK_MS` after connect has a dead
/// decoder (the broken Vestel H264 OMX symptom: connected, RTP flowing,
/// nothing presented). Switch the profile mode — bounded to ONCE per page
/// load, killing the mode ping-pong — and fire `on_failure` so the
/// reconnect requests the other profile (compat mode adds
/// `?profile=compat` to the WHEP POST URL — see `ndi_video::whep_url`).
fn maybe_profile_fallback<F: Fn() + 'static>(
    now: f64,
    stats: &FrameStats,
    pc: &RtcPeerConnection,
    active: &Rc<Cell<bool>>,
    on_failure: &Rc<F>,
) {
    if now - stats.started_at.get() < Watchdog::NO_DECODE_FALLBACK_MS {
        return;
    }
    // Only a CONNECTED session gets a profile verdict: pre-connect states mean
    // media never had a chance (ICE problems are the ICE listener's job).
    if !matches!(
        pc.ice_connection_state(),
        RtcIceConnectionState::Connected | RtcIceConnectionState::Completed
    ) {
        return;
    }
    let Some(new_mode) = switch_profile_mode_once() else {
        // One-shot spent this page load — keep waiting, never ping-pong.
        return;
    };
    leptos::logging::warn!(
        "profile fallback: 0 frames presented {}s after connect — switching to profile mode {new_mode} (once per page load)",
        Watchdog::NO_DECODE_FALLBACK_MS / 1000.0
    );
    active.set(false);
    (on_failure)();
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

/// Sample `pc.getStats()` and POST a beacon. Fire-and-forget; the beacon
/// must never disturb playback.
///
/// The present-gap accumulators are snapshotted-and-reset SYNCHRONOUSLY here
/// (before the async getStats), so each beacon reports exactly the interval
/// since the previous beacon — even though the actual POST happens later on
/// the spawned task.
fn post_stats_beacon(pc: &RtcPeerConnection, source_id: &str, stats: &FrameStats) {
    let (max_gap, over100, fps) = snapshot_present_gaps(stats);
    let pc = pc.clone();
    let source_id = source_id.to_string();
    spawn_local(async move {
        if let Ok(report) = JsFuture::from(pc.get_stats()).await {
            post_client_stats(&source_id, &report, max_gap, over100, fps).await;
        }
    });
}

/// Every 15th watchdog tick (~15s at 1s ticks — slower on throttled TVs,
/// where the rVFC frame-count beacon is the reliable channel instead),
/// post a stats beacon for `source_id`.
fn maybe_post_beacon(
    tick_count: &Cell<u32>,
    pc: &RtcPeerConnection,
    source_id: &str,
    stats: &FrameStats,
) {
    tick_count.set(tick_count.get().wrapping_add(1));
    if tick_count.get() % 15 != 0 {
        return;
    }
    post_stats_beacon(pc, source_id, stats);
}

/// Extract inbound-video stats from an RtcStatsReport (a JS Map) and POST a
/// compact summary to /ndi/client-stats. Fire-and-forget; errors ignored —
/// the beacon must never disturb playback.
///
/// `max_present_gap_ms` / `present_gaps_over100` / `presented_fps` are the
/// render-side presentation-cadence metrics for the interval since the last
/// beacon (already snapshotted-and-reset by the caller). They sit alongside
/// the decode-side getStats fields so a reader can tell a frame that decoded
/// on time but reached the screen late from a genuine decode stall.
async fn post_client_stats(
    source_id: &str,
    report: &JsValue,
    max_present_gap_ms: f64,
    present_gaps_over100: u32,
    presented_fps: Option<f64>,
) {
    let mut frames_decoded = JsValue::NULL;
    let mut fps = JsValue::NULL;
    let mut jb_delay = JsValue::NULL;
    let mut jb_emitted = JsValue::NULL;
    let mut freeze_count = JsValue::NULL;
    let mut frames_dropped = JsValue::NULL;
    let mut codec_id = JsValue::NULL;

    let map: &js_sys::Map = report.unchecked_ref();
    let entries = js_sys::try_iter(&map.values()).ok().flatten();
    if let Some(entries) = entries {
        for entry in entries.flatten() {
            let get = |k: &str| {
                js_sys::Reflect::get(&entry, &JsValue::from_str(k)).unwrap_or(JsValue::NULL)
            };
            if get("type").as_string().as_deref() == Some("inbound-rtp")
                && get("kind").as_string().as_deref() == Some("video")
            {
                frames_decoded = get("framesDecoded");
                fps = get("framesPerSecond");
                jb_delay = get("jitterBufferDelay");
                jb_emitted = get("jitterBufferEmittedCount");
                freeze_count = get("freezeCount");
                frames_dropped = get("framesDropped");
                codec_id = get("codecId");
            }
        }
    }

    // The negotiated codec: inbound-rtp's codecId is the report-map KEY of
    // the matching "codec" entry — look it up directly and read mimeType.
    let codec = codec_id
        .as_string()
        .map(|id| map.get(&JsValue::from_str(&id)))
        .and_then(|entry| js_sys::Reflect::get(&entry, &JsValue::from_str("mimeType")).ok())
        .and_then(|v| v.as_string());

    // Physical screen size, for telling TV models apart in the logs.
    let screen = leptos::web_sys::window()
        .and_then(|w| w.screen().ok())
        .and_then(|s| match (s.width(), s.height()) {
            (Ok(w), Ok(h)) => Some(format!("{w}x{h}")),
            _ => None,
        });

    // jitterBufferDelay is a cumulative sum of seconds each emitted frame
    // spent in the buffer; divide by the emitted count for the average, in ms.
    let jitter_buffer_ms = match (jb_delay.as_f64(), jb_emitted.as_f64()) {
        (Some(d), Some(n)) if n > 0.0 => Some(d / n * 1000.0),
        _ => None,
    };
    let body = serde_json::json!({
        "sourceId": source_id,
        "displayId": display_id(),
        "codec": codec,
        // Which stream profile this display requested ("default"/"compat").
        // The server serves ONE 720p H264 stream regardless of this value
        // (see `StreamProfile::from_query`); it is reported only to record
        // which watchdog mode the display was in when it sent this beacon —
        // there is no 640×480 / VP8 branch.
        "profile": profile_mode_name(profile_mode_is_compat()),
        "screen": screen,
        "framesDecoded": frames_decoded.as_f64(),
        "fps": fps.as_f64(),
        "jitterBufferMs": jitter_buffer_ms,
        "freezeCount": freeze_count.as_f64(),
        "framesDropped": frames_dropped.as_f64(),
        // Render-side presentation-cadence metrics for this beacon interval
        // (the decode-side fields above can't see a frame presented late).
        "maxPresentGapMs": max_present_gap_ms,
        "presentGapsOver100": present_gaps_over100,
        "presentedFps": presented_fps,
    })
    .to_string();

    let init = leptos::web_sys::RequestInit::new();
    init.set_method("POST");
    init.set_body(&JsValue::from_str(&body));
    let Ok(headers) = leptos::web_sys::Headers::new() else {
        return;
    };
    let _ = headers.set("Content-Type", "application/json");
    init.set_headers(&headers);
    let Ok(request) = leptos::web_sys::Request::new_with_str_and_init("/ndi/client-stats", &init)
    else {
        return;
    };
    if let Some(window) = leptos::web_sys::window() {
        let _ = JsFuture::from(window.fetch_with_request(&request)).await;
    }
}

/// ICE state listener for the Watchdog: fires `on_failure` once on
/// Failed / Disconnected / Closed (gated by the shared `active` flag).
fn install_ice_failure_listener<F: Fn() + 'static>(
    pc: &RtcPeerConnection,
    active: std::rc::Rc<std::cell::Cell<bool>>,
    on_failure: std::rc::Rc<F>,
) {
    let pc_clone = pc.clone();
    let cb = Closure::<dyn FnMut(JsValue)>::new(move |_ev: JsValue| {
        if !active.get() {
            return;
        }
        let s = pc_clone.ice_connection_state();
        if matches!(
            s,
            RtcIceConnectionState::Failed
                | RtcIceConnectionState::Disconnected
                | RtcIceConnectionState::Closed
        ) {
            leptos::logging::warn!("watchdog: ICE state={s:?}, triggering reconnect");
            active.set(false);
            (on_failure)();
        }
    });
    pc.set_oniceconnectionstatechange(Some(cb.as_ref().unchecked_ref()));
    cb.forget();
}

#[cfg(test)]
mod tests {
    use super::{should_escalate_reload, Watchdog};

    // ─────────────────────────────────────────────────────────────────────
    // #401 LAST-RESORT page-reload escalation.
    //
    // The stage page recovers a frozen/black/disconnected stream by reconnect
    // alone in the common case (#369/#371). But some failures — a wedged TV
    // WebView, a stale DOM, a WHEP negotiation that never produces frames
    // again — survive every reconnect attempt forever (the reconnect loop in
    // `ndi_video.rs` loops indefinitely with no page-level escape). The
    // escalation rule decides when to give up on reconnect and reload the
    // whole page (fresh WHEP negotiation + fresh DOM) — the adb-independent
    // replacement for the Fully Kiosk auto-reload we lost on com.tcl.browser.
    //
    // The rule is deliberately conservative: it fires ONLY after a long
    // no-decoded-frames window that a normal reconnect cannot reach, so it
    // never short-circuits the healthy reconnect path.
    // ─────────────────────────────────────────────────────────────────────

    const T: f64 = Watchdog::RELOAD_NO_FRAME_MS;

    #[test]
    fn no_reload_while_frames_are_recent() {
        // Frames decoding right now (timer just reset) — never reload.
        assert!(!should_escalate_reload(0.0, T));
    }

    #[test]
    fn no_reload_during_a_normal_brief_reconnect() {
        // A normal reconnect (ICE drop -> reconnect -> frames) is well under
        // the threshold: even a slow reconnect that takes 15s to produce a
        // frame must NOT trigger a reload — reconnect is doing its job.
        assert!(!should_escalate_reload(15_000.0, T));
        // Even right up to the profile-fallback + a couple of backoff cycles.
        assert!(!should_escalate_reload(30_000.0, T));
    }

    #[test]
    fn no_reload_exactly_at_threshold() {
        // Strictly-greater-than boundary: AT the threshold we still wait one
        // more tick (avoids an off-by-one reload on the very first tick that
        // crosses 60s vs 60.000s).
        assert!(!should_escalate_reload(T, T));
    }

    #[test]
    fn reload_after_prolonged_no_decoded_frames() {
        // 60s+ with ZERO decoded frames despite reconnect retrying the whole
        // time -> reconnect has demonstrably failed -> escalate to a full
        // page reload. THIS is the assertion that fails against the RED stub.
        assert!(should_escalate_reload(T + 1.0, T));
        assert!(should_escalate_reload(90_000.0, T));
        assert!(should_escalate_reload(600_000.0, T));
    }

    #[test]
    fn threshold_is_well_above_the_reconnect_path_budget() {
        // The reload horizon MUST exceed the worst-case single reconnect
        // budget (stall detect 10s + no-decode fallback 15s + 5s-capped
        // backoff) by a wide margin, so reconnect gets several full attempts
        // before the page-level reload ever fires. Guards against a future
        // edit lowering RELOAD_NO_FRAME_MS into the reconnect window.
        let reconnect_path_budget = Watchdog::STALL_NO_FRAME_MS + Watchdog::NO_DECODE_FALLBACK_MS;
        assert!(
            Watchdog::RELOAD_NO_FRAME_MS > reconnect_path_budget * 2.0,
            "reload horizon {} must be >2x the reconnect path budget {}",
            Watchdog::RELOAD_NO_FRAME_MS,
            reconnect_path_budget,
        );
    }
}
