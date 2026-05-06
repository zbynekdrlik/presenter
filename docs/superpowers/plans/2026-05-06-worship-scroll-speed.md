# Worship Slide List Scroll Speed Fix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the worship slide list so a single mouse-wheel or trackpad gesture no longer blasts through dozens of rows.

**Architecture:** Replace the current `direction * step` formula in `handle_wheel_event` with a magnitude-respecting cap: use `delta_y.signum() * delta_y.abs().min(step)`. Extract the cap into a pure helper `cap_wheel_delta` so it's natively unit-testable. No new dependencies, no behavioral change to the catch-all `prevent_default` flow.

**Tech Stack:** Rust / Leptos (WASM), web-sys, native cargo test.

**Spec:** `docs/superpowers/specs/2026-05-06-worship-scroll-speed-design.md` (commit 9c6ed1c).

---

## Context

Issue #301 (title-only): "scrolling is extremally fast in worship slides".

`handle_wheel_event` was added in PR #290 (issue #271) to neutralize macOS scroll acceleration. It calls `ev.prevent_default()` then advances the container scroll by `direction * step` where `direction = ev.delta_y().signum()` (always ±1) and `step ≈ card_height + 14.4 px`. The discarded magnitude means every wheel event — no matter how small — moves one full row. Trackpads stream 20-30+ events per gesture → 20-30+ rows per gesture → "extremely fast".

**Key existing code:**

- `crates/presenter-ui/src/components/slide_list_scroll.rs:128-146` — `handle_wheel_event(ev: web_sys::WheelEvent)`.
- `crates/presenter-ui/src/components/slide_list_scroll.rs:109-123` — `step_for_wheel(container)` returns `card_height + 14.4` or `DEFAULT_WHEEL_STEP_PX = 120.0` if no card is rendered. Always returns a positive value.
- `crates/presenter-ui/src/components/slide_list.rs:310` — wired up via `on:wheel=handle_wheel_event`.
- The file currently has NO `#[cfg(test)] mod tests` block — Task 3 adds one at the file end.

---

## File Structure

### Modified Files

| File | Change |
|------|--------|
| `Cargo.toml` | Workspace version 0.4.68 → 0.4.69 |
| `crates/presenter-ui/Cargo.toml` | Version 0.1.37 → 0.1.38 |
| `crates/presenter-ui/src/components/slide_list_scroll.rs` | Add `cap_wheel_delta` pure helper; change `handle_wheel_event` to use it; add `#[cfg(test)] mod tests` with 7 boundary-case unit tests |

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
version = "0.4.69"
```

- [ ] **Step 2: Bump presenter-ui version**

In `crates/presenter-ui/Cargo.toml`, change line 3:

```toml
version = "0.1.38"
```

- [ ] **Step 3: Update lockfiles**

```bash
cargo update --workspace
cargo update --workspace --manifest-path crates/presenter-ui/Cargo.toml
```

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock crates/presenter-ui/Cargo.toml crates/presenter-ui/Cargo.lock
git commit -m "chore: bump version to 0.4.69"
```

---

## Task 2: Extract `cap_wheel_delta` and use it in `handle_wheel_event`

**Files:**
- Modify: `crates/presenter-ui/src/components/slide_list_scroll.rs:128-146` (handler body) + add helper

- [ ] **Step 1: Read the current state of `handle_wheel_event`**

```bash
sed -n '125,146p' crates/presenter-ui/src/components/slide_list_scroll.rs
```

You should see:

