//! The DOM side of drag-to-reorder (#871 review). The decisions are pure in
//! [`super::order`]; this only reads the pointer and the rows' boxes.
//!
//! A row is dragged by its handle (`stream-nameplate-drag`) with POINTER
//! events, the repo's touch-capable drag pattern (the šepkár tip swipe,
//! `interpreter/tips.rs`; the canvas overlay): HTML5 drag and drop never fires
//! on a phone. The press captures the pointer on the handle, so the moves and
//! the release reach it even outside the row; CSS gives the handle
//! `touch-action: none`, so a finger drag never scrolls the page instead.

use wasm_bindgen::JsCast;
use web_sys::PointerEvent;

use super::order;

/// One drag in progress: the dragged plate and the slot it would drop into
/// now. Its own place is read from the list as it is now, never kept: a plate
/// deleted or moved elsewhere mid-drag must not leave a stale drop line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlateDrag {
    pub id: i64,
    pub slot: usize,
}

/// `MouseEvent.button` of the primary button (left mouse, a finger, a pen tip):
/// only it starts a drag.
pub const PRIMARY_BUTTON: i16 = 0;

/// The drag handle a pointer event concerns: the pressed handle, or the one
/// that captured the pointer.
fn handle_of(ev: &PointerEvent) -> Option<web_sys::Element> {
    ev.target()
        .and_then(|target| target.dyn_into::<web_sys::Element>().ok())?
        .closest("[data-role=\"stream-nameplate-drag\"]")
        .ok()
        .flatten()
}

/// Capture the pointer on the pressed handle. False when the press was not on
/// a handle (nothing to drag).
pub fn capture(ev: &PointerEvent) -> bool {
    let Some(handle) = handle_of(ev) else {
        return false;
    };
    // An unknown pointer id is the only error; the drag then still ends on
    // `pointerup` / `pointercancel` over the handle.
    let _ = handle.set_pointer_capture(ev.pointer_id());
    true
}

/// The drop slot under the pointer, measured on the rows of the handle's own
/// list (the editor and the operator view each have one).
pub fn slot_under_pointer(ev: &PointerEvent) -> Option<usize> {
    let list = handle_of(ev)?
        .closest("[data-role=\"stream-nameplate-list\"]")
        .ok()
        .flatten()?;
    let rows = list
        .query_selector_all(":scope > [data-role=\"stream-nameplate\"]")
        .ok()?;
    let middles: Vec<f64> = (0..rows.length())
        .filter_map(|i| rows.item(i))
        .filter_map(|node| node.dyn_into::<web_sys::Element>().ok())
        .map(|row| {
            let rect = row.get_bounding_client_rect();
            rect.top() + rect.height() / 2.0
        })
        .collect();
    Some(order::drop_slot(&middles, f64::from(ev.client_y())))
}
