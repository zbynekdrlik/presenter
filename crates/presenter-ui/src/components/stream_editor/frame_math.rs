//! Pure geometry for the stream-editor canvas overlay (#777).
//!
//! The overlay (`canvas_overlay.rs`) does all the web_sys work (pointer events,
//! `DomRect` reads); this module holds ONLY plain-number geometry so it is
//! host-unit-testable (`cargo test --lib`, no browser). Percent maps 1:1 to the
//! 16:9 canvas box, so the overlay converts a pixel delta to a percent delta
//! ([`px_to_pct_delta`]) and hands it here.
//!
//! Two clamps apply (design #777 point 4):
//!   * **core ranges** — `x/y ∈ -200..=300`, `w/h ∈ MIN..=300` (mirrors
//!     `presenter_core`'s `STREAM_FRAME_POS_*` / `STREAM_FRAME_SIZE_MAX_PCT`), so
//!     a committed value can never make the server 422 on the frame.
//!   * **in-canvas when it fits** — while dragging/resizing, an element that FITS
//!     the canvas (size <= 100 on that axis) is kept inside `0..=100`; a numeric
//!     field keeps the wider core range for deliberate slide-in authoring.

use presenter_core::{
    Frame, STREAM_FRAME_POS_MAX_PCT, STREAM_FRAME_POS_MIN_PCT, STREAM_FRAME_SIZE_MAX_PCT,
};

/// Smallest element size (percent, both axes) a resize handle can produce.
pub const MIN_SIZE_PCT: f32 = 2.0;

/// An edge/centre within this many percent of a snap target snaps to it.
pub const SNAP_THRESHOLD_PCT: f32 = 1.5;

/// The eight resize handles. Each moves one or two of the frame's edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    N,
    S,
    E,
    W,
    Ne,
    Nw,
    Se,
    Sw,
}

impl Handle {
    /// Does this handle move the TOP edge (changes y and h)?
    pub fn moves_top(self) -> bool {
        matches!(self, Handle::N | Handle::Ne | Handle::Nw)
    }
    /// Does this handle move the BOTTOM edge (changes h)?
    pub fn moves_bottom(self) -> bool {
        matches!(self, Handle::S | Handle::Se | Handle::Sw)
    }
    /// Does this handle move the LEFT edge (changes x and w)?
    pub fn moves_left(self) -> bool {
        matches!(self, Handle::W | Handle::Nw | Handle::Sw)
    }
    /// Does this handle move the RIGHT edge (changes w)?
    pub fn moves_right(self) -> bool {
        matches!(self, Handle::E | Handle::Ne | Handle::Se)
    }

    /// Stable lowercase token used for the overlay handle's `data-handle`
    /// attribute and the E2E selector.
    pub fn as_str(self) -> &'static str {
        match self {
            Handle::N => "n",
            Handle::S => "s",
            Handle::E => "e",
            Handle::W => "w",
            Handle::Ne => "ne",
            Handle::Nw => "nw",
            Handle::Se => "se",
            Handle::Sw => "sw",
        }
    }

    /// Parse a `data-handle` token back into a [`Handle`].
    pub fn from_str(s: &str) -> Option<Handle> {
        Some(match s {
            "n" => Handle::N,
            "s" => Handle::S,
            "e" => Handle::E,
            "w" => Handle::W,
            "ne" => Handle::Ne,
            "nw" => Handle::Nw,
            "se" => Handle::Se,
            "sw" => Handle::Sw,
            _ => return None,
        })
    }

    /// All eight handles, for rendering the overlay's handle set.
    pub const ALL: [Handle; 8] = [
        Handle::Nw,
        Handle::N,
        Handle::Ne,
        Handle::W,
        Handle::E,
        Handle::Sw,
        Handle::S,
        Handle::Se,
    ];
}

/// Candidate snap lines per axis (canvas edges + centre + every OTHER element's
/// left/centre/right and top/middle/bottom).
#[derive(Debug, Clone, Default)]
pub struct SnapTargets {
    pub xs: Vec<f32>,
    pub ys: Vec<f32>,
}

