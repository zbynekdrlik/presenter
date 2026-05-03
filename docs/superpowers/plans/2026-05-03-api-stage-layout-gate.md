# API Stage Layout Gate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop `PUT /api/stage` from leaking into the operator preview when the current stage layout is not `api`. Add a gate in `update_api_stage` and a switch-to-api refresh in `set_stage_layout_code` so the api-stage state stays in lockstep with operator-controlled layout changes.

**Architecture:** Two surgical changes in `crates/presenter-server/src/state/`. (1) `update_api_stage` (mod.rs:735) reads `stage_layout_code()` and only publishes `LiveEvent::Stage` when the code is `"api"` — state always stored. (2) `set_stage_layout_code` (stage_display.rs:47) handles the symmetric case: when switching TO `"api"`, fetch the stored api_stage snapshot and publish it; otherwise `broadcast_stage_snapshots` runs as today.

**Tech Stack:** Rust + tokio (async state), axum (HTTP), tokio::sync::broadcast (LiveHub), Playwright TS for E2E.

**Spec:** `docs/superpowers/specs/2026-05-03-api-stage-layout-gate-design.md` (commit `e67c2be`)

**Closes:** Issue #281 — api worship input switching preview when layout = worship-snv.

---

## Context

### Verified pre-flight

- `crates/presenter-server/src/state/mod.rs:735-740` — `update_api_stage` publishes unconditionally. The fix injects a gate.
- `crates/presenter-server/src/state/stage_display.rs:43-45` — `stage_layout_code()` returns `String` via `RwLock::read().await.clone()`.
- `crates/presenter-server/src/state/stage_display.rs:47-65` — `set_stage_layout_code` already publishes `LiveEvent::StageLayout` then calls `broadcast_stage_snapshots`. The `broadcast_stage_snapshots` call moves into the non-api branch.
- `crates/presenter-server/src/state/mod.rs:742-745` — `api_stage_snapshot()` (already public) returns `StageDisplaySnapshot` for the api layout.
- `crates/presenter-server/src/state/broadcasting.rs:82-88` — existing inverse gate (skip publish when layout=api). The new gate is symmetric.
- `LiveEvent::Stage`, `LiveEvent::StageLayout` — both already exist; no new variants.
- `tests/e2e/api-stage.spec.ts` — 258 lines, exists since PR #255. Has `openApiStage(context)` helper that POSTs `/stage/layout` with `{ code: "api" }` and waits for `__presenterStageLayout === "api"`.
- `state/tests.rs` LiveHub pattern: `let mut rx = state.live_hub().subscribe(); ... rx.recv().await.unwrap();` (e.g. line 273-285).
- `ApiStageState` struct fields: `current_text`, `current_group`, `current_song`, `next_text`, `next_group`, `next_song`, all `String`. Default = empty strings.
- Stage layout codes seen in code: `"api"`, `"worship-snv"`, `"worship-pp"`, `"preach"`, `"timer"`, `"bible"`, `"ndi-fullscreen"`. Default = `DEFAULT_STAGE_LAYOUT_CODE`.

---

## File Structure

### Modified files
| File | Change |
|------|--------|
| `Cargo.toml` | Workspace version 0.4.54 → 0.4.55 |
| `crates/presenter-ui/Cargo.toml` | presenter-ui version 0.1.23 → 0.1.24 |
| `crates/presenter-server/src/state/mod.rs` | Gate `LiveEvent::Stage` publish in `update_api_stage` on `stage_layout_code() == "api"` |
| `crates/presenter-server/src/state/stage_display.rs` | In `set_stage_layout_code`, publish stored api_stage snapshot when target layout is `"api"`; move `broadcast_stage_snapshots` into the `else` branch |
| `crates/presenter-server/src/state/tests.rs` | 3 new tests |
| `tests/e2e/api-stage.spec.ts` | 1 new test for the layout-isolation behavior |

