use leptos::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::HtmlElement;

/// Reactive autofit effect: re-runs autofit whenever the trigger signal changes.
///
/// Schedules autofit via `requestAnimationFrame` to ensure DOM is updated before
/// measuring. Use this for PROPORTIONAL text (lyrics) where two strings of the
/// same length can render to different widths — every change must re-fit.
pub fn autofit_effect<T: 'static>(
    node_ref: NodeRef<leptos::html::Div>,
    max_font: f64,
    trigger: impl Fn() -> T + 'static,
) {
    Effect::new(move |_| {
        let _trigger = trigger();
        if let Some(el) = node_ref.get() {
            let html_el: &HtmlElement = &el;
            let el_clone = html_el.clone();
            let cb = wasm_bindgen::closure::Closure::once_into_js(move || {
                autofit_text(&el_clone, max_font);
            });
            let _ = web_sys::window()
                .expect("window")
                .request_animation_frame(cb.as_ref().unchecked_ref());
        }
    });
}

/// Like [`autofit_effect`], but only re-fits when the trigger TEXT changes
/// LENGTH — for FIXED-WIDTH / tabular-nums content (the status bar's clock,
/// song number and connection+latency text).
///
/// # Why
///
/// [`autofit_text`] binary-searches the font size, forcing ~7 synchronous
/// layout reflows per call. The status-bar clock ticks every second, so an
/// unguarded effect ran that search once a second — on a weak stage TV that
/// stalled the compositor into a ~once-a-second micro-hitch (the original reason
/// the plain-JS lite player was forked off). The status-bar texts are
/// fixed-format / tabular (clock `21:28:07`→`21:28:08`, song `#095`,
/// `CONNECTED · 25 MS`), so equal CHARACTER LENGTH ⇒ equal rendered width ⇒ the
/// already-applied font size still fits. Re-fitting only on a length change
/// removes the per-second reflow while still re-fitting on real content changes
/// (`CONNECTED`→`CONNECTING…`, a different-length song number). This must NOT be
/// used for proportional text (lyrics) — there, same length ≠ same width.
pub fn autofit_effect_tabular(
    node_ref: NodeRef<leptos::html::Div>,
    max_font: f64,
    trigger: impl Fn() -> String + 'static,
) {
    let last_len = std::cell::Cell::new(usize::MAX);
    Effect::new(move |_| {
        let len = trigger().chars().count();
        if len == last_len.get() {
            return; // same width as the last fit — skip the reflow search
        }
        last_len.set(len);
        if let Some(el) = node_ref.get() {
            let html_el: &HtmlElement = &el;
            let el_clone = html_el.clone();
            let cb = wasm_bindgen::closure::Closure::once_into_js(move || {
                autofit_text(&el_clone, max_font);
            });
            let _ = web_sys::window()
                .expect("window")
                .request_animation_frame(cb.as_ref().unchecked_ref());
        }
    });
}

/// Auto-fit text to fill a container by binary-searching font size.
///
/// Sets the element's `font-size` style to the largest value in `[1, max_font_px]`
/// where the content does not overflow the container.
pub fn autofit_text(element: &HtmlElement, max_font_px: f64) {
    let style = element.style();

    // Try max first — if it fits, done
    let _ = style.set_property("font-size", &format!("{max_font_px}px"));
    if !is_overflowing(element) {
        return;
    }

    // Binary search between 1px and max
    let mut lo: f64 = 1.0;
    let mut hi: f64 = max_font_px;

    while (hi - lo) > 0.5 {
        let mid = (lo + hi) / 2.0;
        let _ = style.set_property("font-size", &format!("{mid}px"));
        if is_overflowing(element) {
            hi = mid;
        } else {
            lo = mid;
        }
    }

    let _ = style.set_property("font-size", &format!("{lo}px"));
}

fn is_overflowing(el: &HtmlElement) -> bool {
    el.scroll_height() > el.client_height() || el.scroll_width() > el.client_width()
}
