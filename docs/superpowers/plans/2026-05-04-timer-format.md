# Timer Countdown Format and Post-Zero Clearing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the countdown timer (a) auto-clear ~10 seconds after hitting zero on every surface (Resolume, stage display, operator panel) and (b) render large durations as `"1h 31m"` with hour/minute units instead of `"91:30"`.

**Architecture:** Consolidate the countdown format into a single `presenter_core::format_countdown` function. Server `format_countdown_text` becomes a thin wrapper. Two WASM call sites swap their local helper for the core function (countdown only — preach formatting stays unchanged). Remove the `max(0)` clamp on `TimersOverview.seconds_remaining` so the formatter can detect "10s past zero" via a negative value.

**Tech Stack:** Rust core lib (no new deps), Leptos WASM UI, Resolume HTTP client (already exists).

**Spec:** `docs/superpowers/specs/2026-05-04-timer-format-design.md` (commit 8728e1f).

---

## Context

Issue #280 (title-only): user complains that (a) the timer shows `0` indefinitely after the countdown hits zero, and (b) durations longer than 1 hour render as `"91:30"` instead of `"1h 31m"`. Current behavior: `format_countdown_text(seconds_remaining: i64)` in `crates/presenter-server/src/state/stage.rs:325` clamps to `max(0)` and uses `MM:SS` for everything ≥ 60 seconds. Three separate WASM `format_seconds` helpers exist that drift; the operator UI's helper is also used for the **preach timer** (different semantics — elapsed, never negative) and must NOT be changed for that.

**Key existing code:**

- `crates/presenter-core/src/timer.rs:316-343` — `overview` and `overview_with_local_format`. Line 328 has `let remaining_seconds = max(countdown_remaining, 0);` (the clamp to remove).
- `crates/presenter-core/src/timer.rs:346-353` — `CountdownTimerSnapshot { seconds_remaining: i64, ... }`. Type already supports negative.
- `crates/presenter-core/src/lib.rs:64-67` — `pub use timer::{ ... }` re-export list. New `format_countdown` must be added.
- `crates/presenter-server/src/state/stage.rs:325-334` — `format_countdown_text(seconds_remaining: i64) -> String`. Pre-clamps with `max(0)`; uses MM:SS for ≥60.
- `crates/presenter-server/src/state/tests.rs:263-267` — existing assertions to update.
- `crates/presenter-ui/src/components/stage/timer_layout.rs:23` — calls `format_seconds(...)` for the stage countdown. Helper at line 57.
- `crates/presenter-ui/src/components/timer_panel.rs:193` — calls `format_seconds(t.countdown_to_start.seconds_remaining)` for operator panel countdown. Helper at line 6. Same helper is used for preach at lines 220 and 228 — DON'T change those calls.
- `crates/presenter-ui/src/components/stage/preach_layout.rs:74` — separate `format_seconds` for preach. Don't touch.
- `crates/presenter-server/src/resolume/handlers.rs::handle_timer` — already handles empty-string PUTs via the existing dedup (`last_timer_payload`).

---

## File Structure

### Modified Files

| File | Change |
|------|--------|
| `Cargo.toml` | Workspace version 0.4.60 → 0.4.61 |
| `crates/presenter-ui/Cargo.toml` | Version 0.1.29 → 0.1.30 |
| `crates/presenter-core/src/timer.rs` | Add `pub fn format_countdown` near the bottom of the file. Add a `#[cfg(test)] mod` with 13 boundary-case unit tests. Remove `max(0)` clamp in `overview_with_local_format` at line 328. |
| `crates/presenter-core/src/lib.rs:64-67` | Add `format_countdown` to the `pub use timer::{ ... }` re-export list. |
| `crates/presenter-server/src/state/stage.rs:325-334` | Replace `format_countdown_text` body with delegation to `presenter_core::format_countdown`. |
| `crates/presenter-server/src/state/tests.rs:263-267` | Update 2 assertions for the new format (3605 and -12). |
| `crates/presenter-ui/src/components/stage/timer_layout.rs:23` | Swap `format_seconds(...)` → `presenter_core::format_countdown(...)` for the countdown call. Keep the local `format_seconds` for preach. |
| `crates/presenter-ui/src/components/timer_panel.rs:193` | Swap the countdown call. Keep `format_seconds` for preach lines 220, 228. |

---

## Task 1: Version Bump

**Files:**
- Modify: `Cargo.toml:15`
- Modify: `crates/presenter-ui/Cargo.toml:3`
- Modify: `Cargo.lock` (auto)
- Modify: `crates/presenter-ui/Cargo.lock` (auto)

