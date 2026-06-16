//! Screen Wake Lock for the stage display.
//!
//! Stage TVs must never let the screen sleep mid-service. Most of our stage
//! TVs (sd2-4, Hyundai) have `screen_off_timeout=max` set via adb, but sd1
//! (Tesla LEAP-S1) ships with a 5-minute timeout — so if the TV is idle for
//! 5 minutes the stage display goes black. The Screen Wake Lock API
//! (`navigator.wakeLock.request("screen")`) keeps the display on for as long
//! as the stage page is visible, independently of any adb/device setting.
//! See issue #402.
//!
//! Important platform behavior: a screen wake lock is **automatically
//! released by the browser when the page is hidden or backgrounded**
//! (tab switch, app backgrounded, screen actually turning off once). The
//! sentinel's `released` flag flips to `true`. To keep the screen awake for
//! the whole service we therefore RE-ACQUIRE the lock on every
//! `visibilitychange` that returns the document to `visible`.
//!
//! The wake lock target — `com.tcl.browser` on our TCL-based TVs — is
//! Chromium-based and supports the Wake Lock API. The request can still
//! reject (unsupported platform, or no user activation yet); we log a
//! `leptos::logging::warn!` and carry on rather than panicking. WASM panics
//! surface as browser-side JS errors, so even though `presenter-ui` is
//! panic-exempt we handle the `Result`/`Option` cleanly.
//!
//! Implementation note: the Wake Lock API is reached via raw JS interop
//! (`js_sys::Reflect` + `Function.call`) rather than `web-sys`'s typed
//! bindings. `web-sys` 0.3.99 gates Wake Lock behind the unstable
//! `--cfg=web_sys_unstable_apis` flag, and enabling that flag flips the
//! signature of unrelated stable methods used across this crate (e.g.
//! `HtmlElement::set_scroll_top` becomes `f64`), which would break the slide
//! scroll code. Raw interop keeps the change self-contained to this module.
//!
//! The browser-touching code is gated behind `target_arch = "wasm32"` (the
//! only target that runs the UI) so the host test build (`cargo test --lib`)
//! compiles just the pure decision function below and its unit tests.

/// Pure decision: should we (re)acquire the screen wake lock right now?
///
/// Acquire when the document is visible AND we do not currently hold a live
/// (non-released) sentinel. The browser auto-releases the lock when the page
/// is hidden, so on returning to `visible` the held sentinel is `released`
/// and we must request a fresh one. While hidden we never acquire (a request
/// from a hidden document rejects anyway).
///
/// * `held_live` — true when we currently hold a sentinel whose `released`
///   flag is still `false` (i.e. the lock is genuinely active).
/// * `visible` — true when `document.visibilityState == "visible"`.
pub fn should_acquire(held_live: bool, visible: bool) -> bool {
    visible && !held_live
}

#[cfg(target_arch = "wasm32")]
pub use wasm_impl::start_wake_lock_guard;

/// Host stub: there is no browser to acquire a wake lock against when the
/// crate is compiled for the host (only `cargo test --lib` does this). The
/// real implementation lives in the `wasm_impl` module below.
#[cfg(not(target_arch = "wasm32"))]
pub fn start_wake_lock_guard() {}

#[cfg(target_arch = "wasm32")]
mod wasm_impl {
    use std::cell::RefCell;
    use std::rc::Rc;

    use leptos::wasm_bindgen::prelude::*;
    use wasm_bindgen_futures::{spawn_local, JsFuture};

    use super::should_acquire;

    /// Shared slot holding the current live wake-lock sentinel (if any),
    /// stored as the raw `WakeLockSentinel` JS object. `None` when we hold
    /// nothing; `Some(obj)` after a successful acquire. We read its `released`
    /// property (via `Reflect`) to know whether the browser auto-released it.
    type SentinelSlot = Rc<RefCell<Option<JsValue>>>;

    /// Acquire (or re-acquire) the screen wake lock and start the
    /// `visibilitychange` listener that re-acquires it whenever the document
    /// becomes visible again. Call once when the stage page mounts.
    ///
    /// The registered `visibilitychange` `Closure` is `forget()`-ed so it
    /// lives for the page's lifetime (the stage page is a single long-lived
    /// view); when the page unloads the OS releases the lock anyway. This is a
    /// single bounded closure per page load — the same pattern as
    /// `install_pagehide_teardown` in `ndi_video.rs`.
    pub fn start_wake_lock_guard() {
        if !wake_lock_supported() {
            leptos::logging::warn!(
                "wake_lock: navigator.wakeLock unsupported on this platform; \
                 stage screen may sleep"
            );
            return;
        }

        let slot: SentinelSlot = Rc::new(RefCell::new(None));

        // Initial acquire on mount (document is visible at mount time).
        maybe_acquire(Rc::clone(&slot));

        // Re-acquire on every transition back to visible. The browser
        // releases the lock when the page hides, so we must request a new one.
        let slot_for_cb = Rc::clone(&slot);
        let cb = Closure::<dyn FnMut()>::new(move || {
            if document_visible() {
                maybe_acquire(Rc::clone(&slot_for_cb));
            }
        });

        if let Some(document) = leptos::web_sys::window().and_then(|w| w.document()) {
            if document
                .add_event_listener_with_callback("visibilitychange", cb.as_ref().unchecked_ref())
                .is_err()
            {
                leptos::logging::warn!(
                    "wake_lock: failed to register visibilitychange listener; \
                     lock will not be re-acquired after the page is backgrounded"
                );
            }
        }

        cb.forget();
    }

