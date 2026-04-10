# Countdown Timer Bug Fixes (#212) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix four countdown timer regressions: wrong timezone (2h off), buggy start/stop, overly complex input (typing "18" should mean 18:00), and OBS overlay flicker.

**Architecture:** Three-layer fix: (1) Add `SetCountdownTargetLocal` and `AdjustCountdownTarget` commands to `presenter-core` so the server resolves local time → UTC, (2) add `target_local` field to `CountdownTimerSnapshot` formatted server-side via `chrono::Local`, (3) fix OBS overlay JS to derive remaining exclusively from target epoch via `requestAnimationFrame`. The WASM client simplifies to parsing `(hours, minutes)` from user input and sending intent — no UTC conversion.

**Tech Stack:** Rust (chrono, chrono::Local, axum), WASM/Leptos (presenter-ui), JavaScript (overlay), Playwright E2E

**Spec:** `docs/superpowers/specs/2026-04-09-countdown-timer-bugs-design.md`

---

## Context

Issue #212: The operator reported four timer regressions:
1. Time is ~2 hours off — `parse_time_input()` in WASM uses UTC, user expects local time (Czech, UTC+2)
2. Start/stop unreliable — consequence of wrong target, plus `start()` doesn't guard against past targets
3. Input too complex — "18" is parsed as 18 minutes from now, not 18:00
4. OBS overlay flickers — race between `setInterval(1s)` local decrement and WebSocket `seconds_remaining` updates

**Key existing code:**
- `crates/presenter-core/src/timer.rs` — `TimerCommand`, `TimersState`, `CountdownTimerSnapshot`, `apply_command()`, unit tests
- `crates/presenter-server/src/state/timers.rs` — `execute_timer_command()`, `tick_timers()`, `load_or_init_timers()`
- `crates/presenter-server/src/router/timers.rs` — `GET /timers/overview`, `POST /timers/command`
- `crates/presenter-server/src/ui/timer_overlay.rs` — OBS overlay with embedded JS
- `crates/presenter-ui/src/components/timer_panel.rs` — WASM operator panel, `parse_time_input()`, `parse_limit_input()`
- `tests/e2e/wasm-timers.spec.ts` — existing E2E tests
- `crates/presenter-core/src/contract_tests.rs` — `timers_overview_roundtrip` contract test

---

## File Structure

### Modified Files
| File | Change |
|------|--------|
| `crates/presenter-core/src/timer.rs` | Add `SetCountdownTargetLocal`, `AdjustCountdownTarget` commands; add `target_local: String` to `CountdownTimerSnapshot`; update `apply_command()` with new variants; add `overview_with_local_format()` method |
| `crates/presenter-server/src/state/timers.rs` | Use `chrono::Local` to resolve local time in new commands; format `target_local` in overview generation |
| `crates/presenter-server/src/ui/timer_overlay.rs` | Replace `setInterval` with `requestAnimationFrame`; stop overwriting `remaining` from WS messages |
| `crates/presenter-ui/src/components/timer_panel.rs` | Rewrite `parse_time_input()` to return `(u32, u32)`; use `SetCountdownTargetLocal`; use `AdjustCountdownTarget` for ±5; display `target_local` |
| `tests/e2e/wasm-timers.spec.ts` | Add tests for local time input, +5/-5 via API, overlay stability |
| `crates/presenter-core/src/contract_tests.rs` | Update `timers_overview_roundtrip` for new `target_local` field |
| `crates/presenter-server/src/router/tests.rs` | Add tests for new command variants |

---

## Task 1: Add New Timer Commands and `target_local` to Core

**Files:**
- Modify: `crates/presenter-core/src/timer.rs:205-310`

- [ ] **Step 1: Write failing tests for new commands**

In `crates/presenter-core/src/timer.rs`, add these tests at the end of the `mod tests` block (before the closing `}`):