- [ ] **Step 1: Bump workspace version**

In `Cargo.toml`, change line 15:

```toml
[workspace.package]
version = "0.4.61"
```

- [ ] **Step 2: Bump presenter-ui version**

In `crates/presenter-ui/Cargo.toml`, change line 3:

```toml
version = "0.1.30"
```

- [ ] **Step 3: Update lockfiles**

```bash
cargo update --workspace
cargo update --workspace --manifest-path crates/presenter-ui/Cargo.toml
```

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock crates/presenter-ui/Cargo.toml crates/presenter-ui/Cargo.lock
git commit -m "chore: bump version to 0.4.61"
```

---

## Task 2: Add `format_countdown` to `presenter-core`

**Files:**
- Modify: `crates/presenter-core/src/timer.rs` (add function + 13 tests + remove clamp)
- Modify: `crates/presenter-core/src/lib.rs:64-67` (re-export)

- [ ] **Step 1: Add `format_countdown` function**

Open `crates/presenter-core/src/timer.rs` and find the end of the public API section (just before the existing `#[cfg(test)] mod tests` block — search with `grep -n "#\[cfg(test)\]" crates/presenter-core/src/timer.rs | head -1`). Add this free function ABOVE the `#[cfg(test)]` line:

```rust
/// Format a countdown for display on Resolume, stage, and operator UI.
///
/// Spec: docs/superpowers/specs/2026-05-04-timer-format-design.md
///
/// - `< -10` → "" (cleared — 10 s past zero, send empty text)
/// - `-10..=0` → "0"
/// - `1..=59` → "1", "59"
/// - `60..=3599` → "MM:SS"
/// - `>= 3600` → "Xh Ym" (round down, drop seconds)
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

- [ ] **Step 2: Remove `max(0)` clamp in `overview_with_local_format`**

In `crates/presenter-core/src/timer.rs`, find `fn overview_with_local_format` (around line 326). The current body has:

```rust
        let countdown_remaining = self.countdown.remaining(now).num_seconds();
        let remaining_seconds = max(countdown_remaining, 0);
```

Replace those two lines with:

```rust
        let countdown_remaining = self.countdown.remaining(now).num_seconds();
```

Then update the `seconds_remaining: remaining_seconds,` field assignment in the same function — change it to:

```rust
                seconds_remaining: countdown_remaining,
```

The result: `TimersOverview.countdown_to_start.seconds_remaining` is now the raw signed value. Display layers either re-clamp (existing WASM `format_seconds` does `secs.max(0)`) or use the new `format_countdown` (which interprets negative as "cleared").

The `use std::cmp::max;` import at the top of the file may now be unused. If clippy flags it, remove the import line.

- [ ] **Step 3: Add unit tests for `format_countdown`**

In `crates/presenter-core/src/timer.rs`, find the existing `#[cfg(test)] mod tests` block and add this test inside it (near the other tests):

```rust
    #[test]
    fn format_countdown_covers_all_boundary_cases() {
        assert_eq!(format_countdown(3605), "1h 0m");
        assert_eq!(format_countdown(5430), "1h 30m");
        assert_eq!(format_countdown(7199), "1h 59m");
        assert_eq!(format_countdown(7200), "2h 0m");
        assert_eq!(format_countdown(125), "02:05");
        assert_eq!(format_countdown(60), "01:00");
        assert_eq!(format_countdown(59), "59");
        assert_eq!(format_countdown(1), "1");
        assert_eq!(format_countdown(0), "0");
        assert_eq!(format_countdown(-5), "0");
        assert_eq!(format_countdown(-10), "0");
        assert_eq!(format_countdown(-11), "");
        assert_eq!(format_countdown(-100), "");
    }
```

- [ ] **Step 4: Re-export `format_countdown` from `presenter-core`**

In `crates/presenter-core/src/lib.rs`, find the `pub use timer::{ ... }` block (lines 64-67). Add `format_countdown` to the list:

```rust
pub use timer::{
    format_countdown, CountdownTimer, CountdownTimerSnapshot, PreachTimer, PreachTimerSnapshot,
    TimerCommand, TimerState, TimersOverview, TimersState,
};
```

- [ ] **Step 5: Verify build + run tests**

```bash
cargo build -p presenter-core
cargo test -p presenter-core -- --nocapture
```

Expected: all 13 boundary assertions pass plus existing tests.

If any existing test in `presenter-core/src/timer.rs` asserts `seconds_remaining >= 0` on a snapshot AFTER `overview()` was called past target, it may fail because the clamp is gone. Locate via:

```bash
grep -n "seconds_remaining" crates/presenter-core/src/timer.rs
```

