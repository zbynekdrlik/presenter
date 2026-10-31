//! The on-screen keyboard's height (#850 spec §7.1 item 7). iOS keeps the
//! layout viewport when the keyboard opens and only shrinks the visual
//! viewport, so a bottom bar would sit behind the keys; `body`'s
//! `--kb-inset` lifts the console's action bar above them. Chrome Android
//! resizes the layout instead (`interactive-widget=resizes-content`), so the
//! inset stays 0 there.

use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

/// The part of the layout viewport the keyboard covers, in CSS px. A
/// pinch-zoomed page (scale > 1) also has a small visual viewport, without
/// any keyboard: no inset then.
pub fn keyboard_inset(inner_height: f64, visual_height: f64, offset_top: f64, scale: f64) -> f64 {
    if scale > 1.01 {
        return 0.0;
    }
    (inner_height - visual_height - offset_top).max(0.0).round()
}

/// Keep `body`'s `--kb-inset` in step with the visual viewport.
pub fn install_keyboard_inset() {
    let window = crate::utils::window::window();
    let Some(viewport) = window.visual_viewport() else {
        return;
    };
    let watched = viewport.clone();
    let update = Closure::<dyn Fn()>::new(move || {
        let inner = crate::utils::window::window()
            .inner_height()
            .ok()
            .and_then(|h| h.as_f64())
            .unwrap_or(0.0);
        let inset = keyboard_inset(
            inner,
            watched.height(),
            watched.offset_top(),
            watched.scale(),
        );
        if let Some(body) = crate::utils::window::document_body() {
            let _ = body
                .style()
                .set_property("--kb-inset", &format!("{inset}px"));
        }
    });
    for event in ["resize", "scroll"] {
        let _ = viewport.add_event_listener_with_callback(event, update.as_ref().unchecked_ref());
    }
    update.forget();
}

#[cfg(test)]
mod tests {
    use super::keyboard_inset;

    #[test]
    fn the_inset_is_what_the_visual_viewport_lost_at_the_bottom() {
        // iPhone 390x844, keyboard 336 px tall, page not scrolled.
        assert_eq!(keyboard_inset(844.0, 508.0, 0.0, 1.0), 336.0);
        // The visual viewport scrolled 40 px down inside the layout viewport.
        assert_eq!(keyboard_inset(844.0, 508.0, 40.0, 1.0), 296.0);
        // No keyboard, or Chrome Android resizing the layout itself.
        assert_eq!(keyboard_inset(844.0, 844.0, 0.0, 1.0), 0.0);
        // Pinch-zoom: a small visual viewport without any keyboard.
        assert_eq!(keyboard_inset(844.0, 422.0, 0.0, 2.0), 0.0);
        // Never negative.
        assert_eq!(keyboard_inset(844.0, 422.0, 500.0, 1.0), 0.0);
    }
}