```rust
#[test]
fn set_countdown_target_local_converts_to_utc() {
    let now = Utc::now();
    let mut state = TimersState::default(now);
    // Set target to a time that's definitely in the future
    let future_hour = (chrono::Local::now().hour() + 2) % 24;
    state
        .apply_command(
            &TimerCommand::SetCountdownTargetLocal {
                hours: future_hour,
                minutes: 0,
            },
            now,
        )
        .unwrap();
    // Target should be set and in the future
    assert!(state.countdown.target > now);
    // State should be Idle after setting target
    assert_eq!(state.countdown.state, TimerState::Idle);
}

#[test]
fn set_countdown_target_local_past_time_rolls_to_tomorrow() {
    let now = Utc::now();
    let mut state = TimersState::default(now);
    // Set target to an hour ago (local time)
    let local_now = chrono::Local::now();
    let past_hour = if local_now.hour() == 0 { 23 } else { local_now.hour() - 1 };
    state
        .apply_command(
            &TimerCommand::SetCountdownTargetLocal {
                hours: past_hour,
                minutes: 0,
            },
            now,
        )
        .unwrap();
    // Target should be tomorrow, so > 22 hours from now
    let remaining = state.countdown.remaining(now);
    assert!(remaining.num_hours() >= 22, "expected tomorrow, got {} hours remaining", remaining.num_hours());
}

#[test]
fn set_countdown_target_local_preserves_running_state() {
    let now = Utc::now();
    let mut state = TimersState::default(now);
    state.countdown.start();
    assert_eq!(state.countdown.state, TimerState::Running);
    let future_hour = (chrono::Local::now().hour() + 2) % 24;
    state
        .apply_command(
            &TimerCommand::SetCountdownTargetLocal {
                hours: future_hour,
                minutes: 0,
            },
            now,
        )
        .unwrap();
    assert_eq!(state.countdown.state, TimerState::Running);
}

#[test]
fn set_countdown_target_local_rejects_invalid_time() {
    let now = Utc::now();
    let mut state = TimersState::default(now);
    let result = state.apply_command(
        &TimerCommand::SetCountdownTargetLocal {
            hours: 25,
            minutes: 0,
        },
        now,
    );
    assert!(result.is_err());
}

#[test]
fn adjust_countdown_target_adds_minutes() {
    let now = Utc::now();
    let mut state = TimersState::default(now);
    let original_target = state.countdown.target;
    state
        .apply_command(
            &TimerCommand::AdjustCountdownTarget { offset_minutes: 5 },
            now,
        )
        .unwrap();
    let diff = state.countdown.target - original_target;
    assert_eq!(diff.num_minutes(), 5);
}

#[test]
fn adjust_countdown_target_subtracts_minutes() {
    let now = Utc::now();
    let target = now + Duration::minutes(30);
    let mut state = TimersState::new(
        CountdownTimer::new_with_now(target, now).unwrap(),
        PreachTimer::new(),
    );
    state
        .apply_command(
            &TimerCommand::AdjustCountdownTarget { offset_minutes: -5 },
            now,
        )
        .unwrap();
    let diff = target - state.countdown.target;
    assert_eq!(diff.num_minutes(), 5);
}

#[test]
fn adjust_countdown_target_rejects_result_in_past() {
    let now = Utc::now();
    let target = now + Duration::minutes(3);
    let mut state = TimersState::new(
        CountdownTimer::new_with_now(target, now).unwrap(),
        PreachTimer::new(),
    );
    let result = state.apply_command(
        &TimerCommand::AdjustCountdownTarget { offset_minutes: -5 },
        now,
    );
    assert!(result.is_err());
}

#[test]
fn adjust_countdown_target_preserves_running_state() {
    let now = Utc::now();
    let mut state = TimersState::default(now);
    state.countdown.start();
    state
        .apply_command(
            &TimerCommand::AdjustCountdownTarget { offset_minutes: 5 },
            now,
        )
        .unwrap();
    assert_eq!(state.countdown.state, TimerState::Running);
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p presenter-core -- timer --nocapture
```

Expected: compilation errors — `SetCountdownTargetLocal` and `AdjustCountdownTarget` don't exist yet.

- [ ] **Step 3: Add new command variants to `TimerCommand`**

In `crates/presenter-core/src/timer.rs`, replace the `TimerCommand` enum (lines 205-217):

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "command")]
pub enum TimerCommand {
    SetCountdownTarget {
        target: DateTime<Utc>,
    },
    SetCountdownTargetLocal {
        hours: u32,
        minutes: u32,
    },
    AdjustCountdownTarget {
        offset_minutes: i32,
    },
    StartCountdown,
    PauseCountdown,
    ResetCountdown,
    StartPreach,
    PausePreach,
    ResetPreach,
    SetPreachLimit {
        seconds: u64,
    },
    ClearPreachLimit,
}
```

- [ ] **Step 4: Add `target_local` to `CountdownTimerSnapshot`**

In `crates/presenter-core/src/timer.rs`, replace `CountdownTimerSnapshot` (lines 304-310):

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CountdownTimerSnapshot {
    pub state: TimerState,
    pub target: DateTime<Utc>,
    pub target_local: String,
    pub seconds_remaining: i64,
}
```

