# NDI Hardening Design

**Date:** 2026-04-05
**Status:** Accepted

## Problem

Real-world testing at church revealed two critical NDI issues:

1. **Unstable source discovery** — scanning returns different source counts each time (2, 6, 11, 2, 8 across 5 scans on production). Root cause: a new NDI finder is created and destroyed per scan, losing accumulated mDNS state.
2. **Unusable video quality** — stuttering, freezing, and "Connecting..." overlay. Root cause: JPEG encoding blocks the capture thread, causing frame delivery gaps (6 stutters in 10s, max 89ms gap measured on production).

## Verified Measurements

| Metric | Dev (localhost) | Production (network) |
|--------|----------------|---------------------|
| FPS | 30.2 | 29.3 |
| Avg interval | 33.3ms | 34.2ms |
| Max interval | 39.2ms | 89.5ms |
| Jitter | 1.4ms | 8.6ms |
| Stutters (>2x avg) | 0 | 6 in 10s |

Source discovery on production: 2, 6, 11, 2, 8 sources across 5 rapid scans.

## Design

### 1. Persistent NDI Finder

Replace create-scan-destroy with a persistent finder running for the lifetime of `NdiManager`.

- `NdiManager::try_new()` spawns a background thread that creates a finder and loops calling `find_wait_for_sources(5000)`, accumulating sources into `Arc<RwLock<Vec<NdiSourceInfo>>>`
- `discover_sources()` becomes a cheap read from the accumulated list — no blocking API call
- `/ndi/sources` endpoint returns instantly with current known sources
- Finder thread stopped on `NdiManager::drop()`
- Sources that disappear from the NDI network are removed from the list (the SDK handles this — `find_get_current_sources` returns the full current list each time)

### 2. Separated Capture/Encode Pipeline

Split `run_capture_loop` into two threads with a shared frame slot:

- **Capture thread** — tight loop calling `recv_capture_v3(33)` (one frame period). Writes latest frame into a shared slot (`Arc<Mutex<Option<VideoFrame>>>`). Never blocks on encoding. Always overwrites — newest frame wins.
- **Encode thread** — reads latest frame from slot, encodes to JPEG via turbojpeg, broadcasts to WebSocket subscribers via `broadcast::Sender<Bytes>`. If encoding is slower than source fps, frames are skipped gracefully.
- Frame rate and resolution are detected from NDI frame metadata (`frame_rate_n`, `frame_rate_d`, `xres`, `yres`), not hardcoded.
- Capture timeout derived from source frame rate: `1000 / (frame_rate_n / frame_rate_d)` rounded up, with a fallback of 50ms before first frame.

### 3. Canvas Rendering in Browser

Replace Blob URL + `<img>` with `<canvas>` + `createImageBitmap()`:

- Receive JPEG binary via WebSocket `onmessage`
- Create `Blob` from `Uint8Array` with type `image/jpeg`
- Call `createImageBitmap(blob)` — decodes JPEG off main thread (async, GPU-accelerated)
- Draw bitmap to `<canvas>` via `ctx.drawImage(bitmap, 0, 0, canvas.width, canvas.height)`
- `bitmap.close()` after drawing to free GPU memory
- Canvas sized to fill viewport, same as current `<img>` element
- No Blob URL creation/revocation, no `<img>.src` mutation

### 4. Connection Status Events

Capture thread sends `LiveEvent::NdiConnectionStatus` on state changes only:

- `"connected"` — when first video frame is successfully captured after connect
- `"disconnected"` — when capture returns `None` for 3 consecutive seconds
- Events sent through `live_hub.publish()` to all stage clients
- Stage WASM client already handles `NdiConnectionStatus` events and shows/hides the overlay

### 5. E2E Testing

All tests run in CI when NDI SDK is available (dev machine), gracefully skip on GitHub-hosted runners.

- **Discovery stability** — call `/ndi/sources` 5 times with 1s intervals, assert all return the same count (persistent finder should stabilize)
- **Frame delivery** — activate a source, connect to `/ndi/stream` WebSocket, collect frames for 5s, assert: fps > 25, max interval < 100ms, stutter count = 0
- **Canvas rendering** — load `/stage` with ndi-fullscreen layout, verify `<canvas>` element exists, verify frames are being drawn (monitor `drawImage` calls via MutationObserver or performance marks)
- **Connection status** — activate source, verify stage receives "connected" status via LiveEvent; deactivate, verify "disconnected" arrives

## Files to Modify

| File | Change |
|------|--------|
| `crates/presenter-ndi/src/discovery.rs` | Replace blocking scan with read from persistent finder state |
| `crates/presenter-ndi/src/manager.rs` | Add persistent finder thread, split capture/encode into two threads, add status event callback |
| `crates/presenter-ndi/src/receiver.rs` | No changes needed |
| `crates/presenter-ndi/src/encoder.rs` | No changes needed |
| `crates/presenter-ndi/src/lib.rs` | Export new types if needed |
| `crates/presenter-server/src/router/integrations/ndi.rs` | Update `discover_ndi_sources` to use non-blocking read |
| `crates/presenter-server/src/state/integrations.rs` | Pass status event callback to manager |
| `crates/presenter-ui/src/components/stage/ndi_fullscreen.rs` | Replace `<img>` + Blob URL with `<canvas>` + `createImageBitmap()` |
| `tests/e2e/ndi-stage-layout.spec.ts` | Add frame delivery and canvas rendering tests |
| `tests/e2e/video-source-api.spec.ts` | Add discovery stability test |

## Non-Goals

- WebRTC/HLS streaming — MJPEG over WebSocket is correct for LAN
- Audio support — not needed for stage display
- Multiple simultaneous NDI receivers — single active source is sufficient