impl SnapTargets {
    /// No snap targets — used for keyboard nudge (arrow keys never snap).
    pub fn empty() -> Self {
        SnapTargets::default()
    }
}

/// Build the snap targets for a drag: the canvas guides (0 / 50 / 100 on each
/// axis) plus the edges + centre of every OTHER element in the scene.
pub fn snap_targets(others: &[Frame]) -> SnapTargets {
    let mut xs = vec![0.0, 50.0, 100.0];
    let mut ys = vec![0.0, 50.0, 100.0];
    for f in others {
        xs.push(f.x_pct);
        xs.push(f.x_pct + f.w_pct / 2.0);
        xs.push(f.x_pct + f.w_pct);
        ys.push(f.y_pct);
        ys.push(f.y_pct + f.h_pct / 2.0);
        ys.push(f.y_pct + f.h_pct);
    }
    SnapTargets { xs, ys }
}

/// Convert a pixel delta on one axis to a percent delta of the canvas extent.
pub fn px_to_pct_delta(delta_px: f64, canvas_px: f64) -> f32 {
    if canvas_px <= 0.0 {
        0.0
    } else {
        (delta_px / canvas_px * 100.0) as f32
    }
}

/// Round to one decimal place (the frame fields' `step="0.1"`), keeping values
/// clean and free of float accumulation drift across repeated nudges/drags.
pub fn round1(v: f32) -> f32 {
    (v * 10.0).round() / 10.0
}

/// Clamp a position to the core off-canvas-allowed range.
pub fn clamp_pos(v: f32) -> f32 {
    v.clamp(STREAM_FRAME_POS_MIN_PCT, STREAM_FRAME_POS_MAX_PCT)
}

/// Clamp a size to the core positive range (`MIN_SIZE_PCT..=300`).
pub fn clamp_size(v: f32) -> f32 {
    v.clamp(MIN_SIZE_PCT, STREAM_FRAME_SIZE_MAX_PCT)
}

/// Clamp one axis of a MOVE: keep the element inside the canvas when it FITS
/// (size <= 100 → position `0..=100-size`), else fall back to the core range.
fn clamp_move_axis(pos: f32, size: f32) -> f32 {
    if size <= 100.0 {
        pos.clamp(0.0, 100.0 - size)
    } else {
        clamp_pos(pos)
    }
}

/// Best delta to add to ANY of `positions` so it lands on a snap target within
/// [`SNAP_THRESHOLD_PCT`]; `0.0` if none is close enough.
fn snap_delta(positions: &[f32], targets: &[f32]) -> f32 {
    let mut best: Option<f32> = None;
    for &p in positions {
        for &t in targets {
            let d = t - p;
            if d.abs() <= SNAP_THRESHOLD_PCT && best.map(|b: f32| d.abs() < b.abs()).unwrap_or(true)
            {
                best = Some(d);
            }
        }
    }
    best.unwrap_or(0.0)
}

/// Move a frame by a percent delta, optionally snapping, then clamping to the
/// canvas (when it fits) and the core ranges. Size is unchanged.
pub fn move_by(
    frame: &Frame,
    dx_pct: f32,
    dy_pct: f32,
    snap: bool,
    targets: &SnapTargets,
) -> Frame {
    let w = frame.w_pct;
    let h = frame.h_pct;
    let mut x = frame.x_pct + dx_pct;
    let mut y = frame.y_pct + dy_pct;
    if snap {
        x += snap_delta(&[x, x + w / 2.0, x + w], &targets.xs);
        y += snap_delta(&[y, y + h / 2.0, y + h], &targets.ys);
    }
    Frame {
        x_pct: round1(clamp_move_axis(x, w)),
        y_pct: round1(clamp_move_axis(y, h)),
        w_pct: round1(w),
        h_pct: round1(h),
    }
}

