//! Frame-based health watchdog + page-session reload escalation for `NdiVideo`.
//!
//! This file holds the watchdog CORE: the `Watchdog` lifecycle (install / stop /
//! drop), the page-session `ReloadEscalation` (#401 last-resort reload), the ICE
//! failure listener, and the shared `now_ms()` clock + tuning constants. The
//! frame counters, profile-fallback, beacons, and health ticker live in focused
//! sibling modules (`ndi_frame_stats`, `ndi_profile`, `ndi_beacon`,
//! `ndi_health_ticker`) — split out (#418) to keep every file well under the
//! 1000-line hard cap. The server-liveness `/healthz` gate is `ndi_reload_guard`.
//!
//! Frame observation is driven by `requestVideoFrameCallback` (fires once per
//! frame actually PRESENTED to the compositor, NOT throttled like setInterval on
//! TV WebViews); a 1s ticker evaluates the stall and profile-fallback rules
//! against those counters and paces the stats beacons.

use std::cell::Cell;
use std::rc::Rc;

use leptos::wasm_bindgen::{closure::Closure, JsCast, JsValue};
use leptos::web_sys::{HtmlVideoElement, RtcIceConnectionState, RtcPeerConnection};
use wasm_bindgen_futures::spawn_local;

use super::ndi_clock_offset::{self, ClockOffsetEstimator, ClockOffsetSetter};
use super::ndi_frame_stats::{
    start_rvfc_frame_observer, DroppedFramesSetter, FrameStats, FramesLiveSetter,
    VideoLatencySetter,
};
use super::ndi_health_ticker::start_health_ticker;

// Re-export the profile-mode query so `ndi_video.rs` keeps importing it from
// this module's public surface (it owns the whep_url profile decision).
pub(crate) use super::ndi_profile::profile_mode_is_compat;

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
    /// True while a `/healthz` server-liveness check spawned by `maybe_reload`
    /// is in flight (#410). Guards against the 1s health ticker spawning a
    /// fresh fetch on every tick once the no-frames horizon is crossed — at
    /// most one check is outstanding at a time. Cleared when the check resolves.
    check_in_flight: Cell<bool>,
    /// Page-session liveness flag (#417). `true` for the whole page session;
    /// flipped to `false` ONLY by the PAGE-level teardown (`NdiVideo`'s
    /// `on_cleanup`, via `cancel()`), NEVER by `Watchdog::stop()`/`Drop`.
    ///
    /// Load-bearing distinction: `Watchdog::stop()`/`Drop` fire on EVERY
    /// reconnect cycle, but the `escalation` is page-session (created once and
    /// shared by `&Rc` into each `Watchdog::install`), so it OUTLIVES every
    /// Watchdog. A `/healthz` check spawned by `maybe_reload` just before
    /// teardown holds only an `Rc<ReloadEscalation>`; after its `.await` it
    /// could otherwise still call `window.location().reload()` AFTER the page
    /// is being torn down. Gating that post-await reload on `active` closes the
    /// window — while tying the flag to page teardown (not Watchdog teardown)
    /// keeps the #401 last-resort reload alive across normal reconnects.
    active: Cell<bool>,
    /// The no-frames horizon (ms) for THIS page load. Defaults to
    /// `RELOAD_NO_FRAME_MS`; an explicit `?ndiReloadMs=<n>` URL query lowers
    /// it so the E2E can exercise the full reload path (incl. the real
    /// `window.location.reload()`) deterministically without a 60s wait. The
    /// param is read ONCE here; production pages never carry it, so prod always
    /// uses the conservative 60s.
    threshold_ms: f64,
    /// TEST-ONLY (#422): when set (via `?ndiReloadSkipHealthz=1`), the
    /// last-resort reload bypasses the #410 `/healthz` gate and fires on the
    /// no-frames horizon alone. Prod pages never set it. Read ONCE.
    skip_healthz_gate: bool,
}

impl ReloadEscalation {
    /// Create the page-session escalation tracker, seeding the
    /// last-decoded-frame timestamp to "now" (the page just loaded, so the
    /// reload horizon is measured from page start until the first frame).
    pub(crate) fn new() -> Rc<Self> {
        Rc::new(Self {
            last_decoded_frame_at: Cell::new(now_ms()),
            reloaded: Cell::new(false),
            check_in_flight: Cell::new(false),
            active: Cell::new(true),
            threshold_ms: reload_threshold_ms_from_url(),
            skip_healthz_gate: super::ndi_reload_guard::reload_skip_healthz_from_url(),
        })
    }

    /// Record that a frame just decoded — resets the page-level reload timer.
    /// Called from the frame path of EVERY session so a brief reconnect that
    /// resumes decoding clears the escalation well before the threshold.
    pub(crate) fn note_decoded_frame(&self) {
        self.last_decoded_frame_at.set(now_ms());
    }