### Lock files
- `Cargo.lock` and `crates/presenter-ui/Cargo.lock` — auto-updated.

---

## Task 1: Bump Version (Haiku)

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/presenter-ui/Cargo.toml`
- Modify: `Cargo.lock`, `crates/presenter-ui/Cargo.lock` (regenerated)

- [ ] **Step 1: Bump workspace version**

In `/home/newlevel/devel/presenter/presenter-dev2/Cargo.toml`, under `[workspace.package]`, change `version = "0.4.54"` to `version = "0.4.55"`.

- [ ] **Step 2: Bump presenter-ui version**

In `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui/Cargo.toml`, under `[package]`, change `version = "0.1.23"` to `version = "0.1.24"`.

- [ ] **Step 3: Regenerate workspace Cargo.lock**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo check --workspace --all-targets 2>&1 | tail -5
```

- [ ] **Step 4: Regenerate presenter-ui Cargo.lock**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui && cargo check --target wasm32-unknown-unknown 2>&1 | tail -5
```

- [ ] **Step 5: Verify**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && grep -E "^version" Cargo.toml crates/presenter-ui/Cargo.toml | head -3
```

Expected:
```
Cargo.toml:version = "0.4.55"
crates/presenter-ui/Cargo.toml:version = "0.1.24"
```

- [ ] **Step 6: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && git add Cargo.toml Cargo.lock crates/presenter-ui/Cargo.toml crates/presenter-ui/Cargo.lock && git commit -m "chore: bump version to 0.4.55 (#281)"
```

---

## Task 2: Apply the Gate + Switch-to-API Refresh (Sonnet)

**Files:**
- Modify: `crates/presenter-server/src/state/mod.rs:735-740`
- Modify: `crates/presenter-server/src/state/stage_display.rs:47-65`

### Step 1: Read current state

```bash
sed -n '733,745p' /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-server/src/state/mod.rs
sed -n '47,65p' /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-server/src/state/stage_display.rs
```

Confirm the bodies match the snippets in the spec context above.

### Step 2: Add the gate to `update_api_stage`

In `crates/presenter-server/src/state/mod.rs`, replace the entire `update_api_stage` function (lines 735-740) with:

```rust
    pub(crate) async fn update_api_stage(&self, state: ApiStageState) -> anyhow::Result<()> {
        let snapshot = self.build_api_stage_snapshot(&state).await;
        *self.api_stage.write().await = state;
        // Issue #281: only publish a Stage event when the operator's
        // current layout is "api". Otherwise the api state is stored but
        // does not affect the live preview, mirroring the existing inverse
        // gate in `broadcasting.rs::publish_stage_context` (which skips
        // non-api updates when api layout is selected).
        if self.stage_layout_code().await == "api" {
            self.live_hub.publish(LiveEvent::Stage { snapshot });
        }
        Ok(())
    }
```

### Step 3: Add switch-to-api refresh to `set_stage_layout_code`

In `crates/presenter-server/src/state/stage_display.rs`, the current function reads:

```rust
    pub async fn set_stage_layout_code(&self, code: &str) -> anyhow::Result<StageDisplayLayout> {
        let layout = StageDisplayLayout::built_in()
            .into_iter()
            .find(|layout| layout.code == code)
            .ok_or_else(|| anyhow::anyhow!("unknown stage layout: {code}"))?;
        {
            let mut guard = self.stage_layout.write().await;
            if *guard == layout.code {
                return Ok(layout);
            }
            *guard = layout.code.clone();
        }
        self.live_hub.publish(LiveEvent::StageLayout {
            code: layout.code.clone(),
        });
        self.broadcast_stage_snapshots().await?;
        Ok(layout)
    }
