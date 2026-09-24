//! Pure pointer-gesture state machine for the stream-editor canvas overlay (#787).

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