    /// PAGE-level teardown signal (#417): mark the page session as gone so any
    /// in-flight `/healthz` check spawned by `maybe_reload` does NOT reload
    /// after the page is being torn down. Called ONLY from `NdiVideo`'s
    /// `on_cleanup` (where the page `cancelled` flag is set) — NOT from
    /// `Watchdog::stop()`/`Drop`, which fire on every reconnect and would
    /// otherwise permanently suppress the #401 last-resort reload.
    pub(crate) fn cancel(&self) {
        self.active.set(false);
    }

    /// Decide, AFTER the `/healthz` check resolves, whether the detached task
    /// should perform the last-resort `window.location().reload()`. Gathers the
    /// page-session cells + the `now_ms()`-based horizon and delegates to the
    /// pure `should_perform_post_await_reload` (host-testable in
    /// `ndi_reload_guard`). The #417 `active` gate is the first argument — a
    /// torn-down page (cancel()) is never reloaded by an in-flight check.
    fn should_reload_after_check(&self, server_has_streaming: bool) -> bool {
        super::ndi_reload_guard::should_perform_post_await_reload(
            self.active.get(),
            self.reloaded.get(),
            super::ndi_reload_guard::should_escalate_reload(
                self.ms_since_last_decoded_frame(),
                self.threshold_ms,
            ),
            server_has_streaming,
        )
    }

    /// Milliseconds since any session last decoded a frame (page-session).
    fn ms_since_last_decoded_frame(&self) -> f64 {
        now_ms() - self.last_decoded_frame_at.get()
    }

