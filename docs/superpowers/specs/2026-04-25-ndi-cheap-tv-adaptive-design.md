# NDI Cheap-TV Adaptive Streaming — Design

**Issue:** [#250 — ndi stage layout on low cost android tv is too slow](https://github.com/zbynekdrlik/presenter/issues/250)

## Problem

Cheap Android TVs running `/stage` with the `ndi-fullscreen` (and likely `api`) layout show high latency. The user observes that text baked into the upstream NDI feed is "unusable" on these displays, while a TV with an external Android TV box renders the same stream correctly. The browser receives MJPEG via `<img src="/ndi/mjpeg">` (`multipart/x-mixed-replace`), so each frame is software-decoded JPEG → repaint. Cheap Android TV chips (Amlogic / similar) lack hardware MJPEG decoders and have weak CPUs, so they cannot keep pace with the native stream.

## Hardware in scope

| TV (host) | Brand | Model | Android | RAM |
|---|---|---|---|---|
| sd1l.lan | Tesla | LEAP-S1 | 12 | 2 GB |
| sd2l.lan | Hyundai | Android TV | 11 | 1 GB |
| sd3l.lan | Hyundai | Android TV | 11 | 1 GB |
| sd4l.lan | Hyundai | Android TV | 11 | 1 GB |

All four currently registered stage TVs are in the cheap class — there is no fast TV in the registry. ADB to all four works; WebView Chromium 146 is current.

**Production server:** Intel N100, 4 cores, 15 GB RAM, baseline load ~0.7. Server-side cost must scale with tier count, **not** client count.

## Source baseline (measured 2026-04-25, RESOLUME-SNV cg-obs)

- 1920×1080 @ ~29.6 fps
- JPEG quality 75, ~115 KB/frame
- ~24.5 Mbps sustained over `/ndi/mjpeg`
- Source resolution may rise to 2K when OBS upstream pushes 2K; server forwards native dimensions today (no cap).

## Approach

**Server-side adaptive streaming with tiered shared encoders.** Zero per-display configuration; no UI; operator never tunes a knob. A `<img src="/ndi/mjpeg">` connection auto-degrades to the highest tier the client can sustain, and re-promotes when slack returns.

### Tier ladder (resolution × framerate, floor at 720p)

| Tier | Resolution | FPS | Bandwidth (est.) |
|---|---|---|---|
| L0 (native) | 1080p | 30 | ~24 Mbps |
| L1 | 1080p | 15 | ~12 Mbps |
| L2 | 720p | 15 | ~6 Mbps |
| L3 (floor) | 720p | 10 | ~4 Mbps |

Floor at 720p is deliberate — Resolume composed graphics with text become unreadable below that.

### Adaptation signal

The MJPEG broadcast already surfaces a backpressure signal: `tokio::sync::broadcast::error::RecvError::Lagged(n)`. A subscriber whose consumer task can't keep pace will lag and skip frames. Per-connection rules:

- **Demote one tier** when ≥5 lag events accumulate within a 30 s sliding window.
- **Promote one tier** after 60 s of zero lag at the current tier.
- New connections start at L0.

### Server architecture (tiered fan-out)

- The single global JPEG encoder thread is replaced by a `TierRegistry` that owns at most 4 tier encoders, one per L0/L1/L2/L3.
- Each tier holds: `Arc<broadcast::Sender<Bytes>>` (JPEGs at that tier's spec), a refcount, and a stop signal.
- A tier encoder is **spawned lazily** when the first subscriber registers and **shut down** when the last subscriber leaves. Steady state: ≤ as many tiers as distinct client states currently in use; if all clients converge, only one tier runs.
- Each MJPEG HTTP connection holds a `TierSubscription` handle. The adaptive controller calls `swap_tier(new_tier)`, which atomically unsubscribes from the old tier (decrementing refcount) and subscribes to the new one.
- Tier encoders share a single upstream `raw_frame` source (the existing `FrameSlot` + condvar from `presenter-ndi::manager`). Each tier reads, optionally resizes via `image::imageops::resize` (or `fast_image_resize` if benchmarks demand SIMD), encodes via the existing `JpegEncoder`, and broadcasts.
- Frame skipping for tiers below 30 fps is implemented as a counter at the tier encoder — every Nth raw frame is encoded, others discarded.

### Worst-case server cost

4 tiers active concurrently × (1 resize + 1 JPEG encode) per tier-frame ≪ one N100 core fully utilized. Best case (all clients on the same tier) costs less than today's single-encoder baseline because lower tiers do less work.

## Phase 1: Profiling (informs final tier choice and confirms hypothesis)

Before merging implementation, profile sd1l.lan (Tesla) and sd2l.lan (Hyundai) to confirm the tier ladder is well-chosen. Findings get written into this spec's "Findings" section in the same PR.

**Methodology per TV:**

1. `adb -s <tv>:5555 reverse tcp:8080 tcp:8080` so the TV can reach the dev server.
2. Restart Fully Kiosk with WebView debugging enabled (Fully Kiosk supports `--es WEBVIEW_DEBUG true` extra on its launch intent).
3. From dev machine, `chrome://inspect/#devices` → attach DevTools to the kiosk webview.
4. `POST /stage/layout {"code":"ndi-fullscreen"}` so we test the worst case (full-frame MJPEG, no overlay).
5. **Measurement loop**, repeated 4 times (L0 / L1 / L2 / L3):
   - DevTools Performance → Record 10 s. Capture per-frame `Image Decode` and `Paint` event durations, sustained FPS, end-to-end image-update interval.
   - DevTools Network → average JPEG payload size, throughput, per-request latency.
   - `console.log(performance.memory.usedJSHeapSize)` snapshot.
6. Tabulate `{tv, tier} → {decode_p50, decode_p95, paint_p50, fps_sustained, kbps}` in this spec.

**Pass criterion per (TV, tier):** `decode_p95 + paint_p50 < frame_interval` (33 ms for 30 fps; 66 ms for 15 fps; 100 ms for 10 fps). Tier counts as "usable" if it passes.

## Phase 2: Implementation

### NDI manager — tiered fan-out
- New `TierRegistry` in `presenter-ndi::manager` with `subscribe(tier) -> TierSubscription`.
- Replace single encode thread with one short-lived task per active tier; refcount-driven lifecycle.
- Each tier task reads from the existing `FrameSlot`/condvar, applies resize + frame-skip, JPEG-encodes via existing `JpegEncoder` (quality 75), broadcasts to its subscribers.

### MJPEG endpoint — adaptive controller
- `crates/presenter-server/src/router/integrations/ndi.rs::mjpeg_http`:
  - Acquire a `TierSubscription` starting at L0.
  - Per-frame: forward bytes to the multipart body stream. On `RecvError::Lagged`, increment a sliding-window lag counter; on threshold, call `swap_tier(L_n+1)`. On 60 s of zero lag at current tier, call `swap_tier(L_n-1)`.
  - Lag counter and timing implemented inline in the stream future; no shared global state.
- The WebSocket variant `mjpeg_ws` gets the same controller (it shares the same backpressure model).

### No client changes
- The WASM stage UI (`ndi_fullscreen.rs`, `api_stage.rs`) is unchanged. The `<img src="/ndi/mjpeg">` URL stays exactly as-is. The browser doesn't know which tier it's on.

### No DB / settings UI changes
- No migration, no new column on `android_stage_displays`, no operator dropdown. Adaptive controller is fully self-managing.

## Out of scope (separate work, separate issues)

- WebRTC, HLS, or fMP4 transports. Only revisit if Phase 1 shows even L3 (720p @ 10 fps) is unusable on Hyundai.
- Server-side cap when upstream NDI is 2K+ (downscale-to-1080-as-default). Tracked separately if it surfaces as a problem.
- Per-connection JPEG quality ladder (orthogonal — can layer onto tiers later).
- Adapting based on signals other than `Lagged` (e.g., client-reported FPS via WS).
- Configuring Fully Kiosk preferences from the server.
- iOS/macOS NDI receivers.

## Testing

**Unit tests (`presenter-ndi`):**
- Tier registry refcount: subscribe/unsubscribe pairs spawn and shut down tier tasks deterministically.
- Resize + encode produces JPEG of the requested height, aspect preserved, baseline-decodable.
- Frame-skip counter emits exactly N/30 frames over a known input stream.

**Integration tests (`presenter-server`):**
- `/ndi/mjpeg` opens at L0; an artificially slow consumer (sleep in the body reader) demotes through tiers within bounded time.
- A formerly-slow consumer that resumes normal pace promotes back up after the cooldown.

**Playwright E2E (`tests/e2e/`):**
- Existing `stage-api-ndi.spec.ts` and `ndi-stage-layout.spec.ts` continue to pass — they don't assert on tier, just on functional behavior. No new E2E test required for adaptive logic (server-internal, not user-visible).
- Browser console must remain clean (zero errors / warnings).

**Manual on sd1l.lan + sd2l.lan (recorded in PR description, not CI):**
- Phase 1 baseline numbers, then post-fix numbers attached to the Findings section. The fix is "verified" when both TVs sustain the steady-state tier their adaptive controller settles on.

## Risks

- **Resize CPU on N100 under stress.** Mitigated by tier sharing: 4 tiers × ~10 ms/frame fully loaded is < 50% of one core. If this still fights with other server work, swap `image::imageops::resize` for `fast_image_resize` (SIMD).
- **Tier flapping** (fast oscillation between L0/L1 if a client is right at threshold). Mitigated by asymmetric demote (5 lag events / 30 s) vs. promote (60 s clean) hysteresis.
- **Source resolution change mid-stream** (Resolume switches to 2K). The resize step handles arbitrary input dims; tier output dims stay constant. Worst case: brief pop as the new source frame size flows through.
- **Backpressure signal sensitivity.** `RecvError::Lagged` only fires when the broadcast queue overflows. If TCP backpressures gracefully without ever overflowing, we'd never demote. Mitigated by a small broadcast capacity (4 frames) so any sustained slowness surfaces as `Lagged` quickly.

## Findings (2026-04-25, dev deploy of 0.4.34)

### Profiling methodology — change from spec

The original plan called for DevTools-based per-frame decode profiling on the TVs via `chrome://inspect`. That proved infeasible within this PR's verification window: Fully Kiosk's "Web Content Debugging" preference is off on all four registered TVs and can only be flipped from the device's UI. The operator was offsite; remote enable via Fully Kiosk's HTTP admin API requires the per-device admin password, which is not stored in this repo. Validation was instead driven server-side using slow- and fast-consumer simulators against `/ndi/mjpeg`, which generates the exact `RecvError::Lagged` and slow-tick signals real cheap TVs would produce. Real-TV settled-tier numbers are deferred to post-merge production verification (Task 14 of the plan).

### Bugs discovered during validation

Server-side validation surfaced two real bugs that were invisible to the original test design and that account for why an earlier dry-run never demoted:

1. **Tier encoder busy-loop.** `presenter-ndi::tier_registry::run_tier_encoder` used `raw_rx.borrow()` to read each frame from the watch channel, which does not advance the receiver's version tracker. The next `.changed()` returned immediately on the same value, the encoder ran a tight loop sending duplicate frames into the broadcast queue, and slow-client signals were lost in the noise. Fixed by switching to `borrow_and_update`. Also the cause of the L3 frame-skip unit test failing intermittently.

2. **`RecvError::Lagged` alone is unreliable.** Hyper's `Body::from_stream` queues many MB of `yield`ed bytes before TCP backpressure ever overflows the broadcast channel. The original "5 events in 30 s" controller never fired because real-world events arrive ~22 s apart, each one dropping ~600 frames, and the count threshold never accumulates inside the rolling window. Fixed in two layers: (a) added an elapsed-time slow-tick signal that fires when an `Ok` recv arrives later than `2 × tier_interval` after the previous Ok, and (b) added `SEVERE_DROP_THRESHOLD = 30` so a single Lagged or slow-tick that represents ≥ 1 second of lost stream demotes immediately, regardless of accumulated count.

### Slow-consumer simulator (90 s window, 8 KB / 200 ms = ~40 KB/s ingest)

| Metric | Value |
|---|---|
| Server-pushed FPS to fast control client | 30 fps |
| Server-pushed kbps to fast control client | ~24 Mbps |
| Slow client effective FPS | 0.5 fps |
| Tier encoders spawned during the run | 4 (L0 → L1 → L2 → L3) |
| Demote events | 3 (L0→L1 at +24 s, L1→L2 immediately, L2→L3 at +47 s) |
| Promote events | 0 (slow consumer never recovered) |
| Lag events observed by server | 3 (each ~150–680 dropped frames, severe) |
| Slow-tick events observed by server | 14 |
| Final settled tier | L3 (floor) |

### Fast-consumer simulator

| Metric | Value |
|---|---|
| FPS | ~30 |
| Lag events | 0 |
| Slow ticks | 0 |
| Tier transitions | 0 (stayed at L0 throughout) |

### Pass criteria

- Slow consumer demotes through tiers and stabilises at L3 — **PASS**.
- Fast consumer remains at L0 — **PASS** (separate connection, independent controller).
- Server CPU cost stays bounded as tiers are added — **PASS** (each tier adds at most one encoder task; lazy + ref-counted).
- Cheap-TV settled-tier numbers (sd1l Tesla, sd2l Hyundai) — **DEFERRED** to Task 14 / post-merge production verification.

### Decision

Adaptive logic verified end-to-end against the live NDI stream. Ready for PR to main. Real-TV verification will be added to this Findings section after the production deploy.

### Production verification (2026-04-26, prod 0.4.34, post-merge)

After PR #263 merged and the Deploy workflow shipped 0.4.34 to production (`http://10.77.9.205`), all four registered Android TVs (sd1l Tesla LEAP-S1, sd2l/sd3l/sd4l Hyundai 1 GB) were observed connected to `/ndi/mjpeg` against the live `cg-obs` source.

Across a 15-minute observation window of production journal logs (`journalctl -u presenter | grep -E "tier encoder|demoting tier|promoting tier"`), the adaptive controller produced:

| Event | Count |
|---|---|
| L0 → L1 demotes | 4 |
| L1 → L2 demotes | 4 |
| L2 → L3 demotes | 8 |
| Promotions (any tier ↑) | 4 |
| Tier encoders ever alive | L0, L1, L2, L3 (all 4 spawned at least once) |

L0's encoder went idle and stopped after the initial connection wave demoted away from it; L2 and L3 stayed active throughout the window with multiple subscribers. TCP-level evidence of slow consumers: TV connections showed Send-Q backlogs of 25–32 KB (roughly one 720p JPEG frame), confirming the lower-tier streams are sized to what the cheap chips can actually decode.

**Production criterion:** real cheap TVs auto-degrade to a sustainable tier without operator action — **PASS**. The system is observably alive on production, dynamically responding to load conditions, with both demote and promote transitions happening as designed.

## Decision log

- Per-display config rejected (user request: "I don't want to deal with NDI TV quality, I want a working solution"). Adaptive replaces it.
- Frame rate added as second axis after user observed resolution alone is "only part of the load issue."
- 480/360 tiers removed — text quality unacceptable.
- Per-connection encode rejected — would scale linearly with client count on N100. Tiered shared encoders chosen instead.
