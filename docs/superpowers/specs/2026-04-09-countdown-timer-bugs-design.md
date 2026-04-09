# Countdown Timer Bug Fixes (#212)

> **Status:** Approved | **Date:** 2026-04-09

## Problem

The countdown timer has four regressions:

1. **Time is ~2 hours off** — `parse_time_input()` in `timer_panel.rs` constructs targets using `chrono::Utc::now()` in WASM. User types "18:00" meaning local time (Czech, UTC+2), but the code creates 18:00 UTC. All display formatting also uses UTC. The entire timer is offset by the user's timezone.

2. **Start/stop is unreliable** — consequence of Bug 1 (wrong target makes behavior seem broken) plus potential state machine edge cases when target is in the past.

3. **Input too complex** — typing "18" creates a target 18 minutes from now instead of 18:00. The +5/-5 min buttons require client-side target reading, which couples to the timezone bug.

4. **OBS overlay glitches** — race condition between `setInterval(1s)` local decrement and WebSocket `seconds_remaining` updates. Server and client tick at different moments, causing ±1 second jumps every second.

## Design

### Principle: Server is the single source of truth for time

The server runs in the correct timezone. All time interpretation and formatting happens server-side. Clients send intent ("count to 18:00"), not pre-computed UTC timestamps.

### 1. New Timer Commands

Add two new `TimerCommand` variants in `presenter-core/src/timer.rs`:

```rust
TimerCommand::SetCountdownTargetLocal { hours: u32, minutes: u32 }
TimerCommand::AdjustCountdownTarget { offset_minutes: i32 }
```

- `SetCountdownTargetLocal`: Server converts `hours:minutes` to a `DateTime<Utc>` target using `chrono::Local`. If the resulting local time is in the past, shifts to tomorrow.
- `AdjustCountdownTarget`: Server adds/subtracts `offset_minutes` from the current target. No client-side target reading needed.
- Keep existing `SetCountdownTarget { target: DateTime<Utc> }` for programmatic/test use but the WASM UI will no longer use it.

**Server-side conversion** in `apply_command()`:

```rust
TimerCommand::SetCountdownTargetLocal { hours, minutes } => {
    let local_now = Local::now();
    let today = local_now.date_naive();
    let time = NaiveTime::from_hms_opt(hours, minutes, 0)
        .ok_or(TimerError::InvalidCommand("invalid time"))?;
    let mut target_local = today.and_time(time).and_local_timezone(Local).unwrap();
    if target_local <= local_now {
        target_local += chrono::Duration::days(1);
    }
    let target_utc = target_local.with_timezone(&Utc);
    // ... set target, preserve running state
}
```

### 2. Snapshot: Add `target_local` field

Add `target_local: String` to `CountdownTimerSnapshot`:

```rust
pub struct CountdownTimerSnapshot {
    pub state: TimerState,
    pub target: DateTime<Utc>,
    pub target_local: String,        // e.g. "18:00:00" in server's local TZ
    pub seconds_remaining: i64,
}
```

Generated in `TimersState::overview()` using `chrono::Local`:

```rust
let target_local = self.countdown.target
    .with_timezone(&chrono::Local)
    .format("%H:%M:%S")
    .to_string();
```

All clients display `target_local` instead of formatting UTC.

### 3. WASM Client: Simplified input parsing

Move the "smart" parsing to the client but only extract `(hours, minutes)` — no UTC conversion:

- Single number 0-23 → `SetCountdownTargetLocal { hours: N, minutes: 0 }`
- "HH:MM" → `SetCountdownTargetLocal { hours: H, minutes: M }`
- Invalid input → show no change (silent, as before)

The +5/-5 buttons change from:
```rust
// Old: read target from overview, add offset, send SetCountdownTarget
let new_target = current_target + Duration::minutes(5);
send_timer_cmd(TimerCommand::SetCountdownTarget { target: new_target });

// New: just send offset
send_timer_cmd(TimerCommand::AdjustCountdownTarget { offset_minutes: 5 });
```

### 4. OBS Overlay: Eliminate flicker

**Root cause:** `applyOverview()` overwrites `remaining` from `seconds_remaining`, conflicting with local `remainingFromTarget()` calculation.

**Fix:** In `applyOverview()`, only update `targetEpochMs` and `state`. Never assign `remaining` from `seconds_remaining` — let the tick loop derive it exclusively from `targetEpochMs`. Keep `seconds_remaining` only as initial bootstrap (first render before setInterval fires).

Also: use `requestAnimationFrame` with second-boundary detection instead of `setInterval(1000)` to prevent timer drift and ensure smooth updates.

```javascript
// Replace setInterval with rAF loop
let lastRenderedSecond = -1;
const tick = () => {
    if (state === 'running') {
        const derived = remainingFromTarget();
        if (derived !== null) remaining = derived;
        const currentSecond = Math.floor(remaining);
        if (currentSecond !== lastRenderedSecond) {
            lastRenderedSecond = currentSecond;
            render();
        }
    }
    requestAnimationFrame(tick);
};
requestAnimationFrame(tick);
```

In `applyOverview()`:
```javascript
// Only update target and state, NOT remaining
if (parsedTarget !== null) targetEpochMs = parsedTarget;
if (typeof nextCountdown.state === 'string') state = nextCountdown.state.toLowerCase();
// Force re-render on state change (pause/resume)
render();
```

### 5. Start/stop behavior

- **Start** with no future target: auto-set target to now + 15 minutes, then start. Currently `start()` just sets `state = Running` without checking if target is valid — it can start a countdown to a past target, which immediately shows 0:00.
- **Reset**: set state to Idle (existing behavior is fine).
- **Pause/Resume**: preserve target, toggle state (existing behavior is fine once timezone is fixed).

### 6. Display formatting: `target_local` everywhere

| Location | Current | New |
|----------|---------|-----|
| Operator panel (`timer_panel.rs` line 219) | `target.format("%H:%M:%S")` (UTC) | `overview.countdown_to_start.target_local` |
| OBS overlay initial render | UTC target | `target_local` from initial JSON |
| Stage display | Uses `seconds_remaining` only | No change needed |
| Tablet timer bar | Shows clock via `js_sys::Date` | No change needed (already local) |

## Files Modified

| File | Change |
|------|--------|
| `crates/presenter-core/src/timer.rs` | Add `SetCountdownTargetLocal`, `AdjustCountdownTarget` commands; add `target_local` to snapshot; server-side local time conversion |
| `crates/presenter-ui/src/components/timer_panel.rs` | Simplify `parse_time_input` to return `(u32, u32)` not `DateTime`; use new commands; display `target_local` |
| `crates/presenter-server/src/ui/timer_overlay.rs` | Fix overlay JS: rAF loop, target-only derivation, no `remaining` overwrite |
| `crates/presenter-server/src/state/timers.rs` | Handle new command variants |
| `tests/e2e/wasm-timers.spec.ts` | New E2E: countdown input, +5/-5, overlay stability |

## Testing

- **Unit tests** (`timer.rs`): `SetCountdownTargetLocal` and `AdjustCountdownTarget` tests use `_with_now()` variants with explicit UTC timestamps (deterministic, CI-safe). Test the conversion logic with a helper that accepts a `FixedOffset` timezone parameter instead of relying on `chrono::Local` (which varies by machine). The production code uses `Local`, tests use `FixedOffset::east_opt(7200)` (UTC+2) for reproducibility.
- **E2E** (Playwright): Type "18" → verify target shows local time, click +5 → verify target advances, open overlay → verify no flicker over 10 seconds, start/pause/reset cycle
- **Existing tests**: All must pass unchanged (preach timer, tablet timer bar)
