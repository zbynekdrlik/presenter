//! First-tap orientation-lock gesture for the tablet UI (#569).
//!
//! `screen.orientation.lock()` is only honored inside a fullscreen context,
//! and is unsupported entirely on iOS Safari — so the plain-browser-tab flow
//! (not installed as a PWA) needs a one-time user gesture to request both.
//! Split out of `tablet.rs` to keep that file under the size cap.
//!
//! This is ONE layer of the fix; the CSS counter-rotation fallback in
//! `tablet.css` (`@media (orientation: portrait)`) is what actually guarantees
//! the "never flips with phone position" requirement when this gesture never
//! fires, is denied, or is unsupported — see that file for the full picture.

/// Install a ONE-SHOT `pointerdown` listener: on the first tap, if the page is
/// currently portrait AND on a touch device, request fullscreen then lock the
/// orientation to landscape. Removes itself after the first tap regardless of
/// outcome — never nags on every subsequent tap. A no-op when the viewport is
/// already landscape (nothing to fix — this also keeps it inert on
/// desktop-sized E2E runs, which never resize to portrait), or when the
/// primary pointer is fine (mouse/trackpad) — a desktop user with a
/// narrow/portrait-shaped browser window must never get an unsolicited
/// fullscreen + orientation-lock attempt (review finding, PR #579).
///
/// Every promise is explicitly caught: an unsupported/denied lock (iOS
/// Safari, a browser that refuses fullscreen) must never surface as an
/// unhandled-rejection console error — the CSS fallback covers that case.
pub(crate) fn install_orientation_lock_gesture() {
    let _ = js_sys::eval(ORIENTATION_LOCK_GESTURE_JS);
}

const ORIENTATION_LOCK_GESTURE_JS: &str = r#"
(function () {
    function attempt() {
        document.removeEventListener("pointerdown", attempt, true);
        if (window.innerWidth >= window.innerHeight) {
            return; // already landscape — nothing to fix
        }
        if (!window.matchMedia || !window.matchMedia("(pointer: coarse)").matches) {
            return; // not a touch device — never force fullscreen/lock on desktop
        }
        var el = document.documentElement;
        var entered = el.requestFullscreen
            ? el.requestFullscreen()
            : Promise.reject(new Error("Fullscreen API unavailable"));
        Promise.resolve(entered)
            .then(function () {
                if (window.screen && window.screen.orientation && window.screen.orientation.lock) {
                    return window.screen.orientation.lock("landscape");
                }
            })
            .catch(function () {
                // Unsupported or denied (iOS Safari, no fullscreen support,
                // a user-activation edge case) — the CSS rotate-fallback in
                // tablet.css covers this case, so there is nothing more to do.
            });
    }
    document.addEventListener("pointerdown", attempt, true);
})();
"#;
