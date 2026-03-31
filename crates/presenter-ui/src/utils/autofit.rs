use web_sys::HtmlElement;

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