    /// Evaluate the escalation rule. When the no-frames horizon is crossed,
    /// the reload is NOT performed unconditionally: first a lightweight
    /// `/healthz` check decides whether reloading can even help (#410).
    ///
    /// - If the SERVER has an actively-streaming NDI pipeline but THIS consumer
    ///   still has no frames → the page/consumer is genuinely stuck → reload
    ///   (the #401 recovery).
    /// - If the server has NO streaming pipeline → the source itself is down
    ///   (Resolume silent), reloading cannot conjure frames → SKIP the reload
    ///   and reset the escalation timer so it re-evaluates after another full
    ///   window instead of reloading every ~60s forever.
    /// - If the `/healthz` fetch fails → fall back to the existing behavior and
    ///   reload (a fetch error must never SUPPRESS a genuinely-needed reload).
    ///
    /// Returns true once the page is reloading OR a server check has been
    /// spawned for this horizon crossing — in both cases the caller should stop
    /// the rest of this tick (the async check, if any, owns the decision).
    /// Idempotent: the `reloaded` one-shot and the `check_in_flight` guard make
    /// repeated ticks / multiple sessions fire at most one `/healthz` check and
    /// at most one `window.location.reload()`.
    pub(crate) fn maybe_reload(self: &Rc<Self>) -> bool {
        if self.reloaded.get() {
            return true;
        }
        if !super::ndi_reload_guard::should_escalate_reload(
            self.ms_since_last_decoded_frame(),
            self.threshold_ms,
        ) {
            return false;
        }
        // TEST-ONLY (#422): bypass the #410 `/healthz` gate so the E2E exercises
        // the real `window.location.reload()` on the no-frames horizon alone.
        // Prod stage pages never carry `?ndiReloadSkipHealthz`, so this branch is
        // dead in production — the full gated path below is unchanged there.
        if self.skip_healthz_gate {
            if self.reloaded.replace(true) {
                return true;
            }
            leptos::logging::warn!(
                "watchdog: no decoded frame for {:.0}ms — LAST-RESORT reload (healthz gate bypassed, test-only #422)",
                self.ms_since_last_decoded_frame()
            );
            if let Some(window) = leptos::web_sys::window() {
                let _ = window.location().reload();
            }
            return true;
        }
        // Horizon crossed. Don't spawn a second check while one is in flight.
        if self.check_in_flight.get() {
            return true;
        }
        self.check_in_flight.set(true);
        let escalation = Rc::clone(self);
        spawn_local(async move {
            let has_streaming =
                super::ndi_reload_guard::fetch_healthz_has_streaming_pipeline().await;
            escalation.check_in_flight.set(false);
            // The post-await reload decision (incl. the #417 page-teardown gate)
            // is centralized in `should_reload_after_check`. If it says no, the
            // task does NOT reload — but a still-live page whose SOURCE is down
            // resets its timer so #410 re-evaluates after another full window
            // (instead of looping reloads every ~60s).
            if !escalation.should_reload_after_check(has_streaming) {
                if escalation.active.get()
                    && !escalation.reloaded.get()
                    && super::ndi_reload_guard::should_escalate_reload(
                        escalation.ms_since_last_decoded_frame(),
                        escalation.threshold_ms,
                    )
                    && !super::ndi_reload_guard::should_reload_given_pipeline_state(has_streaming)
                {
                    leptos::logging::warn!(
                        "watchdog: no decoded frame for {:.0}ms but server has NO streaming pipeline — source is down, SKIPPING reload (#410)",
                        escalation.ms_since_last_decoded_frame()
                    );
                    escalation.last_decoded_frame_at.set(now_ms());
                }
                return;
            }
            escalation.reloaded.set(true);
            leptos::logging::warn!(
                "watchdog: no decoded frame for {:.0}ms across reconnect attempts AND server has a streaming pipeline — LAST-RESORT full page reload (#401)",
                escalation.ms_since_last_decoded_frame()
            );
            if let Some(window) = leptos::web_sys::window() {
                let _ = window.location().reload();
            }
        });
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
    /// `setInterval` handle for the health ticker, cleared on `stop()`/drop.
    /// Clearing it (not just flipping `active`) is required because
    /// `maybe_reload()` runs BEFORE the `active` gate (#401): a leaked,
    /// never-cleared ticker on a torn-down Watchdog would keep evaluating the
    /// escalation and fire a spurious `window.location.reload()` ~`RELOAD_NO_FRAME_MS`
    /// after the stage is deactivated/unmounted. Also stops ticker accumulation
    /// across reconnects (each replaced Watchdog clears its own).
    health_ticker_handle: i32,
}

impl Watchdog {
    /// Real-freeze threshold: after playback has started, this long without
    /// a single PRESENTED frame triggers a reconnect. 10s tolerates render
    /// hiccups and heavy main-thread throttling — an actual freeze (zero
    /// frames at all) is unambiguous at this horizon.
    pub(crate) const STALL_NO_FRAME_MS: f64 = 10_000.0;
    /// True-no-decode horizon: ICE-connected with ZERO presented frames for
    /// this long after connect → the decoder is dead → profile fallback
    /// (bounded to once per page load).
    pub(crate) const NO_DECODE_FALLBACK_MS: f64 = 15_000.0;
    /// Beacon cadence driver tick (ms). Health decisions are frame-based;
    /// the tick only EVALUATES them and paces beacons. May fire late on
    /// throttled TVs — acceptable, the thresholds are 10-15s.
    pub(crate) const TICK_INTERVAL_MS: i32 = 1000;
    /// Presented-frame count at which the current profile mode is PROVEN to
    /// decode on this display and persisted to localStorage — but ONLY when
    /// those frames arrived within `PROVEN_MODE_WINDOW_MS` of the first one.
    pub(crate) const PROVEN_MODE_FRAMES: u32 = 100;
    /// Proven-mode RATE gate: the `PROVEN_MODE_FRAMES`th frame must land
    /// within this window after the FIRST presented frame (100 frames in
    /// ≤10s ≈ ≥10fps sustained). Without it, 100 frames at 1fps over 100s
    /// "proved" a broken mode (the prod Vestel-VP8 freeze-crawl) and made
    /// it sticky. A session that misses the window persists nothing — the
    /// next page load retries profile selection from the prior stored state.
    pub(crate) const PROVEN_MODE_WINDOW_MS: f64 = 10_000.0;
    /// rVFC-path beacon period (~15s at 30fps) — the reliable beacon channel
    /// on displays whose setInterval is throttled to near-silence (rVFC is
    /// compositor-driven and not throttled while video plays).
    pub(crate) const RVFC_BEACON_FRAME_PERIOD: u32 = 450;
    /// LAST-RESORT full-page-reload horizon. After this long with ZERO
    /// decoded frames despite the reconnect+backoff loop continuously retrying,
    /// the page itself is escalated with `window.location.reload()` (#401 —
    /// Fully Kiosk auto-reload replacement, adb-independent). This timer spans
    /// the WHOLE page session (NOT one Watchdog instance): it is reset only when
    /// a frame actually decodes, so a normal brief reconnect — which produces
    /// frames again within a few seconds — never approaches it.
    ///
    /// 60s ≫ STALL_NO_FRAME_MS (10s) + NO_DECODE_FALLBACK_MS (15s) + the
    /// 5s-capped reconnect backoff. The backoff (`reconnect_backoff_for_watchdog
    /// _step` in `ndi_video.rs`) is applied to BOTH reconnect paths — the
    /// connect-error branch AND the watchdog-triggered reconnect fall-through
    /// (#369) — so even a connect-but-never-decode source reconnects at most
    /// once every 5s, never every cycle with no delay (#371 churn guard). The
    /// reconnect path therefore gets many full attempts before this page-level
    /// reload fires.
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
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn install<F: Fn() + 'static>(
        video: &HtmlVideoElement,
        pc: &RtcPeerConnection,
        source_id: &str,
        escalation: &Rc<ReloadEscalation>,
        video_latency_setter: Option<VideoLatencySetter>,
        frames_live_setter: Option<FramesLiveSetter>,
        clock_offset_setter: Option<ClockOffsetSetter>,
        dropped_frames_setter: Option<DroppedFramesSetter>,
        on_failure: F,
    ) -> Self {
        let active: Rc<Cell<bool>> = Rc::new(Cell::new(true));
        let on_failure = Rc::new(on_failure);

        install_ice_failure_listener(pc, Rc::clone(&active), Rc::clone(&on_failure));

        let stats = FrameStats::new(now_ms());
        // #500: a freshly-installed session has NOT decoded a frame yet, and its
        // per-session `stats.frames_live` cell starts false. Reset the shared
        // page signal to match, so a stale `true` left by a prior (now-torn-down)
        // session can never survive into a connect-but-never-decode session and
        // wrongly hide the neutral cover. The first presented frame re-marks it
        // true; on a healthy mid-stream reconnect the status is `connected`
        // (no cover) so this never flashes the cover.
        if let Some(setter) = &frames_live_setter {
            setter(false);
        }
        // #523: a freshly-installed session hasn't posted a beacon yet, so any
        // dropped/freeze count still on screen is from the prior (torn-down)
        // session. Clear it rather than show a stale figure — the next beacon
        // (up to ~15s away) repopulates it honestly.
        if let Some(setter) = &dropped_frames_setter {
            setter(None);
        }
        // #510/#512: the browser<->server pipeline-clock offset estimator. Created
        // BEFORE the frame observer so the observer can read its current (offset,
        // RTT) each presented frame — the RTT/2 network-transit term of the TRUE
        // server→display latency (#512, T4).
        let clock_offset_estimator = ClockOffsetEstimator::new();
        let rvfc_supported = start_rvfc_frame_observer(
            video,
            pc,
            source_id,
            &active,
            &stats,
            escalation,
            &clock_offset_estimator,
            video_latency_setter,
            // #500: the rVFC path marks frames live on each presented frame; the
            // health ticker (below) flips them back to not-live on staleness, so
            // BOTH share the same per-session setter.
            frames_live_setter.clone(),
            // #523: BOTH beacon paths (rVFC frame-count-driven and the 1s
            // ticker) can post a stats beacon, so both share the same setter.
            dropped_frames_setter.clone(),
        );
        if !rvfc_supported {
            leptos::logging::warn!(
                "watchdog: requestVideoFrameCallback unsupported — using currentTime frame proxy"
            );
        }
        // #510 (T3): an INDEPENDENT rVFC-driven loop (own registration on the
        // same `video` element) doing the browser<->server pipeline-clock
        // offset handshake. Independent of the frame-stats observer above —
        // it needs no decode-side counters, just a periodic tick that survives
        // TV WebView timer throttling the same way. Feeds the SAME estimator the
        // observer reads, so the RTT it measures becomes the latency's network term.
        ndi_clock_offset::start(video, &active, &clock_offset_estimator, clock_offset_setter);
        let health_ticker_handle = start_health_ticker(
            video,
            pc,
            source_id,
            &active,
            &stats,
            rvfc_supported,
            escalation,
            &clock_offset_estimator,
            frames_live_setter,
            dropped_frames_setter,
            on_failure,
        );

        Self {
            active,
            health_ticker_handle,
        }
    }

    /// Disable all observers. Idempotent. Calling `stop` after `on_failure`
    /// has already fired is a safe no-op.
    pub(crate) fn stop(&self) {
        self.active.set(false);
        // Clear the leaked health ticker so it stops evaluating `maybe_reload()`
        // (which runs before the `active` gate). `clear_interval_with_handle` on
        // an already-cleared / -1 handle is a harmless no-op.
        if let Some(window) = leptos::web_sys::window() {
            window.clear_interval_with_handle(self.health_ticker_handle);
        }
    }
}

impl Drop for Watchdog {
    /// Inert the Watchdog when it is replaced on reconnect or dropped on
    /// unmount. Without this the leaked 1s health ticker keeps running and,
    /// because `maybe_reload()` is evaluated before the `active` gate (#401),
    /// would fire a spurious full-page reload after the stage is deactivated.
    fn drop(&mut self) {
        self.stop();
    }
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
pub(crate) fn now_ms() -> f64 {
    leptos::web_sys::window()
        .and_then(|w| w.performance())
        .map(|p| p.now())
        .unwrap_or_else(js_sys::Date::now)
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
    use super::super::ndi_reload_guard::should_escalate_reload;
    use super::Watchdog;

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
