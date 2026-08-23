//! Bounded playback-recovery guard for `NdiVideo`'s `<video>` element (#568).
//!
//! Android TV / weak-WebView browsers draw a big native "start playback"
//! overlay over a `<video>` element the moment it sits in a PAUSED state —
//! a WHEP stream that stalls mid-session (`pause`/`suspend`/`ended` fire on
//! the element even though playback SHOULD continue), or the page/app
//! returning from background (Android TV WebViews often suspend/resume
//! without ever firing `pause`). A rejected INITIAL autoplay `.play()` (the
//! ticket's #1 named cause) is covered indirectly: `attach_ontrack`
//! (`ndi_video.rs`) already asserts `muted` before that first `.play()`, and
//! if a genuine rejection ever leaves the element paused, the browser itself
//! typically follows with a `pause` event — this guard's `pause` listener
//! then re-asserts `muted` and retries, same as any other stall.
//! CSS suppression (`stage.css`) hides the affordance as defense-in-depth, but
//! the root cause is that nothing re-initiates playback — so the element can
//! sit paused indefinitely. This module detects the pause and re-calls
//! `.play()` immediately, bounded so a persistently-broken source doesn't spin
//! `.play()` forever: the existing frame-based `Watchdog` (`ndi_watchdog.rs`)
//! already escalates — reconnect, then a last-resort page reload — for a
//! source that truly cannot recover, so this guard backs off and defers to it
//! rather than retrying without limit.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use leptos::wasm_bindgen::{closure::Closure, JsCast, JsValue};
use leptos::web_sys::HtmlVideoElement;
use wasm_bindgen_futures::{spawn_local, JsFuture};

use super::ndi_watchdog::now_ms;

// #732 diagnostics: window global holding this guard's CUMULATIVE `.play()`
// replay count, read by the stage-display diagnostics collector
// (`ws/stage_diag.rs`) so the owner can see how often the guard fired on a
// given TV. Cumulative across the page session (survives NdiVideo remounts).
// The key name is owned by the collector (single source of truth).
use crate::ws::stage_diag::GUARD_REPLAYS_GLOBAL;

fn guard_replays_global() -> Option<f64> {
    js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str(GUARD_REPLAYS_GLOBAL))
        .ok()
        .and_then(|v| v.as_f64())
}

fn set_guard_replays_global(value: f64) {
    let _ = js_sys::Reflect::set(
        &js_sys::global(),
        &JsValue::from_str(GUARD_REPLAYS_GLOBAL),
        &JsValue::from_f64(value),
    );
}

/// Initialise the replay counter to 0 when a guard is first installed (only if
/// unset) so the collector can distinguish "guard active, 0 replays" (Some(0))
/// from "no guard installed" (None) — without resetting a cumulative count on
/// a later remount.
fn init_guard_replays_global() {
    if guard_replays_global().is_none() {
        set_guard_replays_global(0.0);
    }
}

/// Increment the cumulative replay counter (#732 diagnostics).
fn bump_guard_replays_global() {
    set_guard_replays_global(guard_replays_global().unwrap_or(0.0) + 1.0);
}

/// At most this many `.play()` replay attempts within any `RETRY_WINDOW_MS`
/// window. Beyond that, this guard stops trying and lets the frame-based
/// `Watchdog` escalate instead — a source that is truly dead needs a
/// RECONNECT, not a tighter replay loop hammering `.play()`.
///
/// Note: `RetryWindow` is a FIXED window that resets once fully elapsed, not
/// a true sliding window — a burst straddling the boundary (attempts just
/// before reset, more just after) can allow up to `2×MAX_RETRIES_PER_WINDOW`
/// in a short span. Acceptable here: the frame-based `Watchdog` is the real
/// backstop for a persistently-broken source, so this budget only needs to
/// be "roughly bounded," not exact.
pub(crate) const MAX_RETRIES_PER_WINDOW: usize = 5;
pub(crate) const RETRY_WINDOW_MS: f64 = 30_000.0;

