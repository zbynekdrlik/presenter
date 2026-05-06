//! Operator slide-list scroll behavior. Extracted from `slide_list.rs` to
//! keep that file under the project's 1000-line cap.
//!
//! Issue #271: lookahead scroll, deterministic wheel scroll, scroll-to-top
//! on song open.

use wasm_bindgen::JsCast;

/// Number of columns in the `.operator__slides` grid (CSS:
/// `grid-template-columns: repeat(3, minmax(0, 1fr))`). The next-row anchor
/// for an active slide at DOM index N is the slide at index N + COLUMNS_PER_ROW.
const COLUMNS_PER_ROW: usize = 3;

/// Default fallback step for wheel scroll (pixels) when no slide card is
/// rendered yet to measure.
const DEFAULT_WHEEL_STEP_PX: f64 = 120.0;

/// Lookahead-aware scroll: ensures the active slide AND the next row of
/// slides are visible in the `.operator__slides` container. If the active
/// slide is on the last row (no next-row anchor), falls back to "ensure
/// active is visible". If the active slide is above the viewport (backward
/// navigation), top-aligns it.
///
/// Issue #271 concern 1.
pub(super) fn scroll_slide_into_view(slide_id: &str) {
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let active_selector = format!(".operator__slides [data-slide-id=\"{slide_id}\"]");
    let Ok(Some(active_el)) = document.query_selector(&active_selector) else {
        return;
    };
    let Ok(Some(container_el)) = active_el.closest(".operator__slides") else {
        return;
    };
    let Ok(container) = container_el.dyn_into::<web_sys::HtmlElement>() else {
        return;
    };
    let Ok(active_html) = active_el.dyn_into::<web_sys::HtmlElement>() else {
        return;
    };

    let container_rect = container.get_bounding_client_rect();
    let active_rect = active_html.get_bounding_client_rect();
    let scroll_top = container.scroll_top() as f64;

    // Backward navigation: top-align the active slide if it's above the viewport.
    if active_rect.top() < container_rect.top() {
        let delta = container_rect.top() - active_rect.top();
        container.set_scroll_top((scroll_top - delta) as i32);
        return;
    }

    // Find the next-row anchor: the slide at active_index + COLUMNS_PER_ROW
    // in DOM order within the same container.
    let cards = container.query_selector_all("[data-slide-id]").ok();
    let next_row_el: Option<web_sys::HtmlElement> = cards.and_then(|nodes| {
        let mut active_index: Option<usize> = None;
        for i in 0..nodes.length() {
            if let Some(node) = nodes.item(i) {
                if let Ok(el) = node.dyn_into::<web_sys::Element>() {
                    if el.get_attribute("data-slide-id").as_deref() == Some(slide_id) {
                        active_index = Some(i as usize);
                        break;
                    }
                }
            }
        }
        let target_index = active_index? + COLUMNS_PER_ROW;
        nodes
            .item(target_index as u32)
            .and_then(|n| n.dyn_into::<web_sys::HtmlElement>().ok())
    });

    if let Some(anchor) = next_row_el {
        // Scroll so the next-row anchor's bottom is at the container's bottom.
        let anchor_rect = anchor.get_bounding_client_rect();
        if anchor_rect.bottom() > container_rect.bottom() {
            let delta = anchor_rect.bottom() - container_rect.bottom();
            container.set_scroll_top((scroll_top + delta) as i32);
        }
    } else if active_rect.bottom() > container_rect.bottom() {
        // No next-row anchor (last row) — fall back to bottom-aligning active.
        let delta = active_rect.bottom() - container_rect.bottom();
        container.set_scroll_top((scroll_top + delta) as i32);
    }
}

/// Scrolls the `.operator__slides` container to its top. Used when the
/// operator opens a new presentation so the first slide is visible without
/// manual scroll-up. Issue #271 concern 3.
pub(super) fn scroll_slides_to_top() {
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let Ok(Some(container_el)) = document.query_selector(".operator__slides") else {
        return;
    };
    let Ok(container) = container_el.dyn_into::<web_sys::HtmlElement>() else {
        return;
    };
    container.set_scroll_top(0);
}