```rust
/// Wheel handler for `.operator__slides`: intercepts the native (accelerated)
/// scroll, applies a deterministic per-notch step instead. Issue #271 concern 2:
/// neutralises macOS scroll acceleration so each notch advances ~1 row.
pub(super) fn handle_wheel_event(ev: web_sys::WheelEvent) {
    ev.prevent_default();
    let direction = ev.delta_y().signum();
    if direction == 0.0 {
        return;
    }
    let Some(target) = ev.target() else { return };
    let Ok(el) = target.dyn_into::<web_sys::Element>() else {
        return;
    };
    let Ok(Some(container_el)) = el.closest(".operator__slides") else {
        return;
    };
    let Ok(container) = container_el.dyn_into::<web_sys::HtmlElement>() else {
        return;
    };
    let step = step_for_wheel(&container);
    container.set_scroll_top((container.scroll_top() as f64 + direction * step) as i32);
}
```

- [ ] **Step 2: Add the `cap_wheel_delta` pure helper**

In `crates/presenter-ui/src/components/slide_list_scroll.rs`, ABOVE `handle_wheel_event` (between `step_for_wheel` and `handle_wheel_event`, so just before the `/// Wheel handler` doc comment at line 125), add:

```rust
/// Cap a wheel event's vertical delta to one row of cards (`step`) per event,
/// preserving sign. Returns 0 when `delta_y` is 0 (no-op).
///
/// Issue #301: previously the wheel handler discarded `delta_y` magnitude and
/// always advanced one full row per event, so trackpad gestures (which fire
/// 20+ events per swipe) blasted through dozens of rows.
fn cap_wheel_delta(delta_y: f64, step: f64) -> f64 {
    if delta_y == 0.0 {
        return 0.0;
    }
    delta_y.signum() * delta_y.abs().min(step)
}
```

- [ ] **Step 3: Replace the body of `handle_wheel_event`**

Replace the entire `handle_wheel_event` function (lines 128-146 of the original file, before your insertion in Step 2) with:

```rust
/// Wheel handler for `.operator__slides`: intercepts the native (accelerated)
/// scroll and applies a magnitude-respecting cap. Issue #271 concern 2:
/// neutralises macOS scroll acceleration. Issue #301: per-event delta is
/// capped at one row so high-frequency trackpad/mouse events can't blast
/// through the list.
pub(super) fn handle_wheel_event(ev: web_sys::WheelEvent) {
    ev.prevent_default();
    let delta_y = ev.delta_y();
    if delta_y == 0.0 {
        return;
    }
    let Some(target) = ev.target() else { return };
    let Ok(el) = target.dyn_into::<web_sys::Element>() else {
        return;
    };
    let Ok(Some(container_el)) = el.closest(".operator__slides") else {
        return;
    };
    let Ok(container) = container_el.dyn_into::<web_sys::HtmlElement>() else {
        return;
    };
    let step = step_for_wheel(&container);
    let capped = cap_wheel_delta(delta_y, step);
    container.set_scroll_top((container.scroll_top() as f64 + capped) as i32);
}
```

The differences from the original:
1. Read `delta_y` once instead of `direction = signum`.
2. Early-return when `delta_y == 0.0` (instead of `direction == 0.0`).
3. Compute `capped` from `cap_wheel_delta(delta_y, step)`.
4. Add `capped` to scroll top instead of `direction * step`.

- [ ] **Step 4: Build the WASM crate to verify the change compiles**

```bash
cd crates/presenter-ui && cargo build --target wasm32-unknown-unknown && cd ../..
```

Expected: clean build.

- [ ] **Step 5: Run native + WASM clippy**

```bash
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
cd crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings -W clippy::all && cd ../..
```

Expected: clean.

- [ ] **Step 6: Run fmt**

```bash
cargo fmt --all --check
```

Expected: silent.

- [ ] **Step 7: Commit**

```bash
cargo fmt --all
git add crates/presenter-ui/src/components/slide_list_scroll.rs
git commit -m "fix(ui): cap wheel delta per event to one row (#301)

handle_wheel_event used direction * step (always one row per wheel
event), so trackpad gestures with 20-30 events scrolled 20-30 rows.
Replace with magnitude-respecting cap: delta.signum() * min(|delta|, step).
Small gestures advance proportionally; large gestures still bounded at
one row per event so the original #271 'no macOS acceleration' intent
is preserved.

Extract cap_wheel_delta as a pure helper for native unit testing."
```