- [ ] **Step 5: Update `TimersState::overview()` to include `target_local`**

In `crates/presenter-core/src/timer.rs`, replace the `overview` method (lines 285-301):

```rust
    pub fn overview(&self, now: DateTime<Utc>) -> TimersOverview {
        self.overview_with_local_format(now, &self.countdown.target.with_timezone(&chrono::Local).format("%H:%M:%S").to_string())
    }

    pub fn overview_with_local_format(&self, now: DateTime<Utc>, target_local: &str) -> TimersOverview {
        let countdown_remaining = self.countdown.remaining(now).num_seconds();
        let remaining_seconds = max(countdown_remaining, 0);
        let elapsed_seconds = self.preach.elapsed(now).num_seconds();
        TimersOverview {
            countdown_to_start: CountdownTimerSnapshot {
                state: self.countdown.state,
                target: self.countdown.target,
                target_local: target_local.to_string(),
                seconds_remaining: remaining_seconds,
            },
            preach_timer: PreachTimerSnapshot {
                state: self.preach.state,
                seconds_elapsed: elapsed_seconds,
                limit_seconds: self.preach.limit_seconds(),
            },
        }
    }
```

Add the `chrono::Local` import at the top of the file (after the existing `use chrono::{DateTime, Duration, Utc};`):

```rust
use chrono::{DateTime, Duration, Local, NaiveTime, Utc};
```

Also add `#[cfg(test)]` import for `Local` in the test module is already covered by the wildcard `use super::*`.

- [ ] **Step 6: Implement `apply_command` for new variants**

In `crates/presenter-core/src/timer.rs`, in `apply_command()` (lines 242-283), add the two new match arms after `SetCountdownTarget`:

```rust
            TimerCommand::SetCountdownTargetLocal { hours, minutes } => {
                let time = NaiveTime::from_hms_opt(*hours, *minutes, 0)
                    .ok_or(TimerError::InvalidCommand("invalid hours/minutes"))?;
                let local_now = Local::now();
                let today = local_now.date_naive();
                let candidate = today
                    .and_time(time)
                    .and_local_timezone(Local)
                    .single()
                    .ok_or(TimerError::InvalidCommand("ambiguous local time"))?;
                let target_local = if candidate <= local_now {
                    candidate + Duration::days(1)
                } else {
                    candidate
                };
                let target_utc = target_local.with_timezone(&Utc);
                let previous_state = self.countdown.state;
                self.countdown.set_target_with_now(target_utc, now)?;
                match previous_state {
                    TimerState::Running => self.countdown.start(),
                    TimerState::Paused => self.countdown.state = TimerState::Paused,
                    _ => {}
                }
            }
            TimerCommand::AdjustCountdownTarget { offset_minutes } => {
                let new_target =
                    self.countdown.target + Duration::minutes(i64::from(*offset_minutes));
                let previous_state = self.countdown.state;
                self.countdown.set_target_with_now(new_target, now)?;
                match previous_state {
                    TimerState::Running => self.countdown.start(),
                    TimerState::Paused => self.countdown.state = TimerState::Paused,
                    _ => {}
                }
            }
```

- [ ] **Step 7: Update `TimersOverview::demo()` for new `target_local` field**

In `crates/presenter-core/src/timer.rs`, replace the `demo` method (lines 329-343):

```rust
    pub fn demo(now: DateTime<Utc>) -> Self {
        let countdown_target = now + Duration::minutes(15);
        let target_local = countdown_target
            .with_timezone(&Local)
            .format("%H:%M:%S")
            .to_string();
        Self {
            countdown_to_start: CountdownTimerSnapshot {
                state: TimerState::Running,
                target: countdown_target,
                target_local,
                seconds_remaining: (countdown_target - now).num_seconds(),
            },
            preach_timer: PreachTimerSnapshot {
                state: TimerState::Paused,
                seconds_elapsed: 0,
                limit_seconds: None,
            },
        }
    }
```

- [ ] **Step 8: Run tests**

```bash
cargo test -p presenter-core -- timer --nocapture
```

Expected: All existing tests pass plus 8 new tests pass.

- [ ] **Step 9: Update contract test**

