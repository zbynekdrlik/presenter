//! Pagehide-triggered WHEP session teardown (#670).
//!
//! Extracted out of `ndi_video.rs` into its own sibling module (#672) — the
//! ownership shape mirrors `ndi_playback_guard.rs`'s `PlaybackGuardHandle`
//! (#637): a disposable handle owning a non-`forget()`-leaked `Closure`,
//! `dispose()` as the PRIMARY teardown path, `Drop` as a defense-in-depth
//! safety net guarded by a `disposed` flag so the DOM-removal call never
//! fires twice. Pure relocation — no behavior change from the code this
//! replaced in `ndi_video.rs`.

use std::cell::Cell;

use leptos::wasm_bindgen::{closure::Closure, JsCast};

use super::ndi_video::dispatch_delete;

/// Install a `pagehide` window listener that fires DELETE against
/// `resource_url` if the page is being unloaded. Some browsers (and
/// Playwright's `page.goto` navigation) tear down the JS context before
/// Leptos's `on_cleanup` runs; `pagehide` fires earlier in the unload
/// sequence so the DELETE makes it out the door.
///
/// Returns `None` if `resource_url` is `None` (no WHEP resource to tear
/// down) or `window()` is unavailable.
///
/// Returns a [`PagehideHandle`] the caller MUST dispose of whenever the
/// session `resource_url` belongs to is torn down (the reconnect loop's
/// old-session teardown AND `on_cleanup` — see `ndi_video.rs`'s
/// `ActiveConnection`). Symmetric with `ndi_playback_guard::install`'s
/// handle (#637) — NOT `forget()`-leaked.
///
/// PRIOR versions `forget()`-leaked the closure, assuming this runs (at
/// most) once per `<NdiVideo>` mount. False: it's called from the reconnect
/// loop's `Ok(ConnectOutcome::Connected(_))` arm, i.e. on EVERY successful
/// WHEP connect, including every reconnect a mount goes through — so a
/// long-running stage display accumulated one more permanent `window`-level
/// listener per reconnect, forever (#670).
pub(crate) fn install(resource_url: Option<&str>) -> Option<PagehideHandle> {
    let window = leptos::web_sys::window()?;
    let url = resource_url?.to_string();
    let cb = Closure::<dyn FnMut()>::new(move || {
        dispatch_delete(&url);
    });
    let _ = window.add_event_listener_with_callback("pagehide", cb.as_ref().unchecked_ref());
    Some(PagehideHandle {
        window,
        cb,
        disposed: Cell::new(false),
    })
}

/// Disposer returned by [`install`]. Owns the `pagehide` `Closure` — NOT
/// `forget()`-leaked — so [`dispose`](Self::dispose) can remove it before
/// dropping it. Same shape as `PlaybackGuardHandle` (`ndi_playback_guard.rs`,
/// #637): `dispose` is the PRIMARY teardown path; `Drop` below is only a
/// defense-in-depth safety net.
pub(crate) struct PagehideHandle {
    window: leptos::web_sys::Window,
    cb: Closure<dyn FnMut()>,
    /// Guards `remove_listener` against running its `removeEventListener`
    /// call twice: `dispose(self)` consumes `self` by value, and Rust's
    /// drop glue then runs `Drop::drop` immediately after, on EVERY
    /// ordinary teardown — not just a hypothetical forgotten-dispose case.
    /// Harmless to the DOM (removing an already-removed listener is a
    /// no-op) but would defeat a net add/remove-count E2E assertion, same
    /// as `PlaybackGuardHandle::disposed` (#637) — that exact regression
    /// cost that fix a full CI cycle.
    disposed: Cell<bool>,
}

impl PagehideHandle {
    /// Remove the `pagehide` listener — only the FIRST time this runs for a
    /// given handle. `dispose()` and `Drop` both call this.
    fn remove_listener(&self) {
        if self.disposed.replace(true) {
            return;
        }
        let _ = self
            .window
            .remove_event_listener_with_callback("pagehide", self.cb.as_ref().unchecked_ref());
    }

    /// Remove the listener, then drop the closure. The PRIMARY teardown
    /// path — call it explicitly wherever the owning session is torn down.
    pub(crate) fn dispose(self) {
        self.remove_listener();
    }
}

/// Safety net mirroring `PlaybackGuardHandle`'s `Drop` (#637): removes the
/// listener even if a handle is ever dropped without an explicit
/// `dispose()`, so a stale listener can never fire on a destroyed `Closure`
/// (which would panic). Guarded by `disposed`, so a true no-op on the
/// normal path where `dispose()` already ran.
impl Drop for PagehideHandle {
    fn drop(&mut self) {
        self.remove_listener();
    }
}