/// Resize a frame by dragging one handle. The OPPOSITE edge stays fixed; the
/// moving edge(s) shift by the delta, optionally snap, are kept inside the canvas
/// (`0..=100`), and never cross the min-size floor.
pub fn resize_by(
    frame: &Frame,
    handle: Handle,
    dx_pct: f32,
    dy_pct: f32,
    snap: bool,
    targets: &SnapTargets,
) -> Frame {
    let mut left = frame.x_pct;
    let mut right = frame.x_pct + frame.w_pct;
    let mut top = frame.y_pct;
    let mut bottom = frame.y_pct + frame.h_pct;

    if handle.moves_left() {
        left += dx_pct;
    }
    if handle.moves_right() {
        right += dx_pct;
    }
    if handle.moves_top() {
        top += dy_pct;
    }
    if handle.moves_bottom() {
        bottom += dy_pct;
    }

    if snap {
        if handle.moves_left() {
            left += snap_delta(&[left], &targets.xs);
        }
        if handle.moves_right() {
            right += snap_delta(&[right], &targets.xs);
        }
        if handle.moves_top() {
            top += snap_delta(&[top], &targets.ys);
        }
        if handle.moves_bottom() {
            bottom += snap_delta(&[bottom], &targets.ys);
        }
    }

    // Keep resize inside the canvas.
    left = left.clamp(0.0, 100.0);
    right = right.clamp(0.0, 100.0);
    top = top.clamp(0.0, 100.0);
    bottom = bottom.clamp(0.0, 100.0);

    // Enforce the min-size floor by pushing the MOVING edge off the fixed one.
    if handle.moves_left() && right - left < MIN_SIZE_PCT {
        left = (right - MIN_SIZE_PCT).max(0.0);
    }
    if handle.moves_right() && right - left < MIN_SIZE_PCT {
        right = (left + MIN_SIZE_PCT).min(100.0);
    }
    if handle.moves_top() && bottom - top < MIN_SIZE_PCT {
        top = (bottom - MIN_SIZE_PCT).max(0.0);
    }
    if handle.moves_bottom() && bottom - top < MIN_SIZE_PCT {
        bottom = (top + MIN_SIZE_PCT).min(100.0);
    }

    Frame {
        x_pct: round1(left),
        y_pct: round1(top),
        w_pct: round1(clamp_size(right - left)),
        h_pct: round1(clamp_size(bottom - top)),
    }
}

