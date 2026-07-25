//! Bounded playback-recovery guard for `NdiVideo`'s `<video>` element (#568).
//!
//! Android TV / weak-WebView browsers draw a big native "start playback"
//! overlay over a `<video>` element the moment it sits in a PAUSED state —
//! whether that's a rejected autoplay `.play()`, a WHEP stream that stalls
//! mid-session (`pause`/`suspend`/`ended` fire on the element even though
//! playback SHOULD continue), or the page/app returning from background
//! (Android TV WebViews often suspend/resume without ever firing `pause`).
//! CSS suppression (`stage.css`) hides the affordance as defense-in-depth, but
//! the root cause is that nothing re-initiates playback — so the element can
//! sit paused indefinitely. This module detects the pause and re-calls
//! `.play()` immediately, bounded so a persistently-broken source doesn't spin
//! `.play()` forever: the existing frame-based `Watchdog` (`ndi_watchdog.rs`)
//! already escalates — reconnect, then a last-resort page reload — for a
//! source that truly cannot recover, so this guard backs off and defers to it
//! rather than retrying without limit.

use std::cell::RefCell;
use std::rc::Rc;

use leptos::wasm_bindgen::{closure::Closure, JsCast};
use leptos::web_sys::HtmlVideoElement;
use wasm_bindgen_futures::{spawn_local, JsFuture};

use super::ndi_watchdog::now_ms;

/// At most this many `.play()` replay attempts within any rolling
/// `RETRY_WINDOW_MS` window. Beyond that, this guard stops trying and lets the
/// frame-based `Watchdog` escalate instead — a source that is truly dead needs
/// a RECONNECT, not a tighter replay loop hammering `.play()`.
pub(crate) const MAX_RETRIES_PER_WINDOW: usize = 5;
pub(crate) const RETRY_WINDOW_MS: f64 = 30_000.0;

/// Rolling-window bookkeeping for the bounded replay retries. Pure state, no
/// DOM — kept separate from the DOM wiring so the decision logic is
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
/// rolling-window bookkeeping. A window with no activity for
/// `RETRY_WINDOW_MS` resets the attempt counter — a source that recovered and
/// later blips again gets a fresh budget rather than staying permanently
/// capped by an old failure streak. Pure + unit-tested.
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
/// The closures are intentionally `forget()`-leaked into JS, same rationale as
/// `install_pagehide_teardown` in `ndi_video.rs`: one bounded set per
/// `NdiVideo` mount lifetime, each capturing only a cloned `HtmlVideoElement` +
/// a shared `Rc<RefCell<RetryWindow>>` (a few dozen bytes) — negligible next
/// to a page-load-scoped stage display, and released with everything else
/// when the page unloads/reloads.
pub(crate) fn install(video: &HtmlVideoElement) {
    let state = Rc::new(RefCell::new(RetryWindow::new(now_ms())));

    for event_name in ["pause", "ended", "suspend"] {
        let video_clone = video.clone();
        let state_clone = Rc::clone(&state);
        let cb = Closure::<dyn FnMut()>::new(move || {
            replay_if_within_budget(&video_clone, &state_clone, event_name);
        });
        let _ = video.add_event_listener_with_callback(event_name, cb.as_ref().unchecked_ref());
        cb.forget();
    }

    // The page/app returning to the foreground (Android TV WebViews often
    // suspend/resume without firing pause/suspend/ended of their own) can
    // leave the element paused with no element-level event at all — recheck
    // on every visibility restore. `replay_if_within_budget` itself checks
    // `video.paused()` first, so a visibility flip while genuinely playing is
    // a cheap no-op.
    if let Some(document) = leptos::web_sys::window().and_then(|w| w.document()) {
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
        cb.forget();
    }
}

/// If `video` is actually paused and the rolling-window budget allows it,
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
    video.set_muted(true);
    match video.play() {
        Ok(promise) => {
            spawn_local(async move {
                if let Err(e) = JsFuture::from(promise).await {
                    leptos::logging::warn!(
                        "ndi_playback_guard: replay after {source} rejected: {e:?}"
                    );
                }
            });
        }
        Err(e) => {
            leptos::logging::warn!("ndi_playback_guard: replay after {source} threw: {e:?}");
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