    /// Acquire the lock if our decision function says we should. Spawns the
    /// async request; on success stores the sentinel in `slot`, on rejection
    /// logs and leaves `slot` unchanged.
    fn maybe_acquire(slot: SentinelSlot) {
        let held_live = slot.borrow().as_ref().is_some_and(sentinel_is_live);
        if !should_acquire(held_live, document_visible()) {
            return;
        }
        spawn_local(async move {
            match request_screen_wake_lock().await {
                Ok(sentinel) => {
                    *slot.borrow_mut() = Some(sentinel);
                    leptos::logging::log!("wake_lock: screen wake lock acquired");
                }
                Err(e) => {
                    leptos::logging::warn!(
                        "wake_lock: request('screen') rejected: {e:?}; stage screen may sleep"
                    );
                }
            }
        });
    }

    /// Read `navigator.wakeLock` via `Reflect`, returning the object when the
    /// platform exposes it (undefined/null/error → `None`).
    fn navigator_wake_lock() -> Option<JsValue> {
        let navigator = leptos::web_sys::window()?.navigator();
        let wl = js_sys::Reflect::get(&navigator, &JsValue::from_str("wakeLock")).ok()?;
        if wl.is_undefined() || wl.is_null() {
            None
        } else {
            Some(wl)
        }
    }

    /// True when the platform exposes `navigator.wakeLock`.
    fn wake_lock_supported() -> bool {
        navigator_wake_lock().is_some()
    }

    /// True when the held sentinel's `released` property is `false` (the lock
    /// is genuinely active). A missing/undefined `released` is treated as live
    /// so we don't double-request.
    fn sentinel_is_live(sentinel: &JsValue) -> bool {
        match js_sys::Reflect::get(sentinel, &JsValue::from_str("released")) {
            Ok(v) => v.as_bool() != Some(true),
            Err(_) => true,
        }
    }

    /// True when `document.visibilityState == "visible"`.
    fn document_visible() -> bool {
        leptos::web_sys::window()
            .and_then(|w| w.document())
            .map(|d| d.visibility_state() == web_sys::VisibilityState::Visible)
            .unwrap_or(false)
    }

    /// Request a screen wake lock via raw JS interop:
    /// `navigator.wakeLock.request("screen")`, awaiting the returned promise.
    /// Returns the resolved `WakeLockSentinel` object.
    async fn request_screen_wake_lock() -> Result<JsValue, JsValue> {
        let wake_lock =
            navigator_wake_lock().ok_or_else(|| JsValue::from_str("navigator.wakeLock missing"))?;
        let request_fn = js_sys::Reflect::get(&wake_lock, &JsValue::from_str("request"))?;
        let request_fn = request_fn
            .dyn_into::<js_sys::Function>()
            .map_err(|_| JsValue::from_str("navigator.wakeLock.request is not a function"))?;
        let promise = request_fn.call1(&wake_lock, &JsValue::from_str("screen"))?;
        let promise = promise
            .dyn_into::<js_sys::Promise>()
            .map_err(|_| JsValue::from_str("wakeLock.request did not return a Promise"))?;
        JsFuture::from(promise).await
    }
}

#[cfg(test)]
mod tests {
    use super::should_acquire;

    /// Visible + no live lock → acquire. This is the mount-time case and the
    /// post-`visibilitychange` re-acquire case (the browser auto-released the
    /// previous sentinel while hidden, so we hold nothing live).
    #[test]
    fn acquires_when_visible_and_no_live_lock() {
        assert!(should_acquire(false, true));
    }

    /// Visible but we still hold a live (non-released) lock → do NOT request a
    /// duplicate. Re-requesting while already holding wastes a request and can
    /// leak sentinels.
    #[test]
    fn does_not_reacquire_when_already_held_live() {
        assert!(!should_acquire(true, true));
    }

    /// Hidden document → never acquire. A `request('screen')` from a hidden
    /// document rejects, and the browser would auto-release immediately
    /// anyway. The re-acquire must wait for the visible transition.
    #[test]
    fn does_not_acquire_when_hidden() {
        assert!(!should_acquire(false, false));
        // Even if we somehow think we hold one, hidden → no acquire.
        assert!(!should_acquire(true, false));
    }
}