/// Keyboard nudge: a move by a fixed percent step with NO snapping (arrow keys
/// are for fine placement). Same in-canvas + core clamp as a drag move.
pub fn nudge(frame: &Frame, dx_pct: f32, dy_pct: f32) -> Frame {
    move_by(frame, dx_pct, dy_pct, false, &SnapTargets::empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(x: f32, y: f32, w: f32, h: f32) -> Frame {
        Frame {
            x_pct: x,
            y_pct: y,
            w_pct: w,
            h_pct: h,
        }
    }

    #[test]
    fn px_to_pct_delta_maps_and_guards_zero() {
        assert_eq!(px_to_pct_delta(96.0, 960.0), 10.0);
        assert_eq!(px_to_pct_delta(-48.0, 960.0), -5.0);
        assert_eq!(px_to_pct_delta(10.0, 0.0), 0.0);
    }

    #[test]
    fn move_translates_without_changing_size() {
        let r = move_by(
            &f(10.0, 10.0, 20.0, 20.0),
            5.0,
            -3.0,
            false,
            &SnapTargets::empty(),
        );
        assert_eq!(
            (r.x_pct, r.y_pct, r.w_pct, r.h_pct),
            (15.0, 7.0, 20.0, 20.0)
        );
    }

    #[test]
    fn move_clamps_inside_canvas_when_it_fits() {
        // A 20-wide, 20-tall element cannot go past 80 on either axis.
        let r = move_by(
            &f(70.0, 70.0, 20.0, 20.0),
            50.0,
            50.0,
            false,
            &SnapTargets::empty(),
        );
        assert_eq!((r.x_pct, r.y_pct), (80.0, 80.0));
        // ...nor below 0.
        let r2 = move_by(
            &f(10.0, 10.0, 20.0, 20.0),
            -50.0,
            -50.0,
            false,
            &SnapTargets::empty(),
        );
        assert_eq!((r2.x_pct, r2.y_pct), (0.0, 0.0));
    }

    #[test]
    fn move_of_oversized_element_uses_core_range_not_canvas() {
        // w=150 cannot fit → the wide core range applies (down to -200).
        let r = move_by(
            &f(0.0, 0.0, 150.0, 20.0),
            -500.0,
            0.0,
            false,
            &SnapTargets::empty(),
        );
        assert_eq!(r.x_pct, STREAM_FRAME_POS_MIN_PCT);
    }

    #[test]
    fn move_snaps_left_edge_to_canvas_when_enabled_and_not_when_disabled() {
        // x=1.0 is within 1.5 of the 0 guide → snaps to 0 when enabled.
        let on = move_by(
            &f(1.0, 40.0, 20.0, 20.0),
            0.0,
            0.0,
            true,
            &snap_targets(&[]),
        );
        assert_eq!(on.x_pct, 0.0);
        // Shift held (snap=false) → stays put.
        let off = move_by(
            &f(1.0, 40.0, 20.0, 20.0),
            0.0,
            0.0,
            false,
            &snap_targets(&[]),
        );
        assert_eq!(off.x_pct, 1.0);
    }

    #[test]
    fn move_snaps_to_another_elements_edge() {
        let other = f(50.0, 10.0, 10.0, 10.0); // left edge at x=50
                                               // Our left edge lands at 49.2 → within threshold of the other's 50 edge.
        let r = move_by(
            &f(49.2, 40.0, 10.0, 10.0),
            0.0,
            0.0,
            true,
            &snap_targets(&[other]),
        );
        assert_eq!(r.x_pct, 50.0);
    }

    #[test]
    fn resize_each_handle_moves_the_right_edges() {
        let base = f(20.0, 20.0, 40.0, 40.0); // left20 right60 top20 bottom60
        let no = SnapTargets::empty();

        // E: right edge only (+10 → w 50), x/y/h unchanged.
        let e = resize_by(&base, Handle::E, 10.0, 0.0, false, &no);
        assert_eq!(
            (e.x_pct, e.y_pct, e.w_pct, e.h_pct),
            (20.0, 20.0, 50.0, 40.0)
        );
        // W: left edge (-10 → x10, w50).
        let w = resize_by(&base, Handle::W, -10.0, 0.0, false, &no);
        assert_eq!(
            (w.x_pct, w.y_pct, w.w_pct, w.h_pct),
            (10.0, 20.0, 50.0, 40.0)
        );
        // S: bottom edge (+10 → h50).
        let s = resize_by(&base, Handle::S, 0.0, 10.0, false, &no);
        assert_eq!(
            (s.x_pct, s.y_pct, s.w_pct, s.h_pct),
            (20.0, 20.0, 40.0, 50.0)
        );
        // N: top edge (-10 → y10, h50).
        let n = resize_by(&base, Handle::N, 0.0, -10.0, false, &no);
        assert_eq!(
            (n.x_pct, n.y_pct, n.w_pct, n.h_pct),
            (20.0, 10.0, 40.0, 50.0)
        );
        // SE: right + bottom.
        let se = resize_by(&base, Handle::Se, 10.0, 10.0, false, &no);
        assert_eq!(
            (se.x_pct, se.y_pct, se.w_pct, se.h_pct),
            (20.0, 20.0, 50.0, 50.0)
        );
        // NW: left + top.
        let nw = resize_by(&base, Handle::Nw, -10.0, -10.0, false, &no);
        assert_eq!(
            (nw.x_pct, nw.y_pct, nw.w_pct, nw.h_pct),
            (10.0, 10.0, 50.0, 50.0)
        );
        // NE: right + top.
        let ne = resize_by(&base, Handle::Ne, 10.0, -10.0, false, &no);
        assert_eq!(
            (ne.x_pct, ne.y_pct, ne.w_pct, ne.h_pct),
            (20.0, 10.0, 50.0, 50.0)
        );
        // SW: left + bottom.
        let sw = resize_by(&base, Handle::Sw, -10.0, 10.0, false, &no);
        assert_eq!(
            (sw.x_pct, sw.y_pct, sw.w_pct, sw.h_pct),
            (10.0, 20.0, 50.0, 50.0)
        );
    }

    #[test]
    fn resize_enforces_min_size() {
        let base = f(20.0, 20.0, 40.0, 40.0);
        let no = SnapTargets::empty();
        // Drag the E handle far left → width floored at MIN_SIZE_PCT, x fixed.
        let e = resize_by(&base, Handle::E, -100.0, 0.0, false, &no);
        assert_eq!(e.x_pct, 20.0);
        assert_eq!(e.w_pct, MIN_SIZE_PCT);
        // Drag the W handle far right → width floored, RIGHT edge (60) fixed.
        let w = resize_by(&base, Handle::W, 100.0, 0.0, false, &no);
        assert_eq!(w.w_pct, MIN_SIZE_PCT);
        assert_eq!(w.x_pct, 60.0 - MIN_SIZE_PCT);
    }

    #[test]
    fn resize_clamps_edges_into_canvas() {
        let base = f(20.0, 20.0, 40.0, 40.0);
        let no = SnapTargets::empty();
        // Push the right edge way past 100 → clamps to 100 (w=80).
        let e = resize_by(&base, Handle::E, 90.0, 0.0, false, &no);
        assert_eq!(e.w_pct, 80.0);
        // Push the top edge above 0 → clamps to 0 (y=0, h=60).
        let n = resize_by(&base, Handle::N, 0.0, -50.0, false, &no);
        assert_eq!((n.y_pct, n.h_pct), (0.0, 60.0));
    }

    #[test]
    fn resize_snaps_moving_edge() {
        let base = f(20.0, 20.0, 39.0, 40.0); // right edge at 59
                                              // E handle +0 keeps right at 59, within 1.5 of the 60 centre? 60 is a guide.
        let e = resize_by(&base, Handle::E, 0.6, 0.0, true, &snap_targets(&[]));
        // right 59.6 snaps to the 60 canvas... no guide at 60; guides are 0/50/100.
        // Add an explicit target at 60 via a sibling whose left edge is 60.
        let sib = f(60.0, 0.0, 10.0, 10.0);
        let e2 = resize_by(&base, Handle::E, 0.6, 0.0, true, &snap_targets(&[sib]));
        assert_eq!(e2.w_pct, 40.0); // right snapped to 60 → w = 60-20
                                    // (the no-target case just applies the delta, no snap)
        assert_eq!(e.w_pct, round1(39.6));
    }

    #[test]
    fn nudge_steps_and_clamps() {
        let r = nudge(&f(10.0, 10.0, 20.0, 20.0), 0.1, 0.0);
        assert_eq!(r.x_pct, 10.1);
        let r2 = nudge(&f(10.0, 10.0, 20.0, 20.0), 1.0, 0.0);
        assert_eq!(r2.x_pct, 11.0);
    }

    #[test]
    fn clamp_helpers_hit_core_bounds() {
        assert_eq!(clamp_pos(9999.0), STREAM_FRAME_POS_MAX_PCT);
        assert_eq!(clamp_pos(-9999.0), STREAM_FRAME_POS_MIN_PCT);
        assert_eq!(clamp_size(0.0), MIN_SIZE_PCT);
        assert_eq!(clamp_size(9999.0), STREAM_FRAME_SIZE_MAX_PCT);
    }

    #[test]
    fn handle_str_roundtrips() {
        for h in Handle::ALL {
            assert_eq!(Handle::from_str(h.as_str()), Some(h));
        }
        assert_eq!(Handle::from_str("bogus"), None);
    }
}
