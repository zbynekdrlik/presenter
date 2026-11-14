//! The DOM half of a drag-to-reorder list (#871, shared by #951): which handle a
//! pointer event concerns and the drop slot under the pointer, measured on the
//! rows on screen. The decisions are pure in `nameplates::order`. Used by the
//! plate list, the sermon order and the study bookmarks: each passes its own
//! handle / list / row selectors.

use wasm_bindgen::JsCast;
use web_sys::{Element, PointerEvent};

/// The handle (`handle` = a CSS selector) a pointer event concerns: the pressed
/// one, or the one that captured the pointer.
pub fn handle_of(ev: &PointerEvent, handle: &str) -> Option<Element> {
    ev.target()
        .and_then(|target| target.dyn_into::<Element>().ok())?
        .closest(handle)
        .ok()
        .flatten()
}

/// The drop slot under the pointer: how many rows (`row`, direct children of
/// the handle's `list`) have their middle above it.
pub fn slot_under(ev: &PointerEvent, handle: &str, list: &str, row: &str) -> Option<usize> {
    let list = handle_of(ev, handle)?.closest(list).ok().flatten()?;
    let rows = list.query_selector_all(&format!(":scope > {row}")).ok()?;
    let middles: Vec<f64> = (0..rows.length())
        .filter_map(|i| rows.item(i))
        .filter_map(|node| node.dyn_into::<Element>().ok())
        .map(|row| {
            let rect = row.get_bounding_client_rect();
            rect.top() + rect.height() / 2.0
        })
        .collect();
    Some(super::nameplates::order::drop_slot(
        &middles,
        f64::from(ev.client_y()),
    ))
}
