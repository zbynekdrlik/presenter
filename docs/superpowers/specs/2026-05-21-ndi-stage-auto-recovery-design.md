# NDI Stage Auto-Recovery — Design

> **Successor to** `2026-05-18-ndi-webrtc-transport-design.md` (the WebRTC transport, shipped in PR #330 / v0.4.93). This spec does NOT replace the transport; it adds resilience around it.

**Status:** Proposed
**Date:** 2026-05-21

## Problem

After the WebRTC transport shipped to prod, the stage display NDI video plays for a few minutes, then goes black and stays black until the page is manually refreshed. Latency was inadequate while it WAS playing — but that turned out to be source-side (Resolume / OBS); restarting OBS at the source restored quality. The remaining defect is purely the **failure-to-recover** behaviour: a stage display that needs a manual page refresh is unreliable for a worship service.

The prod log (2026-05-21) shows the immediate trigger:

```
10:03:22  ERROR presenter_ndi::pipeline: pipeline error
          error=Internal data stream error.: GstNdiSrc:ndisrc4
10:03:37  INFO  NDI auto-reconnect: source restored
```

The server-side `ndisrc` element emits an "Internal data stream error" during normal operation. The existing 30 s DB-polled auto-reconnect rebuilds the GStreamer pipeline — but the browser's `RTCPeerConnection` is still bound to the now-destroyed pipeline, sees no further frames, and there is no client-side recovery code. Manual refresh is the only path back to live video.

## Goal

Stage display recovers from any single failure — `ndisrc` crash, NDI source restart, browser network blip — within 5–7 seconds, without any operator interaction. Comparable in feel to vdo.ninja's "stream just resumes" behaviour.

## Non-goals

- Re-architect transport (we explicitly chose to keep webrtcsink server-encode)
- Source-side encoding (rejected: we keep N100 as the encoder)
- Reduce encode latency further (already adequate once source is healthy)
- Stress testing N stage displays × M failures/min (math says load is trivial; will revisit if real numbers show otherwise)

## Architecture

Two independent self-healing layers. Neither depends on the other to act.

```
┌──────────────────────────────────────────────────────────────────┐
│ Browser <NdiVideo>                                               │
│  ┌────────────────────┐    ┌───────────────────────────────┐    │
│  │ RTCPeerConnection  │    │ Watchdog                      │    │
│  │  ice state changes ├───►│  - oniceconnectionstatechange │    │
│  │  ontrack frames    │    │  - <video> stall timer (3s)   │    │
│  └────────────────────┘    │  - reconnect w/ backoff       │    │
│                            └───────────────────────────────┘    │
└──────────────────────────────────────────────────────────────────┘
                  ▲    │ POST/DELETE /ndi/whep/<id>
                  │    ▼
┌──────────────────────────────────────────────────────────────────┐
│ Server NdiManager                                                │
│  ┌─────────────────────┐  state changes  ┌────────────────────┐ │
│  │ NdiPipeline (gst)   ├───────────────►│ Pipeline Supervisor │ │
│  │  Streaming / Errored│                │  - rebuild on Err   │ │
│  │  Stopped (ndisrc)   │◄───────────────│  - rate-limit (2s)  │ │
│  └─────────────────────┘   restart       │  - exp. pause @ 5x │ │
│                                          └────────────────────┘ │
│  + 30s ticker as backstop (unchanged)                            │
└──────────────────────────────────────────────────────────────────┘
```

**Invariants:**

- Server pipeline is "live again" within ~2 s of an `ndisrc` crash (vs 30 s today)
- Browser detects "stream is dead" within ~3-5 s (ICE timeout OR video stall)
- Combined: stage display is back to live video within 5-7 s of any single failure

**No protocol changes.** WHEP routes (`POST/PATCH/DELETE /ndi/whep/:source_id`) stay identical.

## Components

### Client side — 1 file modified

`crates/presenter-ui/src/components/stage/ndi_video.rs`

| Unit | Responsibility | Interface |
|---|---|---|
| `WhepSession` (existing) | Owns one PC + resource URL | unchanged |
| **`Watchdog` (new)** | Subscribes to PC state + video stall events; triggers reconnect | `start(video, pc, on_failure)` / `stop()` |
| **`reconnect_loop` (new)** | Tears down old session, builds new one, backoff on failure | `async fn(source_id, video) -> WhepSession` |
| `connect_whep` (existing) | Single connect attempt | unchanged signature |

The watchdog holds a stall timer (`setInterval` checking `video.currentTime` delta) and an ICE listener (`pc.set_oniceconnectionstatechange`). The reconnect loop is a fixed pattern:

```rust
async fn reconnect_loop(video: HtmlVideoElement, source_id: String) -> WhepSession {
    let mut backoff = [500, 1000, 2000, 4000, 5000, 5000, 5000];
    let mut i = 0;
    loop {
        match connect_whep(&video, &source_id).await {
            Ok(session) => return session,
            Err(_) => {
                sleep_ms(backoff[i.min(backoff.len() - 1)]).await;
                i += 1;
            }
        }
    }
}
```

Backoff caps at 5 s so recovery never drifts beyond "feels broken" territory.

### Server side — 1 file modified + minor pipeline.rs addition

`crates/presenter-ndi/src/manager.rs`, `crates/presenter-ndi/src/pipeline.rs`

| Unit | Responsibility | Interface |
|---|---|---|
| `NdiPipeline` (existing) | Owns one gst pipeline + state channel | unchanged externally; `state_watcher()` already exposed |
| **`PipelineSupervisor` (new, inside `NdiManager`)** | One `tokio::spawn`'d task per active source watching its `state_watcher()`; rebuilds on Errored/Stopped with rate-limiting | spawned in `start_pipeline`; cancelled in `stop_pipeline` |
| `start_pipeline` (existing) | Build + start + insert into HashMap | now also spawns the supervisor task |
| `stop_pipeline` (existing) | Drop entry, stop pipeline | now also aborts supervisor task |
| 30 s auto-reconnect (existing, `state/mod.rs`) | DB-driven re-activation backstop | unchanged — still useful when `start_pipeline` itself fails (libndi can't find the source yet) |

Supervisor structure:

```rust
struct SupervisorState {
    last_rebuild_at: Instant,
    consecutive_failures: u32,
}

async fn supervisor_task(
    manager: Arc<NdiManager>,
    source_id: String,
    ndi_name: String,
    mut state_rx: watch::Receiver<PipelineState>,
) {
    let mut state = SupervisorState { last_rebuild_at: Instant::now(), consecutive_failures: 0 };
    loop {
        if state_rx.changed().await.is_err() { return; }  // pipeline dropped
        let current = state_rx.borrow().clone();
        match current {
            PipelineState::Errored(_) | PipelineState::Stopped => {
                let now = Instant::now();
                if now.duration_since(state.last_rebuild_at) < Duration::from_secs(2) {
                    continue;  // rate-limited
                }
                if state.consecutive_failures >= 5 {
                    let pause = Duration::from_secs(2_u64 << (state.consecutive_failures - 5).min(4));  // 2, 4, 8, 16, 30 cap
                    let pause = pause.min(Duration::from_secs(30));
                    sleep(pause).await;
                }
                state.last_rebuild_at = Instant::now();
                state.consecutive_failures += 1;
                if manager.rebuild_pipeline(&source_id, &ndi_name).await.is_ok() {
                    state.consecutive_failures = 0;
                }
            }
            PipelineState::Streaming => { state.consecutive_failures = 0; }
            PipelineState::Starting => {}
        }
    }
}
```

The supervisor is owned by `NdiManager` and stored alongside the `ActiveSource` (so `stop_pipeline` can abort it).

`NdiManager::rebuild_pipeline` is a small extraction of the existing `start_pipeline` body that: (a) acquires the active mutex, (b) drops the dead entry, (c) builds a fresh `NdiPipeline`, (d) waits for caps-ready, (e) re-inserts. The supervisor is a separate `tokio` task and holds no mutex when calling it. The existing `check_active_entry` already encapsulates the dead-entry check; `rebuild_pipeline` reuses it via a normal mutex acquire.

### Diagnostic improvement — 1 line

In `pipeline.rs` bus watcher, the EOS branch currently produces no log. Adding `tracing::warn!(state = "stopped", "pipeline EOS received")` on the EOS branch matches the existing ERROR-level logging on the Error branch. Future investigation of `ndisrc` crashes has parity in the logs.

## Data flow

### Failure: `ndisrc` "Internal data stream error" (production case)

```
T+0.0s   ndisrc emits Error → bus watcher → state = Errored("Internal data stream error")
T+0.0s   Supervisor.state_watcher fires; sees Errored
T+0.1s   Supervisor checks rate-limit: last_rebuild > 2s ago → proceed
T+0.1s   Supervisor calls manager.rebuild_pipeline(source_id, ndi_name)
T+0.2s   Old pipeline.stop() → state = Stopped, entry removed from HashMap
T+0.3s   New NdiPipeline::build() + start()
T+0.5-2s ndisrc reconnects to the NDI source, caps negotiated
T+2.0s   New pipeline reaches Streaming → HashMap re-populated
─────────────────────────────────────────────────────────────────────
T+0.0s   Browser: RTC track stops receiving frames
T+3.0s   Watchdog stall timer fires (no <video>.timeupdate for 3s)
T+3.1s   reconnect_loop: pc.close() + DELETE old resource URL (best-effort)
T+3.2s   connect_whep(source_id) → POST /ndi/whep/<id> → 200 (server up)
T+3.5s   New PC negotiated, ontrack fires, video plays

         Total user-visible blackout: ~3.5 seconds
```

### Failure: NDI source machine restart (Resolume / OBS)

```
T+0.0s    Source machine goes down → ndisrc EOS/Error → Stopped
T+0.0-2s  Supervisor rebuilds → ndisrc tries to find source again
T+0.0-Ns  Source not found yet → ndisrc errors → Errored
T+2.0s    Supervisor backs off (rate-limit) → 1 rebuild attempt every 2s
T+~30s    After 5 consecutive failures, supervisor pauses 2→4→8→16→30s
T+Ns      Source reappears → next supervisor rebuild succeeds → Streaming
T+Ns+3s   Browser watchdog notices no frames → reconnects → live again

          While source is down: browser reconnects every ≤5s, gets 404
          ("source not active"), backs off identically. Idempotent.
```

### Failure: Browser ICE failure (network blip, NAT timeout)

```
T+0.0s   ICE connection state → "failed" or "disconnected"
T+0.0s   Watchdog ICE handler fires immediately
T+0.1s   reconnect_loop → pc.close() + DELETE + new connect_whep
T+0.5s   New PC negotiated, live again

         Total user-visible blackout: ~0.5 seconds
```

**Common path:** ONE recovery primitive per side — `reconnect_loop(source_id)` on the browser, `PipelineSupervisor` on the server. Each triggered by its own observable signal, neither depending on the other.

## Error handling

| Case | Behaviour |
|---|---|
| `ndisrc` crashes every <2 s tight-loop | Rate-limiter caps to 1 rebuild / 2 s; after 5 failures, exponential backoff (2→4→8→16→30 s cap). WARN log with consecutive-failure count. |
| Browser reconnects in tight loop while source down | Exponential backoff (cap 5 s) → max 12 attempts/min. Idempotent server-side (POSTs fresh offer, server 404s quickly). |
| Source machine permanently dead | Server settles at 30 s rebuild attempts. Browser settles at 5 s retry. Stops only when operator deactivates the source. |
| Browser unmounts mid-reconnect (page navigation) | Existing `cancelled` AtomicBool + `on_cleanup` flow. Watchdog cleanup added to `on_cleanup`. |
| Server-side rebuild fails (libndi load error, OOM) | Errored bubbles to state_watcher → supervisor sees Errored → backoff. Logged at ERROR. Same recovery path. |
| Multi-display reconnect storm | Each browser independently retries, each costs ~50 ms server CPU per PC negotiation. 5 displays × 1 retry / 5 s = trivial load. |
| `<video>` element paused (kiosk app backgrounded) | `timeupdate` won't fire → stall timer triggers reconnect. False positive — recovery still works, just unnecessarily. YAGNI; could gate on `document.visibilityState === 'visible'` later. |

**Explicitly NOT handled:** NDI source name change (operator must re-activate). Database-driven activation toggling (existing 30 s ticker still covers).

## Testing

### Unit (Rust)

Runs on every CI host without libndi / GPU. Extends the existing `start_pipeline_state_check_tests` module in `manager.rs`.

- Supervisor rate-limiter: simulate two rapid Errored transitions via `set_state_for_test`; assert exactly one rebuild within a 2 s window
- Supervisor consecutive-failure backoff: simulate 5 Errored states in a row; assert the next rebuild is scheduled at +2 s, +4 s, +8 s, +16 s, +30 s
- Supervisor cancellation: assert `stop_pipeline` aborts the supervisor task (no leak)

### Playwright E2E

One new test: `tests/e2e/ndi-webrtc-recovery.spec.ts`

1. Activate NDI source (existing dev source). Navigate stage display layout.
2. Assert `<video data-role="ndi-video">` has `videoWidth > 0` (live)
3. Inject server-side pipeline kill via `POST /test/ndi/kill-pipeline/:id`
4. Wait 8 s
5. Assert `<video>` again has `videoWidth > 0` (recovered without page refresh)
6. Assert console array is empty (no errors logged)

**Guarded test surface:** the kill-pipeline route is compiled in only under a new cargo feature `test-helpers` on `presenter-server` (declared in its `Cargo.toml`). The dev pipeline (`pipeline.yml`) builds with `--features test-helpers`; deploy / release builds (prod) build without it, so production binaries have no such route. Alternative considered: kill `ndisrc` indirectly by activating a non-existent source then re-activating the real one — less precise but no feature flag needed. Decision: take the feature-gated route for test precision (test is the only consumer; surface stays tiny and is absent in prod binaries).

### Manual verification on dev (controller, Playwright MCP)

1. Activate Resolume NDI source on dev
2. Open `/stage/ndi-fullscreen` in Playwright, confirm video plays
3. SSH to dev, restart the NDI broadcaster (or trigger the test route)
4. Observe video freezes
5. Confirm video recovers automatically without page interaction
6. Inspect console: one WARN about ICE failure / stall, then INFO about reconnect, no ERROR

## Files touched

- `crates/presenter-ui/src/components/stage/ndi_video.rs` — add `Watchdog`, `reconnect_loop`; rewire `Effect` to use `reconnect_loop` instead of one-shot `connect_whep`
- `crates/presenter-ndi/src/manager.rs` — add `PipelineSupervisor` task, `rebuild_pipeline` method, supervisor handle in `ActiveSource`
- `crates/presenter-ndi/src/pipeline.rs` — add 1-line WARN log on EOS branch
- `crates/presenter-server/src/router/integrations/ndi_whep.rs` — add `cfg`-gated `POST /test/ndi/kill-pipeline/:id` route
- `tests/e2e/ndi-webrtc-recovery.spec.ts` — new file

Net code change: ~150 LoC added, 0 removed. Single PR, dev → main.

## Version bump

Per `version-bumping.md`, the first commit on dev bumps workspace `0.4.93 → 0.4.94`.

## Out of scope (explicitly)

- Investigating the root cause of `ndisrc` "Internal data stream error" (separate issue if it keeps recurring after this lands — the diagnostic log line will make that easier to file)
- Source-side encoding (vdo.ninja-style) — rejected during brainstorming
- Multi-source layouts (NDI in a tile, not full-screen) — already work, no change
- Visible "Reconnecting…" overlay — rejected during brainstorming (user wants fully automatic, no UI noise)
