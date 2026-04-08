# Tablet Preach Timer Bar Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a persistent timer bar to the tablet Bible view showing wall clock, preach elapsed time, and progressive color alerts when approaching/exceeding a configurable preach limit.

**Architecture:** Extend `PreachTimer` in presenter-core with an optional `limit` field. Add two new `TimerCommand` variants (`SetPreachLimit`, `ClearPreachLimit`). Propagate limit through `PreachTimerSnapshot` → WebSocket → tablet WASM UI. Add Companion commands to set/clear the limit. Tablet renders a fixed top bar with clock, elapsed, and color-coded background.

**Tech Stack:** Rust (presenter-core, presenter-server, presenter-persistence, presenter-migration), Leptos/WASM (presenter-ui), CSS, Playwright E2E

**Spec:** `docs/superpowers/specs/2026-04-07-tablet-preach-timer-bar-design.md`

---

## Context

Issue #171. The speaker/pastor uses the tablet during services but has no visibility into elapsed speaking time. The preach timer already exists (counts up) but has no limit concept and isn't shown on the tablet. The tablet currently only shows Bible slides.

**Key existing files:**
- `crates/presenter-core/src/timer.rs` — `PreachTimer`, `TimerCommand`, `PreachTimerSnapshot`, `TimersOverview`
- `crates/presenter-server/src/companion/protocol.rs` — `parse_command()`, `handle_command()`
- `crates/presenter-server/src/companion/variables.rs` — `write_timer_variables()`
- `crates/presenter-server/src/state/timers.rs` — `execute_timer_command()`, `tick_timers()`
- `crates/presenter-persistence/src/repository/mod.rs:616-656` — `get_timers_state()`, `upsert_timers_state()`
- `crates/presenter-persistence/src/repository/util.rs:333-349` — `timers_model_to_state()`
- `crates/presenter-persistence/src/entities.rs:439-456` — timers entity model
- `crates/presenter-migration/src/m20250927_000001_create_core_tables.rs:619-665,1129-1139` — timers table DDL + enum
- `crates/presenter-ui/src/pages/tablet.rs` — `TabletPage`, `TabletHeader`, WebSocket event handler
- `crates/presenter-ui/src/state/tablet.rs` — `TabletContext`
- `crates/presenter-ui/styles/tablet.css` — tablet styles
- `tests/e2e/tablet.spec.ts` — existing tablet E2E tests

---

## File Structure

### Modified Files
| File | Change |
|------|--------|
| `crates/presenter-core/src/timer.rs` | Add `limit` to `PreachTimer`, new commands, snapshot field |
| `crates/presenter-server/src/companion/protocol.rs` | Parse `timer.set_preach_limit` and `timer.clear_preach_limit` |
| `crates/presenter-server/src/companion/variables.rs` | Add `timer_preach_limit_seconds` variable |
| `crates/presenter-persistence/src/entities.rs` | Add `preach_limit_seconds` column to timers model |
| `crates/presenter-persistence/src/repository/mod.rs` | Persist/load preach limit |
| `crates/presenter-persistence/src/repository/util.rs` | Map limit in `timers_model_to_state` |
| `crates/presenter-migration/src/m20250927_000001_create_core_tables.rs` | Add `PreachLimitSeconds` column |
| `crates/presenter-ui/src/pages/tablet.rs` | Add timer bar component and WebSocket timer handling |
| `crates/presenter-ui/src/state/tablet.rs` | Add timer signals to `TabletContext` |
| `crates/presenter-ui/styles/tablet.css` | Timer bar styles with color zones |
| `Cargo.toml` | Bump version to 0.4.7 |

### New Files
| File | Purpose |
|------|---------|
| `tests/e2e/tablet-timer.spec.ts` | E2E test for tablet timer bar |

---

## Task 1: Version Bump

**Files:**
- Modify: `Cargo.toml` (line 15)

- [ ] **Step 1: Fetch and check version**