---

## Task 3: Unit tests for `cap_wheel_delta`

**Files:**
- Modify: `crates/presenter-ui/src/components/slide_list_scroll.rs` (append `#[cfg(test)] mod tests` at end of file)

- [ ] **Step 1: Append the tests module at the end of the file**

After the closing `}` of `handle_wheel_event` (the very last line of the file), append:

```rust

#[cfg(test)]
mod tests {
    use super::cap_wheel_delta;

    #[test]
    fn zero_delta_returns_zero() {
        assert_eq!(cap_wheel_delta(0.0, 90.0), 0.0);
    }

    #[test]
    fn small_positive_delta_passes_through() {
        assert_eq!(cap_wheel_delta(10.0, 90.0), 10.0);
    }

    #[test]
    fn small_negative_delta_passes_through() {
        assert_eq!(cap_wheel_delta(-10.0, 90.0), -10.0);
    }

    #[test]
    fn delta_exactly_at_step_passes_through() {
        assert_eq!(cap_wheel_delta(90.0, 90.0), 90.0);
    }

    #[test]
    fn negative_delta_exactly_at_step_passes_through() {
        assert_eq!(cap_wheel_delta(-90.0, 90.0), -90.0);
    }

    #[test]
    fn large_positive_delta_is_capped_at_step() {
        assert_eq!(cap_wheel_delta(500.0, 90.0), 90.0);
    }

    #[test]
    fn large_negative_delta_is_capped_at_negative_step() {
        assert_eq!(cap_wheel_delta(-500.0, 90.0), -90.0);
    }
}
```

- [ ] **Step 2: Run the new tests**

The presenter-ui crate is excluded from the workspace (per `Cargo.toml:11`), so run the tests from inside the crate dir:

```bash
cd crates/presenter-ui && cargo test --lib && cd ../..
```

Expected: 7 new tests pass. Output should include `test result: ok. 7 passed; 0 failed;` (or more if the crate already has other tests).

If the tests don't run because the file isn't picked up by cargo-test, verify the module is exported (it should be — slide_list_scroll is a `mod` declaration in `components/mod.rs` already, since `handle_wheel_event` is used by `slide_list.rs`).

- [ ] **Step 3: Run native clippy + fmt to confirm cleanliness**

```bash
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
cd crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings -W clippy::all && cd ../..
cargo fmt --all --check
```

Expected: clean.

- [ ] **Step 4: Commit**

```bash
cargo fmt --all
git add crates/presenter-ui/src/components/slide_list_scroll.rs
git commit -m "test(ui): cover cap_wheel_delta boundaries (#301)

Seven boundary-case unit tests for the new pure helper:
zero, small ±, exactly ±step, large ± (capped). Native cargo test
exercises the cap math without needing a WASM browser environment."
```

---

## Task 4: Local checks, push, monitor CI, deploy verify, PR, completion report

**Controller-handled task.** Each step is what the controller does after Tasks 1-3 are committed.

- [ ] **Step 1: Run all local checks**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
cd crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings -W clippy::all && cd ../..
cargo test --workspace -- --nocapture
cd crates/presenter-ui && cargo test --lib && cd ../..
```

Expected: all pass.

- [ ] **Step 2: Push to dev**

```bash
git push origin dev
```

- [ ] **Step 3: Monitor CI to terminal state**

```bash
gh run list --branch dev --limit 1 --json databaseId --jq '.[0].databaseId'
sleep 1500 && gh run view <run-id> --json status,conclusion,jobs --jq '{status, conclusion, failed: [.jobs[] | select(.conclusion == "failure") | .name]}'
```

If any job fails, `gh run view <run-id> --log-failed`, fix in ONE commit, push again, re-monitor.

- [ ] **Step 4: Verify dev deployment is live**

```bash
curl -s http://10.77.8.134:8080/healthz
```

Expected: `{"channel":"dev","status":"ok","version":"0.4.69"}`.

- [ ] **Step 5: Manual verification on dev**

Open `http://10.77.8.134:8080/ui/operator` in a browser with at least 30 worship slides loaded. Use a trackpad if available:

