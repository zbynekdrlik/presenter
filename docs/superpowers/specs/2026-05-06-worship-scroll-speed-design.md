# Worship slide list scroll speed fix

**Issue:** #301

**Date:** 2026-05-06

**Branch:** dev (workspace 0.4.68, presenter-ui 0.1.37)

## Problem

Mouse-wheel and trackpad scrolling in the worship slide list ("`.operator__slides`") is "extremely fast" — a single gesture jumps dozens of rows. Filed by the user on 2026-05-06.

The current `handle_wheel_event` in `crates/presenter-ui/src/components/slide_list_scroll.rs:128-146` was added in PR #290 (issue #271) to neutralize macOS scroll acceleration. It calls `ev.prevent_default()`, then advances the scroll position by `direction * step` where:

- `direction = ev.delta_y().signum()` — only `+1` or `-1`
- `step = step_for_wheel(container)` — one row of cards (≈ card_height + 14.4 px gap)

The fix in #271 ignored the wheel delta's magnitude and always scrolled exactly one row per event. Browsers / OSes fire many wheel events per gesture (trackpads stream 20-30+ events per swipe; high-DPI mice fire continuously while the wheel turns). One row per event × 30 events = 30 rows per gesture → "extremely fast".

## Goal

Wheel scroll feels natural — small gestures advance a few pixels, big gestures advance multiple rows, but no single event can jump more than one row (which would feel jumpy and bypass the original #271 "no momentum acceleration" intent).

## Architecture

Single-line behavior change in `handle_wheel_event`. Use the wheel's `delta_y` magnitude directly, capped at `step` per event:

```rust
let delta_y = ev.delta_y();
if delta_y == 0.0 { return; }
// ... container lookup unchanged ...
let step = step_for_wheel(&container);
let capped = delta_y.signum() * delta_y.abs().min(step);
container.set_scroll_top((container.scroll_top() as f64 + capped) as i32);
```

### Behavior matrix

| `delta_y` | Old behavior | New behavior |
|-----------|--------------|--------------|
| `+10` (small swipe nudge) | scrolls one full row (~90 px) | scrolls 10 px |
| `+90` (one notch on a Mighty Mouse) | scrolls one full row | scrolls 90 px (still ≈ one row) |
| `+500` (high-DPI fast swipe, single event) | scrolls one full row | scrolls one row (capped) |
| `-200` (negative direction) | scrolls one full row up | scrolls one row up (capped) |
| `0` | no-op | no-op |

For trackpad gestures that fire 20+ events: each is capped at one row, but small-delta events advance proportionally. A gentle 100-px gesture made of twenty 5-px events advances 100 px (about one row). A fast flick made of ten 50-px events advances 10 rows — appropriate for the gesture intensity.

## Tests

The existing `slide_list_scroll.rs` likely has minimal unit-test coverage for `handle_wheel_event` because it depends on `web_sys::WheelEvent` (browser-only type). Two acceptable approaches:

1. **Extract the cap logic into a pure function** that takes `delta_y` and `step` as floats. Unit-test that. Wire the helper into `handle_wheel_event`.
2. **Skip unit tests, rely on manual verification** on dev with both a trackpad and a mouse wheel.

Recommend (1) — the math is simple and worth one unit test for the boundary cases (delta = 0, delta < step, delta > step, negative delta).

### Manual verification on dev

After deploy:

1. Open `http://10.77.8.134:8080/ui/operator` in a browser with at least 30 worship slides loaded.
2. Trackpad: gentle two-finger swipe → list scrolls smoothly, ≈ 1-2 rows for a small gesture, several rows for a larger one. No "blast through 30 rows".
3. Mouse wheel: each notch advances about one row of cards. Smooth but bounded.
4. The list does NOT scroll past the last card (browser default behavior).
5. Browser console clean.

## File-level changes

| File | Change |
|------|--------|
| `crates/presenter-ui/src/components/slide_list_scroll.rs` | `handle_wheel_event` uses `delta_y.signum() * delta_y.abs().min(step)` instead of `direction * step`. Optional: extract `cap_wheel_delta(delta_y, step) -> f64` for unit testing. |

## Acceptance

- Manual scroll test on dev passes (smooth, bounded scroll).
- Workspace tests pass.
- Native + WASM clippy clean.
- Browser console clean during E2E.
- The original #271 fix (no macOS unbounded acceleration) is still in effect — verified by the per-event cap.

## Out of scope

- Other scroll containers (catalog, presentation list, bible). The bug report and the wheel handler both target `.operator__slides` only.
- Keyboard navigation (arrow keys, Page Up/Down) — handled by separate code paths.
- Scrollbar behavior — unchanged.

## Risk

Low. Single function body change. The cap preserves the upper bound that #271 wanted; only the lower bound (small gestures) becomes proportional to the gesture rather than always one row.
