//! A long press (≥ 500 ms, spec §6.2): MySword's tap = step, long tap = list.
//! The press ends at the first of: the timer, the finger moving away, the
//! finger lifting. Android reports its own long press as `contextmenu`, and
//! that counts too; the browser's menu never opens over a chip.

use gloo_timers::callback::Timeout;
use leptos::prelude::*;

const HOLD_MS: u32 = 500;
/// The finger may wander this far during a press (as a slide card's tap).
const SLOP_PX: i32 = 10;

#[derive(Clone, Copy)]
pub(super) struct LongPress {
    timer: StoredValue<Option<Timeout>, LocalStorage>,
    start: StoredValue<(i32, i32)>,
    fired: StoredValue<bool>,
}

impl LongPress {
    pub(super) fn new() -> Self {
        Self {
            timer: StoredValue::new_local(None),
            start: StoredValue::new((0, 0)),
            fired: StoredValue::new(false),
        }
    }

    pub(super) fn begin(self, ev: &web_sys::PointerEvent, action: impl FnOnce() + 'static) {
        self.fired.set_value(false);
        self.start.set_value((ev.client_x(), ev.client_y()));
        let press = self;
        self.timer.set_value(Some(Timeout::new(HOLD_MS, move || {
            press.fired.set_value(true);
            action();
        })));
    }

    pub(super) fn moved(self, ev: &web_sys::PointerEvent) {
        let (x, y) = self.start.get_value();
        if (ev.client_x() - x).abs() > SLOP_PX || (ev.client_y() - y).abs() > SLOP_PX {
            self.cancel();
        }
    }

    pub(super) fn cancel(self) {
        self.timer.set_value(None);
    }

    /// The browser's own long press: `action` runs unless the timer already
    /// ran it. Its menu stays closed.
    pub(super) fn context_menu(self, ev: &web_sys::MouseEvent, action: impl FnOnce()) {
        ev.prevent_default();
        self.cancel();
        if !self.fired.get_value() {
            self.fired.set_value(true);
            action();
        }
    }

    /// The `click` after a press: true when that press was a long one, and the
    /// click must do nothing.
    pub(super) fn take_fired(self) -> bool {
        let fired = self.fired.get_value();
        self.fired.set_value(false);
        fired
    }
}
