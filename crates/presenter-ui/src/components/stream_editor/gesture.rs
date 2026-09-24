//! Pure pointer-gesture state machine for the stream-editor canvas overlay (#787).
//!
//! The overlay (`canvas_overlay.rs`) owns every web_sys event; this module holds
//! only the decisions, so it is host-unit-testable (`cargo test --lib`):
//!
//! * pointerdown → [`Gesture::Pending`] remembers the pointer + the frame at the
//!   start. Nothing moves yet, so a click with hand jitter never edits the frame.
//! * a move of at least [`DRAG_THRESHOLD_PX`] from the origin → `Dragging`, and
//!   the whole movement since pointerdown is applied (the threshold is not
//!   swallowed); after that each move applies its delta from the last point.
//! * a move reporting NO pressed button ends the gesture: the button was released
//!   somewhere the overlay never heard about (lost capture, another window), so
//!   the drag must never outlive the button.
//! * Escape during a gesture hands back the start frame to restore; Escape when
//!   idle asks the overlay to deselect.

use presenter_core::Frame;

use super::frame_math::Handle;

/// Pointer travel (CSS px, from pointerdown) before a press becomes a drag.
pub const DRAG_THRESHOLD_PX: f64 = 4.0;

/// What a drag edits: the element body (move) or one resize handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GestureKind {
    Move,
    Resize(Handle),
}

/// The overlay's pointer gesture.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Gesture {
    /// No button held on the canvas.
    #[default]
    Idle,
    /// Pressed, but not yet moved past the threshold.
    Pending {
        kind: GestureKind,
        origin: (f64, f64),
        start_frame: Frame,
    },
    /// Moved past the threshold; `last` is the previous pointer position.
    Dragging {
        kind: GestureKind,
        last: (f64, f64),
        start_frame: Frame,
    },
}

/// The overlay's response to a pointermove.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MoveAction {
    /// Nothing to do (idle, or still under the threshold).
    None,
    /// Apply this pixel delta to the frame (move or resize per `kind`).
    Apply {
        kind: GestureKind,
        dx_px: f64,
        dy_px: f64,
    },
    /// The gesture just ended (no button held) — release capture, apply nothing.
    End,
}

/// The overlay's response to Escape.
#[derive(Debug, Clone, PartialEq)]
pub enum EscapeAction {
    /// A gesture was cancelled: put this frame (the one at pointerdown) back.
    Restore(Frame),
    /// No gesture: deselect the element.
    Deselect,
}

impl Gesture {
    /// Start a gesture at pointer `pt` over an element whose frame is `start_frame`.
    pub fn begin(kind: GestureKind, pt: (f64, f64), start_frame: Frame) -> Self {
        Gesture::Pending {
            kind,
            origin: pt,
            start_frame,
        }
    }

    /// True while a button press on the canvas is being tracked.
    pub fn is_active(&self) -> bool {
        !matches!(self, Gesture::Idle)
    }

    /// Feed a pointermove: `buttons` is `MouseEvent.buttons` (0 = none held).
    pub fn on_move(&mut self, buttons: u16, pt: (f64, f64)) -> MoveAction {
        if !self.is_active() {
            return MoveAction::None;
        }
        if buttons == 0 {
            *self = Gesture::Idle;
            return MoveAction::End;
        }
        match self {
            Gesture::Idle => MoveAction::None,
            Gesture::Pending {
                kind,
                origin,
                start_frame,
            } => {
                let (dx, dy) = (pt.0 - origin.0, pt.1 - origin.1);
                if dx.hypot(dy) < DRAG_THRESHOLD_PX {
                    return MoveAction::None;
                }
                let kind = *kind;
                *self = Gesture::Dragging {
                    kind,
                    last: pt,
                    start_frame: start_frame.clone(),
                };
                MoveAction::Apply {
                    kind,
                    dx_px: dx,
                    dy_px: dy,
                }
            }
            Gesture::Dragging { kind, last, .. } => {
                let (dx, dy) = (pt.0 - last.0, pt.1 - last.1);
                *last = pt;
                MoveAction::Apply {
                    kind: *kind,
                    dx_px: dx,
                    dy_px: dy,
                }
            }
        }
    }

    /// End the gesture (pointerup / cancel / lost capture / window blur).
    /// Returns whether one was active.
    pub fn end(&mut self) -> bool {
        let was_active = self.is_active();
        *self = Gesture::Idle;
        was_active
    }