/// Fixed-window bookkeeping for the bounded replay retries (see the
/// `MAX_RETRIES_PER_WINDOW` note on the fixed-vs-rolling distinction). Pure
/// state, no DOM — kept separate from the DOM wiring so the decision logic is
/// unit-testable without a browser.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RetryWindow {
    attempts: usize,
    window_start_ms: f64,
}

impl RetryWindow {
    pub(crate) fn new(now_ms: f64) -> Self {
        Self {
            attempts: 0,
            window_start_ms: now_ms,
        }
    }
}

/// Decide whether a pause/ended/suspend/visibility-restore event observed at
/// `now_ms` should trigger another `.play()` attempt, advancing `state`'s
/// fixed-window bookkeeping. A window with no activity for `RETRY_WINDOW_MS`
/// resets the attempt counter — a source that recovered and later blips
/// again gets a fresh budget rather than staying permanently capped by an
/// old failure streak. Pure + unit-tested.
pub(crate) fn should_attempt_replay(state: &mut RetryWindow, now_ms: f64) -> bool {
    if now_ms - state.window_start_ms >= RETRY_WINDOW_MS {
        state.attempts = 0;
        state.window_start_ms = now_ms;
    }
    if state.attempts >= MAX_RETRIES_PER_WINDOW {
        return false;
    }
    state.attempts += 1;
    true
}

