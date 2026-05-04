# Timer countdown formatting and post-zero clearing

**Issue:** #280

**Date:** 2026-05-04

**Branch:** dev (workspace 0.4.60)

## Problem

Two complaints about the countdown timer:

1. **Zero stays on screen forever.** When the countdown reaches 0, the display shows "0" indefinitely on stage, in the operator panel, and on Resolume. The user wants the timer to show "0" briefly (so the operator sees it hit zero), then clear.
2. **Multi-hour formatting is unreadable.** A 91-minute countdown currently renders as "91:30" (MM:SS rolled past 60 minutes). The user wants "1h 31m" with hour and minute units.

A third issue revealed by code reading: there are three separate `format_*` helpers across the codebase (server, WASM stage, WASM operator panel) that drift. This spec consolidates the countdown format into a single function in `presenter-core`.

## Goal

After hitting zero, the countdown shows "0" for 10 seconds, then clears (empty string sent to Resolume; blank cell on stage and operator panel). Long countdowns render with hour/minute precision (`"1h 31m"`) — minutes only, seconds dropped, round down. Format is consistent everywhere the countdown is shown.

## Format spec

```
seconds_remaining        display
< -10                    ""              (cleared — 10s past zero)
-10..=0                  "0"
1..=59                   "1", "59"       (just the digit)
60..=3599                "MM:SS"         e.g. "01:00", "59:59"
>= 3600                  "Xh Ym"         e.g. "1h 31m" (round down, drop seconds)
```

Boundary samples used in tests:

| seconds_remaining | display |
|---|---|
| 3605 | `"1h 0m"` |
| 5430 | `"1h 30m"` |
| 7199 | `"1h 59m"` |
| 7200 | `"2h 0m"` |
| 125 | `"02:05"` |
| 60 | `"01:00"` |
| 59 | `"59"` |
| 1 | `"1"` |
| 0 | `"0"` |
| -5 | `"0"` |
| -10 | `"0"` |
| -11 | `""` |
| -100 | `""` |

## Architecture

### Single source: `presenter_core::format_countdown`

Add a free function in `crates/presenter-core/src/timer.rs`:

```rust
/// Format a countdown for display on Resolume, stage, and operator UI.
/// See spec: docs/superpowers/specs/2026-05-04-timer-format-design.md
pub fn format_countdown(seconds_remaining: i64) -> String {
    if seconds_remaining < -10 {
        return String::new();
    }
    if seconds_remaining <= 0 {
        return "0".to_string();
    }
    let secs = seconds_remaining;
    if secs < 60 {
        return secs.to_string();
    }
    if secs < 3600 {
        let m = secs / 60;
        let s = secs % 60;
        return format!("{m:02}:{s:02}");
    }
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    format!("{h}h {m}m")
}
```

### Allow signed seconds_remaining in the overview

In `crates/presenter-core/src/timer.rs:326-343` (`overview_with_local_format`), remove the `max(countdown_remaining, 0)` clamp on `remaining_seconds` so the snapshot can carry a negative value. The formatter handles negative.

```rust
fn overview_with_local_format(&self, now: DateTime<Utc>, target_local: &str) -> TimersOverview {
    let countdown_remaining = self.countdown.remaining(now).num_seconds();
    let elapsed_seconds = self.preach.elapsed(now).num_seconds();
    TimersOverview {
        countdown_to_start: CountdownTimerSnapshot {
            state: self.countdown.state,
            target: self.countdown.target,
            target_local: target_local.to_string(),
            seconds_remaining: countdown_remaining, // no clamp
        },
        ...
    }
}
```

### Server side

- `crates/presenter-server/src/state/stage.rs:325` — replace `format_countdown_text` with a thin wrapper, or remove entirely and import `presenter_core::format_countdown` everywhere it's called.

```rust
pub(crate) fn format_countdown_text(seconds_remaining: i64) -> String {
    presenter_core::format_countdown(seconds_remaining)
}
```

The Resolume worker already handles empty-string PUTs correctly via the existing timer dedup logic (`last_timer_payload`). When the formatter returns `""`, the worker sends an empty text PUT to Resolume's text param, which displays as blank. Subsequent ticks dedup on `""` until the value changes again.

### WASM side

Two call sites switch from the local `format_seconds` to `presenter_core::format_countdown`:

- `crates/presenter-ui/src/components/stage/timer_layout.rs:23` — stage display
- `crates/presenter-ui/src/components/timer_panel.rs:193` — operator timer panel

Both files KEEP their `format_seconds` helper for the preach timer (elapsed time, different semantics — never negative, never has a "post-zero clear" concept).

### Existing `format_countdown_text` test update

`crates/presenter-server/src/state/tests.rs:263-267` currently asserts:

```rust
assert_eq!(format_countdown_text(3605), "60:05");
assert_eq!(format_countdown_text(125), "02:05");
assert_eq!(format_countdown_text(59), "59");
assert_eq!(format_countdown_text(0), "0");
assert_eq!(format_countdown_text(-12), "0");
```

The new format changes the first and last:

```rust
assert_eq!(format_countdown_text(3605), "1h 0m");
assert_eq!(format_countdown_text(125), "02:05");
assert_eq!(format_countdown_text(59), "59");
assert_eq!(format_countdown_text(0), "0");
assert_eq!(format_countdown_text(-12), "");
```

## Testing

### Unit tests in `crates/presenter-core/src/timer.rs`

Add 13 assertions covering the format matrix from the spec table. One test function with all the asserts.

### Existing test updates

- `crates/presenter-server/src/state/tests.rs:263-267` — update to new format.
- Any test that asserts `seconds_remaining >= 0` on the overview must be examined; if it relied on the clamp, update to expect signed.

### Manual verification on dev

1. Set the countdown target ~1h 31m in the future via the operator UI Timers panel. Verify operator panel + stage display + Resolume #timer all show `"1h 31m"`.
2. Trigger the countdown to hit 0. Verify all three surfaces show `"0"` for ~10 seconds, then go blank/empty.

## File-level changes

| File | Change |
|------|--------|
| `crates/presenter-core/src/timer.rs` | Add `format_countdown` (+13 tests). Remove `max(0)` clamp at line 328. |
| `crates/presenter-server/src/state/stage.rs:325` | Replace body with delegation to `presenter_core::format_countdown`. |
| `crates/presenter-server/src/state/tests.rs:263-267` | Update assertions for new format. |
| `crates/presenter-ui/src/components/stage/timer_layout.rs:23` | Swap helper for countdown. Keep `format_seconds` for preach. |
| `crates/presenter-ui/src/components/timer_panel.rs:193` | Same swap. Keep `format_seconds` for preach lines 220, 228. |

## Acceptance

- `format_countdown` unit tests pass for all 13 boundary cases.
- All existing presenter-server and presenter-ui tests pass after the format update.
- `cargo clippy --workspace --all-targets -- -D warnings -W clippy::all` clean.
- Manual verification: 1h 31m countdown shows `"1h 31m"` everywhere; reaching 0 shows `"0"` for 10s then clears on every surface.
- Browser console clean.

## Out of scope

- Preach timer formatting (different semantics — elapsed, never negative).
- Consolidating the 3 `format_seconds` WASM helpers (only `format_countdown` is consolidated; preach helpers stay).
- Operator countdown editing UX, target-time pickers, etc.
- API/snapshot field renames or new fields — only the existing `seconds_remaining: i64` semantics change (clamp removed).