In `crates/presenter-core/src/contract_tests.rs`, the `timers_overview_roundtrip` test (line 289) should still pass since `target_local` is a `String` that serializes/deserializes cleanly. Run to verify:

```bash
cargo test -p presenter-core -- contract --nocapture
```

Expected: All contract tests pass.

- [ ] **Step 10: Commit**

```bash
cargo fmt --all
git add crates/presenter-core/src/timer.rs
git commit -m "feat(timer): add SetCountdownTargetLocal and AdjustCountdownTarget commands (#212)

Add server-side local time resolution via chrono::Local. Single
numbers (18) are interpreted as hours (18:00 local). Add target_local
field to CountdownTimerSnapshot for pre-formatted local display.
AdjustCountdownTarget replaces client-side +5/-5 offset logic."
```

---

## Task 2: Update Server State to Use New Commands

**Files:**
- Modify: `crates/presenter-server/src/state/timers.rs`
- Modify: `crates/presenter-server/src/state/mod.rs:655-658`
- Modify: `crates/presenter-server/src/router/tests.rs`

- [ ] **Step 1: Write failing router tests for new commands**

In `crates/presenter-server/src/router/tests.rs`, add after the `timers_command_endpoint_rejects_past_targets` test (after line 1450):

```rust
#[tokio::test]
async fn timers_command_set_countdown_target_local() {
    let app = build_router(AppState::in_memory().await.unwrap());
    // Use an hour in the future (local time)
    let future_hour = (chrono::Local::now().hour() + 2) % 24;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/timers/command")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "command": "set_countdown_target_local",
                        "hours": future_hour,
                        "minutes": 0
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(
        status,
        StatusCode::OK,
        "error body: {}",
        String::from_utf8_lossy(&body)
    );

    let payload: TimersOverview = serde_json::from_slice(&body).unwrap();
    assert!(payload.countdown_to_start.seconds_remaining > 0);
    assert!(!payload.countdown_to_start.target_local.is_empty());
}

#[tokio::test]
async fn timers_command_adjust_countdown_target() {
    let app = build_router(AppState::in_memory().await.unwrap());

    // Get initial overview to know the default target
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/timers/overview")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let initial: TimersOverview = serde_json::from_slice(&bytes).unwrap();
    let initial_remaining = initial.countdown_to_start.seconds_remaining;

    // Adjust by +5 minutes
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/timers/command")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "command": "adjust_countdown_target",
                        "offset_minutes": 5
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(
        status,
        StatusCode::OK,
        "error body: {}",
        String::from_utf8_lossy(&body)
    );

    let payload: TimersOverview = serde_json::from_slice(&body).unwrap();
    // Should be ~5 minutes (300s) more than initial, with some tolerance for test execution time
    let diff = payload.countdown_to_start.seconds_remaining - initial_remaining;
    assert!(
        (295..=305).contains(&diff),
        "expected ~300s increase, got {diff}"
    );
}

#[tokio::test]
async fn timers_overview_includes_target_local() {
    let app = build_router(AppState::in_memory().await.unwrap());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/timers/overview")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let payload: TimersOverview = serde_json::from_slice(&bytes).unwrap();
    // target_local should be a non-empty HH:MM:SS string
    assert!(
        payload.countdown_to_start.target_local.len() >= 7,
        "expected HH:MM:SS format, got: {}",
        payload.countdown_to_start.target_local
    );
}
```

Add `use chrono::Local;` to the imports at the top of the test file if not already present.

- [ ] **Step 2: Run tests to verify they fail or pass**

```bash
cargo test -p presenter-server -- timers_command_set_countdown_target_local timers_command_adjust_countdown_target timers_overview_includes_target_local --nocapture
```

These should pass because the core logic is already implemented — `apply_command` handles the new variants, and the router deserializes via serde. This verifies the integration works end-to-end.

- [ ] **Step 3: Run all existing timer tests**

```bash
cargo test -p presenter-server -- timer --nocapture
```

Expected: All pass. The `timers_overview_endpoint_returns_snapshot` test should still pass because `target_local` is included in the JSON response.

- [ ] **Step 4: Commit**

```bash
cargo fmt --all
git add crates/presenter-server/src/router/tests.rs
git commit -m "test(timer): add router integration tests for new timer commands (#212)

Tests SetCountdownTargetLocal, AdjustCountdownTarget, and
target_local field in overview response."
```

---

## Task 3: Update WASM Timer Panel for Local Time Input