Run:
```bash
git fetch origin
DEV_VER=$(grep -m1 '^version = ' Cargo.toml | sed 's/version = "\(.*\)"/\1/')
MAIN_VER=$(git show origin/main:Cargo.toml | grep -m1 '^version = ' | sed 's/version = "\(.*\)"/\1/')
echo "dev=$DEV_VER main=$MAIN_VER"
```

Expected: Both are `0.4.6`. Bump needed.

- [ ] **Step 2: Bump version**

In `Cargo.toml` line 15, change:
```
version = "0.4.6"
```
to:
```
version = "0.4.7"
```

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml
git commit -m "chore: bump version to 0.4.7"
```

---

## Task 2: Extend PreachTimer with Limit (Core)

**Files:**
- Modify: `crates/presenter-core/src/timer.rs`

- [ ] **Step 1: Write failing tests for preach limit**

Add these tests at the end of the `mod tests` block in `crates/presenter-core/src/timer.rs` (before the closing `}`):

```rust
    #[test]
    fn preach_timer_set_limit() {
        let mut timer = PreachTimer::new();
        assert_eq!(timer.limit_seconds(), None);
        timer.set_limit(300);
        assert_eq!(timer.limit_seconds(), Some(300));
    }

    #[test]
    fn preach_timer_clear_limit() {
        let mut timer = PreachTimer::new();
        timer.set_limit(300);
        timer.clear_limit();
        assert_eq!(timer.limit_seconds(), None);
    }

    #[test]
    fn preach_timer_limit_in_snapshot() {
        let now = Utc::now();
        let mut preach = PreachTimer::new();
        preach.set_limit(600);
        let countdown_target = now + TimeDelta::try_minutes(15).unwrap();
        let state = TimersState::new(
            CountdownTimer { target: countdown_target, state: TimerState::Idle },
            preach,
        );
        let overview = state.overview(now);
        assert_eq!(overview.preach_timer.limit_seconds, Some(600));
    }

    #[test]
    fn preach_timer_snapshot_no_limit() {
        let now = Utc::now();
        let preach = PreachTimer::new();
        let countdown_target = now + TimeDelta::try_minutes(15).unwrap();
        let state = TimersState::new(
            CountdownTimer { target: countdown_target, state: TimerState::Idle },
            preach,
        );
        let overview = state.overview(now);
        assert_eq!(overview.preach_timer.limit_seconds, None);
    }

    #[test]
    fn set_preach_limit_command() {
        let now = Utc::now();
        let countdown_target = now + TimeDelta::try_minutes(15).unwrap();
        let mut state = TimersState::new(
            CountdownTimer { target: countdown_target, state: TimerState::Idle },
            PreachTimer::new(),
        );
        state.apply_command(&TimerCommand::SetPreachLimit { seconds: 300 }, now).unwrap();
        assert_eq!(state.preach.limit_seconds(), Some(300));
    }

    #[test]
    fn clear_preach_limit_command() {
        let now = Utc::now();
        let countdown_target = now + TimeDelta::try_minutes(15).unwrap();
        let mut state = TimersState::new(
            CountdownTimer { target: countdown_target, state: TimerState::Idle },
            PreachTimer::new(),
        );
        state.apply_command(&TimerCommand::SetPreachLimit { seconds: 300 }, now).unwrap();
        state.apply_command(&TimerCommand::ClearPreachLimit, now).unwrap();
        assert_eq!(state.preach.limit_seconds(), None);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p presenter-core --lib -- preach_timer_set_limit preach_timer_clear_limit preach_timer_limit_in_snapshot preach_timer_snapshot_no_limit set_preach_limit_command clear_preach_limit_command 2>&1 | tail -20`

Expected: Compilation errors — `limit_seconds`, `set_limit`, `clear_limit`, `SetPreachLimit`, `ClearPreachLimit` don't exist yet.

- [ ] **Step 3: Add limit field and methods to PreachTimer**

In `crates/presenter-core/src/timer.rs`, modify the `PreachTimer` struct (line 116) to add the `limit` field:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PreachTimer {
    pub state: TimerState,
    started_at: Option<DateTime<Utc>>,
    accumulated: Duration,
    limit: Option<Duration>,
}
```

Update `PreachTimer::new()` (line 129) to initialize limit:

```rust
    pub fn new() -> Self {
        Self {
            state: TimerState::Idle,
            started_at: None,
            accumulated: Duration::zero(),
            limit: None,
        }
    }
```

Add these methods to the `impl PreachTimer` block (after the `accumulated_duration` method, before `from_parts`):

```rust
    pub fn set_limit(&mut self, seconds: u64) {
        self.limit = Some(Duration::seconds(seconds as i64));
    }

    pub fn clear_limit(&mut self) {
        self.limit = None;
    }

    pub fn limit_seconds(&self) -> Option<u64> {
        self.limit.map(|d| d.num_seconds().max(0) as u64)
    }
```

Update `from_parts` to accept the limit parameter:

```rust
    pub fn from_parts(
        state: TimerState,
        started_at: Option<DateTime<Utc>>,
        accumulated: Duration,
        limit: Option<Duration>,
    ) -> Self {
        Self {
            state,
            started_at,
            accumulated,
            limit,
        }
    }
```

- [ ] **Step 4: Add limit_seconds to PreachTimerSnapshot**

In `crates/presenter-core/src/timer.rs`, modify `PreachTimerSnapshot` (line 287):

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PreachTimerSnapshot {
    pub state: TimerState,
    pub seconds_elapsed: i64,
    pub limit_seconds: Option<u64>,
}
```

Update `TimersState::overview()` (line 261) to include limit:

```rust
    pub fn overview(&self, now: DateTime<Utc>) -> TimersOverview {
        let countdown_remaining = self.countdown.remaining(now).num_seconds();
        let remaining_seconds = max(countdown_remaining, 0);
        let elapsed_seconds = self.preach.elapsed(now).num_seconds();
        TimersOverview {
            countdown_to_start: CountdownTimerSnapshot {
                state: self.countdown.state,
                target: self.countdown.target,
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

Update `TimersOverview::demo()` (line 303) to include limit:

```rust
    pub fn demo(now: DateTime<Utc>) -> Self {
        let countdown_target = now + Duration::minutes(15);
        Self {
            countdown_to_start: CountdownTimerSnapshot {
                state: TimerState::Running,
                target: countdown_target,
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

- [ ] **Step 5: Add new TimerCommand variants**

In `crates/presenter-core/src/timer.rs`, modify the `TimerCommand` enum (line 189):

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "command")]
pub enum TimerCommand {
    SetCountdownTarget { target: DateTime<Utc> },
    StartCountdown,
    PauseCountdown,
    ResetCountdown,
    StartPreach,
    PausePreach,
    ResetPreach,
    SetPreachLimit { seconds: u64 },
    ClearPreachLimit,
}
```

Add command handling in `TimersState::apply_command()` — add these arms before the closing `}` of the match block (after `ResetPreach`):

```rust
            TimerCommand::SetPreachLimit { seconds } => {
                self.preach.set_limit(*seconds);
            }
            TimerCommand::ClearPreachLimit => {
                self.preach.clear_limit();
            }
```

- [ ] **Step 6: Fix from_parts call sites in persistence**

In `crates/presenter-persistence/src/repository/util.rs`, update `timers_model_to_state` (line 342) to pass the limit:

```rust
    let preach = PreachTimer::from_parts(
        preach_state,
        model.preach_started_at.map(Into::into),
        chrono::Duration::seconds(model.preach_accumulated_seconds),
        model.preach_limit_seconds.map(|s| chrono::Duration::seconds(s)),
    );
```

- [ ] **Step 7: Run tests to verify they pass**

Run: `cargo test -p presenter-core --lib 2>&1 | tail -10`

Expected: All tests pass, including the 6 new ones.

- [ ] **Step 8: Commit**

```bash
git add crates/presenter-core/src/timer.rs
git commit -m "feat(core): add preach timer limit field and commands (#171)

Add optional limit to PreachTimer with SetPreachLimit/ClearPreachLimit
commands. Include limit_seconds in PreachTimerSnapshot for clients."
```

---

## Task 3: Database Schema and Persistence

**Files:**
- Modify: `crates/presenter-migration/src/m20250927_000001_create_core_tables.rs`
- Modify: `crates/presenter-persistence/src/entities.rs`
- Modify: `crates/presenter-persistence/src/repository/mod.rs`
- Modify: `crates/presenter-persistence/src/repository/util.rs`

- [ ] **Step 1: Add column to migration**

In `crates/presenter-migration/src/m20250927_000001_create_core_tables.rs`, add `PreachLimitSeconds` to the `Timers` enum (line 1129):

```rust
#[derive(DeriveIden)]
enum Timers {
    Table,
    Id,
    CountdownTarget,
    CountdownState,
    PreachState,
    PreachStartedAt,
    PreachAccumulatedSeconds,
    PreachLimitSeconds,
    CreatedAt,
    UpdatedAt,
}
```

Add the column definition to the `create_table` call. Insert this `.col(...)` block after the `PreachAccumulatedSeconds` column definition (after line 651, before the `CreatedAt` column):

```rust
                    .col(
                        ColumnDef::new(Timers::PreachLimitSeconds)
                            .big_integer()
                            .null(),
                    )
```

- [ ] **Step 2: Add field to entity model**

In `crates/presenter-persistence/src/entities.rs`, modify the timers `Model` struct (line 440). Add after `preach_accumulated_seconds`:

```rust
        pub preach_limit_seconds: Option<i64>,
```

The full struct becomes:

```rust
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: String,
        pub countdown_target: DateTimeWithTimeZone,
        pub countdown_state: String,
        pub preach_state: String,
        pub preach_started_at: Option<DateTimeWithTimeZone>,
        pub preach_accumulated_seconds: i64,
        pub preach_limit_seconds: Option<i64>,
        pub created_at: DateTimeWithTimeZone,
        pub updated_at: DateTimeWithTimeZone,
    }
```

- [ ] **Step 3: Update timers_model_to_state**

In `crates/presenter-persistence/src/repository/util.rs`, update `timers_model_to_state` (line 342):

```rust
    let preach = PreachTimer::from_parts(
        preach_state,
        model.preach_started_at.map(Into::into),
        chrono::Duration::seconds(model.preach_accumulated_seconds),
        model.preach_limit_seconds.map(chrono::Duration::seconds),
    );
```

- [ ] **Step 4: Update upsert_timers_state**

In `crates/presenter-persistence/src/repository/mod.rs`, update `upsert_timers_state` (line 626). Add to the `ActiveModel` construction, after `preach_accumulated_seconds`:

```rust
            preach_limit_seconds: Set(state.preach.limit_seconds().map(|s| s as i64)),
```

And add `timers::Column::PreachLimitSeconds` to the `update_columns` list in the `on_conflict` block:

```rust
                    .update_columns([
                        timers::Column::CountdownTarget,
                        timers::Column::CountdownState,
                        timers::Column::PreachState,
                        timers::Column::PreachStartedAt,
                        timers::Column::PreachAccumulatedSeconds,
                        timers::Column::PreachLimitSeconds,
                        timers::Column::UpdatedAt,
                    ])
```

- [ ] **Step 5: Commit**

```bash
git add crates/presenter-migration/ crates/presenter-persistence/
git commit -m "feat(persistence): add preach_limit_seconds column (#171)

Add nullable preach_limit_seconds to timers table schema, entity
model, and repository upsert/load logic."
```

---

## Task 4: Companion Protocol Integration

**Files:**
- Modify: `crates/presenter-server/src/companion/protocol.rs`
- Modify: `crates/presenter-server/src/companion/variables.rs`

- [ ] **Step 1: Add command parsing**

In `crates/presenter-server/src/companion/protocol.rs`, add a new payload struct after `BroadcastSetLivePayload` (line 346):

```rust
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PreachLimitPayload {
    seconds: u64,
}
```

Add these two arms to `parse_command()` in the match block, before the `"bible.trigger"` arm (after line 137):

```rust
        "timer.set_preach_limit" => {
            let data: PreachLimitPayload = serde_json::from_value(payload)
                .map_err(|err| format!("invalid timer.set_preach_limit payload: {err}"))?;
            Ok(CompanionCommand::Timer(TimerCommand::SetPreachLimit {
                seconds: data.seconds,
            }))
        }
        "timer.clear_preach_limit" => Ok(CompanionCommand::Timer(TimerCommand::ClearPreachLimit)),
```

- [ ] **Step 2: Add preach limit variable**

In `crates/presenter-server/src/companion/variables.rs`, update `write_timer_variables()` (line 292). Add after the `timer_preach_elapsed_readable` line (line 349), before the closing `}` of the `Some(timers)` branch:

```rust
            builder.set(
                "timer_preach_limit_seconds",
                match timers.preach_timer.limit_seconds {
                    Some(s) => s.to_string(),
                    None => "".into(),
                },
            );
```

Add the same variable to the `None` branch (after line 364, before the closing `}`):

```rust
            builder.set("timer_preach_limit_seconds", "".into());
```

- [ ] **Step 3: Commit**

```bash
git add crates/presenter-server/src/companion/
git commit -m "feat(companion): add set/clear preach limit commands (#171)

New commands timer.set_preach_limit and timer.clear_preach_limit.
New variable timer_preach_limit_seconds exposed to Companion."
```

---

## Task 5: Tablet Timer Bar UI

**Files:**
- Modify: `crates/presenter-ui/src/state/tablet.rs`
- Modify: `crates/presenter-ui/src/pages/tablet.rs`
- Modify: `crates/presenter-ui/styles/tablet.css`

- [ ] **Step 1: Add timer signals to TabletContext**

In `crates/presenter-ui/src/state/tablet.rs`, add to the imports (line 2):

```rust
use presenter_core::TimersOverview;
```

Add these fields to the `TabletContext` struct (after `ws_connected`):

```rust
    pub timers: RwSignal<Option<TimersOverview>>,
```

Add initialization in `TabletContext::new()` (after `ws_connected: RwSignal::new(false),`):

```rust
            timers: RwSignal::new(None),
```

- [ ] **Step 2: Handle timer WebSocket events in tablet page**

In `crates/presenter-ui/src/pages/tablet.rs`, add timer event handling to the WebSocket event match block (line 47). Add a new arm before the `_ => {}` catch-all:

```rust
                LiveEvent::Timers { overview } => {
                    ctx.timers.set(Some(overview));
                }
```

- [ ] **Step 3: Add TabletTimerBar component**

In `crates/presenter-ui/src/pages/tablet.rs`, add these imports at the top (after line 3):

```rust
use presenter_core::{TimerState, TimersOverview};
```

Add the `TabletTimerBar` component. Place it before the `TabletHeader` component definition (before line 151):

```rust
#[component]
fn TabletTimerBar() -> impl IntoView {
    let ctx = use_ctx!(TabletContext);

    // Wall clock — update every second
    let clock = RwSignal::new(current_hhmm());
    let _clock_interval = gloo_timers::callback::Interval::new(1_000, move || {
        clock.set(current_hhmm());
    });
    _clock_interval.forget();

    let elapsed_text = move || {
        let timers = ctx.timers.get();
        match timers {
            Some(t) if t.preach_timer.state != TimerState::Idle => {
                format_mmss(t.preach_timer.seconds_elapsed)
            }
            _ => "\u{2014}".to_string(), // em-dash
        }
    };

    let state_text = move || {
        let timers = ctx.timers.get();
        match timers {
            Some(t) => match t.preach_timer.state {
                TimerState::Running => "RUNNING",
                TimerState::Paused => "PAUSED",
                TimerState::Completed => "DONE",
                TimerState::Idle => "IDLE",
            },
            None => "IDLE",
        }
    };

    let color_zone = move || {
        let timers = ctx.timers.get();
        match timers {
            Some(ref t) if t.preach_timer.state == TimerState::Running => {
                match t.preach_timer.limit_seconds {
                    Some(limit) if limit > 0 => {
                        let elapsed = t.preach_timer.seconds_elapsed as f64;
                        let limit_f = limit as f64;
                        let ratio = elapsed / limit_f;
                        if ratio >= 1.0 {
                            "red"
                        } else if ratio >= 0.9 {
                            "orange"
                        } else {
                            "green"
                        }
                    }
                    _ => "neutral",
                }
            }
            _ => "neutral",
        }
    };

    view! {
        <div
            class="tablet-timer-bar"
            data-role="timer-bar"
            data-zone=color_zone
        >
            <span class="tablet-timer-bar__clock" data-role="timer-clock">{clock}</span>
            <span class="tablet-timer-bar__elapsed" data-role="timer-elapsed">{elapsed_text}</span>
            <span class="tablet-timer-bar__state" data-role="timer-state">{state_text}</span>
        </div>
    }
}

fn current_hhmm() -> String {
    let now = js_sys::Date::new_0();
    let h = now.get_hours();
    let m = now.get_minutes();
    format!("{h:02}:{m:02}")
}

fn format_mmss(seconds: i64) -> String {
    let total = seconds.abs();
    let m = total / 60;
    let s = total % 60;
    if m >= 60 {
        let h = m / 60;
        let rm = m % 60;
        format!("{h}:{rm:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}
```

- [ ] **Step 4: Render the timer bar in the tablet view**

In `crates/presenter-ui/src/pages/tablet.rs`, update the `TabletPage` view (line 137). Add `TabletTimerBar` above `TabletHeader`:

```rust
    view! {
        <TabletTimerBar />
        <TabletHeader />
        <main class="tablet-layout">
            <TabletSidebar />
            <TabletMain />
        </main>
        <TabletToast />
    }
```

- [ ] **Step 5: Add CSS for timer bar**

In `crates/presenter-ui/styles/tablet.css`, add the timer bar styles at the end of the file:

```css
/* --- Timer bar --- */
.tablet-timer-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0.5rem 1.25rem;
  font-family: "Inter", "Segoe UI", system-ui, sans-serif;
  font-variant-numeric: tabular-nums;
  color: #f8fafc;
  background: #1e293b;
  border-bottom: 2px solid #334155;
  transition: background-color 0.6s ease, border-color 0.6s ease;
  flex-shrink: 0;
}

.tablet-timer-bar[data-zone="green"] {
  background: #166534;
  border-bottom-color: #22c55e;
}

.tablet-timer-bar[data-zone="orange"] {
  background: #92400e;
  border-bottom-color: #f59e0b;
}

.tablet-timer-bar[data-zone="red"] {
  background: #991b1b;
  border-bottom-color: #ef4444;
}

.tablet-timer-bar__clock {
  font-size: 1.1rem;
  font-weight: 500;
  opacity: 0.8;
  min-width: 4rem;
}

.tablet-timer-bar__elapsed {
  font-size: 1.4rem;
  font-weight: 700;
  letter-spacing: 0.02em;
}

.tablet-timer-bar__state {
  font-size: 0.75rem;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.08em;
  opacity: 0.7;
  min-width: 5rem;
  text-align: right;
}
```

- [ ] **Step 6: Add js-sys to presenter-ui dependencies (if not already present)**

Check if `js-sys` is already in `crates/presenter-ui/Cargo.toml`:

```bash
grep 'js-sys' crates/presenter-ui/Cargo.toml
```

If not present, add to `[dependencies]`:
```toml
js-sys = "0.3"
```

- [ ] **Step 7: Commit**

```bash
git add crates/presenter-ui/ 
git commit -m "feat(tablet): add persistent preach timer bar (#171)

Render a fixed top bar on the tablet Bible view showing wall clock,
preach elapsed time, and progressive color zones (green/orange/red)
based on configurable preach limit. Subscribes to LiveEvent::Timers
via existing WebSocket connection."
```

---

## Task 6: E2E Playwright Test

**Files:**
- Create: `tests/e2e/tablet-timer.spec.ts`

- [ ] **Step 1: Write E2E test**

Create `tests/e2e/tablet-timer.spec.ts`:

```typescript
import { test, expect } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

test.describe.configure({ timeout: 180_000 });

let serverHandle: ServerHandle | undefined;
let baseURL: string;
test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  await refreshDevData(config.dbUrl);
  serverHandle = await startTestServer(
    config.port,
    config.dbUrl,
    config.oscPort,
  );
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

test("tablet timer bar shows clock and responds to preach timer", async ({
  page,
  request,
}) => {
  // Wait for server readiness
  await expect(async () => {
    const response = await request.get(
      new URL("/healthz", baseURL).toString(),
      { timeout: 120_000 },
    );
    expect(response.ok()).toBeTruthy();
  }).toPass({ timeout: 180_000 });

  // Collect console errors
  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  // Navigate to tablet
  await page.goto(new URL("/ui/tablet", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', {
    timeout: 30_000,
  });

  // --- Timer bar should be visible with clock ---
  const timerBar = page.locator('[data-role="timer-bar"]');
  await expect(timerBar).toBeVisible({ timeout: 5_000 });

  // Clock should show HH:MM format
  const clock = page.locator('[data-role="timer-clock"]');
  await expect(clock).toHaveText(/^\d{2}:\d{2}$/, { timeout: 5_000 });

  // Elapsed should show em-dash when idle
  const elapsed = page.locator('[data-role="timer-elapsed"]');
  await expect(elapsed).toHaveText("—", { timeout: 5_000 });

  // State should show IDLE
  const state = page.locator('[data-role="timer-state"]');
  await expect(state).toHaveText("IDLE", { timeout: 5_000 });

  // Zone should be neutral
  await expect(timerBar).toHaveAttribute("data-zone", "neutral");

  // --- Start preach timer via API ---
  const startResponse = await request.post(
    new URL("/timers/command", baseURL).toString(),
    {
      data: { command: "start_preach" },
      headers: { "Content-Type": "application/json" },
      timeout: 10_000,
    },
  );
  expect(startResponse.ok()).toBeTruthy();

  // Elapsed should update to show a time value (not em-dash)
  await expect(async () => {
    const text = await elapsed.textContent();
    expect(text).toMatch(/^\d+:\d{2}$/);
  }).toPass({ timeout: 10_000, intervals: [500] });

  // State should show RUNNING
  await expect(state).toHaveText("RUNNING", { timeout: 5_000 });

  // --- Set preach limit and verify color zone ---
  const limitResponse = await request.post(
    new URL("/timers/command", baseURL).toString(),
    {
      data: { command: "set_preach_limit", seconds: 3 },
      headers: { "Content-Type": "application/json" },
      timeout: 10_000,
    },
  );
  expect(limitResponse.ok()).toBeTruthy();

  // With 3-second limit, initially green, then transitions
  // Wait for orange zone (at 90% = 2.7s)
  await expect(async () => {
    const zone = await timerBar.getAttribute("data-zone");
    expect(zone).toBe("orange");
  }).toPass({ timeout: 10_000, intervals: [300] });

  // Wait for red zone (at 100% = 3s)
  await expect(async () => {
    const zone = await timerBar.getAttribute("data-zone");
    expect(zone).toBe("red");
  }).toPass({ timeout: 10_000, intervals: [300] });

  // --- Pause preach timer ---
  const pauseResponse = await request.post(
    new URL("/timers/command", baseURL).toString(),
    {
      data: { command: "pause_preach" },
      headers: { "Content-Type": "application/json" },
      timeout: 10_000,
    },
  );
  expect(pauseResponse.ok()).toBeTruthy();

  // State should show PAUSED, zone back to neutral
  await expect(state).toHaveText("PAUSED", { timeout: 5_000 });
  await expect(timerBar).toHaveAttribute("data-zone", "neutral", {
    timeout: 5_000,
  });

  // --- Reset preach timer ---
  const resetResponse = await request.post(
    new URL("/timers/command", baseURL).toString(),
    {
      data: { command: "reset_preach" },
      headers: { "Content-Type": "application/json" },
      timeout: 10_000,
    },
  );
  expect(resetResponse.ok()).toBeTruthy();

  // Should show IDLE and em-dash again
  await expect(state).toHaveText("IDLE", { timeout: 5_000 });
  await expect(elapsed).toHaveText("—", { timeout: 5_000 });

  // Clean console check
  expect(consoleMessages).toEqual([]);
});
```

- [ ] **Step 2: Commit**

```bash
git add tests/e2e/tablet-timer.spec.ts
git commit -m "test(e2e): add tablet timer bar E2E test (#171)

Verifies timer bar visibility, clock format, preach start/pause/reset,
and progressive color zone transitions with a 3-second limit."
```

---

## Task 7: Format Check, Push, Monitor CI

- [ ] **Step 1: Run cargo fmt**

```bash
cargo fmt --all --check
```

If it fails:
```bash
cargo fmt --all
```

Then re-commit the formatted files.

- [ ] **Step 2: Push to dev**

```bash
git push origin dev
```

- [ ] **Step 3: Monitor CI**

```bash
gh run list --branch dev --limit 3
```

Wait for all jobs to reach terminal state. If any fail, investigate with `gh run view <run-id> --log-failed`, fix ALL issues in ONE commit, push again.

---

## Task 8: Create PR

- [ ] **Step 1: Create PR from dev to main**

```bash
gh pr create --title "feat: tablet preach timer bar (#171)" --body "$(cat <<'EOF'
## Summary
- Adds persistent timer bar to tablet Bible view showing wall clock, preach elapsed, and state
- Progressive color zones: green → orange (90% of limit) → red (at limit)
- Preach limit configurable from Companion via `timer.set_preach_limit` command
- New Companion variable `timer_preach_limit_seconds`
- Extends `PreachTimer` with optional `limit` field, persisted in DB
- Closes #171

## Test plan
- [x] Unit tests for set/clear limit, snapshot, commands
- [x] E2E: timer bar visible, clock format, preach start/pause/reset
- [x] E2E: color zone transitions (green → orange → red) with 3s limit
- [x] E2E: clean browser console

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 2: Verify CI green and PR mergeable**

```bash
gh pr checks <PR_NUMBER> --watch
gh api repos/zbynekdrlik/presenter/pulls/<PR_NUMBER> --jq '{mergeable: .mergeable, mergeable_state: .mergeable_state}'
```

---

## Verification Summary

| Check | How to verify |
|-------|---------------|
| Limit persists across restart | Set limit via API, restart server, GET /timers/overview shows limit |
| Color zones work | E2E test with 3s limit → green → orange → red |
| Clock updates | Timer bar shows HH:MM matching system time |
| Companion variable | Connect Companion, verify `timer_preach_limit_seconds` appears |
| No regressions | All existing tablet E2E tests still pass |
| Clean console | E2E test asserts zero console errors/warnings |
