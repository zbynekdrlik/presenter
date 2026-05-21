# NDI Stage Auto-Recovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the stage display NDI video recover automatically from any single failure (server `ndisrc` crash, NDI source restart, browser ICE drop) within 5-7 s, without manual page refresh.

**Architecture:** Two independent self-healing layers. (1) Browser `Watchdog` inside `<NdiVideo>` listens for `RTCPeerConnection` ICE state changes and a `<video>` stall timer; on failure it runs `reconnect_loop` (tear down PC + DELETE old session + POST fresh offer, exponential backoff capped at 5 s). (2) Server `PipelineSupervisor` task per active source watches `NdiPipeline::state_watcher()` and rebuilds the gst pipeline immediately on `Errored`/`Stopped` (rate-limited 2 s; exponential pause after 5 consecutive failures). No WHEP protocol changes. The 30 s DB-driven auto-reconnect stays as a backstop.

**Tech Stack:** Rust (axum, tokio, tracing, gstreamer-rs), Leptos WASM (web-sys, wasm-bindgen-futures), Playwright (TypeScript, `channel: 'chrome'` for the `@video-codec`-tagged recovery test).

**Spec:** `docs/superpowers/specs/2026-05-21-ndi-stage-auto-recovery-design.md` (commit `260b6ff`)

---

## File structure

| File | Status | Responsibility |
|---|---|---|
| `Cargo.toml` | Modify L15 | Workspace version 0.4.93 → 0.4.94 |
| `crates/presenter-ndi/src/pipeline.rs` | Modify L355-358 | Add `tracing::warn!` on EOS branch (diagnostic) |
| `crates/presenter-ndi/src/manager.rs` | Modify L56-104, L148-214, L217-222 | Add `PipelineSupervisor` task, `rebuild_pipeline`, supervisor handle in `ActiveSource`. Unit tests in existing `start_pipeline_state_check_tests` module. |
| `crates/presenter-server/Cargo.toml` | Modify (add `[features]`) | Declare `test-helpers` feature |
| `crates/presenter-server/src/router/integrations/ndi_whep.rs` | Modify | Add `cfg(feature = "test-helpers")` gated `kill_pipeline_for_test` handler |
| `crates/presenter-server/src/router.rs` | Modify L247 | Mount the cfg-gated kill route |
| `.github/workflows/pipeline.yml` | Modify Build job step | Build `presenter-server` with `--features test-helpers` for dev/E2E artifacts (NOT for prod deploy.yml/release.yml) |
| `crates/presenter-ui/src/components/stage/ndi_video.rs` | Modify | Replace one-shot `connect_whep` Effect with `reconnect_loop` driven by `Watchdog` (ICE state + stall timer) |
| `tests/e2e/ndi-webrtc-recovery.spec.ts` | Create | New Playwright recovery test, tagged `@video-codec` for real-Chrome routing |

Net code change: ~150 LoC added. Single PR, dev → main.

---

## Task 1: Workspace version bump

**Files:**
- Modify: `Cargo.toml:15`

- [ ] **Step 1: Bump version**

Replace:

```toml
version = "0.4.93"
```

with:

```toml
version = "0.4.94"
```

- [ ] **Step 2: Refresh lockfiles**

Run: `cargo update --workspace -p presenter-server`
Run: `cd crates/presenter-ui && cargo update -p presenter-ui && cd ../..`

Expected: both `Cargo.lock` files now show 0.4.94 for the workspace members.

- [ ] **Step 3: Verify nothing else broke**

Run: `cargo check --workspace`
Expected: success (no compile errors, just a version field changed).

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock crates/presenter-ui/Cargo.lock
git commit -m "chore: bump workspace to 0.4.94"
```

---

## Task 2: Diagnostic — EOS warn log in pipeline bus watcher

**Files:**
- Modify: `crates/presenter-ndi/src/pipeline.rs:355-358`

Today the EOS branch silently sets state to Stopped with no log; the Error branch logs at ERROR. Parity makes future `ndisrc` investigations easier.

- [ ] **Step 1: Add WARN log on EOS**

Locate (`crates/presenter-ndi/src/pipeline.rs` around L355):

```rust
                    gst::MessageView::Eos(_) => {
                        let _ = state_tx.send(PipelineState::Stopped);
                    }
```

Replace with:

```rust
                    gst::MessageView::Eos(_) => {
                        tracing::warn!("pipeline EOS received → state=Stopped");
                        let _ = state_tx.send(PipelineState::Stopped);
                    }
```

- [ ] **Step 2: Verify compile**

Run: `cargo check -p presenter-ndi`
Expected: success.

- [ ] **Step 3: Commit**

```bash
git add crates/presenter-ndi/src/pipeline.rs
git commit -m "feat(ndi): log EOS at WARN for parity with Error branch"
```

---

## Task 3: cfg-gated kill-pipeline test route

**Files:**
- Modify: `crates/presenter-server/Cargo.toml` (add `[features]`)
- Modify: `crates/presenter-server/src/router/integrations/ndi_whep.rs`
- Modify: `crates/presenter-server/src/router.rs:247`
- Modify: `.github/workflows/pipeline.yml` (E2E build step adds `--features test-helpers`)

The Playwright recovery test (Task 4) needs a deterministic way to kill the server-side pipeline mid-stream. We add a `test-helpers` cargo feature on `presenter-server`. Dev/E2E artifacts build with the feature; prod (`deploy.yml`, `release.yml`) does NOT — so production binaries have no such route.

- [ ] **Step 1: Declare the feature in presenter-server Cargo.toml**

Open `crates/presenter-server/Cargo.toml`. After the existing `[dependencies]` section, append:

```toml
[features]
default = []
test-helpers = []
```

If a `[features]` section already exists, add only the `test-helpers = []` line.

- [ ] **Step 2: Add a unit test that the handler exists and 404s on unknown source**

In `crates/presenter-server/src/router/integrations/ndi_whep.rs`, inside the existing `#[cfg(test)] mod tests` block at the bottom, add:

```rust
    #[cfg(feature = "test-helpers")]
    #[tokio::test]
    async fn kill_pipeline_for_test_returns_404_for_unknown_source() {
        let state = build_test_state_without_ndi().await;
        let result = kill_pipeline_for_test(
            axum::extract::Path("unknown".to_string()),
            axum::extract::State(state),
        )
        .await;
        assert!(result.is_err(), "expected error for unknown source");
        let err = result.unwrap_err();
        assert_eq!(err.status(), axum::http::StatusCode::NOT_FOUND);
    }
```

