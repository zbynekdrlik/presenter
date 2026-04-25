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

## Findings (filled in during Phase 1, before merging)

_To be populated by the profiling pass on sd1l.lan and sd2l.lan. Tables for `{tv, tier} → {decode_p50, decode_p95, paint_p50, fps_sustained, kbps}` go here._

## Decision log

- Per-display config rejected (user request: "I don't want to deal with NDI TV quality, I want a working solution"). Adaptive replaces it.
- Frame rate added as second axis after user observed resolution alone is "only part of the load issue."
- 480/360 tiers removed — text quality unacceptable.
- Per-connection encode rejected — would scale linearly with client count on N100. Tiered shared encoders chosen instead.
