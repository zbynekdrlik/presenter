//! A sideways swipe of a finger (spec §6.3): the reader turns chapters with it,
//! the phone's compare view verses.

use presenter_core::bible::resolver::Step;

use crate::state::study::swipe_step;

/// Where the finger went down; empty once it no longer can be a swipe.
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct SwipeTracker {
    start: Option<(i32, f64, f64)>,
}

impl SwipeTracker {
    pub(super) fn down(&mut self, ev: &web_sys::PointerEvent) {
        let touch = ev.pointer_type() != "mouse";
        self.start = (touch && ev.is_primary()).then(|| {
            (
                ev.pointer_id(),
                f64::from(ev.client_x()),
                f64::from(ev.client_y()),
            )
        });
    }

    /// The step this finger made, if it was a swipe.
    pub(super) fn up(&mut self, ev: &web_sys::PointerEvent) -> Option<Step> {
        let (id, x, y) = self.start.take()?;
        if id != ev.pointer_id() {
            return None;
        }
        swipe_step(f64::from(ev.client_x()) - x, f64::from(ev.client_y()) - y)
    }

    pub(super) fn cancel(&mut self) {
        self.start = None;
    }
}
