# Tablet Preach Timer Bar — Design Spec

**Issue:** #171
**Date:** 2026-04-07
**Status:** Approved

## Problem

The speaker/pastor uses the tablet (Bible viewer at `/ui/tablet`) during services but has no visibility into how long they've been speaking or whether they're approaching their assigned time limit. They must rely on external clocks or guesswork.

## Solution

Add a persistent timer bar at the top of the tablet Bible view that shows:
1. **Wall clock** (HH:MM) — current time
2. **Preach elapsed** (MM:SS) — how long the speaker has been going
3. **Progressive color alerts** — background shifts green → orange → red → flashing as elapsed approaches/exceeds the configurable limit

The preach time limit is set from Bitfocus Companion via a new WebSocket command.

## Data Model Changes

### `PreachTimer` (presenter-core/src/timer.rs)

Add a `limit` field:

```rust
pub struct PreachTimer {
    pub state: TimerState,
    started_at: Option<DateTime<Utc>>,
    accumulated: Duration,
    limit: Option<Duration>,  // NEW
}
```

New methods:
- `set_limit(seconds: u64)` — sets `limit = Some(Duration::seconds(seconds as i64))`
- `clear_limit()` — sets `limit = None`
- `limit_seconds(&self) -> Option<u64>` — getter

### `PreachTimerSnapshot`

```rust
pub struct PreachTimerSnapshot {
    pub state: TimerState,
    pub seconds_elapsed: u64,
    pub limit_seconds: Option<u64>,  // NEW
}
```

### `TimerCommand`

Two new variants:

```rust
pub enum TimerCommand {
    // ... existing variants ...
    SetPreachLimit { seconds: u64 },
    ClearPreachLimit,
}
```

## Companion Integration

### New Commands

| Command | Payload | Effect |
|---------|---------|--------|
| `timer.set_preach_limit` | `{ "seconds": 300 }` | Sets preach limit to 300s (5 min) |
| `timer.clear_preach_limit` | `{}` | Removes preach limit |

### New Variable

| Variable | Format | Example |
|----------|--------|---------|
| `timer_preach_limit_seconds` | integer or `""` | `"300"` or `""` |

## Tablet UI Changes

### WebSocket Subscription

The tablet currently subscribes to `/live/ws` for Bible events only. Add handling for `LiveEvent::Timers` to receive `TimersOverview` updates (broadcast every second by the server's `tick_timers` background task).

### TabletTimerBar Component

New component rendered at the top of the tablet page, inside the existing layout but above the Bible content area.

**Layout (fixed top bar):**

```
┌──────────────────────────────────────────┐
│  14:23      ● 09:42  PREACH     RUNNING  │  ← timer bar
├──────────────────────────────────────────┤
│  Bible Tablet                    [≡]     │  ← existing header
│  ┌────────┐ ┌──────────────────────────┐ │
│  │Sidebar │ │ Slide content...         │ │
│  │        │ │                          │ │
│  └────────┘ └──────────────────────────┘ │
└──────────────────────────────────────────┘
```

**Bar content:**
- **Left:** Wall clock in HH:MM format, updates every second via `set_interval`
- **Center:** Preach elapsed in MM:SS (or H:MM:SS if > 1 hour), large bold text. Shows "—" when idle.
- **Right:** State label — IDLE / RUNNING / PAUSED

**Bar visibility:**
- Always visible (even when preach timer is idle — shows clock and "—")
- Compact height (~40px) to not steal too much screen from Bible content

### Progressive Color Zones

Color logic is computed client-side in WASM from `elapsed` and `limit_seconds`:

| Zone | Condition | Background | Border |
|------|-----------|------------|--------|
| No limit | `limit_seconds` is `None` | `#1e293b` (neutral dark) | `#334155` |
| Green | `elapsed < 90% of limit` | `#166534` | `#22c55e` |
| Orange | `elapsed >= 90% && < 100%` | `#92400e` | `#f59e0b` |
| Red | `elapsed >= 100% && < 120%` | `#991b1b` | `#ef4444` |
| Flashing | `elapsed >= 120%` | `#991b1b` pulsing | `#ef4444` |

CSS transition on background-color for smooth zone changes. `@keyframes` animation for the flashing state.

When preach timer is idle/paused, use the no-limit neutral style regardless of limit setting.

## Files Modified

| File | Change |
|------|--------|
| `crates/presenter-core/src/timer.rs` | Add `limit` field, `SetPreachLimit`/`ClearPreachLimit` commands, snapshot update |
| `crates/presenter-server/src/companion/protocol.rs` | Parse new commands |
| `crates/presenter-server/src/companion/variables.rs` | Add `timer_preach_limit_seconds` variable |
| `crates/presenter-ui/src/pages/tablet.rs` | Add WebSocket timer subscription, render `TabletTimerBar` |
| `crates/presenter-ui/styles/tablet.css` | Timer bar styles, color zones, flashing animation |

## Testing

### Unit Tests (presenter-core)

- `test_set_preach_limit` — set limit, verify `limit_seconds()` returns value
- `test_clear_preach_limit` — set then clear, verify `None`
- `test_preach_limit_in_snapshot` — verify snapshot includes limit
- `test_set_preach_limit_command` — apply command via `TimersState`, verify limit set

### Companion Protocol Tests

- `test_parse_set_preach_limit` — parse `timer.set_preach_limit` with `{ "seconds": 300 }`
- `test_parse_clear_preach_limit` — parse `timer.clear_preach_limit`
- `test_preach_limit_variable` — verify variable is included in companion variables

### E2E Playwright Test

- Start preach timer via API
- Open tablet page
- Verify timer bar shows elapsed time updating
- Set preach limit via API
- Advance time / wait and verify color transitions
- Verify wall clock displays current time