1. **Gentle two-finger swipe**: list scrolls smoothly, advancing roughly 1-3 rows for a small gesture. NOT 30 rows.
2. **Fast flick**: list advances multiple rows but bounded — one row per event. No "blast through to bottom".
3. **Mouse wheel notch**: each notch advances about one row. Smooth but bounded.
4. **Browser console clean** (zero errors/warnings).

If trackpad isn't available, use Playwright MCP to fire `page.mouse.wheel(deltaY=200)` and inspect the resulting scroll position to confirm it advanced ≤ one row.

Capture a brief screenshot or DOM dump for the PR body.

- [ ] **Step 6: Open PR**

```bash
gh pr create --title "fix(ui): cap wheel delta per event to one row (#301)" --body "$(cat <<'EOF'
## Summary

Fixes #301: scrolling in worship slides was "extremely fast" because the wheel handler discarded the wheel event's `delta_y` magnitude and always advanced exactly one row per event. Trackpads fire 20-30+ events per gesture, so a single swipe blasted through 20-30 rows.

## What changed

- New pure helper `cap_wheel_delta(delta_y, step) -> f64` returns `delta_y.signum() * delta_y.abs().min(step)`. Sign-preserving, magnitude-respecting, capped at one row per event.
- `handle_wheel_event` now uses the cap instead of `direction * step`.
- 7 unit tests for the cap function (zero, small ±, exactly ±step, large ± capped).

## Behavior matrix

| `delta_y` | Old | New |
|---|---|---|
| `+10` (small swipe) | one full row (~90 px) | 10 px |
| `+90` (one notch) | one full row | 90 px (≈ row) |
| `+500` (high-DPI fast event) | one full row | 90 px (capped) |
| `-200` | one full row up | 90 px up (capped) |
| `0` | no-op | no-op |

The original #271 fix (no macOS unbounded acceleration) is preserved — the per-event cap is still ~one row.

## Test plan

- [x] 7 new unit tests for `cap_wheel_delta`
- [x] Workspace tests pass
- [x] Native + WASM clippy clean
- [x] Dev `/healthz` reports v0.4.69
- [x] Manual: trackpad swipe on dev advances proportionally; fast flick still bounded; no console errors

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 7: Verify PR is mergeable**

```bash
gh pr view <pr-number> --json mergeable,mergeStateStatus
```

Expected: `mergeable: MERGEABLE`, `mergeStateStatus: CLEAN` (after Mutation Testing + Label PR finish). If `UNSTABLE` due to checks still pending, wait. If anything failed, fix.

- [ ] **Step 8: Run pre-completion gates**

Invoke `/plan-check` skill — must come back N/N fulfilled. Invoke `/review` skill on this PR — must come back `0 🔴 0 🟡 0 🔵`. Fix any findings inside the diff before sending the completion report.

- [ ] **Step 9: Send completion report**

Per `core/completion-report.md`. Include CI run id, plan-check N/N, review clean, deploy verify (dev shows v0.4.69, manual scroll test passed), URLs, PR title + URL.

---

## Verification Summary

| Check | How to verify |
|-------|---------------|
| `cap_wheel_delta` zero / small / boundary / capped | 7 unit tests in `crates/presenter-ui/src/components/slide_list_scroll.rs::tests` |
| `handle_wheel_event` uses the cap | grep for `cap_wheel_delta` in the handler body |
| No regressions | Workspace tests + WASM clippy clean |
| Live behavior on dev | Trackpad swipe scrolls proportionally; fast flick bounded |
| Original #271 intent preserved | Per-event upper bound is still `step` (~one row) |
| Clean console | Browser session shows zero errors |