For each existing test that checks the snapshot's `seconds_remaining` past target, update the expectation to the new signed value. Common pattern:

```rust
// OLD: assert_eq!(overview.countdown_to_start.seconds_remaining, 0);
// NEW: assert!(overview.countdown_to_start.seconds_remaining < 0);
//      OR exact-value if test uses fixed timestamps
```

If you find such a test, fix it inline in this task. Don't fight it; the new behavior is correct.

- [ ] **Step 6: Verify clippy + fmt**

```bash
cargo clippy -p presenter-core --all-targets -- -D warnings -W clippy::all
cargo fmt --all --check
```

Expected: clean.

- [ ] **Step 7: Commit**

```bash
cargo fmt --all
git add crates/presenter-core/src/timer.rs crates/presenter-core/src/lib.rs
git commit -m "feat(core): add format_countdown shared formatter (#280)

Single source of truth for countdown display across server (Resolume),
stage display, and operator panel. Spec rules:
  - < -10 → '' (cleared, 10s past zero)
  - -10..=0 → '0'
  - 1..=59 → digit
  - 60..=3599 → 'MM:SS'
  - >= 3600 → 'Xh Ym' (drop seconds)

Removes the max(0) clamp on TimersOverview.seconds_remaining so the
formatter can detect 'past zero' via a negative value."
```

---

## Task 3: Server delegates to `presenter_core::format_countdown`

**Files:**
- Modify: `crates/presenter-server/src/state/stage.rs:325-334`
- Modify: `crates/presenter-server/src/state/tests.rs:263-267`

- [ ] **Step 1: Replace `format_countdown_text` body**

In `crates/presenter-server/src/state/stage.rs`, replace the entire `format_countdown_text` function (lines 325-334) with:

```rust
pub(crate) fn format_countdown_text(seconds_remaining: i64) -> String {
    presenter_core::format_countdown(seconds_remaining)
}
```

The public name stays so all existing call sites (in `state/timers.rs:42` etc.) keep compiling.

- [ ] **Step 2: Update existing tests in `state/tests.rs`**

In `crates/presenter-server/src/state/tests.rs`, find the existing test that asserts `format_countdown_text` output (search `grep -n "format_countdown_text" crates/presenter-server/src/state/tests.rs`). The current assertions around lines 263-267 are:

```rust
    assert_eq!(format_countdown_text(3605), "60:05");
    assert_eq!(format_countdown_text(125), "02:05");
    assert_eq!(format_countdown_text(59), "59");
    assert_eq!(format_countdown_text(0), "0");
    assert_eq!(format_countdown_text(-12), "0");
```

Replace with:

```rust
    assert_eq!(format_countdown_text(3605), "1h 0m");
    assert_eq!(format_countdown_text(125), "02:05");
    assert_eq!(format_countdown_text(59), "59");
    assert_eq!(format_countdown_text(0), "0");
    assert_eq!(format_countdown_text(-12), "");
```

- [ ] **Step 3: Verify build + tests**

```bash
cargo build -p presenter-server
cargo test -p presenter-server -- --nocapture
```

Expected: all tests pass. Note: there may be other tests anywhere in the workspace that asserted the old format (e.g. an integration test asserting `seconds_remaining` was clamped to 0). If anything fails:

1. If it asserts the old format string → update to new format.
2. If it asserts `seconds_remaining == 0` after target → update to `seconds_remaining < 0` or to the actual signed value the test produces.
3. If it's a regression unrelated to the format — investigate, don't paper over.

- [ ] **Step 4: Verify clippy + fmt**

```bash
cargo clippy -p presenter-server --all-targets -- -D warnings -W clippy::all
cargo fmt --all --check
```

Expected: clean.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/presenter-server/src/state/stage.rs crates/presenter-server/src/state/tests.rs
git commit -m "feat(server): delegate format_countdown_text to presenter-core (#280)

