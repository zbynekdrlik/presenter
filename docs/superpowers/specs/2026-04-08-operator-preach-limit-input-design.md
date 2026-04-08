# Operator Preach Limit Input — Design Spec

**Issue:** #171 (continuation)
**Date:** 2026-04-08
**Status:** Approved

## Problem

The preach timer limit (added in PR #216) can only be set from Companion. The operator needs to see and set the limit from the operator UI timer panel at `/ui/operator/timers`.

## Solution

Add a preach limit input field and display to the existing timer panel component. Follow the same UI patterns as the countdown target input.

## Changes

### Timer Panel Component (`timer_panel.rs`)

**Preach card (top display):**
- Show current limit below elapsed time: "Limit: 5:00" or "No limit" when unset
- Read from `timers.preach_timer.limit_seconds`

**Preach control section:**
- Add a labeled input field "Preach limit" above the Start/Pause/Reset buttons
- Same pattern as countdown target input: focus/blur/input/keydown handlers
- Input accepts HH:MM or minutes-only format (reuse `parse_time_input()` logic)
- On Enter: parse input to seconds, send `TimerCommand::SetPreachLimit { seconds }`
- Add a "Clear" button next to the input that sends `TimerCommand::ClearPreachLimit`
- When not focused and not dirty, display the current limit value from the API response

**Input parsing:**
- Reuse `parse_time_input()` approach but convert to duration in seconds:
  - "5" → 5 minutes → 300 seconds
  - "1:30" → 1 hour 30 minutes → 5400 seconds
  - "0:30" → 30 minutes → 1800 seconds
- New helper: `parse_limit_input(value: &str) -> Option<u64>` returns seconds

**State signals:**
- Add `preach_limit_input_active: RwSignal<bool>` to `OperatorState`
- Add `preach_limit_input_dirty: RwSignal<bool>` to `OperatorState`

### Styling (`operator.css`)

No new classes needed — reuse existing `.operator__timer-field`, `.operator__timer-field input`, and button styles.

## Files Modified

| File | Change |
|------|--------|
| `crates/presenter-ui/src/components/timer_panel.rs` | Add limit display, input field, clear button, event handlers |
| `crates/presenter-ui/src/state/operator.rs` | Add preach limit input state signals |

## Testing

### E2E Playwright Test

- Navigate to `/ui/operator/timers`
- Verify preach limit input is visible
- Type "5" and press Enter → verify limit is set (API returns `limitSeconds: 300`)
- Verify preach card shows "Limit: 5:00"
- Click Clear → verify limit is removed
- Clean console assertion
