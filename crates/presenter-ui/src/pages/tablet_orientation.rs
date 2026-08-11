//! First-tap orientation-lock gesture for the tablet UI (#569), plus the
//! landscape-primary/landscape-secondary 180°-flip watcher (#638).
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
//!
//! #638: the lock above (and the PWA manifest) now request the *specific*
//! `landscape-primary` orientation rather than generic `landscape` — generic
//! `landscape` is honored by resolving to EITHER landscape-primary or
//! landscape-secondary, so a device locked to it could still visually flip
//! 180° on a physical turn. `install_orientation_flip_watcher` below is the
//! fallback layer for wherever the lock isn't honored (iOS Safari has no
//! `.lock()` support at all, same as the gesture above) or hasn't resolved
//! yet: CSS's `orientation` media feature is inherently 2-state
//! (portrait/landscape) and structurally cannot see a landscape-primary ↔
//! landscape-secondary flip (both keep `width > height`), so this needs the
//! Screen Orientation API's `type`/`angle`, which only JS can read.

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
                    // #638: the SPECIFIC landscape-primary, not generic
                    // "landscape" (which permits either landscape variant
                    // and would still let a physical 180° turn through).
                    return window.screen.orientation.lock("landscape-primary");
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

/// #638: continuously mirrors `screen.orientation.type` onto
/// `body[data-tablet-flip]` so `tablet.css` can counter-rotate a device that
/// is physically landscape-secondary (turned 180° from landscape-primary).
///
/// Runs once at mount (covers a device that LOADS the page already
/// landscape-secondary, before any lock ever resolves) and again on every
/// `screen.orientation` `change` event plus every `resize` (defensive
/// fallback for engines that never fire the former), scoped to
/// `(pointer: coarse)` like the rest of this feature so a desktop browser
/// window is never affected.
pub(crate) fn install_orientation_flip_watcher() {
    let _ = js_sys::eval(ORIENTATION_FLIP_WATCHER_JS);
}

const ORIENTATION_FLIP_WATCHER_JS: &str = r#"
(function () {
    function isSecondaryLandscape() {
        var so = window.screen && window.screen.orientation;
        if (!so) { return false; }
        // Require the window to actually BE landscape-shaped too (review
        // finding): a browser reporting an inconsistent
        // type/angle-vs-actual-shape state must never win against the
        // portrait CSS fallback above, which is MORE specific for the
        // `transform` property but LESS specific overall — a mismatched
        // combination would otherwise mix the 180deg flip transform with
        // the portrait fallback's fixed/width/height rules.
        var isLandscapeShaped = window.innerWidth > window.innerHeight;
        if (typeof so.type === "string") {
            return so.type === "landscape-secondary" && isLandscapeShaped;
        }
        // Best-effort fallback for engines exposing only `.angle` (no
        // `.type`): on a device whose NATURAL orientation is landscape
        // (most tablets), angle 180 is the secondary landscape hold. This
        // does not cover a natural-portrait phone lacking `.type` (its
        // landscape holds are 90/270, not 180) — `.type` is broadly
        // supported (Chrome/Firefox/Edge, Safari 16.4+), so that narrower
        // gap is accepted rather than guessed at.
        return typeof so.angle === "number" && so.angle === 180 && isLandscapeShaped;
    }
    function apply() {
        if (!window.matchMedia || !window.matchMedia("(pointer: coarse)").matches) {
            return; // not a touch device — never touch a desktop browser window
        }
        if (isSecondaryLandscape()) {
            document.body.setAttribute("data-tablet-flip", "true");
        } else {
            document.body.removeAttribute("data-tablet-flip");
        }
    }
    apply();
    if (window.screen && window.screen.orientation && window.screen.orientation.addEventListener) {
        window.screen.orientation.addEventListener("change", apply);
    }
    window.addEventListener("resize", apply);
})();
"#;