Server-side format_countdown_text becomes a thin wrapper around the
shared presenter_core::format_countdown. Updates existing tests for
the new format ('1h 0m' instead of '60:05'; '' instead of '0' past
the 10s clear window)."
```

---

## Task 4: WASM call sites use `presenter_core::format_countdown` for the countdown

**Files:**
- Modify: `crates/presenter-ui/src/components/stage/timer_layout.rs:23` (the call site only — keep local `format_seconds` for preach if any uses remain)
- Modify: `crates/presenter-ui/src/components/timer_panel.rs:193` (countdown call site only)

### Important: do NOT touch the preach formatting

`timer_panel.rs::format_seconds` is used at lines **193, 220, 228**. ONLY line 193 is the countdown — lines 220 and 228 are preach (elapsed time, always positive, no clearing concept). Leave the local `format_seconds` helper as-is so preach still works.

`timer_layout.rs` only has one call site at line 23 which is the countdown. Verify with `grep -n "format_seconds" crates/presenter-ui/src/components/stage/timer_layout.rs` — should show only one call site outside the helper definition itself. If preach uses it too in this file, similarly leave the helper for preach and only swap the countdown line.

- [ ] **Step 1: Read the current `timer_layout.rs` countdown call**

```bash
sed -n '20,30p' crates/presenter-ui/src/components/stage/timer_layout.rs
```

The current line should be along the lines of:

```rust
            .map(|s| format_seconds(s.timers.countdown_to_start.seconds_remaining))
```

- [ ] **Step 2: Swap the timer_layout.rs call**

In `crates/presenter-ui/src/components/stage/timer_layout.rs`, find the call at line 23:

```rust
            .map(|s| format_seconds(s.timers.countdown_to_start.seconds_remaining))
```

Replace with:

```rust
            .map(|s| presenter_core::format_countdown(s.timers.countdown_to_start.seconds_remaining))
```

If there's a `use presenter_core::*` or similar at the top of the file, you can drop the `presenter_core::` prefix. Check with `grep -n "use presenter_core" crates/presenter-ui/src/components/stage/timer_layout.rs`. If no import exists, prefer the inline qualifier (smaller change surface).

- [ ] **Step 3: Read the current `timer_panel.rs` countdown call**

```bash
sed -n '190,200p' crates/presenter-ui/src/components/timer_panel.rs
```

The current line at 193 should be:

```rust
                            .map(|t| format_seconds(t.countdown_to_start.seconds_remaining))
```

- [ ] **Step 4: Swap the timer_panel.rs countdown call**

In `crates/presenter-ui/src/components/timer_panel.rs`, find the line at 193:

```rust
                            .map(|t| format_seconds(t.countdown_to_start.seconds_remaining))
```

Replace with:

```rust
                            .map(|t| presenter_core::format_countdown(t.countdown_to_start.seconds_remaining))
```

DO NOT touch lines 220 and 228 — those are preach calls (`t.preach_timer.seconds_elapsed` and the limit). They keep the local `format_seconds`.

- [ ] **Step 5: Verify WASM clippy still clean**

```bash
cd crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings -W clippy::all && cd ../..
```

Expected: clean. The local `format_seconds` helpers may now produce a "function is never used" warning IF the only call site was the countdown one we just swapped. If clippy flags `format_seconds` as unused in `timer_layout.rs`, delete the helper. In `timer_panel.rs` it's still used at lines 220 and 228, so keep it.

- [ ] **Step 6: Run the workspace clippy and tests**

```bash
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
cargo test --workspace -- --nocapture
```

Expected: clean and all tests pass. If a `presenter-ui` unit test asserted the old format on the countdown side, update its expectation. The helper-removal warning (if any) was handled in Step 5.

- [ ] **Step 7: Verify fmt**

```bash
cargo fmt --all --check
```

- [ ] **Step 8: Commit**

```bash
cargo fmt --all
git add crates/presenter-ui/src/components/stage/timer_layout.rs crates/presenter-ui/src/components/timer_panel.rs
git commit -m "feat(ui): unify countdown display via presenter_core::format_countdown (#280)

Stage timer layout and operator timer panel both call the shared
presenter-core formatter for the countdown, so Resolume, stage, and
operator panel render identically (1h 31m above an hour, '0' for 10s
past zero, blank thereafter).

Preach timer formatting unchanged — still uses the local format_seconds
helper because elapsed time is never negative."
```

---

## Task 5: Local checks, push, monitor CI, deploy verify, PR, completion report

**Controller-handled task.** Each step is what the controller does after Tasks 1-4 are committed.

- [ ] **Step 1: Run all local checks**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
cargo clippy --target wasm32-unknown-unknown --all-targets --manifest-path crates/presenter-ui/Cargo.toml -- -D warnings -W clippy::all
cargo test --workspace -- --nocapture
```

If any fail, fix in ONE commit and re-run.

- [ ] **Step 2: Push to dev**

```bash
git push origin dev
```

- [ ] **Step 3: Monitor CI to terminal state**

```bash
gh run list --branch dev --limit 1 --json databaseId --jq '.[0].databaseId'
# Capture run id, then:
sleep 1500 && gh run view <run-id> --json status,conclusion,jobs --jq '{status, conclusion, failed: [.jobs[] | select(.conclusion == "failure") | .name]}'
```