**Files:**
- Modify: `crates/presenter-ui/src/components/timer_panel.rs`

- [ ] **Step 1: Rewrite `parse_time_input` to return `(u32, u32)` instead of `DateTime<Utc>`**

In `crates/presenter-ui/src/components/timer_panel.rs`, replace the `parse_time_input` function (lines 317-351):

```rust
fn parse_time_input(input: &str) -> Option<(u32, u32)> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    let parts: Vec<&str> = trimmed.split(':').collect();

    match parts.len() {
        1 => {
            // Single number: interpret as hours (e.g. "18" → 18:00)
            let hours: u32 = parts[0].parse().ok()?;
            if hours > 23 {
                return None;
            }
            Some((hours, 0))
        }
        2 => {
            let h: u32 = parts[0].trim().parse().ok()?;
            let m: u32 = parts[1].trim().parse().ok()?;
            if h > 23 || m > 59 {
                return None;
            }
            Some((h, m))
        }
        _ => None,
    }
}
```

- [ ] **Step 2: Update countdown input handlers to use `SetCountdownTargetLocal`**

In `crates/presenter-ui/src/components/timer_panel.rs`, replace the `on_countdown_blur` handler (lines 53-72):

```rust
    let on_countdown_blur = {
        let countdown_input_active = op.countdown_input_active;
        let countdown_input_dirty = op.countdown_input_dirty;
        move |ev: web_sys::FocusEvent| {
            countdown_input_active.set(false);
            if countdown_input_dirty.get_untracked() {
                if let Some(input) = ev.target().and_then(|t| {
                    use wasm_bindgen::JsCast;
                    t.dyn_into::<web_sys::HtmlInputElement>().ok()
                }) {
                    let val = input.value();
                    if let Some((hours, minutes)) = parse_time_input(&val) {
                        send_timer_cmd(TimerCommand::SetCountdownTargetLocal { hours, minutes });
                    }
                }
                countdown_input_dirty.set(false);
            }
        }
    };
```

Replace the `on_countdown_target` handler (lines 81-96):

```rust
    let on_countdown_target = move |ev: web_sys::KeyboardEvent| {
        if ev.key() != "Enter" {
            return;
        }
        let input = ev.target().and_then(|t| {
            use wasm_bindgen::JsCast;
            t.dyn_into::<web_sys::HtmlInputElement>().ok()
        });
        if let Some(el) = input {
            let val = el.value();
            if let Some((hours, minutes)) = parse_time_input(&val) {
                send_timer_cmd(TimerCommand::SetCountdownTargetLocal { hours, minutes });
                op.countdown_input_dirty.set(false);
            }
        }
    };
```

- [ ] **Step 3: Update +5/-5 offset buttons to use `AdjustCountdownTarget`**

In `crates/presenter-ui/src/components/timer_panel.rs`, replace `on_offset_minus` and `on_offset_plus` (lines 98-114):

```rust
    let on_offset_minus = move |_| {
        send_timer_cmd(TimerCommand::AdjustCountdownTarget { offset_minutes: -5 });
    };

    let on_offset_plus = move |_| {
        send_timer_cmd(TimerCommand::AdjustCountdownTarget { offset_minutes: 5 });
    };
```

- [ ] **Step 4: Update target display to use `target_local`**

In `crates/presenter-ui/src/components/timer_panel.rs`, replace the `<small id="countdown-target">` block (lines 216-222):

```rust
                <small id="countdown-target">
                    {move || {
                        ctx.timers.get()
                            .map(|t| format!("Target {}", t.countdown_to_start.target_local))
                            .unwrap_or_default()
                    }}
                </small>
```

- [ ] **Step 5: Run clippy and fix any issues**

```bash
cargo clippy -p presenter-ui --all-targets -- -D warnings -W clippy::all
```

The `chrono` import for `DateTime`, `Duration`, `Utc` is no longer needed in this file. Remove unused imports.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/presenter-ui/src/components/timer_panel.rs
git commit -m "fix(timer): use local time input and server-side resolution (#212)

