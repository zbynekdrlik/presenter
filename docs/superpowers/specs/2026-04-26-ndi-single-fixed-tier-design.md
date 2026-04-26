# NDI Single Fixed-Tier MJPEG — Design

**Issue:** [#250 — ndi stage layout on low cost android tv is too slow](https://github.com/zbynekdrlik/presenter/issues/250) (reopened after PR #263 regressed)

## Problem

The adaptive tier ladder shipped in PR #263 made things worse, not better:

- **Lockstep flapping**: Server-side stalls under N100 CPU pressure (load 2.77 / 4 cores ≈ 69%, vs 17% pre-PR baseline) caused all four cheap-TV connections to register slow ticks at the same wall-clock moment. The per-connection AdaptController interpreted shared server stalls as per-client slowness and demoted everyone in lockstep, cycling L2↔L3 every 22–90 s.
- **Settled at the floor**: All 4 TVs ended at L3 (720p @ 10 fps). 10 fps with visible flapping is unwatchable.
- **Premise was wrong**: The design assumed "any tier downscale is invisibly better than dropped frames at 1080p@30." Real observation: humans see flapping much more than they see internal frame drops. Pre-PR the cheap TVs silently decoded what they could of a 1080p@30 stream and just looked OK; post-PR they look worse.

But pre-PR was *also* unwatchable for sd2/3/4 (Hyundai 1 GB) — the original bug from #250. Reverting just goes back to that broken state. We need a third design.

## Approach

**One stream for everyone, fixed at 720p @ 20 fps, quality 75.** Single shared encoder. No tier ladder, no per-connection adaptive logic, no per-display configuration. The math:

- 720p @ 20 fps = ~6 Mbps. Manageable on cheap WiFi (sd2/3/4's likely real bottleneck).
- 720p decode at 20 fps on Amlogic + 1 GB RAM is well within software JPEG capability (50 ms decode budget per frame).
- 1080p panels upscale 720p with mild text softening — operator-acceptable trade-off (chosen over the alternative of leaving sd1l fast and sd2/3/4 broken).
- One encoder running at 20 fps × ~5 ms (resize + encode with SIMD) = ~10 % of one core. N100 returns to ~baseline.

This is explicitly the **last MJPEG iteration**. The next step (separate issue) is migrating to a non-MJPEG video transport (WebRTC / low-latency HLS / fMP4) for further latency and quality gains. Don't over-engineer this iteration.

## Architecture

```
NDI capture thread (sync, OS thread)
  │  watch::Sender<Option<Arc<VideoFrame>>>   ← raw BGRA/UYVY frames, newest-wins
  ▼
Single encoder task (tokio task)
  - watch::Receiver — wait for new frame
  - frame skip to enforce TARGET_FPS = 20
  - convert UYVY→BGRA if needed
  - SIMD resize to 1280×720 (fast_image_resize, reusable buffer)
  - JPEG encode at quality 75 (turbojpeg)
  - broadcast::Sender<Bytes>
  ▼
mjpeg_http / mjpeg_ws (per connection)
  - broadcast::Receiver
  - forward bytes to client; on Lagged, log debug; on Closed, exit
  - no decision logic, no controller
```

Same shape as pre-PR but with three improvements baked in:

1. **Resolution downscale to 720p** (fixes the original cheap-TV decode problem).
2. **Frame rate throttle to 20 fps** (further reduces decode burden + bandwidth).
3. **SIMD resize** via `fast_image_resize` instead of `image::imageops::resize` (cuts resize cost ~4× on x86 with AVX2 — N100 supports it).

## Code changes

### Delete entirely
| File | Why |
|---|---|
| `crates/presenter-ndi/src/tier.rs` | Tier enum + ladder — no tiers anymore. |
| `crates/presenter-ndi/src/tier_registry.rs` | Lazy ref-counted per-tier encoders — single encoder now. |
| `crates/presenter-server/src/adaptive_mjpeg.rs` | AdaptController, slow-tick threshold — no adaptive logic. |

### Modify
| File | Change |
|---|---|
| `crates/presenter-ndi/src/lib.rs` | Drop `pub mod tier;`, `pub mod tier_registry;`, and re-exports. Keep `manager`, `encoder`, `discovery`, `receiver`, `ndi_sdk`. |
| `crates/presenter-ndi/src/manager.rs` | Restore single-broadcast architecture. Spawn one encode task that consumes from the watch channel (kept) and produces JPEG into a single `broadcast::Sender<Bytes>`. Add `subscribe_frames()` back. Hardcode `TARGET_HEIGHT = 720`, `TARGET_FPS = 20`. |
| `crates/presenter-ndi/src/encoder.rs` | Retarget `encode_bgra_resized` to use `fast_image_resize` with caller-supplied destination buffer (avoid per-frame alloc). Keep `uyvy_to_bgra` pub and `encode_bgra` direct path. |
| `crates/presenter-ndi/Cargo.toml` | Replace `image = "0.25"` with `fast_image_resize = "5"`. |
| `crates/presenter-server/src/main.rs` | Drop `mod adaptive_mjpeg;`. |
| `crates/presenter-server/src/router/integrations/ndi.rs` | Drop `handle_ok_frame`, `handle_lag`, `estimate_dropped`, `FrameDecision`, the unit tests for them. `mjpeg_http` and `mjpeg_ws` revert to short subscribe-and-forward loops calling `manager.subscribe_frames()`. |
| `Cargo.toml` workspace | Bump `version = "0.4.35"`. |

### Keep
- `manager.rs` watch-channel raw-frame distribution between capture and encode threads (clean architecture, low-cost replacement for the old `FrameSlot + Condvar`).
- `encoder.rs::uyvy_to_bgra` public (still useful).
- `encoder.rs::encode_bgra` direct path (single-shot, used by the new resize path).

### Constants
```rust
// crates/presenter-ndi/src/manager.rs
const TARGET_HEIGHT: u32 = 720;
const TARGET_FPS: u32 = 20;
const JPEG_QUALITY: i32 = 75;
```

### Frame-rate throttle (accumulator, not modulus)

Source rate (e.g. 30 fps from Resolume, 60 fps from OBS) is not generally an integer multiple of `TARGET_FPS`. A modulus-based skip can't express 30→20 cleanly. Use a phase accumulator:

```rust
// In the encoder task, per-frame:
phase += TARGET_FPS;                    // u32; e.g. += 20
if phase >= source_fps {
    phase -= source_fps;                // e.g. -= 30
    encode_and_broadcast(frame);
}
// otherwise: skip this raw frame
```

Pattern for 30→20: `skip, emit, emit, skip, emit, emit, …` — 2 of every 3 raw frames pass through, average emit rate exactly 20 fps. Pattern for 60→20: every 3rd frame. The accumulator handles arbitrary source rates without code change.

`source_fps` is read from the NDI frame's `frame_rate_n / frame_rate_d` metadata on each capture; if it changes mid-stream (rare but possible), the accumulator naturally re-stabilises within a few frames.

## SIMD resize details

`fast_image_resize` v5 API:
```rust
use fast_image_resize::{images::Image, IntoImageView, ResizeOptions, Resizer, FilterType};
use fast_image_resize::PixelType;

let src = Image::from_slice_u8(src_w, src_h, &bgra, PixelType::U8x4)?;
let mut dst = Image::new(target_w, target_h, PixelType::U8x4);  // pre-allocated, reused
let mut resizer = Resizer::new();
resizer.resize(&src, &mut dst, &ResizeOptions::new().resize_alg(FilterType::Bilinear))?;
```

The `Resizer` and `dst` `Image` are owned by the encoder task and reused across frames — zero per-frame allocation for the resize path. This is the central performance win vs the `image::imageops::resize` approach used in PR #263 (which allocated a fresh BGRA buffer per frame, ~16 MB churn at 1080p × 30 fps).

`Bilinear` (rather than `Lanczos3`) chosen for cheap CPU; quality difference at typical NDI display sizes is imperceptible. Same trade-off the original PR made with the `image` crate's `Triangle` filter, just SIMD-accelerated.

## Performance target

| Metric | Pre-PR (single 1080p@30) | Post-PR adaptive | This design (720p@20) |
|---|---|---|---|
| N100 load avg | ~0.7 / 4 cores (~17 %) | 2.77 / 4 cores (~69 %) | ≤ 1.0 / 4 cores (~25 %) |
| Encoder CPU per frame | ~3 ms (encode only) | ~10 ms × 4 tiers (resize + encode, alloc churn) | ~3 ms (SIMD resize + encode) |
| Bandwidth per client | ~24 Mbps | varies (3–24 Mbps) | ~6 Mbps |
| Behavior under TV slowness | Silent decode drops | Lockstep flap to floor | Static — cheap TV decodes 20 fps cleanly |

Pass criterion: production load avg returns to baseline (≤ 1.0) within minutes of deploy, and qualitative TV check confirms no flapping.

## Tests

**Keep**:
- `encoder::tests::encode_bgra_resized_passthrough_when_target_equals_source`
- `encoder::tests::encode_bgra_resized_downscales_aspect_preserved` (retarget for fast_image_resize, dims should still match)
- `encoder::tests::uyvy_to_bgra_produces_4bytes_per_pixel`
- Existing `manager::tests` (watch newest-wins, watch starts empty)

**Delete** (the underlying code is gone):
- All 5 `tier::tests`
- All 4 `tier_registry::tests`
- All 10 `adaptive_mjpeg::tests`
- All 12 `router::integrations::ndi::tests` (the helper-function tests added in PR #263)

**Add**:
- `encoder::tests::encode_bgra_resized_reuses_destination_buffer` — pass the same `Resizer` + `dst` Image twice; assert no panic, second encode produces valid JPEG.
- `manager::tests::frame_skip_modulus_targets_20fps` — feed 60 raw frames at 30fps simulated; assert ~40 JPEGs produced (skip every 3rd → 20fps).

Net test count for new code: ~6 (from 35 in PR #263 — most were testing logic we're deleting).

E2E: existing `tests/e2e/stage-api-ndi.spec.ts` and `tests/e2e/ndi-stage-layout.spec.ts` continue to pass — they assert the user-facing `/stage` page renders, which is unaffected by this internal restructure.

## Verification

1. **Local**: `cargo test` (all green), `cargo clippy --workspace --all-targets -- -D warnings -W clippy::all` (no warnings).
2. **Dev deploy**: confirm `/healthz` reports `0.4.35`, fast control client gets ~20 fps × ~6 Mbps, no tier_registry / adaptive log lines (they don't exist anymore), encoder log lines show "encoder started target_height=720 target_fps=20".
3. **Production deploy** (after PR review + user merge instruction): same checks against `http://10.77.9.205`. Confirm `load avg` on the N100 returns to ~1.0 within 5 minutes.
4. **Real TV check**: user observes sd1l..sd4l visually. Pass = "watchable, no flapping". Reportable indicator from server side: `journalctl -u presenter` shows steady JPEG broadcast with no lag warnings.

## Out of scope

- WebRTC / HLS-low-latency / fMP4 transport. Separate issue, separate spec, after this lands and is verified stable.
- Per-display configuration. Explicitly chosen against in option-A discussion; revisit only if WebRTC migration introduces per-device tuning needs.
- Adaptive auto-detection. Removed entirely; if a future bandwidth-constrained scenario needs it, design from scratch with proper server-stall detection.
- Settings UI for `mjpeg_max_height`. Not needed when the value is hardcoded.

## Risks

- **720p loses sharpness on sd1l**: Tesla LEAP-S1 (2 GB) was happy at 1080p@30. Now gets 720p upscaled by the panel. Text remains readable but slightly softer. Operator-accepted trade-off (chose option C with full awareness).
- **20 fps may still be too aggressive for the slowest Hyundai**: if real-TV check shows residual choppiness, drop `TARGET_FPS` to 15 in a one-line follow-up. The design tolerates this knob change without rearchitecting.
- **`fast_image_resize` is a new dep**: well-maintained, single-purpose, MIT-licensed. Replaces a heavier dep (`image`); net dep weight should decrease.
- **Code deletion churn risk**: the diff will *delete* ~700 lines of working code. Any bug in the new path is a regression vs the (broken) production state but the production state is already broken, so the bar is "make it watchable", not "preserve current behavior".

## Decision log

- 720p over 1080p: cheap-TV decode capability (RAM, software JPEG) is the bottleneck; pixel count cut nearly halves decode work.
- 20 fps over 30 fps: bandwidth halves; decode budget per frame doubles to 50 ms (well within Amlogic capability).
- 20 fps over 15 fps: still feels live for text and Resolume-composed graphics; user explicitly approved 20.
- SIMD resize: addresses the second root cause (encoder CPU on N100). Without it, even a single encoder at 720p@20 with `image::imageops::resize` would be ~3× cheaper than the adaptive design but not as cheap as pre-PR. With SIMD, comfortably below baseline.
- No env var / kill switch: hardcoded constants per "this is the last MJPEG iteration before WebRTC migration." YAGNI on configurability.
- Single shared encoder over per-connection: lockstep flapping showed per-connection adaptive doesn't add value; the single-stream architecture is cheaper and predictable.