If any job fails, `gh run view <run-id> --log-failed`, fix in ONE commit, push again, re-monitor.

- [ ] **Step 4: Verify dev deployment is live**

```bash
curl -s http://10.77.8.134:8080/healthz
```

Expected: `{"channel":"dev","status":"ok","version":"0.4.61"}`.

- [ ] **Step 5: Manual verification on dev — multi-hour format**

Open the operator UI in Playwright at `http://10.77.8.134:8080/ui/operator/timers`. Set the countdown target to roughly 1 hour 31 minutes in the future. Verify:

- Operator panel shows `"1h 31m"` (NOT `"91:30"`).
- Stage display at `http://10.77.8.134:8080/stage` shows `"1h 31m"`.
- Resolume #timer clip (if connected on dev) shows `"1h 31m"`.

If Resolume isn't connected on dev, verify via journalctl that the resolume worker emitted the expected payload:

```bash
sudo journalctl -u presenter-dev --since '1 minute ago' | grep "resolume.update_text"
```

The `payload=` field should be `"1h 31m"`.

- [ ] **Step 6: Manual verification on dev — post-zero clear**

Set the countdown target to 5 seconds in the future. Wait for it to hit 0. Verify:

- For ~10 seconds past zero, all three surfaces (operator, stage, Resolume) show `"0"`.
- After 10+ seconds past zero, all three surfaces show empty/blank.

Capture a screenshot or DOM dump for the PR body.

- [ ] **Step 7: Open PR**

```bash
gh pr create --title "feat: timer countdown formatting + post-zero clear (#280)" --body "$(cat <<'EOF'
## Summary

Fixes #280: timer countdown now (a) shows '0' for ~10 seconds after hitting zero then clears, and (b) renders durations >1 hour as 'Xh Ym' (e.g. '1h 31m') instead of MM:SS rolled past 60.

## What changed

- New `presenter_core::format_countdown(seconds_remaining: i64) -> String` is the single source of truth for countdown display across server, stage, and operator panel.
- Server's `format_countdown_text` becomes a thin wrapper.
- WASM stage timer + operator timer panel call the core formatter for the countdown (preach formatting unchanged).
- `TimersOverview.seconds_remaining` is no longer clamped to 0 — the formatter detects 'past zero' via a negative value.

## Format spec

| seconds_remaining | display |
|---|---|
| < -10 | "" (cleared) |
| -10..=0 | "0" |
| 1..=59 | "1", "59" |
| 60..=3599 | "MM:SS" |
| >= 3600 | "Xh Ym" |

## Test plan

- [ ] All workspace tests pass
- [ ] 13 boundary unit tests for `format_countdown` (in `presenter-core/src/timer.rs`)
- [ ] Existing `format_countdown_text` test updated for new format
- [ ] Dev `/healthz` reports v0.4.61
- [ ] Manual: 1h 31m countdown shows '1h 31m' on operator, stage, and Resolume
- [ ] Manual: post-zero countdown shows '0' for 10s then clears on all three surfaces
- [ ] Browser console clean

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 8: Verify PR is mergeable**

```bash
gh pr view <pr-number> --json mergeable,mergeStateStatus
```

Expected: `mergeable: true`, `mergeStateStatus: CLEAN`. If UNSTABLE due to mutation testing or PR Automation still pending, wait. If anything else, investigate.

- [ ] **Step 9: Run pre-completion gates**

Invoke `/plan-check` skill — must come back N/N fulfilled. Invoke `/review` skill on this PR — must come back `0 🔴 0 🟡 0 🔵`. Fix any findings inside the diff before sending the completion report.

- [ ] **Step 10: Send completion report**

Per `core/completion-report.md`. Include CI run ID, plan-check N/N, review clean, deploy verification (dev shows v0.4.61 with both manual scenarios verified), URLs, PR title + URL.

---

## Verification Summary

| Check | How to verify |
|-------|---------------|
| `format_countdown` correctness | 13 boundary unit tests in `presenter-core/src/timer.rs` pass |
| Server delegation | Existing `format_countdown_text` test passes with new format |
| WASM countdown swap | Workspace tests pass; clippy clean for both native and WASM targets |
| Preach untouched | Operator panel preach still shows MM:SS; preach helper still in use at lines 220, 228 |
| Multi-hour format | Manual: 1h 31m countdown shows '1h 31m' on operator + stage + Resolume |
| Post-zero clear | Manual: 0 shown for 10s, then blank on all three surfaces |
| No regressions | Existing `presenter-core` and `presenter-server` tests still pass |
| Clean console | Playwright session shows zero browser console errors |