/// Install the bounded pause/ended/suspend/visibility-restore replay guard on
/// `video`. Attaches listeners for the element's lifetime — one `<video>` per
/// `NdiVideo` mount; reconnects reuse the same element (see `ndi_video.rs`),
/// so this is called ONCE per mount, not per WHEP session.
///
/// Returns a [`PlaybackGuardHandle`] the caller MUST dispose of (from
/// `on_cleanup`) when the mount that owns `video` unmounts. `<NdiVideo>` is
/// NOT page-load-scoped — `<Show>` (`ndi_fullscreen.rs`) tears the component
/// down and remounts it on every source (de)activation within one page load.
/// An earlier version of this function `forget()`-leaked these closures under
/// the false assumption that a `NdiVideo` mount lifetime coincides with the
/// page lifetime; in fact every unmount/remount cycle leaked one more
/// document-level `visibilitychange` listener plus the detached `<video>`
/// element it held a strong clone of, unbounded, for the life of the page
/// (#637). The caller (`ndi_video.rs`) stores the handle and calls
/// `.dispose()` on it from `on_cleanup`, symmetric with how the WHEP session
/// and watchdog are torn down there.
pub(crate) fn install(video: &HtmlVideoElement) -> PlaybackGuardHandle {
    init_guard_replays_global();
    let state = Rc::new(RefCell::new(RetryWindow::new(now_ms())));
    let mut video_listeners: Vec<(&'static str, Closure<dyn FnMut()>)> = Vec::with_capacity(3);

    for event_name in ["pause", "ended", "suspend"] {
        let video_clone = video.clone();
        let state_clone = Rc::clone(&state);
        let cb = Closure::<dyn FnMut()>::new(move || {
            replay_if_within_budget(&video_clone, &state_clone, event_name);
        });
        let _ = video.add_event_listener_with_callback(event_name, cb.as_ref().unchecked_ref());
        video_listeners.push((event_name, cb));
    }

    // The page/app returning to the foreground (Android TV WebViews often
    // suspend/resume without firing pause/suspend/ended of their own) can
    // leave the element paused with no element-level event at all — recheck
    // on every visibility restore. `replay_if_within_budget` itself checks
    // `video.paused()` first, so a visibility flip while genuinely playing is
    // a cheap no-op.
    let document = leptos::web_sys::window().and_then(|w| w.document());
    let mut document_listener = None;
    if let Some(document) = &document {
        let video_clone = video.clone();
        let state_clone = Rc::clone(&state);
        let document_for_check = document.clone();
        let cb = Closure::<dyn FnMut()>::new(move || {
            if !document_for_check.hidden() {
                replay_if_within_budget(&video_clone, &state_clone, "visibilitychange");
            }
        });
        let _ = document
            .add_event_listener_with_callback("visibilitychange", cb.as_ref().unchecked_ref());
        document_listener = Some(cb);
    }

    PlaybackGuardHandle {
        video: video.clone(),
        video_listeners,
        document,
        document_listener,
        disposed: Cell::new(false),
    }
}

/// Disposer returned by [`install`]. Owns the guard's event-listener
/// `Closure`s — NOT `forget()`-leaked — so [`dispose`](Self::dispose) can
/// remove them from the DOM before dropping them. ALWAYS call `dispose()`
/// explicitly from `on_cleanup` when the `<NdiVideo>` mount that owns the
/// guarded `video` unmounts (see `ndi_video.rs`) — the `Drop` impl below is
/// only a defense-in-depth safety net (same posture as `Watchdog`'s
/// `stop()`/`Drop` pairing in `ndi_watchdog.rs`), never the primary teardown
/// path. If a handle is EVER dropped without `dispose()` (e.g. a future bug
/// that replaces a live, undisposed handle in `ndi_video.rs`'s
/// `playback_guard_holder`), a still-registered listener firing on the
/// now-destroyed `wasm_bindgen` `Closure` PANICS ("closure invoked
/// recursively or destroyed already") — it does not merely leak — which is
/// exactly what `Drop` here prevents by removing the listeners first.
pub(crate) struct PlaybackGuardHandle {
    video: HtmlVideoElement,
    video_listeners: Vec<(&'static str, Closure<dyn FnMut()>)>,
    document: Option<leptos::web_sys::Document>,
    document_listener: Option<Closure<dyn FnMut()>>,
    /// Guards against `remove_listeners` actually touching the DOM twice.
    /// `dispose(self)` consumes `self` by value — when it returns, `self`
    /// still runs through Rust's normal drop glue, so `Drop::drop` fires
    /// RIGHT AFTER `dispose`'s own explicit call, on every ordinary
    /// teardown, not only the "forgotten dispose" fallback case the struct
    /// doc above describes. Without this flag `remove_listeners` — and thus
    /// each `removeEventListener` — ran twice per unmount: harmless to the
    /// DOM itself (a documented no-op on an already-removed listener) but it
    /// defeats `tests/e2e/stage-ndi-playback-guard.spec.ts`'s net
    /// add/remove-count assertion (#637), which is the exact regression
    /// this flag closes.
    disposed: Cell<bool>,
}

impl PlaybackGuardHandle {
    /// Remove every listener this guard installed — but only the FIRST time
    /// this is called for a given handle. `dispose()` and `Drop` both call
    /// this; the `disposed` flag makes the second call (whichever path
    /// fires it) a genuine no-op instead of re-issuing `removeEventListener`
    /// for listeners already removed.
    fn remove_listeners(&self) {
        if self.disposed.replace(true) {
            return;
        }
        for (event_name, cb) in &self.video_listeners {
            let _ = self
                .video
                .remove_event_listener_with_callback(event_name, cb.as_ref().unchecked_ref());
        }
        if let (Some(document), Some(cb)) = (&self.document, &self.document_listener) {
            let _ = document.remove_event_listener_with_callback(
                "visibilitychange",
                cb.as_ref().unchecked_ref(),
            );
        }
    }

    /// Remove every listener this guard installed, then drop the closures.
    /// This is the PRIMARY teardown path — call it explicitly on unmount.
    pub(crate) fn dispose(self) {
        self.remove_listeners();
    }
}

/// Safety net mirroring `Watchdog`'s `stop()`/`Drop` pairing (`ndi_watchdog.rs`):
/// removes the listeners even if a handle is ever dropped without an explicit
/// `dispose()` call, so a stale registered listener can never fire on a
/// destroyed `Closure` (see the struct doc above for why that would panic).
/// Guarded by `disposed` so this is a true no-op on the (normal) path where
/// `dispose()` already ran `remove_listeners()`.
impl Drop for PlaybackGuardHandle {
    fn drop(&mut self) {
        self.remove_listeners();
    }
}

/// If `video` is actually paused and the fixed-window budget allows it,
/// re-assert `muted` (matches `attach_ontrack`'s Chrome-autoplay-policy fix —
/// a stream reassigned or renegotiated could reset the live `muted` property)
/// and re-call `.play()`. Silently no-ops when the element isn't paused (nothing
/// to recover) or the retry budget is exhausted (logs once, defers to the
/// frame-based Watchdog).
fn replay_if_within_budget(
    video: &HtmlVideoElement,
    state: &Rc<RefCell<RetryWindow>>,
    source: &'static str,
) {
    if !video.paused() {
        return;
    }
    let should_retry = should_attempt_replay(&mut state.borrow_mut(), now_ms());
    if !should_retry {
        leptos::logging::warn!(
            "ndi_playback_guard: {source} exceeded {MAX_RETRIES_PER_WINDOW} replay \
             attempts in {RETRY_WINDOW_MS}ms — leaving recovery to the frame watchdog"
        );
        return;
    }
    bump_guard_replays_global();
    video.set_muted(true);
    play_and_log(video, format!("ndi_playback_guard: replay after {source}"));
}

/// Call `.play()` on `video` and log (WARN, never propagated) if the returned
/// promise rejects or the call itself throws. Shared by `ndi_video.rs`'s
/// initial `attach_ontrack` play and this module's replay above — both need
/// identical "fire, await, log on failure" handling, and duplicating it was
/// flagged in review (PR #579).
pub(crate) fn play_and_log(video: &HtmlVideoElement, context: String) {
    match video.play() {
        Ok(promise) => {
            spawn_local(async move {
                if let Err(e) = JsFuture::from(promise).await {
                    leptos::logging::warn!("{context}: play() rejected: {e:?}");
                }
            });
        }
        Err(e) => {
            leptos::logging::warn!("{context}: play() threw: {e:?}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{should_attempt_replay, RetryWindow, MAX_RETRIES_PER_WINDOW, RETRY_WINDOW_MS};

    #[test]
    fn allows_up_to_the_cap_within_one_window() {
        let mut state = RetryWindow::new(0.0);
        for i in 0..MAX_RETRIES_PER_WINDOW {
            assert!(
                should_attempt_replay(&mut state, i as f64 * 10.0),
                "attempt {i} should be allowed"
            );
        }
    }

    #[test]
    fn rejects_beyond_the_cap_within_the_same_window() {
        let mut state = RetryWindow::new(0.0);
        for i in 0..MAX_RETRIES_PER_WINDOW {
            assert!(should_attempt_replay(&mut state, i as f64 * 10.0));
        }
        // The (MAX+1)th attempt, still well inside the 30s window, must be
        // rejected — this is the bound that stops an infinite `.play()` spin.
        assert!(!should_attempt_replay(&mut state, 100.0));
        assert!(!should_attempt_replay(&mut state, RETRY_WINDOW_MS - 1.0));
    }

    #[test]
    fn resets_the_budget_once_the_window_elapses() {
        let mut state = RetryWindow::new(0.0);
        for i in 0..MAX_RETRIES_PER_WINDOW {
            assert!(should_attempt_replay(&mut state, i as f64 * 10.0));
        }
        assert!(!should_attempt_replay(&mut state, RETRY_WINDOW_MS - 1.0));
        // A blip AFTER the rolling window has fully elapsed gets a fresh
        // budget — a source that recovered and stayed healthy for 30s+
        // should not stay permanently capped by an old failure streak.
        assert!(should_attempt_replay(&mut state, RETRY_WINDOW_MS + 1.0));
    }

    #[test]
    fn budget_is_exactly_max_retries_not_off_by_one() {
        let mut state = RetryWindow::new(0.0);
        let mut allowed = 0;
        for i in 0..(MAX_RETRIES_PER_WINDOW * 3) {
            if should_attempt_replay(&mut state, i as f64) {
                allowed += 1;
            }
        }
        assert_eq!(allowed, MAX_RETRIES_PER_WINDOW);
    }
}
