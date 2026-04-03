use leptos::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::HtmlElement;

/// Reactive autofit effect: re-runs autofit whenever the trigger signal changes.
///
/// Schedules autofit via `requestAnimationFrame` to ensure DOM is updated before measuring.
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