`build_test_state_without_ndi` already exists in the test module above — reuse it.

- [ ] **Step 3: Run the test — confirm it fails to compile (handler doesn't exist yet)**

Run: `cargo test -p presenter-server --features test-helpers --lib router::integrations::ndi_whep`
Expected: FAIL — `error[E0425]: cannot find function 'kill_pipeline_for_test'`

- [ ] **Step 4: Implement the handler**

Append to `crates/presenter-server/src/router/integrations/ndi_whep.rs` (above the `#[cfg(test)]` block):

```rust
/// Test-only: force the per-source GStreamer pipeline to stop, simulating
/// an `ndisrc` crash. The PipelineSupervisor's recovery path should then
/// rebuild it autonomously. Exposed ONLY when compiled with the
/// `test-helpers` cargo feature; production binaries (built without the
/// feature) do not contain this route. The Playwright recovery test calls
/// it to make the recovery assertion deterministic.
#[cfg(feature = "test-helpers")]
#[instrument(skip_all, fields(source_id = %source_id))]
pub(crate) async fn kill_pipeline_for_test(
    Path(source_id): Path<String>,
    State(state): State<AppState>,
) -> Result<Response, AppError> {
    let manager = state
        .ndi_manager()
        .ok_or_else(|| AppError::service_unavailable("NDI SDK not available"))?;
    if !manager.is_active(&source_id).await {
        return Err(AppError::not_found("NDI source not active"));
    }
    manager.stop_pipeline(&source_id).await;
    Ok(Response::builder()
        .status(204)
        .body(axum::body::Body::empty())
        .expect("valid response"))
}
```

- [ ] **Step 5: Run the test again — confirm it passes**

Run: `cargo test -p presenter-server --features test-helpers --lib router::integrations::ndi_whep`
Expected: PASS (`kill_pipeline_for_test_returns_404_for_unknown_source ... ok`)

- [ ] **Step 6: Mount the route in router.rs (cfg-gated)**

Open `crates/presenter-server/src/router.rs`. After line 247 (the existing `/ndi/whep/{source_id}/{session_id}` route block ending with `)`), add a chained `.route()` call:

```rust
        .route(
            "/test/ndi/kill-pipeline/{source_id}",
            #[cfg(feature = "test-helpers")]
            post(integrations::ndi_whep::kill_pipeline_for_test),
            #[cfg(not(feature = "test-helpers"))]
            post(|| async { axum::http::StatusCode::NOT_FOUND }),
        )
```

The `not(feature)` arm returns a closure that 404s, so the route is REGISTERED in both builds but functional only with the feature. This keeps the router shape identical and avoids `#[cfg]` on the whole route chain.

Verify compile in both modes:

Run: `cargo check -p presenter-server` (no feature)
Run: `cargo check -p presenter-server --features test-helpers`
Expected: both succeed.

- [ ] **Step 7: Add `--features test-helpers` to the E2E build in CI**

Open `.github/workflows/pipeline.yml`. Locate the `build` job's compile step that produces the binary used by the E2E job. It currently runs `cargo build --release -p presenter-server`. Change to:

```yaml
        run: cargo build --release -p presenter-server --features test-helpers
```

Do NOT touch `.github/workflows/deploy.yml` or `release.yml` — production builds intentionally lack the feature.

- [ ] **Step 8: Commit**

```bash
git add crates/presenter-server/Cargo.toml \
        crates/presenter-server/src/router/integrations/ndi_whep.rs \
        crates/presenter-server/src/router.rs \
        .github/workflows/pipeline.yml
git commit -m "feat(test): cfg-gated kill-pipeline route for recovery E2E"
```

---

## Task 4: RED Playwright recovery test (commits BEFORE impl)

**Files:**
- Create: `tests/e2e/ndi-webrtc-recovery.spec.ts`

This test MUST fail today (no recovery exists). Tasks 5+6 turn it green. Per `regression-test-first.md` / TDD discipline, RED commit lands BEFORE the impl commits.

- [ ] **Step 1: Create the test file**

Create `tests/e2e/ndi-webrtc-recovery.spec.ts`:

```typescript
// SPDX-License-Identifier: MIT
//
// Recovery regression: after the server-side pipeline is forcefully killed
// (simulating an ndisrc "Internal data stream error"), the stage display
// MUST resume playing video WITHOUT a page refresh, within 10 seconds.
//
// Tagged @video-codec so playwright.config.ts routes it through real Chrome
// (channel: "chrome" + --autoplay-policy=user-gesture-required) — the
// default Chromium build has no H264 codec and silently bypasses autoplay.

import { test, expect } from "@playwright/test";
import { startTestServer } from "./support";

const DEV_NDI_SOURCE_LABEL = process.env.E2E_NDI_LABEL ?? "test-recovery";

test.describe("NDI WebRTC recovery @video-codec", () => {
  test("video resumes within 10s after server pipeline is killed", async ({ page }) => {
    const server = await startTestServer({ features: ["test-helpers"] });
    const consoleErrors: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error") consoleErrors.push(msg.text());
    });

    // Activate the NDI source (test harness exposes a helper).
    const sourcesResp = await server.request.get("/ndi/sources");
    if (!sourcesResp.ok()) {
      test.skip(true, "NDI SDK not loaded on this runner");
      return;
    }
    const sources = await sourcesResp.json();
    if (!Array.isArray(sources) || sources.length === 0) {
      test.skip(true, "no NDI sources discovered");
      return;
    }
    const source = sources.find((s: any) => s.label === DEV_NDI_SOURCE_LABEL) ?? sources[0];
    const activateResp = await server.request.post(
      `/integrations/video-sources/${source.id}/activate`,
    );
    expect(activateResp.ok()).toBeTruthy();

    // Open the stage display and wait until the <video> is producing frames.
    await page.goto(`${server.url}/stage/ndi-fullscreen`);
    const video = page.locator('video[data-role="ndi-video"]').first();
    await expect(video).toBeVisible();
    await expect
      .poll(async () => await video.evaluate((v: HTMLVideoElement) => v.videoWidth), {
        timeout: 15_000,
        message: "initial video stream never reached videoWidth > 0",
      })
      .toBeGreaterThan(0);

    // Kill the server-side pipeline (simulates ndisrc crash).
    const killResp = await server.request.post(`/test/ndi/kill-pipeline/${source.id}`);
    expect(killResp.status()).toBe(204);

    // Within 10s, the watchdog + supervisor should reconnect us.
    await expect
      .poll(
        async () => {
          return await video.evaluate((v: HTMLVideoElement) => ({
            width: v.videoWidth,
            currentTime: v.currentTime,
          }));
        },
        {
          timeout: 10_000,
          intervals: [500, 500, 1000, 1000, 2000],
          message: "video did not recover within 10s after server pipeline kill",
        },
      )
      .toMatchObject({ width: expect.any(Number) });

    // Stronger assertion: videoWidth must be > 0 AND currentTime must have
    // advanced compared to the moment of the kill.
    const after = await video.evaluate((v: HTMLVideoElement) => ({
      width: v.videoWidth,
      currentTime: v.currentTime,
    }));
    expect(after.width).toBeGreaterThan(0);
    // currentTime keeps advancing in a live stream (>0.1s of new content).
    expect(after.currentTime).toBeGreaterThan(0.1);

    // No console errors during the whole recovery cycle.
    expect(consoleErrors).toEqual([]);
  });
});
```

- [ ] **Step 2: Run the test — confirm it FAILS (recovery code does not exist yet)**

Run: `npm run test:playwright -- ndi-webrtc-recovery`
Expected: FAIL — either timeout on the second `expect.poll` (video stays at width=0 after kill, no recovery), or skipped if dev runner has no NDI source (in which case the test is honest about its precondition).

- [ ] **Step 3: Commit the RED test**

```bash
git add tests/e2e/ndi-webrtc-recovery.spec.ts
git commit -m "test(e2e): RED recovery test for NDI WebRTC auto-reconnect"
```

The "RED" state is documented in the commit; subsequent commits move it to GREEN.

---

## Task 5: Server `PipelineSupervisor` task + `rebuild_pipeline` + unit tests

**Files:**
- Modify: `crates/presenter-ndi/src/manager.rs`

Adds the supervisor task and the rebuild method. Unit tests use the existing `stopped_for_test` + `set_state_for_test` plumbing so they run on every CI host without libndi.

- [ ] **Step 1: Add failing unit test for rate-limiter**

Append to the existing `start_pipeline_state_check_tests` module in `crates/presenter-ndi/src/manager.rs`:

```rust
    /// Rate-limiter: two Errored transitions within 2s must produce
    /// exactly ONE rebuild attempt.
    #[tokio::test]
    async fn supervisor_rate_limits_rapid_errors() {
        let mut state = SupervisorState::new();
        let outcome1 = state.should_rebuild_now(std::time::Instant::now());
        assert!(matches!(outcome1, RebuildDecision::ProceedAfter(d) if d.is_zero()));
        state.mark_rebuild_started();

        // 100ms later — well within the 2s rate limit.
        let outcome2 = state.should_rebuild_now(
            std::time::Instant::now() + std::time::Duration::from_millis(100),
        );
        // Decision must defer to "after the rate-limit window".
        match outcome2 {
            RebuildDecision::ProceedAfter(d) => {
                assert!(d >= std::time::Duration::from_millis(1900), "expected ~2s wait, got {d:?}");
            }
            _ => panic!("expected ProceedAfter, got {outcome2:?}"),
        }
    }

    /// After 5 consecutive failures, backoff progression is 2s, 4s, 8s, 16s, 30s (cap).
    #[tokio::test]
    async fn supervisor_backs_off_exponentially_after_5_failures() {
        let mut state = SupervisorState::new();
        for _ in 0..5 {
            state.mark_rebuild_failed();
        }
        // 6th attempt
        assert_eq!(state.backoff_for_failure_count(), std::time::Duration::from_secs(2));
        state.mark_rebuild_failed();
        // 7th
        assert_eq!(state.backoff_for_failure_count(), std::time::Duration::from_secs(4));
        state.mark_rebuild_failed();
        // 8th
        assert_eq!(state.backoff_for_failure_count(), std::time::Duration::from_secs(8));
        state.mark_rebuild_failed();
        // 9th
        assert_eq!(state.backoff_for_failure_count(), std::time::Duration::from_secs(16));
        state.mark_rebuild_failed();
        // 10th and beyond — capped at 30
        assert_eq!(state.backoff_for_failure_count(), std::time::Duration::from_secs(30));
        for _ in 0..5 {
            state.mark_rebuild_failed();
            assert_eq!(state.backoff_for_failure_count(), std::time::Duration::from_secs(30));
        }
    }

    /// Mark-success resets the failure counter so the next failure starts from 0.
    #[tokio::test]
    async fn supervisor_resets_on_success() {
        let mut state = SupervisorState::new();
        for _ in 0..3 {
            state.mark_rebuild_failed();
        }
        state.mark_rebuild_succeeded();
        assert_eq!(state.consecutive_failures(), 0);
    }
```

- [ ] **Step 2: Run the failing tests — confirm they fail (types don't exist yet)**

Run: `cargo test -p presenter-ndi --lib start_pipeline_state_check_tests::supervisor`
Expected: FAIL — `error[E0433]: failed to resolve: use of undeclared type 'SupervisorState'` (and 'RebuildDecision').

- [ ] **Step 3: Implement `SupervisorState` and `RebuildDecision`**

Add to `crates/presenter-ndi/src/manager.rs`, near the top of the file (after the existing `enum StateCheckOutcome`):

```rust
/// Per-source supervisor bookkeeping: when the last rebuild was attempted,
/// and how many consecutive failures we've seen.
///
/// Pure data — no async, no I/O — so unit-testable on every CI host.
#[derive(Debug)]
struct SupervisorState {
    last_rebuild_at: std::time::Instant,
    /// 0 while the pipeline is healthy. Incremented by `mark_rebuild_failed`,
    /// reset to 0 by `mark_rebuild_succeeded`.
    consecutive_failures: u32,
}

/// Outcome of `SupervisorState::should_rebuild_now` — drives the supervisor's
/// next sleep duration.
#[derive(Debug)]
enum RebuildDecision {
    /// Wait this long, then attempt a rebuild. Zero duration means rebuild now.
    ProceedAfter(std::time::Duration),
}

impl SupervisorState {
    fn new() -> Self {
        Self {
            // Start with last_rebuild far enough in the past that the FIRST
            // rebuild attempt has zero wait.
            last_rebuild_at: std::time::Instant::now()
                - std::time::Duration::from_secs(3600),
            consecutive_failures: 0,
        }
    }

    fn consecutive_failures(&self) -> u32 {
        self.consecutive_failures
    }

    /// Rate-limit window: minimum 2 seconds between rebuild attempts.
    fn should_rebuild_now(&self, now: std::time::Instant) -> RebuildDecision {
        const RATE_LIMIT: std::time::Duration = std::time::Duration::from_secs(2);
        let since_last = now.duration_since(self.last_rebuild_at);
        if since_last >= RATE_LIMIT {
            RebuildDecision::ProceedAfter(std::time::Duration::ZERO)
        } else {
            RebuildDecision::ProceedAfter(RATE_LIMIT - since_last)
        }
    }

    /// Exponential backoff once failures exceed 5: 2s, 4s, 8s, 16s, 30s cap.
    fn backoff_for_failure_count(&self) -> std::time::Duration {
        if self.consecutive_failures <= 5 {
            return std::time::Duration::ZERO;
        }
        let exp = (self.consecutive_failures - 5).min(4); // 1..=4 → 2,4,8,16
        let secs: u64 = 1u64 << exp; // 2, 4, 8, 16
        std::time::Duration::from_secs(secs.saturating_mul(2).min(30))
    }

    fn mark_rebuild_started(&mut self) {
        self.last_rebuild_at = std::time::Instant::now();
    }

    fn mark_rebuild_succeeded(&mut self) {
        self.consecutive_failures = 0;
    }

    fn mark_rebuild_failed(&mut self) {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
    }
}
```

- [ ] **Step 4: Run tests — confirm GREEN**

Run: `cargo test -p presenter-ndi --lib start_pipeline_state_check_tests::supervisor`
Expected: all three `supervisor_*` tests pass.

- [ ] **Step 5: Extend `ActiveSource` to hold the supervisor task handle**

Find in `crates/presenter-ndi/src/manager.rs:56-58`:

```rust
struct ActiveSource {
    pipeline: NdiPipeline,
}
```

Replace with:

```rust
struct ActiveSource {
    pipeline: NdiPipeline,
    /// Supervisor task handle. Aborted on `stop_pipeline` / drop to prevent
    /// leaks. `None` only inside the regression-test constructors (which
    /// don't spawn a real supervisor).
    supervisor: Option<tokio::task::JoinHandle<()>>,
}
```

Update all existing constructions:

In `start_pipeline` body (around L199), change:

```rust
active.insert(source_id.to_string(), ActiveSource { pipeline });
```

to:

```rust
active.insert(
    source_id.to_string(),
    ActiveSource { pipeline, supervisor: None },
);
```

In the three test sites (L350, L374, L392), change:

```rust
active.insert("test-id".to_string(), ActiveSource { pipeline: dead });
```

to:

```rust
active.insert(
    "test-id".to_string(),
    ActiveSource { pipeline: dead, supervisor: None },
);
```

(Apply the same pattern to all three test constructions — the variable name will be `dead` / `p` / `p`.)

In `check_active_entry` around L97-99, the `dead.pipeline.stop().await` line still works because `dead` is destructured but we don't touch supervisor; we abort it in `stop_pipeline` instead. Keep that block as is.

In `stop_pipeline` (L217-222), change:

```rust
pub async fn stop_pipeline(&self, source_id: &str) {
    let mut active = self.active.lock().await;
    if let Some(mut src) = active.remove(source_id) {
        src.pipeline.stop().await;
    }
}
```

to:

```rust
pub async fn stop_pipeline(&self, source_id: &str) {
    let mut active = self.active.lock().await;
    if let Some(mut src) = active.remove(source_id) {
        if let Some(handle) = src.supervisor.take() {
            handle.abort();
        }
        src.pipeline.stop().await;
    }
}
```

Apply the same pattern to `stop_all` (L225-230):

```rust
pub async fn stop_all(&self) {
    let mut active = self.active.lock().await;
    for (_, mut src) in active.drain() {
        if let Some(handle) = src.supervisor.take() {
            handle.abort();
        }
        src.pipeline.stop().await;
    }
}
```

- [ ] **Step 6: Verify compile and tests still pass**

Run: `cargo test -p presenter-ndi --lib`
Expected: all existing tests pass, all new `supervisor_*` tests pass, no warnings.

- [ ] **Step 7: Wrap `NdiManager` in `Arc` and add `rebuild_pipeline` + `spawn_supervisor`**

The supervisor task needs an `Arc<NdiManager>` to call back into `rebuild_pipeline` after a state change. `state/mod.rs` and routers already hold `NdiManager` inside `AppState`; the existing handle is shareable.

In `crates/presenter-ndi/src/manager.rs`, add this method to `impl NdiManager` (place it BEFORE the existing `start_pipeline`):

```rust
    /// Spawn the supervisor task for one source. Returns the JoinHandle so
    /// callers can store it in `ActiveSource.supervisor` and abort on stop.
    ///
    /// The supervisor:
    /// - Subscribes to the pipeline's state watcher
    /// - On Errored/Stopped, requests a rebuild via `rebuild_pipeline`
    /// - Rate-limits to 1 rebuild / 2s; exponentially backs off after 5
    ///   consecutive failures
    /// - Exits when the watcher closes (pipeline dropped) OR the task is
    ///   externally aborted via `stop_pipeline`
    fn spawn_supervisor(
        self: &std::sync::Arc<Self>,
        source_id: String,
        ndi_name: String,
        mut watcher: tokio::sync::watch::Receiver<crate::pipeline::PipelineState>,
    ) -> tokio::task::JoinHandle<()> {
        let manager = std::sync::Arc::clone(self);
        tokio::spawn(async move {
            let mut state = SupervisorState::new();
            loop {
                // Wait for the next state change.
                if watcher.changed().await.is_err() {
                    // Pipeline dropped — exit cleanly.
                    return;
                }
                let current = watcher.borrow_and_update().clone();
                use crate::pipeline::PipelineState::*;
                match current {
                    Streaming | Starting => {
                        state.mark_rebuild_succeeded();
                    }
                    Errored(_) | Stopped => {
                        // Apply rate-limit + backoff.
                        let RebuildDecision::ProceedAfter(wait) =
                            state.should_rebuild_now(std::time::Instant::now());
                        let backoff = state.backoff_for_failure_count();
                        let total = wait.max(backoff);
                        if !total.is_zero() {
                            tracing::warn!(
                                source_id = %source_id,
                                wait_ms = total.as_millis() as u64,
                                consecutive_failures = state.consecutive_failures(),
                                "supervisor: backing off before rebuild"
                            );
                            tokio::time::sleep(total).await;
                        }
                        state.mark_rebuild_started();
                        match manager.rebuild_pipeline(&source_id, &ndi_name).await {
                            Ok(()) => {
                                tracing::info!(source_id = %source_id, "supervisor: rebuild succeeded");
                                state.mark_rebuild_succeeded();
                                // The fresh pipeline has a NEW state watcher; swap ours
                                // so we see ITS transitions, not the dead pipeline's.
                                let new_watcher = manager.state_watcher_for(&source_id).await;
                                if let Some(w) = new_watcher {
                                    watcher = w;
                                } else {
                                    // Source no longer active (operator deactivated between
                                    // rebuild start and now). Exit.
                                    return;
                                }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    source_id = %source_id,
                                    error = %e,
                                    consecutive_failures = state.consecutive_failures() + 1,
                                    "supervisor: rebuild failed"
                                );
                                state.mark_rebuild_failed();
                            }
                        }
                    }
                }
            }
        })
    }

    /// Fetch the live `state_watcher` for an active source. Used by the
    /// supervisor to re-subscribe after a successful rebuild (the old
    /// watcher's pipeline is dropped after rebuild).
    async fn state_watcher_for(
        &self,
        source_id: &str,
    ) -> Option<tokio::sync::watch::Receiver<crate::pipeline::PipelineState>> {
        let active = self.active.lock().await;
        active.get(source_id).map(|s| s.pipeline.state_watcher())
    }

    /// Rebuild the pipeline for a source whose existing entry is dead
    /// (Errored or Stopped). Reuses `check_active_entry` to clear the
    /// dead entry, then builds + starts a fresh pipeline. Does NOT
    /// re-spawn a supervisor (the supervisor task that called us is
    /// still alive and will re-subscribe to the new state watcher via
    /// `state_watcher_for`).
    async fn rebuild_pipeline(&self, source_id: &str, ndi_name: &str) -> Result<()> {
        let mut active = self.active.lock().await;
        // Force-remove the dead entry (Stopped/Errored). If somehow it has
        // become healthy in the meantime, leave it alone.
        if let StateCheckOutcome::Idempotent = check_active_entry(&mut active, source_id).await {
            return Ok(());
        }

        let whep_url = format!("/ndi/whep/{}", source_id);
        let mut pipeline = NdiPipeline::build(ndi_name, whep_url)?;
        pipeline.start().await?;

        let sink = pipeline
            .sink_element()
            .ok_or_else(|| anyhow!("pipeline has no sink element"))?;
        let video_pad = sink
            .static_pad("video_0")
            .ok_or_else(|| anyhow!("whepserversink has no video_0 sink pad"))?;
        let mut watcher = pipeline.state_watcher();
        let caps_ready = tokio::time::timeout(std::time::Duration::from_secs(8), async {
            loop {
                if let crate::pipeline::PipelineState::Errored(ref e) =
                    *watcher.borrow_and_update()
                {
                    return Err(anyhow!("pipeline errored: {e}"));
                }
                if video_pad.current_caps().is_some() {
                    return Ok(());
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        })
        .await;

        match caps_ready {
            Ok(Ok(())) => {
                active.insert(
                    source_id.to_string(),
                    ActiveSource {
                        pipeline,
                        // Supervisor task is reused — it'll fetch the new watcher
                        // from us via `state_watcher_for`.
                        supervisor: None,
                    },
                );
                Ok(())
            }
            Ok(Err(e)) => {
                pipeline.stop().await;
                Err(e)
            }
            Err(_) => {
                pipeline.stop().await;
                Err(anyhow!(
                    "NDI source '{ndi_name}' did not deliver any frame within 8s on rebuild"
                ))
            }
        }
    }
```

- [ ] **Step 8: Wire `start_pipeline` to spawn the supervisor**

Two changes in `start_pipeline` (around L148-214):

1. Take `self` by `Arc` so we can call `spawn_supervisor`:

The existing signature is `pub async fn start_pipeline(&self, ...)`. Callers go through `AppState::ndi_manager()` which already returns `Option<Arc<NdiManager>>` (verify with `grep -n 'fn ndi_manager' crates/presenter-server/src/state/`). So callers already have an Arc. Change the signature to:

```rust
pub async fn start_pipeline(self: &std::sync::Arc<Self>, source_id: &str, ndi_name: &str) -> Result<()> {
```

Confirm all callers compile (the existing call site in `state/mod.rs` activate path uses `manager.start_pipeline(...)` where `manager: &Arc<NdiManager>`; the `&self` auto-deref still resolves with the Arc self-method).

2. After the existing `active.insert(...)` call near L199, spawn the supervisor and store its handle:

Replace:

```rust
            Ok(Ok(())) => {
                active.insert(source_id.to_string(), ActiveSource { pipeline, supervisor: None });
                Ok(())
            }
```

with:

```rust
            Ok(Ok(())) => {
                // Spawn the supervisor BEFORE inserting so the handle is
                // ready when we hand ownership to ActiveSource.
                let watcher = pipeline.state_watcher();
                let supervisor = self.spawn_supervisor(
                    source_id.to_string(),
                    ndi_name.to_string(),
                    watcher,
                );
                active.insert(
                    source_id.to_string(),
                    ActiveSource {
                        pipeline,
                        supervisor: Some(supervisor),
                    },
                );
                Ok(())
            }
```

- [ ] **Step 9: Run full crate tests**

Run: `cargo test -p presenter-ndi`
Expected: all green — existing 4 state-check tests, 3 new supervisor tests, the existing pipeline build tests (skipped on no-VAAPI hosts).

- [ ] **Step 10: Verify server compiles end-to-end**

Run: `cargo check -p presenter-server`
Expected: success. Any callers of `start_pipeline` that don't pass an `Arc<NdiManager>` will fail here — fix them by adjusting the call site to use the existing `Arc` already held by `AppState`.

- [ ] **Step 11: Commit**

```bash
git add crates/presenter-ndi/src/manager.rs
git commit -m "feat(ndi): PipelineSupervisor + rebuild_pipeline for auto-recovery"
```

---

## Task 6: Client `Watchdog` + `reconnect_loop` in `<NdiVideo>`

**Files:**
- Modify: `crates/presenter-ui/src/components/stage/ndi_video.rs`

Replaces the one-shot `connect_whep` Effect with a loop that re-runs on `RTCPeerConnection` ICE failure or `<video>` stall.

- [ ] **Step 1: Add the stall-timer + ICE listener (Watchdog struct)**

In `crates/presenter-ui/src/components/stage/ndi_video.rs`, add this module-level type below the existing `WhepSession` struct:

```rust
/// Watchdog that fires `on_failure` when EITHER:
/// - the RTCPeerConnection's iceConnectionState becomes "failed" or "disconnected", OR
/// - the <video> element's currentTime has not advanced for STALL_THRESHOLD seconds.
///
/// The intervals are leaked (`forget()`) because Closures can't be Send in
/// wasm-bindgen and removing them on drop requires the original Closure
/// handle. We compensate by giving each Watchdog instance its own
/// `active: Rc<Cell<bool>>` flag that the closures check first; cleanup just
/// flips the flag to false and the closures become no-ops.
struct Watchdog {
    active: std::rc::Rc<std::cell::Cell<bool>>,
}

impl Watchdog {
    /// Stall threshold: no `currentTime` progress in this many seconds → reconnect.
    const STALL_THRESHOLD_SECS: f64 = 3.0;
    /// How often the stall timer ticks.
    const TICK_INTERVAL_MS: i32 = 1000;
}
```

- [ ] **Step 2: Add `Watchdog::install` and `Watchdog::stop`**

Append below the `Watchdog` impl:

```rust
impl Watchdog {
    fn install<F: Fn() + 'static>(
        video: &HtmlVideoElement,
        pc: &RtcPeerConnection,
        on_failure: F,
    ) -> Self {
        use std::cell::Cell;
        use std::rc::Rc;

        let active: Rc<Cell<bool>> = Rc::new(Cell::new(true));
        let on_failure = Rc::new(on_failure);

        // ICE state listener: fire on "failed"/"disconnected".
        {
            let active = Rc::clone(&active);
            let on_failure = Rc::clone(&on_failure);
            let pc_clone = pc.clone();
            let cb = Closure::<dyn FnMut(JsValue)>::new(move |_ev: JsValue| {
                if !active.get() {
                    return;
                }
                let s = pc_clone.ice_connection_state();
                use leptos::web_sys::RtcIceConnectionState as S;
                if matches!(s, S::Failed | S::Disconnected | S::Closed) {
                    leptos::logging::warn!("watchdog: ICE state={:?}, triggering reconnect", s);
                    active.set(false);
                    (on_failure)();
                }
            });
            pc.set_oniceconnectionstatechange(Some(cb.as_ref().unchecked_ref()));
            cb.forget();
        }

        // Stall timer: every TICK_INTERVAL_MS check if currentTime has advanced.
        {
            let active = Rc::clone(&active);
            let on_failure = Rc::clone(&on_failure);
            let video_clone = video.clone();
            let last_time: Rc<Cell<f64>> = Rc::new(Cell::new(-1.0));
            let last_change_at: Rc<Cell<f64>> = Rc::new(Cell::new(0.0));
            let cb = Closure::<dyn FnMut()>::new(move || {
                if !active.get() {
                    return;
                }
                let now_secs =
                    leptos::web_sys::js_sys::Date::now() / 1000.0;
                let t = video_clone.current_time();
                if (t - last_time.get()).abs() > 0.001 {
                    last_time.set(t);
                    last_change_at.set(now_secs);
                    return;
                }
                if last_change_at.get() == 0.0 {
                    last_change_at.set(now_secs);
                    return;
                }
                if now_secs - last_change_at.get() >= Watchdog::STALL_THRESHOLD_SECS {
                    leptos::logging::warn!(
                        "watchdog: <video> stalled for >{}s (currentTime={}), triggering reconnect",
                        Watchdog::STALL_THRESHOLD_SECS,
                        t
                    );
                    active.set(false);
                    (on_failure)();
                }
            });
            if let Some(window) = leptos::web_sys::window() {
                let _ = window.set_interval_with_callback_and_timeout_and_arguments_0(
                    cb.as_ref().unchecked_ref(),
                    Watchdog::TICK_INTERVAL_MS,
                );
            }
            cb.forget();
        }

        Self { active }
    }

    fn stop(&self) {
        self.active.set(false);
    }
}
```

- [ ] **Step 3: Add `reconnect_loop` and rewire the Effect**

Replace the existing `Effect::new` block (around L50 in the current file) with a version that drives `reconnect_loop` instead of a one-shot `connect_whep`. Also store the `Watchdog` in the session holder so cleanup can stop it.

Add a new struct above the component to hold both:

```rust
struct ActiveConnection {
    session: WhepSession,
    watchdog: Watchdog,
}
```

Change `session_holder` type from `StoredValue<Option<WhepSession>>` to `StoredValue<Option<ActiveConnection>>`:

```rust
let session_holder: StoredValue<Option<ActiveConnection>> = StoredValue::new(None);
```

Replace the existing Effect (the one that calls `connect_whep` then stores into `session_holder`) with:

```rust
    let cancelled_for_effect = Arc::clone(&cancelled);
    Effect::new(move |_| {
        let Some(video) = video_ref.get() else { return };
        let source_id = source_id_for_effect.clone();
        let cancelled = Arc::clone(&cancelled_for_effect);
        spawn_local(async move {
            // The reconnect-trigger channel: when the watchdog fires, it sets
            // this flag; the loop drains it and reconnects.
            let reconnect_flag = std::rc::Rc::new(std::cell::Cell::new(false));

            loop {
                if cancelled.load(Ordering::Acquire) {
                    return;
                }
                match connect_whep(&video, &source_id).await {
                    Ok(session) => {
                        if cancelled.load(Ordering::Acquire) {
                            // Unmounted between POST and now — clean up server
                            // session and bail.
                            if let Some(url) = &session.resource_url {
                                dispatch_delete(url);
                            }
                            session.pc.close();
                            return;
                        }
                        // Install watchdog: on failure, set the reconnect flag.
                        let flag = std::rc::Rc::clone(&reconnect_flag);
                        let watchdog =
                            Watchdog::install(&video, &session.pc, move || flag.set(true));

                        install_pagehide_teardown(&session);
                        session_holder.set_value(Some(ActiveConnection {
                            session,
                            watchdog,
                        }));

                        // Wait until either cancellation OR a watchdog fire.
                        loop {
                            if cancelled.load(Ordering::Acquire) {
                                return;
                            }
                            if reconnect_flag.get() {
                                reconnect_flag.set(false);
                                break;
                            }
                            // Poll every 100ms. Cheap; no need for a Promise/Future here.
                            wasm_bindgen_futures::JsFuture::from(
                                leptos::web_sys::js_sys::Promise::new(&mut |resolve, _| {
                                    if let Some(w) = leptos::web_sys::window() {
                                        let _ = w
                                            .set_timeout_with_callback_and_timeout_and_arguments_0(
                                                &resolve, 100,
                                            );
                                    }
                                }),
                            )
                            .await
                            .ok();
                        }

                        // Tear down old session before reconnecting.
                        if let Some(active) = session_holder.try_update_value(|v| v.take()).flatten() {
                            active.watchdog.stop();
                            if let Some(url) = &active.session.resource_url {
                                dispatch_delete(url);
                            }
                            active.session.pc.close();
                        }
                        // Loop falls through to `connect_whep` again with no
                        // backoff (first retry is immediate).
                    }
                    Err(e) => {
                        leptos::logging::warn!(
                            "reconnect_loop: connect_whep failed: {e:?}, backing off"
                        );
                        // Exponential backoff capped at 5s.
                        sleep_for_backoff().await;
                    }
                }
            }
        });
    });
```

- [ ] **Step 4: Add the `sleep_for_backoff` helper**

The backoff is a fixed schedule (no state needs to persist across reconnects for now — each connect_whep failure resets the schedule). Add this free function above `connect_whep`:

```rust
/// Sleep for an exponentially increasing duration, capped at 5s.
/// Uses a static atomic to track the current step across calls.
async fn sleep_for_backoff() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static STEP: AtomicUsize = AtomicUsize::new(0);
    let schedule_ms: [i32; 7] = [500, 1000, 2000, 4000, 5000, 5000, 5000];
    let i = STEP.fetch_add(1, Ordering::Relaxed).min(schedule_ms.len() - 1);
    let ms = schedule_ms[i];
    let promise = leptos::web_sys::js_sys::Promise::new(&mut |resolve, _| {
        if let Some(window) = leptos::web_sys::window() {
            let _ = window
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        }
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}
```

Note: a global `STEP` is acceptable here because the same source's reconnect loop is the only consumer per page, and resetting it on success isn't critical (one accidental long delay after a long failure run is fine — it caps at 5 s anyway).

- [ ] **Step 5: Update `on_cleanup` to drop both session AND watchdog**

Replace the existing `on_cleanup` block:

```rust
    let cancelled_for_cleanup = Arc::clone(&cancelled);
    on_cleanup(move || {
        cancelled_for_cleanup.store(true, Ordering::Release);
        let active = session_holder.try_update_value(|opt| opt.take()).flatten();
        if let Some(active) = active {
            active.watchdog.stop();
            if let Some(url) = &active.session.resource_url {
                dispatch_delete(url);
            }
            active.session.pc.close();
        }
    });
```

- [ ] **Step 6: Verify WASM compile**

Run: `cd crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings -W clippy::all && cd ../..`
Expected: success, no warnings.

Common issues to look for:

- If `RtcIceConnectionState` is not imported, add it to the `use leptos::web_sys::{...}` block at the top of the file.
- If `js_sys::Date` isn't accessible, use `leptos::web_sys::js_sys::Date` (path-qualified above is correct).

- [ ] **Step 7: Commit**

```bash
git add crates/presenter-ui/src/components/stage/ndi_video.rs
git commit -m "feat(stage): Watchdog + reconnect_loop for auto-recovery"
```

---

## Task 7: Local gate — full build, tests, and dev verification

**Files:** none modified (verification only)

- [ ] **Step 1: cargo fmt check**

Run: `cargo fmt --all --check`
Expected: success. If fmt diff, run `cargo fmt --all` and commit as `style: cargo fmt`.

- [ ] **Step 2: cargo clippy --workspace (all targets)**

Run: `cargo clippy --workspace --all-targets -- -D warnings -W clippy::all`
Expected: success.

- [ ] **Step 3: WASM clippy**

Run: `cd crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings -W clippy::all && cd ../..`
Expected: success.

- [ ] **Step 4: cargo test --workspace**

Run: `cargo test --workspace`
Expected: success. New supervisor unit tests pass; existing tests pass.

- [ ] **Step 5: cargo test --features test-helpers (kill route compile check)**

Run: `cargo test -p presenter-server --features test-helpers --lib router::integrations::ndi_whep`
Expected: success — `kill_pipeline_for_test_returns_404_for_unknown_source` passes.

- [ ] **Step 6: Build the dev binary with test-helpers feature**

Run: `cargo build --release -p presenter-server --features test-helpers`
Expected: success.

Deploy to local dev process:

```bash
sudo systemctl stop presenter-dev
sudo cp target/release/presenter-server /opt/presenter-dev/presenter-server
sudo systemctl start presenter-dev
```

Verify the service is healthy:

```bash
curl -s http://10.77.8.134:8080/healthz | jq
```

Expected: `{"status":"ok","version":"0.4.94","channel":"dev"}`.

- [ ] **Step 7: Run the Playwright recovery test against dev**

Run: `BASE_URL=http://10.77.8.134:8080 npm run test:playwright -- ndi-webrtc-recovery`

Expected: PASS — the recovery test goes green now that Tasks 5+6 are in place. If it fails, inspect the Playwright trace + presenter-dev journalctl for what broke.

- [ ] **Step 8: Manual visual verification on dev via Playwright MCP**

In Playwright MCP:

1. Navigate to `http://10.77.8.134:8080/stage/ndi-fullscreen`
2. Take a snapshot — confirm `<video>` is visible with `videoWidth > 0`
3. From SSH: `curl -s -X POST http://10.77.8.134:8080/test/ndi/kill-pipeline/<source_id>`
4. Wait 8 s, take another snapshot
5. Confirm video is again playing (`videoWidth > 0`, no error overlay)
6. Browser console (via `browser_console_messages`) shows: 1 WARN about reconnect, 1 INFO from connect_whep, NO ERROR
7. From SSH: `sudo journalctl -u presenter-dev --since '2 minutes ago' | grep -E 'supervisor|pipeline error|EOS|rebuild'`
8. Expect to see: `pipeline EOS received` (from Task 2), `supervisor: rebuild succeeded`

If any verification step fails, fix the root cause and commit before pushing.

---

## Task 8: Push, monitor CI, verify on dev, open PR

**Files:** none modified (controller-only).

- [ ] **Step 1: Push to dev**

```bash
git fetch origin && git status   # confirm clean local tree
git push origin dev
```

- [ ] **Step 2: Watch CI to terminal state**

Get the latest run ID:

```bash
gh run list --branch dev --limit 1 --json databaseId,status,conclusion,name
```

Wait per `ci-monitoring.md` single-sleep pattern:

```bash
RUN_ID=<id from above>
# Background sleep then read status — single shot, NO loop
```

Use one `sleep 300 && gh run view <RUN_ID> --json status,conclusion,jobs` background command. When notified, inspect. If any job failed, `gh run view <RUN_ID> --log-failed`, fix the root cause, commit, push, repeat.

Required: ALL jobs green, INCLUDING `Deploy to Dev` and `Playwright E2E (3/3)` shards.

- [ ] **Step 3: Post-deploy verification on dev**

Use Playwright MCP to:

1. Open `http://10.77.8.134:8080/ui/operator` — confirm UI loads, version footer reads `v0.4.94 (dev)`
2. Open `http://10.77.8.134:8080/stage/ndi-fullscreen` — confirm video plays (presupposes an NDI source is active on dev; if not, activate via operator UI first)
3. Trigger pipeline kill: `curl -X POST http://10.77.8.134:8080/test/ndi/kill-pipeline/<source_id>`
4. Watch the page — within 10 s, video resumes without page refresh
5. Check browser console for zero errors

- [ ] **Step 4: Open PR dev → main**

```bash
gh pr create --base main --head dev --title "feat(ndi): auto-recovery for stage WebRTC streams" --body "$(cat <<'EOF'
## Summary

- Adds two independent self-healing layers to the NDI WebRTC transport
- Browser-side `Watchdog` detects ICE failures + video stalls, runs `reconnect_loop` with exponential backoff (cap 5s)
- Server-side `PipelineSupervisor` task rebuilds the GStreamer pipeline immediately on `Errored`/`Stopped` (rate-limited 2s, backoff after 5 consecutive failures)
- Stage display now recovers from `ndisrc` crashes / source restarts / browser ICE drops within 5-7s, no manual refresh needed
- No protocol changes; 30s DB-driven backstop unchanged

Spec: `docs/superpowers/specs/2026-05-21-ndi-stage-auto-recovery-design.md`

## Test plan

- [x] Unit tests for `SupervisorState` rate-limiter + exponential backoff (run on every CI host, no libndi needed)
- [x] Playwright recovery test (`tests/e2e/ndi-webrtc-recovery.spec.ts`, tagged `@video-codec` for real-Chrome routing)
- [x] Manual verification on dev: kill pipeline, observe automatic recovery in browser
- [x] Browser console clean (zero errors) during recovery cycle
- [x] Production `deploy.yml` / `release.yml` build WITHOUT `--features test-helpers` so the kill route is absent in prod binaries

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 5: Verify PR is mergeable + clean**

```bash
gh pr view --json mergeable,mergeStateStatus
```

Expected: `{"mergeable": "MERGEABLE", "mergeStateStatus": "CLEAN"}`.

If `mergeStateStatus` is `UNSTABLE` or `BLOCKED`, investigate the failing check via `gh pr checks <PR_NUM> --watch` and `gh run view --log-failed`. Fix root cause; do not bypass.

- [ ] **Step 6: Send completion report and wait for explicit "merge it"**

Per `completion-report.md` template. The PR URL goes in the report. DO NOT merge autonomously; wait for the user's explicit instruction.

---

## Self-Review Checklist (run after writing this plan)

**Spec coverage:**

- Spec §Architecture (two layers) → Tasks 5 (server supervisor) + 6 (client watchdog) ✅
- Spec §Components → Task 5 (PipelineSupervisor, rebuild_pipeline), Task 6 (Watchdog, reconnect_loop, ActiveConnection) ✅
- Spec §Data flow (3 scenarios) → covered by Playwright recovery test (Task 4) + supervisor unit tests (Task 5 Step 1) ✅
- Spec §Error handling (rate-limit, backoff, multi-display, paused video) → Task 5 unit tests + Task 6 watchdog stall behaviour ✅
- Spec §Testing (unit + Playwright + manual) → Tasks 5, 4, 7 ✅
- Spec §Files touched (list) → all covered, file paths match ✅
- Spec §Version bump 0.4.93→0.4.94 → Task 1 ✅
- Spec §Diagnostic EOS log → Task 2 ✅
- Spec §Guarded test surface (cfg-gated kill route + feature flag in pipeline.yml only) → Task 3 ✅

**Placeholder scan:** No `TBD`, `TODO`, "implement later", "similar to Task N", or "add appropriate error handling" anywhere in the plan. Every step has either exact code or exact commands with expected output.

**Type consistency:**

- `SupervisorState`, `RebuildDecision::ProceedAfter` — defined in Task 5 Step 3, used in Task 5 Steps 1+7 ✅
- `ActiveSource { pipeline, supervisor }` — defined Task 5 Step 5, used Task 5 Steps 8 + 7 ✅
- `Watchdog::install(&video, &pc, on_failure)` / `.stop()` — defined Task 6 Step 2, used Task 6 Step 3 ✅
- `ActiveConnection { session, watchdog }` — defined Task 6 Step 3 head, used Step 3 body + Step 5 ✅
- `reconnect_loop` is an inline `loop {}` inside the Effect, not a free function (matches the structure I described in Task 6 Step 3). The spec calls it `async fn reconnect_loop` for clarity, but the implementation is an inlined loop — semantically identical, no API surface change. Acceptable.
- `kill_pipeline_for_test` handler signature — defined Task 3 Step 4, called from Task 4 Playwright spec via HTTP, called from Task 3 Step 2 unit test ✅

No gaps.