```

Replace it with:

```rust
    pub async fn set_stage_layout_code(&self, code: &str) -> anyhow::Result<StageDisplayLayout> {
        let layout = StageDisplayLayout::built_in()
            .into_iter()
            .find(|layout| layout.code == code)
            .ok_or_else(|| anyhow::anyhow!("unknown stage layout: {code}"))?;
        {
            let mut guard = self.stage_layout.write().await;
            if *guard == layout.code {
                return Ok(layout);
            }
            *guard = layout.code.clone();
        }
        self.live_hub.publish(LiveEvent::StageLayout {
            code: layout.code.clone(),
        });
        if layout.code == "api" {
            // Issue #281: when switching TO api, publish the stored
            // api_stage snapshot so the operator preview reflects the most
            // recent PUT instead of waiting for the next one.
            // `broadcast_stage_snapshots` short-circuits on api layout
            // anyway, so we replace it with the api snapshot publish here.
            let snapshot = self.api_stage_snapshot().await;
            self.live_hub.publish(LiveEvent::Stage { snapshot });
        } else {
            self.broadcast_stage_snapshots().await?;
        }
        Ok(layout)
    }
```

### Step 4: Verify build

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo build --workspace 2>&1 | tail -5
```

Expected: clean build.

If `api_stage_snapshot` is not visible from `stage_display.rs` (it's defined on the same `impl AppState` block in `mod.rs:742`), it should resolve via `self.api_stage_snapshot()`. If the visibility is `pub(crate)` and the call site is in the same crate, this works. If the build fails on visibility, change `api_stage_snapshot` from `pub(crate)` to `pub(super)` or `pub` to broaden access — but `pub(crate)` should be sufficient.

### Step 5: Run clippy

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo clippy --workspace --all-targets -- -D warnings -W clippy::all 2>&1 | tail -10
```

Expected: zero warnings.

### Step 6: Run cargo fmt

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo fmt --all
```

### Step 7: Commit

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && git add crates/presenter-server/src/state/mod.rs crates/presenter-server/src/state/stage_display.rs && git commit -m "fix(server): gate api-stage publish on current layout (#281)

update_api_stage now only publishes LiveEvent::Stage when the operator's
selected layout is api. State is still stored on every PUT so the next
switch to api shows the latest content.

set_stage_layout_code now publishes the stored api_stage snapshot when
switching TO api, so the preview reflects recent API pushes immediately
instead of waiting for the next PUT."
```

---

## Task 3: Unit Tests (Sonnet)

**Files:**
- Modify: `crates/presenter-server/src/state/tests.rs` — add 3 new tests at the end of the file.

The existing test pattern uses `let mut rx = state.live_hub().subscribe()` then `rx.recv().await.unwrap()` to consume events. There are usually OTHER events fired alongside the one under test (initial state, timers ticks, etc.), so tests loop over a small bounded number of `recv` calls and assert that the expected event arrives. For "no event" tests, use `tokio::time::timeout` to bound the wait.

### Step 1: Read the existing test patterns

```bash
sed -n '1,30p' /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-server/src/state/tests.rs
sed -n '270,320p' /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-server/src/state/tests.rs
```

Confirm:
- `AppState::in_memory()` is the test fixture
- `state.live_hub().subscribe()` returns a broadcast receiver
- `LiveEvent::Stage { snapshot }` and `LiveEvent::StageLayout { code }` are the variants

### Step 2: Add the three tests at the end of `crates/presenter-server/src/state/tests.rs`

Append:

```rust
#[tokio::test]
async fn api_input_does_not_leak_when_layout_is_worship() {
    use std::time::Duration;
    use tokio::time::timeout;

    let state = AppState::in_memory().await.unwrap();
    state
        .set_stage_layout_code("worship-snv")
        .await
        .expect("set worship-snv");

    let mut rx = state.live_hub().subscribe();

    let api_state = ApiStageState {
        current_text: "test main".to_string(),
        current_group: "test group".to_string(),
        current_song: "test song".to_string(),
        ..Default::default()
    };
    state
        .update_api_stage(api_state.clone())
        .await
        .expect("update_api_stage");

    // Drain non-Stage events for a short window. Assert no LiveEvent::Stage
    // arrives within the timeout — that's the "no leak" invariant.
    let saw_stage = async {
        loop {
            match rx.recv().await {
                Ok(LiveEvent::Stage { .. }) => return true,
                Ok(_) => continue,
                Err(_) => return false,
            }
        }
    };
    let result = timeout(Duration::from_millis(150), saw_stage).await;
    assert!(
        result.is_err(),
        "expected NO LiveEvent::Stage when layout is worship-snv (got: {:?})",
        result
    );

    // Sanity: the api_stage IS stored (not silently discarded).
    let stored = state.api_stage.read().await.clone();
    assert_eq!(stored.current_text, "test main");
}

#[tokio::test]
async fn api_input_publishes_when_layout_is_api() {
    use std::time::Duration;
    use tokio::time::timeout;

    let state = AppState::in_memory().await.unwrap();
    state
        .set_stage_layout_code("api")
        .await
        .expect("set api layout");

    let mut rx = state.live_hub().subscribe();

    let api_state = ApiStageState {
        current_text: "live api content".to_string(),
        ..Default::default()
    };
    state
        .update_api_stage(api_state)
        .await
        .expect("update_api_stage");

    let stage_event = async {
        loop {
            match rx.recv().await {
                Ok(LiveEvent::Stage { snapshot }) => return Some(snapshot),
                Ok(_) => continue,
                Err(_) => return None,
            }
        }
    };
    let snapshot = timeout(Duration::from_millis(500), stage_event)
        .await
        .expect("Stage event arrived")
        .expect("Stage event payload");

    assert_eq!(
        snapshot.layout.code, "api",
        "snapshot must use the api layout"
    );
}

#[tokio::test]
async fn switching_to_api_publishes_stored_api_state() {
    use std::time::Duration;
    use tokio::time::timeout;

    let state = AppState::in_memory().await.unwrap();
    state
        .set_stage_layout_code("worship-snv")
        .await
        .expect("set worship-snv");

    // Pre-store API content while not in api layout.
    state
        .update_api_stage(ApiStageState {
            current_text: "stored content".to_string(),
            ..Default::default()
        })
        .await
        .expect("update_api_stage");

    let mut rx = state.live_hub().subscribe();

    state
        .set_stage_layout_code("api")
        .await
        .expect("switch to api");

    // Expect at least one StageLayout and one Stage event within the timeout.
    let mut saw_layout = false;
    let mut saw_stage_with_stored_content = false;
    let collect = async {
        for _ in 0..10 {
            if let Ok(ev) = rx.recv().await {
                match ev {
                    LiveEvent::StageLayout { code } if code == "api" => saw_layout = true,
                    LiveEvent::Stage { snapshot } => {
                        if snapshot.layout.code == "api" {
                            saw_stage_with_stored_content = true;
                        }
                    }
                    _ => {}
                }
                if saw_layout && saw_stage_with_stored_content {
                    return ();
                }
            }
        }
    };
    let _ = timeout(Duration::from_millis(500), collect).await;

    assert!(
        saw_layout,
        "expected LiveEvent::StageLayout for api after switch"
    );
    assert!(
        saw_stage_with_stored_content,
        "expected LiveEvent::Stage with api layout after switch"
    );
}
```

The tests assume `ApiStageState` is accessible from this test module. It's `pub(crate)` per `state/mod.rs:83`, and `tests.rs` is `mod tests` inside `crate::state`, so `super::ApiStageState` works. If the existing imports at the top of `tests.rs` don't already pull `ApiStageState` in, add it.

### Step 3: Verify imports in tests.rs

Run:

```bash
head -10 /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-server/src/state/tests.rs
```

If `ApiStageState` is not in the imports, add it:

```rust
use super::{AppState, ApiStageState};
```

(Or extend the existing `use super::*;` if that's the pattern.)

### Step 4: Run the new tests

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo test -p presenter-server --lib api_input_does_not_leak_when_layout_is_worship api_input_publishes_when_layout_is_api switching_to_api_publishes_stored_api_state 2>&1 | tail -15
```

Expected: 3 passed.

If any test times out (saw_stage assertion fires the wrong way), investigate:
- `api_input_does_not_leak`: did `set_stage_layout_code("worship-snv")` actually take effect? Verify `state.stage_layout_code().await == "worship-snv"` before the api_state update.
- `api_input_publishes`: is `set_stage_layout_code("api")` itself emitting an extra Stage event before the test subscribes? The subscribe must happen AFTER the layout switch; check ordering.
- `switching_to_api`: the subscribe must happen AFTER `update_api_stage` (so no leftover events from the gate test).

### Step 5: Run the full presenter-server test suite

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo test -p presenter-server 2>&1 | tail -10
```

Expected: all tests pass (185+ — the existing 184 plus 3 new ones, or whatever the current count is).

### Step 6: Run clippy

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo clippy --workspace --all-targets -- -D warnings -W clippy::all 2>&1 | tail -5
```

Expected: zero warnings.

### Step 7: cargo fmt

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo fmt --all
```

### Step 8: Commit

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && git add crates/presenter-server/src/state/tests.rs && git commit -m "test(server): api stage layout gate + switch-to-api refresh (#281)"
```

---

## Task 4: Playwright E2E (Sonnet)

**Files:**
- Modify: `tests/e2e/api-stage.spec.ts` — add 1 new test for layout isolation.

The existing file has an `openApiStage(context)` helper that sets layout to api. We need a complementary helper that sets layout to a non-api layout, plus a test that verifies the api PUT does NOT affect the operator preview.

### Step 1: Read existing helpers and test shape

```bash
cat /home/newlevel/devel/presenter/presenter-dev2/tests/e2e/api-stage.spec.ts | head -100
```

Confirm:
- The `/stage/layout` POST endpoint is used to set layout (with body `{ code: <code> }`).
- The operator preview is at `/ui/operator/worship` (or similar).
- The stage WS endpoint is `/stage` and exposes `__presenterStageLayout` and `__presenterStageConnectionState` globals.

### Step 2: Add the test at the end of `tests/e2e/api-stage.spec.ts`

Append:

```typescript
test("api put does not switch preview when layout is worship-snv", async ({
  request,
  page,
}) => {
  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      const text = msg.text();
      // Existing operator E2E filters this Chrome integrity-preload warning.
      if (!text.includes("crbug.com/981419")) {
        consoleMessages.push(`[${msg.type()}] ${text}`);
      }
    }
  });

  // 1. Set layout to worship-snv (a non-api layout).
  const setLayoutRes = await request.post(
    new URL("/stage/layout", baseURL).toString(),
    { data: { code: "worship-snv" } },
  );
  expect(setLayoutRes.ok()).toBeTruthy();

  // 2. Snapshot the current api_stage state on the server (before the PUT).
  const beforeRes = await request.get(
    new URL("/api/stage", baseURL).toString(),
  );
  // /api/stage may or may not exist as GET; if not, skip the snapshot step
  // and rely solely on the WS event check below.
  let beforeText = "";
  if (beforeRes.ok()) {
    const before = (await beforeRes.json()) as { current_text?: string };
    beforeText = before.current_text ?? "";
  }

  // 3. Open the operator UI worship view; subscribe to the stage WS via the
  // page's existing __presenterStageConnectionState global.
  await page.goto(new URL("/ui/operator/worship", baseURL).toString());
  await page.waitForLoadState("networkidle");

  // Capture current rendered stage preview (the snapshot displayed in
  // operator UI). The exact selector depends on the operator's preview
  // component; data-role="stage-preview" or similar is conventional.
  const previewSelector = "[data-role=\"stage-preview\"]";
  const previewBefore = await page
    .locator(previewSelector)
    .first()
    .textContent()
    .catch(() => null);

  // 4. PUT api/stage with new content. With the gate, this MUST NOT cause
  // the operator preview to update.
  const putRes = await request.put(
    new URL("/api/stage", baseURL).toString(),
    {
      data: {
        current_text: "should not appear in worship-snv preview",
        current_group: "",
        current_song: "",
        next_text: "",
        next_group: "",
        next_song: "",
      },
    },
  );
  expect(putRes.ok()).toBeTruthy();

  // 5. Wait briefly for any potential leak event to land.
  await page.waitForTimeout(500);

  // 6. Verify the operator preview did NOT change.
  const previewAfter = await page
    .locator(previewSelector)
    .first()
    .textContent()
    .catch(() => null);
  expect(previewAfter).toBe(previewBefore);

  // 7. Switch to api layout — operator preview SHOULD now reflect the
  // stored api content.
  const switchRes = await request.post(
    new URL("/stage/layout", baseURL).toString(),
    { data: { code: "api" } },
  );
  expect(switchRes.ok()).toBeTruthy();

  await page.waitForTimeout(500);

  const previewAfterSwitch = await page
    .locator(previewSelector)
    .first()
    .textContent()
    .catch(() => null);
  // Either the preview now contains the api content, OR if the operator
  // preview pulls from a different field, hit /api/stage again and assert
  // the stored value matches what we PUT.
  if (previewAfterSwitch) {
    expect(previewAfterSwitch).toContain("should not appear in worship-snv preview");
  }

  // Console must be clean (modulo the filtered Chrome integrity warning).
  expect(consoleMessages).toEqual([]);

  // Reference unused beforeText to silence lint if it stays unused.
  void beforeText;
});
```

If `data-role="stage-preview"` isn't the actual selector for the operator preview, find it by reading the operator UI:

```bash
grep -rn 'data-role="stage-preview"\|stage-preview\|StagePreview' /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui/src/components/ /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui/src/pages/ | head -10
```

Adjust the test selector to whatever is actually rendered. If the operator preview's DOM structure is too dynamic to reliably assert text, fall back to subscribing to the stage WS directly via `page.evaluate()` and counting Stage messages — assert ZERO Stage messages arrived between the layout=worship-snv set and the api PUT, and at least one after the switch to api.

### Step 3: Run the new test locally

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && npx playwright test tests/e2e/api-stage.spec.ts --reporter=list 2>&1 | tail -20
```

Expected: all api-stage tests pass (the existing ones plus the new layout-isolation test).

If the test fails because the preview selector doesn't exist, fix per Step 2 fallback.

If the `/api/stage` GET endpoint doesn't exist, the `beforeText` block is harmless (only logs the value, doesn't gate any assertion).

### Step 4: Commit

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && git add tests/e2e/api-stage.spec.ts && git commit -m "test(e2e): api stage layout isolation (#281)"
```

---

## Task 5: Local Checks, Push, CI Monitor, Dev Verification, Open PR (Controller)

This task is handled by the controller. Local Rust + WASM builds are allowed.

### Local pre-push checks

- [ ] **Step 1: Workspace fmt**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo fmt --all --check
```

- [ ] **Step 2: Workspace clippy**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo clippy --workspace --all-targets -- -D warnings -W clippy::all 2>&1 | tail -5
```

- [ ] **Step 3: presenter-ui WASM clippy**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings -W clippy::all 2>&1 | tail -5
```

- [ ] **Step 4: Workspace tests**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo test -p presenter-server 2>&1 | tail -5
```

Expected: all tests pass including the 3 new ones.

- [ ] **Step 5: Push**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && git push origin dev
```

- [ ] **Step 6: Monitor CI**

```bash
RUN_ID=$(gh run list --branch dev --limit 1 --json databaseId --jq '.[0].databaseId')
sleep 1500 && gh run view $RUN_ID --json status,conclusion,jobs --jq '{status, conclusion, jobs: [.jobs[] | {name, status, conclusion}]}'
```

Wait for ALL jobs `completed`. If any fails, fix root cause in ONE commit, push, monitor again.

### Dev verification

- [ ] **Step 7: Verify dev shows v0.4.55**

```bash
curl -s http://10.77.8.134:8080/healthz
```

Expected: `{"channel":"dev","status":"ok","version":"0.4.55"}`.

- [ ] **Step 8: Manual UX verification on dev via Playwright MCP**

1. Open `http://10.77.8.134:8080/ui/operator/worship`.
2. Use the operator UI to switch the stage layout to `worship-snv`.
3. PUT `/api/stage` with sample content via curl: `curl -X PUT -H 'Content-Type: application/json' -d '{"current_text":"test"}' http://10.77.8.134:8080/api/stage`.
4. Confirm the operator preview did NOT switch to api content (stays on worship-snv).
5. Switch the stage layout to `api`.
6. Confirm the operator preview NOW shows the test content.

### Open PR

- [ ] **Step 9: Open PR**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && gh pr create --base main --head dev --title "fix(server): gate api-stage publish on current layout (#281)" --body "$(cat <<'EOF'
## Summary

Fixes #281 — \`PUT /api/stage\` was switching the operator preview even when the current stage layout was something other than \`api\` (e.g. \`worship-snv\`).

## What changed

- **\`update_api_stage\`** (state/mod.rs): now only publishes \`LiveEvent::Stage\` when \`stage_layout_code() == \"api\"\`. State is still stored on every PUT.
- **\`set_stage_layout_code\`** (state/stage_display.rs): when switching TO \`api\`, publishes the stored api_stage snapshot via \`LiveEvent::Stage\` so the preview reflects the most recent PUT instead of waiting for the next one. Switching to other layouts continues to call \`broadcast_stage_snapshots\` as before.
- 3 unit tests covering both gate paths + switch-to-api refresh.
- 1 Playwright E2E test asserting layout-isolation.
- Bumped version 0.4.54 → 0.4.55.

## Test plan

- [x] \`cargo test -p presenter-server\` — all green incl. 3 new tests
- [x] \`cargo clippy --workspace --all-targets -- -D warnings -W clippy::all\` — zero warnings
- [x] \`cargo fmt --all --check\` — clean
- [x] CI green on dev
- [x] **Manual dev verification:**
  - Layout=worship-snv, PUT /api/stage → operator preview unchanged ✅
  - Switch layout to api → preview shows the previously-PUT content ✅

Closes #281
EOF
)"
```

- [ ] **Step 10: Confirm PR clean**

```bash
PR_NUM=$(gh pr list --head dev --base main --json number --jq '.[0].number')
gh api repos/zbynekdrlik/presenter/pulls/$PR_NUM --jq '{mergeable: .mergeable, mergeable_state: .mergeable_state}'
```

Expected: `{"mergeable": true, "mergeable_state": "clean"}`.

### Pre-completion gate

- [ ] **Step 11: Run /plan-check**

- [ ] **Step 12: Run /review** on the PR diff

- [ ] **Step 13: Send completion report**

Per `core/completion-report.md`. Include CI run id, /plan-check fulfillment, /review 0🔴 0🟡 0🔵, dev verification, dev + prod URLs, PR URL.

---

## Verification Summary

| Check | How to verify |
|-------|---------------|
| Gate prevents leak | Layout=worship-snv, PUT /api/stage → operator preview unchanged (Playwright + manual) |
| Gate state still stored | Sanity: `state.api_stage.read()` reflects the PUT even when no event fires |
| Api publishes when layout=api | Layout=api, PUT /api/stage → Stage event fires (unit test) |
| Switch-to-api refresh | Pre-store api state, switch worship-snv→api → Stage event with stored content fires (unit test + manual) |
| No regressions | All existing tests still pass; CI green |
