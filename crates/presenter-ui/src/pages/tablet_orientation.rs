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
//! Screen Orientation API's `type`, which only JS can read (#694 dropped the
//! `.angle` fallback — it is natural-orientation-dependent and mis-maps on a
//! phone; see `install_orientation_flip_watcher` below).

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

/// #638/#694: mirrors a STABLE landscape-secondary orientation onto
/// `body[data-tablet-flip]` so `tablet.css` can counter-rotate a device that is
/// genuinely displayed landscape-secondary (turned 180° from landscape-primary
/// — #638's original scenario).
///
/// #694 hardened this against a rotation-LOCKED phone. Per the W3C Screen
/// Orientation spec, `screen.orientation` tracks the device's PHYSICAL
/// orientation and fires `change` on physical tilt, while an OS rotation lock
/// keeps only the DISPLAYED viewport fixed — so lifting / laying a locked phone
/// flat transiently reports `landscape-secondary` with the viewport never
/// actually rotating, and the old instantaneous-read watcher applied the very
/// 180° flip it was built to suppress. The distinguisher between that false
/// trigger and a genuine turn is STABILITY (a real turn settles at secondary
/// and stays; a lift/put-down flap reverts), so:
///   * it trusts ONLY `screen.orientation.type === "landscape-secondary"` — the
///     `.angle` fallback was dropped, because angle 180 is portrait-secondary
///     (NOT landscape-secondary = 270°) on a natural-portrait phone, i.e. it
///     mis-fired for exactly the reported device class;
///   * it never re-evaluates on `resize` — a 180° landscape↔landscape turn
///     never resizes, and mobile browser-chrome show/hide on lift only added
///     false triggers (the 90° portrait fallback is pure CSS, no JS needed);
///   * it never SETS the flip from an instantaneous read — a candidate flip is
///     applied only after the reading stays stable past a short settle window,
///     filtering the transient sensor flaps a locked phone emits on lift /
///     put-down. CLEARING is immediate (a stuck upside-down UI is worse).
///
/// Runs once at mount and on every `screen.orientation` `change`, scoped to
/// `(pointer: coarse)` so a desktop browser window is never affected.
pub(crate) fn install_orientation_flip_watcher() {
    let _ = js_sys::eval(ORIENTATION_FLIP_WATCHER_JS);
}

const ORIENTATION_FLIP_WATCHER_JS: &str = r#"
(function () {
    var STABILITY_MS = 300;
    var pending = null;
    function isTouch() {
        return !!(window.matchMedia && window.matchMedia("(pointer: coarse)").matches);
    }
    function isStableSecondaryLandscape() {
        // Trust ONLY screen.orientation.type — the single API that
        // distinguishes landscape-primary from landscape-secondary. The
        // .angle fallback was dropped (#694): angle 180 is portrait-secondary,
        // not landscape-secondary (= 270deg), on a natural-portrait phone, so
        // it mis-mapped there. .type is broadly supported (Chrome/Firefox/Edge,
        // Safari 16.4+); ancient engines without it simply never flip, which is
        // the safe default. Also require the window to actually BE
        // landscape-shaped, so a mismatched type/shape state can never mix the
        // 180deg flip with the portrait CSS fallback (review finding, #638).
        var so = window.screen && window.screen.orientation;
        if (!so || typeof so.type !== "string") { return false; }
        return so.type === "landscape-secondary" && window.innerWidth > window.innerHeight;
    }
    function evaluate() {
        if (pending !== null) { window.clearTimeout(pending); pending = null; }
        if (!isTouch()) {
            document.body.removeAttribute("data-tablet-flip"); // never touch a desktop window
            return;
        }
        if (!isStableSecondaryLandscape()) {
            // Not secondary (or not landscape-shaped): CLEAR immediately. A
            // stuck upside-down UI is worse than a slightly-delayed flip, so
            // clearing is never debounced.
            document.body.removeAttribute("data-tablet-flip");
            return;
        }
        // Candidate flip — do NOT apply from this instantaneous read. Wait for
        // the reading to stay stable, filtering the transient sensor flaps a
        // rotation-locked phone emits when lifted / laid flat (#694).
        pending = window.setTimeout(function () {
            pending = null;
            if (isTouch() && isStableSecondaryLandscape()) {
                document.body.setAttribute("data-tablet-flip", "true");
            }
        }, STABILITY_MS);
    }
    evaluate();
    if (window.screen && window.screen.orientation && window.screen.orientation.addEventListener) {
        window.screen.orientation.addEventListener("change", evaluate);
    }
})();
"#;