/// Returns the pixel distance one wheel notch should scroll the
/// `.operator__slides` container. Measures the first rendered slide card's
/// height + the grid row gap so the step adapts to user font-size scaling.
/// Falls back to `DEFAULT_WHEEL_STEP_PX` if no card is rendered.
fn step_for_wheel(container: &web_sys::HtmlElement) -> f64 {
    let Ok(Some(card_el)) = container.query_selector(".operator__slide-card") else {
        return DEFAULT_WHEEL_STEP_PX;
    };
    let Ok(card) = card_el.dyn_into::<web_sys::HtmlElement>() else {
        return DEFAULT_WHEEL_STEP_PX;
    };
    let card_height = card.get_bounding_client_rect().height();
    if card_height <= 0.0 {
        return DEFAULT_WHEEL_STEP_PX;
    }
    // Grid row gap from operator.css `.operator__slides`: `gap: 0.9rem`.
    // 0.9rem at 16px base = 14.4px. Hardcoded — if CSS changes, update here.
    card_height + 14.4
}

/// Cap a wheel event's vertical delta to one row of cards (`step`) per event,
/// preserving sign. Returns 0 when `delta_y` is 0 (no-op).
///
/// Issue #301: previously the wheel handler discarded `delta_y` magnitude and
/// always advanced one full row per event, so trackpad gestures (which fire
/// 20+ events per swipe) blasted through dozens of rows.
fn cap_wheel_delta(delta_y: f64, step: f64) -> f64 {
    if delta_y == 0.0 {
        return 0.0;
    }
    delta_y.signum() * delta_y.abs().min(step)
}

/// Wheel handler for `.operator__slides`: intercepts the native (accelerated)
/// scroll and applies a magnitude-respecting cap. Issue #271 concern 2:
/// neutralises macOS scroll acceleration. Issue #301: per-event delta is
/// capped at one row so high-frequency trackpad/mouse events can't blast
/// through the list.
pub(super) fn handle_wheel_event(ev: web_sys::WheelEvent) {
    ev.prevent_default();
    let delta_y = ev.delta_y();
    if delta_y == 0.0 {
        return;
    }
    let Some(target) = ev.target() else { return };
    let Ok(el) = target.dyn_into::<web_sys::Element>() else {
        return;
    };
    let Ok(Some(container_el)) = el.closest(".operator__slides") else {
        return;
    };
    let Ok(container) = container_el.dyn_into::<web_sys::HtmlElement>() else {
        return;
    };
    let step = step_for_wheel(&container);
    let capped = cap_wheel_delta(delta_y, step);
    container.set_scroll_top((container.scroll_top() as f64 + capped) as i32);
}

#[cfg(test)]
mod tests {
    use super::cap_wheel_delta;

    #[test]
    fn zero_delta_returns_zero() {
        assert_eq!(cap_wheel_delta(0.0, 90.0), 0.0);
    }

    #[test]
    fn small_positive_delta_passes_through() {
        assert_eq!(cap_wheel_delta(10.0, 90.0), 10.0);
    }

    #[test]
    fn small_negative_delta_passes_through() {
        assert_eq!(cap_wheel_delta(-10.0, 90.0), -10.0);
    }

    #[test]
    fn delta_exactly_at_step_passes_through() {
        assert_eq!(cap_wheel_delta(90.0, 90.0), 90.0);
    }

    #[test]
    fn negative_delta_exactly_at_step_passes_through() {
        assert_eq!(cap_wheel_delta(-90.0, 90.0), -90.0);
    }

    #[test]
    fn large_positive_delta_is_capped_at_step() {
        assert_eq!(cap_wheel_delta(500.0, 90.0), 90.0);
    }

    #[test]
    fn large_negative_delta_is_capped_at_negative_step() {
        assert_eq!(cap_wheel_delta(-500.0, 90.0), -90.0);
    }
}
