//! The DOM side of drag-to-reorder (#871 review). The decisions are pure in
//! [`super::order`]; this only reads the pointer and the rows' boxes.
//!
//! A row is dragged by its handle (`stream-nameplate-drag`) with POINTER
//! events, the repo's touch-capable drag pattern (the šepkár tip swipe,
//! `interpreter/tips.rs`; the canvas overlay): HTML5 drag and drop never fires
//! on a phone. The press captures the pointer on the handle, so the moves and
//! the release reach it even outside the row; CSS gives the handle
//! `touch-action: none`, so a finger drag never scrolls the page instead.

use web_sys::PointerEvent;

use crate::components::drag_rows;

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

const HANDLE: &str = "[data-role=\"stream-nameplate-drag\"]";

/// Capture the pointer on the pressed handle. False when the press was not on
/// a handle (nothing to drag).
pub fn capture(ev: &PointerEvent) -> bool {
    let Some(handle) = drag_rows::handle_of(ev, HANDLE) else {
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
    drag_rows::slot_under(
        ev,
        HANDLE,
        "[data-role=\"stream-nameplate-list\"]",
        "[data-role=\"stream-nameplate\"]",
    )
}