Typing '18' now means 18:00 local time, not 18 minutes from now.
+5/-5 buttons use AdjustCountdownTarget instead of client-side
target reading. Target display shows server-formatted local time."
```

---

## Task 4: Fix OBS Timer Overlay Flicker

**Files:**
- Modify: `crates/presenter-server/src/ui/timer_overlay.rs`

- [ ] **Step 1: Replace the overlay JavaScript**

In `crates/presenter-server/src/ui/timer_overlay.rs`, replace the entire `script` format string (lines 14-131) with:

```rust
    let script = format!(
        r"(function() {{
  const initial = {timers_json};
  let overview = initial || {{}};
  let countdown = overview.countdown_to_start || overview.countdownToStart || {{}};
  let remaining = Number(countdown.seconds_remaining ?? countdown.secondsRemaining ?? 0);
  let state = String(countdown.state ?? 'idle').toLowerCase();
  const valueEl = document.getElementById('timer-value');

  const coerceTargetEpoch = (value) => {{
    if (typeof value !== 'string') return null;
    const parsed = Date.parse(value);
    return Number.isNaN(parsed) ? null : parsed;
  }};

  let targetEpochMs = coerceTargetEpoch(
    countdown.target ?? countdown.targetUtc ?? countdown.targetUTC ?? null
  );

  const clampNumber = (value) => (Number.isFinite(value) ? value : 0);
  const format = (seconds) => {{
    const total = Math.max(0, Math.floor(clampNumber(seconds)));
    if (total < 60) {{
      return String(total);
    }}
    const minutes = Math.floor(total / 60);
    const secs = total % 60;
    return `${{String(minutes).padStart(2, '0')}}:${{String(secs).padStart(2, '0')}}`;
  }};

  const remainingFromTarget = () => {{
    if (!Number.isFinite(targetEpochMs)) return null;
    return Math.max(0, Math.round((targetEpochMs - Date.now()) / 1000));
  }};

  const publishState = () => {{
    window.__presenterTimerOverlayState = {{ remaining, state }};
  }};

  const render = () => {{
    if (valueEl) {{
      valueEl.textContent = format(remaining);
    }}
    publishState();
  }};

  const applyOverview = (nextOverview) => {{
    if (!nextOverview) return;
    const nextCountdown =
      nextOverview.countdown_to_start ||
      nextOverview.countdownToStart ||
      {{}};
    if (typeof nextCountdown.state === 'string') {{
      state = nextCountdown.state.toLowerCase();
    }}
    const candidateTarget =
      nextCountdown.target ??
      nextCountdown.targetUtc ??
      nextCountdown.targetUTC ??
      null;
    const parsedTarget = coerceTargetEpoch(candidateTarget);
    if (parsedTarget !== null) {{
      targetEpochMs = parsedTarget;
    }}
    // Derive remaining from target, not from seconds_remaining
    const derived = remainingFromTarget();
    if (derived !== null) {{
      remaining = derived;
    }}
    render();
  }};

  // Initial render
  applyOverview(overview);

  // Use requestAnimationFrame for smooth, drift-free updates
  let lastRenderedSecond = -1;
  const tick = () => {{
    if (state === 'running') {{
      const derived = remainingFromTarget();
      if (derived !== null) {{
        remaining = derived;
      }}
      const currentSecond = Math.floor(remaining);
      if (currentSecond !== lastRenderedSecond) {{
        lastRenderedSecond = currentSecond;
        render();
      }}
    }}
    requestAnimationFrame(tick);
  }};
  requestAnimationFrame(tick);

  const connect = () => {{
    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const socket = new WebSocket(`${{protocol}}//${{window.location.host}}/live/ws`);
    window.__presenterTimerOverlaySocket = socket;

    socket.addEventListener('message', (event) => {{
      try {{
        const data = JSON.parse(event.data);
        if (data.type === 'timers') {{
          applyOverview(data.overview);
        }}
      }} catch (error) {{
        console.warn('timer overlay parse error', error);
      }}
    }});

    const scheduleReconnect = () => {{
      window.setTimeout(connect, 1500);
    }};

    socket.addEventListener('close', scheduleReconnect);
    socket.addEventListener('error', () => {{
      try {{ socket.close(); }} catch (_) {{}}
    }});
  }};

  connect();
}})();",
        timers_json = timers_json
    );
```

Key changes from the original:
1. `applyOverview` no longer assigns `remaining` from `seconds_remaining` — it derives from `targetEpochMs` only
2. Replaced `setInterval(1000)` with `requestAnimationFrame` loop that only re-renders when the displayed second changes
3. `lastRenderedSecond` prevents redundant DOM writes

- [ ] **Step 2: Run existing overlay test**

```bash
cargo test -p presenter-server -- timer_overlay --nocapture
```

Expected: Pass (the existing router test for `/overlays/timer` verifies it renders).

- [ ] **Step 3: Commit**

```bash
cargo fmt --all
git add crates/presenter-server/src/ui/timer_overlay.rs
git commit -m "fix(timer): eliminate OBS overlay flicker with rAF and target-only derivation (#212)