    /// Escape: cancel a gesture (restore its start frame) or, when idle, deselect.
    pub fn escape(&mut self) -> EscapeAction {
        match std::mem::take(self) {
            Gesture::Idle => EscapeAction::Deselect,
            Gesture::Pending { start_frame, .. } | Gesture::Dragging { start_frame, .. } => {
                EscapeAction::Restore(start_frame)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::stream_editor::frame_math::Handle;
    use presenter_core::Frame;

    fn frame() -> Frame {
        Frame {
            x_pct: 10.0,
            y_pct: 20.0,
            w_pct: 30.0,
            h_pct: 40.0,
        }
    }

    const PRESSED: u16 = 1;

    fn pending() -> Gesture {
        Gesture::begin(GestureKind::Move, (100.0, 100.0), frame())
    }

    #[test]
    fn begin_is_pending_and_active() {
        let g = pending();
        assert!(g.is_active());
        assert!(matches!(g, Gesture::Pending { .. }));
    }

    #[test]
    fn idle_is_not_active_and_ignores_moves() {
        let mut g = Gesture::Idle;
        assert!(!g.is_active());
        assert_eq!(g.on_move(PRESSED, (500.0, 500.0)), MoveAction::None);
        assert_eq!(g.on_move(0, (500.0, 500.0)), MoveAction::None);
        assert_eq!(g, Gesture::Idle);
    }

    #[test]
    fn jitter_below_threshold_does_not_start_a_drag() {
        let mut g = pending();
        assert_eq!(g.on_move(PRESSED, (102.0, 101.0)), MoveAction::None);
        assert_eq!(g.on_move(PRESSED, (98.0, 102.0)), MoveAction::None);
        // Diagonal 2.83 px — still below the 4 px threshold.
        assert_eq!(g.on_move(PRESSED, (102.0, 102.0)), MoveAction::None);
        assert!(matches!(g, Gesture::Pending { .. }));
    }

    #[test]
    fn crossing_threshold_starts_drag_with_full_delta_from_origin() {
        let mut g = pending();
        // Diagonal 4.24 px >= 4 px: the drag starts and the WHOLE movement since
        // pointerdown is applied (the threshold distance is not swallowed).
        assert_eq!(
            g.on_move(PRESSED, (103.0, 103.0)),
            MoveAction::Apply {
                kind: GestureKind::Move,
                dx_px: 3.0,
                dy_px: 3.0,
            }
        );
        assert!(matches!(g, Gesture::Dragging { .. }));
    }

    #[test]
    fn exactly_threshold_distance_starts_drag() {
        let mut g = pending();
        let act = g.on_move(PRESSED, (100.0 + DRAG_THRESHOLD_PX, 100.0));
        assert_eq!(
            act,
            MoveAction::Apply {
                kind: GestureKind::Move,
                dx_px: DRAG_THRESHOLD_PX,
                dy_px: 0.0,
            }
        );
    }

    #[test]
    fn dragging_applies_incremental_deltas() {
        let mut g = pending();
        g.on_move(PRESSED, (110.0, 100.0));
        assert_eq!(
            g.on_move(PRESSED, (115.0, 90.0)),
            MoveAction::Apply {
                kind: GestureKind::Move,
                dx_px: 5.0,
                dy_px: -10.0,
            }
        );
        assert_eq!(
            g.on_move(PRESSED, (115.0, 90.0)),
            MoveAction::Apply {
                kind: GestureKind::Move,
                dx_px: 0.0,
                dy_px: 0.0,
            }
        );
    }

    #[test]
    fn resize_kind_is_carried_into_apply() {
        let mut g = Gesture::begin(GestureKind::Resize(Handle::Se), (0.0, 0.0), frame());
        assert_eq!(
            g.on_move(PRESSED, (10.0, 0.0)),
            MoveAction::Apply {
                kind: GestureKind::Resize(Handle::Se),
                dx_px: 10.0,
                dy_px: 0.0,
            }
        );
    }

    #[test]
    fn move_with_no_button_pressed_ends_a_pending_gesture() {
        let mut g = pending();
        assert_eq!(g.on_move(0, (100.0, 100.0)), MoveAction::End);
        assert_eq!(g, Gesture::Idle);
    }

    #[test]
    fn move_with_no_button_pressed_ends_a_drag_without_applying() {
        let mut g = pending();
        g.on_move(PRESSED, (120.0, 100.0));
        assert_eq!(g.on_move(0, (200.0, 100.0)), MoveAction::End);
        assert_eq!(g, Gesture::Idle);
        // A further plain move after the end does nothing.
        assert_eq!(g.on_move(0, (300.0, 100.0)), MoveAction::None);
    }

    #[test]
    fn end_returns_whether_a_gesture_was_active() {
        let mut g = pending();
        g.on_move(PRESSED, (120.0, 100.0));
        assert!(g.end());
        assert_eq!(g, Gesture::Idle);
        assert!(!g.end());
    }

    #[test]
    fn escape_during_drag_restores_start_frame_and_ends() {
        let mut g = pending();
        g.on_move(PRESSED, (150.0, 100.0));
        assert_eq!(g.escape(), EscapeAction::Restore(frame()));
        assert_eq!(g, Gesture::Idle);
        // Still pressed, but the gesture is over: nothing moves.
        assert_eq!(g.on_move(PRESSED, (200.0, 100.0)), MoveAction::None);
    }

    #[test]
    fn escape_while_pending_restores_and_ends() {
        let mut g = pending();
        assert_eq!(g.escape(), EscapeAction::Restore(frame()));
        assert_eq!(g, Gesture::Idle);
    }

    #[test]
    fn escape_when_idle_deselects() {
        let mut g = Gesture::Idle;
        assert_eq!(g.escape(), EscapeAction::Deselect);
        assert_eq!(g, Gesture::Idle);
    }
}