Replace setInterval(1s) with requestAnimationFrame for drift-free
updates. Derive remaining exclusively from target epoch timestamp
instead of overwriting from WebSocket seconds_remaining. This
eliminates the ±1 second jump between local and server ticks."
```

---

## Task 5: E2E Tests for Timer Bug Fixes

**Files:**
- Modify: `tests/e2e/wasm-timers.spec.ts`

- [ ] **Step 1: Add E2E test for local time input**

In `tests/e2e/wasm-timers.spec.ts`, add inside the `test.describe("WASM Operator Timer Tests", () => {` block, after the existing tests:

```typescript
test("typing hour number sets local time target (#212 bug 1+3)", async ({
  page,
  request,
}) => {
  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  await navigateToTimers(page);

  // Use SetCountdownTargetLocal via API to set a known future time
  const now = new Date();
  const futureHour = (now.getHours() + 2) % 24;

  const response = await request.post(
    new URL("/timers/command", baseURL).toString(),
    {
      data: {
        command: "set_countdown_target_local",
        hours: futureHour,
        minutes: 0,
      },
      headers: { "Content-Type": "application/json" },
      timeout: 10_000,
    },
  );
  expect(response.ok()).toBeTruthy();
  const data = await response.json();

  // target_local should show the local time we set
  const expectedPrefix = `${String(futureHour).padStart(2, "0")}:00`;
  expect(data.countdownToStart.targetLocal).toContain(expectedPrefix);

  // Remaining should be roughly 2 hours (within 10 min tolerance)
  expect(data.countdownToStart.secondsRemaining).toBeGreaterThan(6000);
  expect(data.countdownToStart.secondsRemaining).toBeLessThan(8000);

  // Verify the operator UI shows the local target
  const targetDisplay = page.locator("#countdown-target");
  await expect(targetDisplay).toContainText(expectedPrefix, {
    timeout: 10_000,
  });

  // Now test typing "18" in the input (use a future hour)
  const countdownInput = page.locator(
    '[data-role="countdown-target-input"]',
  );
  await countdownInput.fill(String(futureHour));
  await countdownInput.press("Enter");

  // Wait for API response
  await page
    .waitForResponse(
      (resp) => resp.url().includes("/timers/") && resp.status() === 200,
      { timeout: 5_000 },
    )
    .catch(() => {});

  // Target display should still show the same local time
  await expect(targetDisplay).toContainText(expectedPrefix, {
    timeout: 5_000,
  });

  expect(consoleMessages).toEqual([]);
});

test("adjust countdown target +5/-5 via API (#212 bug 3)", async ({
  request,
}) => {
  // Set initial target
  const now = new Date();
  const futureHour = (now.getHours() + 2) % 24;
  await request.post(new URL("/timers/command", baseURL).toString(), {
    data: {
      command: "set_countdown_target_local",
      hours: futureHour,
      minutes: 0,
    },
    headers: { "Content-Type": "application/json" },
  });

  // Get baseline
  const baselineResp = await request.get(
    new URL("/timers/overview", baseURL).toString(),
  );
  const baseline = await baselineResp.json();
  const baselineRemaining = baseline.countdownToStart.secondsRemaining;

  // Adjust +5
  const plusResp = await request.post(
    new URL("/timers/command", baseURL).toString(),
    {
      data: { command: "adjust_countdown_target", offset_minutes: 5 },
      headers: { "Content-Type": "application/json" },
    },
  );
  expect(plusResp.ok()).toBeTruthy();
  const plusData = await plusResp.json();
  const plusDiff =
    plusData.countdownToStart.secondsRemaining - baselineRemaining;
  expect(plusDiff).toBeGreaterThan(290);
  expect(plusDiff).toBeLessThan(310);

  // Adjust -5 (back to baseline)
  const minusResp = await request.post(
    new URL("/timers/command", baseURL).toString(),
    {
      data: { command: "adjust_countdown_target", offset_minutes: -5 },
      headers: { "Content-Type": "application/json" },
    },
  );
  expect(minusResp.ok()).toBeTruthy();
  const minusData = await minusResp.json();
  const totalDiff = Math.abs(
    minusData.countdownToStart.secondsRemaining - baselineRemaining,
  );
  expect(totalDiff).toBeLessThan(5); // Back to roughly original
});

test("timer overlay renders without flicker (#212 bug 4)", async ({
  page,
  request,
}) => {
  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  // Set a target and start the countdown
  const now = new Date();
  const futureHour = (now.getHours() + 1) % 24;
  await request.post(new URL("/timers/command", baseURL).toString(), {
    data: {
      command: "set_countdown_target_local",
      hours: futureHour,
      minutes: 0,
    },
    headers: { "Content-Type": "application/json" },
  });
  await request.post(new URL("/timers/command", baseURL).toString(), {
    data: { command: "start_countdown" },
    headers: { "Content-Type": "application/json" },
  });

  // Open overlay
  await page.goto(new URL("/overlays/timer", baseURL).toString());
  await page.waitForSelector("#timer-value", { timeout: 10_000 });

  // Collect displayed values over 5 seconds
  const values: string[] = [];
  for (let i = 0; i < 10; i++) {
    await page.waitForTimeout(500);
    const text = await page.locator("#timer-value").textContent();
    if (text) values.push(text);
  }

  // Parse values to seconds for monotonicity check
  const toSeconds = (v: string): number => {
    const parts = v.split(":").map(Number);
    if (parts.length === 1) return parts[0];
    return parts[0] * 60 + parts[1];
  };

  const seconds = values.map(toSeconds);

  // Check monotonically non-increasing (countdown should only go down)
  for (let i = 1; i < seconds.length; i++) {
    expect(seconds[i]).toBeLessThanOrEqual(seconds[i - 1]);
  }

  // No value should jump UP (flicker = value goes down then up)
  let flickerCount = 0;
  for (let i = 1; i < seconds.length; i++) {
    if (seconds[i] > seconds[i - 1]) flickerCount++;
  }
  expect(flickerCount).toBe(0);

  // Clean up
  await request.post(new URL("/timers/command", baseURL).toString(), {
    data: { command: "reset_countdown" },
    headers: { "Content-Type": "application/json" },
  });

  expect(consoleMessages).toEqual([]);
});
```

- [ ] **Step 2: Run the E2E tests locally**

```bash
npm run test:playwright -- wasm-timers
```

Expected: All existing and new tests pass.

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/wasm-timers.spec.ts
git commit -m "test(e2e): add timer local time, adjust, and overlay flicker tests (#212)

Verifies: typing hour sets local time target, +5/-5 adjusts
remaining by 300s, overlay countdown is monotonically decreasing
(no flicker). All with clean console assertion."
```

---

## Task 6: Version Bump, Format Check, Push, Monitor CI

- [ ] **Step 1: Bump version**

```bash
git fetch origin
# Check current version
grep '^version' Cargo.toml | head -1
```

Bump the patch version in `Cargo.toml` workspace `[workspace.package].version` (e.g. 0.4.10 → 0.4.11).

- [ ] **Step 2: Commit version bump**

```bash
cargo fmt --all --check
git add Cargo.toml Cargo.lock
git commit -m "chore: bump version to 0.4.11"
```

- [ ] **Step 3: Run local checks**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
cargo test -p presenter-core -- timer --nocapture
cargo test -p presenter-server -- timer --nocapture
```

Fix any issues in ONE commit if needed.

- [ ] **Step 4: Push and monitor CI**

```bash
git push origin dev
gh run list --branch dev --limit 3
```

Monitor until ALL jobs complete. If any fail, `gh run view <run-id> --log-failed`, fix ALL issues in ONE commit, push again.

---

## Verification Summary

| Check | How to verify |
|-------|---------------|
| Timezone fix | Type "18" → target shows "18:00:00" (local), remaining is correct for local 18:00 |
| Input simplification | "18" = 18:00, "18:30" = 18:30, "25" = rejected |
| +5/-5 buttons | Click +5 → remaining increases ~300s, no client-side target reading |
| OBS overlay stable | Open overlay, watch for 10s → countdown values monotonically decrease, no flicker |
| Start/stop reliable | Set target → Start → countdown runs → Pause → resumes → Reset → idle |
| `target_local` in API | `GET /timers/overview` returns `targetLocal: "18:00:00"` in server's local TZ |
| Existing tests pass | All wasm-timers, tablet-timer, and router timer tests green |
| Clean console | No browser console errors or warnings |
