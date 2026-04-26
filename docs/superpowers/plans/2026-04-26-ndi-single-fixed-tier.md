# NDI Single Fixed-Tier MJPEG Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the failed adaptive tier ladder (PR #263) with a single shared encoder at fixed 720p @ 20 fps, SIMD-accelerated resize via `fast_image_resize`. Restore N100 to baseline CPU and stop the lockstep flapping that made cheap TVs unwatchable in production.

**Architecture:** One global encoder task (NDI watch-channel → frame-skip accumulator → SIMD resize → JPEG encode → broadcast::Sender). All MJPEG clients subscribe to the same broadcast. No tier ladder, no AdaptController, no per-display config. Stateful `ResizingEncoder` reuses the destination buffer across frames to avoid per-frame allocator churn.

**Tech Stack:** Rust (tokio, axum, broadcast, watch), turbojpeg, **fast_image_resize 5** (replaces `image` crate).

**Spec:** `docs/superpowers/specs/2026-04-26-ndi-single-fixed-tier-design.md` (commit `d7e6451`)

---

## Context

PR #263 shipped the adaptive tier ladder for issue #250 and regressed all four cheap TVs in production. Diagnosis (recorded in spec): four concurrent tier encoders push N100 load to 2.77/4 cores, server-side stalls register as per-connection slow-ticks across all clients in lockstep, the controller demotes everyone to the L3 floor (720p @ 10 fps), and 10 fps with visible flapping is unwatchable.

This plan replaces that machinery with a single fixed encoder at 720p @ 20 fps. Cheap TVs decode 720p @ 20 cleanly; sd1l Tesla loses 1080p sharpness but stays readable; bandwidth halves to ~6 Mbps; N100 returns to ~baseline.

**Last MJPEG iteration before WebRTC migration** — keep scope minimal, don't add knobs, don't preserve the tier infrastructure for "future use".

---

## File Structure

| File | Status | Responsibility |
|---|---|---|
| `Cargo.toml` (workspace) | Modify | Bump `version = "0.4.35"`. |
| `crates/presenter-ndi/Cargo.toml` | Modify | Replace `image = "0.25"` with `fast_image_resize = "5"`. |
| `crates/presenter-ndi/src/encoder.rs` | Modify | Retarget `encode_bgra_resized` to call `fast_image_resize`. Add new `ResizingEncoder` struct with reusable Resizer + dst buffer. Keep `encode_bgra`, `encode_uyvy`, `uyvy_to_bgra` unchanged. |
| `crates/presenter-ndi/src/manager.rs` | Rewrite | Restore single-broadcast architecture: capture thread → watch channel → encode task (uses `ResizingEncoder`, frame-skip accumulator, broadcasts JPEG). Re-add `pub fn subscribe_frames() -> broadcast::Receiver<Bytes>`. Drop `Tier`/`TierRegistry` references. |
| `crates/presenter-ndi/src/lib.rs` | Modify | Drop `pub mod tier;`, `pub mod tier_registry;`, `pub use tier::*`, `pub use tier_registry::*`. Keep manager + encoder + receiver + discovery + ndi_sdk re-exports. |
| `crates/presenter-ndi/src/tier.rs` | **Delete** | Tier enum no longer needed. |
| `crates/presenter-ndi/src/tier_registry.rs` | **Delete** | TierRegistry no longer needed. |
| `crates/presenter-server/src/main.rs` | Modify | Drop `mod adaptive_mjpeg;`. |
| `crates/presenter-server/src/adaptive_mjpeg.rs` | **Delete** | AdaptController no longer needed. |
| `crates/presenter-server/src/router/integrations/ndi.rs` | Rewrite | Drop `handle_ok_frame`/`handle_lag`/`estimate_dropped`/`FrameDecision` and their tests. `mjpeg_http` and `mjpeg_ws` become thin subscribe-to-broadcast forwarders calling `manager.subscribe_frames()`. |

---

## Task 1: Workspace prep — version bump + fast_image_resize dep swap

**Files:**
- Modify: `Cargo.toml:15`
- Modify: `crates/presenter-ndi/Cargo.toml:9-20`

- [ ] **Step 1: Bump workspace version**

In `Cargo.toml`, change line 15 from `version = "0.4.34"` to `version = "0.4.35"`.

- [ ] **Step 2: Swap `image` for `fast_image_resize` in presenter-ndi**

In `crates/presenter-ndi/Cargo.toml`, find the line:
```toml
image = { version = "0.25", default-features = false }
```
Replace with:
```toml
fast_image_resize = "5"
```

- [ ] **Step 3: Verify deps resolve**

```bash
cargo build -p presenter-ndi 2>&1 | tail -5
```

Expected: `error[E0432]` complaining that `image::ImageBuffer` is not found in `encoder.rs`. **This is expected** — Task 2 retargets `encode_bgra_resized` to use the new dep. Do not fix the error in this task.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock crates/presenter-ndi/Cargo.toml
git commit -m "chore: bump version to 0.4.35 and swap image for fast_image_resize (#250)"
```

---

## Task 2: Retarget `encode_bgra_resized` to fast_image_resize

**Files:**
- Modify: `crates/presenter-ndi/src/encoder.rs`

This task retains the existing `encode_bgra_resized(bgra, src_w, src_h, target_h) -> Result<Vec<u8>>` API but routes it through `fast_image_resize`. Per-call allocation persists for now — Task 3 adds the stateful struct that reuses buffers. Existing tests for `encode_bgra_resized` continue to pass after this task.

- [ ] **Step 1: Replace the `encode_bgra_resized` impl block**

In `crates/presenter-ndi/src/encoder.rs`, find the second `impl JpegEncoder` block (the one with `encode_bgra_resized`) and replace it with:

```rust
impl JpegEncoder {
    /// Resize BGRA pixel data to `target_height` (preserving aspect) and JPEG-encode.
    ///
    /// If `src_height == target_height`, this is a fast path that skips resize.
    /// Otherwise uses `fast_image_resize` with the `Bilinear` filter, chosen
    /// for cheap CPU cost over Lanczos quality (the difference is imperceptible
    /// at typical NDI display sizes). Allocates a fresh destination buffer per
    /// call — for hot loops that want buffer reuse, see `ResizingEncoder` below.
    pub fn encode_bgra_resized(
        &self,
        bgra: &[u8],
        src_width: u32,
        src_height: u32,
        target_height: u32,
    ) -> Result<Vec<u8>> {
        if target_height == src_height {
            return self.encode_bgra(bgra, src_width, src_height);
        }
        let target_width = (src_width * target_height) / src_height;
        // Make even — turbojpeg with Sub2x2 chroma requires even dims.
        let target_width = target_width & !1;
        let target_height = target_height & !1;

        let expected = (src_width as usize) * (src_height as usize) * 4;
        if bgra.len() < expected {
            return Err(anyhow::anyhow!(
                "BGRA buffer size mismatch: {} bytes for {}x{}",
                bgra.len(),
                src_width,
                src_height
            ));
        }

        let src = fast_image_resize::images::ImageRef::new(
            src_width,
            src_height,
            bgra,
            fast_image_resize::PixelType::U8x4,
        )
        .map_err(|e| anyhow::anyhow!("fast_image_resize source error: {e}"))?;

        let mut dst = fast_image_resize::images::Image::new(
            target_width,
            target_height,
            fast_image_resize::PixelType::U8x4,
        );

        let mut resizer = fast_image_resize::Resizer::new();
        resizer
            .resize(
                &src,
                &mut dst,
                &fast_image_resize::ResizeOptions::new()
                    .resize_alg(fast_image_resize::ResizeAlg::Convolution(
                        fast_image_resize::FilterType::Bilinear,
                    )),
            )
            .map_err(|e| anyhow::anyhow!("fast_image_resize error: {e}"))?;

        self.encode_bgra(dst.buffer(), target_width, target_height)
    }
}
```

- [ ] **Step 2: Run existing encoder tests to verify behaviour preserved**

```bash
cargo test -p presenter-ndi encoder:: 2>&1 | tail -15
```

Expected: 4 passed (`encode_bgra_resized_passthrough_when_target_equals_source`, `encode_bgra_resized_downscales_aspect_preserved`, `encode_bgra_resized_rejects_wrong_buffer_size`, `uyvy_to_bgra_produces_4bytes_per_pixel`).

If `encode_bgra_resized_rejects_wrong_buffer_size` fails because the error message changed, accept the new wording — adjust the test assertion to match (`assert!(err.to_string().contains("buffer size mismatch") || err.to_string().contains("BGRA buffer"))`).

- [ ] **Step 3: Commit**

```bash
git add crates/presenter-ndi/src/encoder.rs
git commit -m "feat(ndi): retarget encode_bgra_resized to fast_image_resize SIMD (#250)"
```

---

## Task 3: Add `ResizingEncoder` with reusable destination buffer

**Files:**
- Modify: `crates/presenter-ndi/src/encoder.rs`

The hot path (single global encoder running 20 fps continuously) wants zero per-frame allocation. `ResizingEncoder` owns the `Resizer`, the destination `Image`, and the `JpegEncoder`, reusing all three across frames. The `dst` buffer is recreated only when target dimensions change (effectively never in this design — fixed 720p target).

- [ ] **Step 1: Append `ResizingEncoder` to `encoder.rs`**

After the second `impl JpegEncoder { ... }` block (the one added in Task 2), append:

```rust
/// Stateful encoder for the hot loop: owns a `fast_image_resize::Resizer`,
/// a destination buffer, and a `JpegEncoder`, and reuses them across calls
/// so the per-frame path does zero allocation for resize state.
///
/// The destination buffer is reallocated only if the target dimensions change
/// (i.e. when the source resolution changes mid-stream — uncommon in practice).
pub struct ResizingEncoder {
    encoder: JpegEncoder,
    resizer: fast_image_resize::Resizer,
    dst: Option<fast_image_resize::images::Image<'static>>,
    target_height: u32,
}

impl ResizingEncoder {
    pub fn new(quality: i32, target_height: u32) -> Self {
        Self {
            encoder: JpegEncoder::new(quality),
            resizer: fast_image_resize::Resizer::new(),
            dst: None,
            target_height,
        }
    }

    /// Encode a BGRA frame, resizing to the configured target height if needed.
    /// Reuses the internal destination buffer across calls when `src_width × scaled_target_height`
    /// matches the previously-cached output dimensions.
    pub fn encode(&mut self, bgra: &[u8], src_width: u32, src_height: u32) -> Result<Vec<u8>> {
        if src_height == self.target_height {
            return self.encoder.encode_bgra(bgra, src_width, src_height);
        }
        let target_width = ((src_width * self.target_height) / src_height) & !1;
        let target_height = self.target_height & !1;

        let expected_src = (src_width as usize) * (src_height as usize) * 4;
        if bgra.len() < expected_src {
            return Err(anyhow::anyhow!(
                "BGRA buffer size mismatch: {} bytes for {}x{}",
                bgra.len(),
                src_width,
                src_height
            ));
        }

        let need_new_dst = match &self.dst {
            None => true,
            Some(dst) => dst.width() != target_width || dst.height() != target_height,
        };
        if need_new_dst {
            self.dst = Some(fast_image_resize::images::Image::new(
                target_width,
                target_height,
                fast_image_resize::PixelType::U8x4,
            ));
        }
        let dst = self.dst.as_mut().unwrap();

        let src = fast_image_resize::images::ImageRef::new(
            src_width,
            src_height,
            bgra,
            fast_image_resize::PixelType::U8x4,
        )
        .map_err(|e| anyhow::anyhow!("fast_image_resize source error: {e}"))?;

        self.resizer
            .resize(
                &src,
                dst,
                &fast_image_resize::ResizeOptions::new()
                    .resize_alg(fast_image_resize::ResizeAlg::Convolution(
                        fast_image_resize::FilterType::Bilinear,
                    )),
            )
            .map_err(|e| anyhow::anyhow!("fast_image_resize error: {e}"))?;

        self.encoder.encode_bgra(dst.buffer(), target_width, target_height)
    }
}
```

Replace the existing `mod tests` block at the bottom of `encoder.rs` (the one that has the four `encode_bgra_resized_*` and `uyvy_to_bgra_*` tests) with:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn make_bgra(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                out.push((x % 256) as u8);
                out.push((y % 256) as u8);
                out.push(((x + y) % 256) as u8);
                out.push(255);
            }
        }
        out
    }

    #[test]
    fn encode_bgra_resized_passthrough_when_target_equals_source() {
        let bgra = make_bgra(64, 64);
        let enc = JpegEncoder::new(75);
        let jpeg = enc.encode_bgra_resized(&bgra, 64, 64, 64).unwrap();
        assert!(jpeg.starts_with(&[0xff, 0xd8, 0xff]));
    }

    #[test]
    fn encode_bgra_resized_downscales_aspect_preserved() {
        let bgra = make_bgra(1920, 1080);
        let enc = JpegEncoder::new(75);
        let jpeg = enc.encode_bgra_resized(&bgra, 1920, 1080, 720).unwrap();
        assert!(jpeg.starts_with(&[0xff, 0xd8, 0xff]));

        let img = turbojpeg::decompress(&jpeg, turbojpeg::PixelFormat::BGRA).unwrap();
        assert_eq!(img.height, 720);
        assert_eq!(img.width, 1280);
    }

    #[test]
    fn encode_bgra_resized_rejects_wrong_buffer_size() {
        let bgra = vec![0u8; 16];
        let enc = JpegEncoder::new(75);
        let err = enc.encode_bgra_resized(&bgra, 100, 100, 50).unwrap_err();
        assert!(err.to_string().contains("buffer size mismatch"));
    }

    #[test]
    fn uyvy_to_bgra_produces_4bytes_per_pixel() {
        let uyvy = vec![128u8; 4 * 2 * 2];
        let bgra = uyvy_to_bgra(&uyvy, 4, 2);
        assert_eq!(bgra.len(), 4 * 2 * 4);
    }

    #[test]
    fn resizing_encoder_passthrough_when_dims_match() {
        let bgra = make_bgra(64, 64);
        let mut enc = ResizingEncoder::new(75, 64);
        let jpeg = enc.encode(&bgra, 64, 64).unwrap();
        assert!(jpeg.starts_with(&[0xff, 0xd8, 0xff]));
    }

    #[test]
    fn resizing_encoder_reuses_destination_buffer_across_calls() {
        let bgra = make_bgra(1920, 1080);
        let mut enc = ResizingEncoder::new(75, 720);

        let jpeg1 = enc.encode(&bgra, 1920, 1080).unwrap();
        assert!(jpeg1.starts_with(&[0xff, 0xd8, 0xff]));

        // Second call: dst dims unchanged, internal buffer should be reused.
        let jpeg2 = enc.encode(&bgra, 1920, 1080).unwrap();
        assert!(jpeg2.starts_with(&[0xff, 0xd8, 0xff]));

        // Both decode to the same target dims.
        for jpeg in [&jpeg1, &jpeg2] {
            let img = turbojpeg::decompress(jpeg, turbojpeg::PixelFormat::BGRA).unwrap();
            assert_eq!(img.height, 720);
            assert_eq!(img.width, 1280);
        }
    }

    #[test]
    fn resizing_encoder_rebuilds_dst_when_source_dims_change() {
        // Constructed once, fed two different source resolutions — must succeed both times.
        let mut enc = ResizingEncoder::new(75, 720);
        let bgra_1080 = make_bgra(1920, 1080);
        let bgra_4k = make_bgra(2560, 1440);

        let j1 = enc.encode(&bgra_1080, 1920, 1080).unwrap();
        assert!(j1.starts_with(&[0xff, 0xd8, 0xff]));

        let j2 = enc.encode(&bgra_4k, 2560, 1440).unwrap();
        assert!(j2.starts_with(&[0xff, 0xd8, 0xff]));

        // 1440p source → 720 target ⇒ width = 2560 * 720 / 1440 = 1280, even-aligned.
        let img = turbojpeg::decompress(&j2, turbojpeg::PixelFormat::BGRA).unwrap();
        assert_eq!(img.height, 720);
        assert_eq!(img.width, 1280);
    }
}
```

- [ ] **Step 2: Run encoder tests**

```bash
cargo test -p presenter-ndi encoder:: 2>&1 | tail -20
```

Expected: 7 tests pass (4 existing + 3 new for ResizingEncoder).

- [ ] **Step 3: Commit**

```bash
git add crates/presenter-ndi/src/encoder.rs
git commit -m "feat(ndi): add ResizingEncoder with reusable destination buffer (#250)"
```

---

## Task 4: Atomic teardown — rewrite manager.rs + ndi.rs, delete obsolete files

**Files:**
- Rewrite: `crates/presenter-ndi/src/manager.rs`
- Modify: `crates/presenter-ndi/src/lib.rs`
- **Delete**: `crates/presenter-ndi/src/tier.rs`
- **Delete**: `crates/presenter-ndi/src/tier_registry.rs`
- Rewrite: `crates/presenter-server/src/router/integrations/ndi.rs`
- Modify: `crates/presenter-server/src/main.rs`
- **Delete**: `crates/presenter-server/src/adaptive_mjpeg.rs`

This is one atomic task because the changes are interdependent — `manager.rs` drops `subscribe_tier`/`Tier` in the same step that `ndi.rs` drops the calls to them. Splitting would leave the workspace in a non-compilable state.

- [ ] **Step 1: Overwrite `crates/presenter-ndi/src/manager.rs`**

```rust
use std::sync::Arc;

use anyhow::Result;
use bytes::Bytes;
use tokio::sync::{broadcast, watch, Mutex};

use crate::discovery::{self, FinderShutdown, SourceList};
use crate::encoder::{uyvy_to_bgra, ResizingEncoder};
use crate::ndi_sdk::NdiLib;
use crate::receiver::{NdiReceiver, VideoFrame};

const TARGET_HEIGHT: u32 = 720;
const TARGET_FPS: u32 = 20;
const JPEG_QUALITY: i32 = 75;
const JPEG_BROADCAST_CAPACITY: usize = 8;

/// Callback for reporting NDI connection status changes.
pub type StatusCallback = Arc<dyn Fn(String) + Send + Sync>;

struct ActiveStream {
    stop_signal: watch::Sender<bool>,
    capture_thread: Option<std::thread::JoinHandle<()>>,
    encode_task: Option<tokio::task::JoinHandle<()>>,
}

/// Orchestrates NDI discovery, capture, and a single shared MJPEG broadcast.
///
/// Discovery runs in a persistent background thread (mDNS source list).
/// Capture runs in an OS thread that publishes raw frames to a `tokio::sync::watch`
/// channel; one async encode task consumes them, applies a frame-rate accumulator
/// to throttle to `TARGET_FPS`, resizes to `TARGET_HEIGHT` via `ResizingEncoder`,
/// JPEG-encodes at quality `JPEG_QUALITY`, and broadcasts to all connected clients.
pub struct NdiManager {
    sdk: Arc<NdiLib>,
    source_list: SourceList,
    _finder_shutdown: FinderShutdown,
    active_stream: Mutex<Option<ActiveStream>>,
    raw_frame_tx: watch::Sender<Option<Arc<VideoFrame>>>,
    raw_frame_rx: watch::Receiver<Option<Arc<VideoFrame>>>,
    jpeg_tx: broadcast::Sender<Bytes>,
}

impl NdiManager {
    /// Try to create a new manager by loading the NDI SDK.
    ///
    /// Returns `None` if the NDI runtime is not available on this system.
    pub fn try_new() -> Option<Self> {
        let sdk = NdiLib::load().ok()?;
        let sdk = Arc::new(sdk);
        let (source_list, finder_shutdown) =
            discovery::spawn_persistent_finder(Arc::clone(&sdk));
        let (raw_frame_tx, raw_frame_rx) = watch::channel(None);
        let (jpeg_tx, _) = broadcast::channel(JPEG_BROADCAST_CAPACITY);
        Some(Self {
            sdk,
            source_list,
            _finder_shutdown: finder_shutdown,
            active_stream: Mutex::new(None),
            raw_frame_tx,
            raw_frame_rx,
            jpeg_tx,
        })
    }

    pub fn is_available(&self) -> bool {
        true
    }

    pub fn discover_sources(
        &self,
        _timeout_ms: u32,
    ) -> Result<Vec<discovery::NdiSourceInfo>> {
        Ok(self.source_list.read())
    }

    /// Subscribe to the single shared JPEG broadcast.
    pub fn subscribe_frames(&self) -> broadcast::Receiver<Bytes> {
        self.jpeg_tx.subscribe()
    }

    /// Start capturing from the named NDI source.
    pub async fn start_stream(
        &self,
        ndi_name: &str,
        status_cb: Option<StatusCallback>,
    ) -> Result<()> {
        self.stop_stream().await;

        let sdk = Arc::clone(&self.sdk);
        let raw_tx = self.raw_frame_tx.clone();
        let source_name = ndi_name.to_string();
        let (stop_tx, stop_rx) = watch::channel(false);

        let capture_thread = std::thread::Builder::new()
            .name("ndi-capture".into())
            .spawn({
                let stop_rx = stop_rx.clone();
                move || {
                    run_capture_thread(sdk, source_name, raw_tx, stop_rx, status_cb);
                }
            })?;

        let encode_task = tokio::spawn(run_encode_task(
            self.raw_frame_rx.clone(),
            self.jpeg_tx.clone(),
            stop_rx,
        ));

        let mut active = self.active_stream.lock().await;
        *active = Some(ActiveStream {
            stop_signal: stop_tx,
            capture_thread: Some(capture_thread),
            encode_task: Some(encode_task),
        });

        Ok(())
    }

    pub async fn is_streaming(&self) -> bool {
        self.active_stream.lock().await.is_some()
    }

    pub async fn stop_stream(&self) {
        let mut active = self.active_stream.lock().await;
        if let Some(mut stream) = active.take() {
            let _ = stream.stop_signal.send(true);
            let _ = self.raw_frame_tx.send(None);
            if let Some(h) = stream.capture_thread.take() {
                let _ = h.join();
            }
            if let Some(h) = stream.encode_task.take() {
                h.abort();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Capture thread
// ---------------------------------------------------------------------------

fn run_capture_thread(
    sdk: Arc<NdiLib>,
    source_name: String,
    raw_tx: watch::Sender<Option<Arc<VideoFrame>>>,
    mut stop_rx: watch::Receiver<bool>,
    status_cb: Option<StatusCallback>,
) {
    let receiver = match NdiReceiver::connect(&sdk, &source_name, 10) {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("failed to connect to NDI source '{source_name}': {e}");
            if let Some(cb) = &status_cb {
                cb("disconnected".to_string());
            }
            return;
        }
    };

    let mut connected = false;
    let mut last_frame_time = std::time::Instant::now();
    let mut capture_timeout_ms: u32 = 50;

    tracing::info!("NDI capture thread started for '{source_name}'");

    loop {
        if *stop_rx.borrow() {
            break;
        }

        match receiver.capture_video(capture_timeout_ms) {
            Ok(Some(frame)) => {
                if frame.frame_rate_d > 0 && frame.frame_rate_n > 0 {
                    let period = (1000 * frame.frame_rate_d as u64)
                        / frame.frame_rate_n as u64;
                    capture_timeout_ms = (period as u32).clamp(16, 200);
                }

                if !connected {
                    connected = true;
                    tracing::info!(
                        "NDI connected: {}x{} @ {}/{}fps",
                        frame.width,
                        frame.height,
                        frame.frame_rate_n,
                        frame.frame_rate_d
                    );
                    if let Some(cb) = &status_cb {
                        cb("connected".to_string());
                    }
                }
                last_frame_time = std::time::Instant::now();

                let _ = raw_tx.send(Some(Arc::new(frame)));
            }
            Ok(None) => {
                if connected
                    && last_frame_time.elapsed() > std::time::Duration::from_secs(3)
                {
                    connected = false;
                    tracing::warn!("NDI signal lost for '{source_name}'");
                    if let Some(cb) = &status_cb {
                        cb("disconnected".to_string());
                    }
                }
                if stop_rx.has_changed().unwrap_or(false)
                    && *stop_rx.borrow_and_update()
                {
                    break;
                }
            }
            Err(e) => {
                tracing::error!("NDI capture error: {e}");
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        }
    }

    tracing::info!("NDI capture thread stopped");
}

// ---------------------------------------------------------------------------
// Encode task — frame-skip accumulator, SIMD resize, JPEG encode, broadcast.
// ---------------------------------------------------------------------------

async fn run_encode_task(
    mut raw_rx: watch::Receiver<Option<Arc<VideoFrame>>>,
    jpeg_tx: broadcast::Sender<Bytes>,
    mut stop_rx: watch::Receiver<bool>,
) {
    let fourcc_uyvy = u32::from_le_bytes([b'U', b'Y', b'V', b'Y']);
    let fourcc_bgra = u32::from_le_bytes([b'B', b'G', b'R', b'A']);
    let fourcc_bgrx = u32::from_le_bytes([b'B', b'G', b'R', b'X']);
    let mut encoder = ResizingEncoder::new(JPEG_QUALITY, TARGET_HEIGHT);

    // Frame-skip phase accumulator: emit when phase >= source_fps.
    let mut phase: u64 = 0;

    tracing::info!(
        target_height = TARGET_HEIGHT,
        target_fps = TARGET_FPS,
        "NDI encode task started"
    );

    loop {
        tokio::select! {
            res = stop_rx.changed() => {
                if res.is_err() || *stop_rx.borrow() { break; }
            }
            res = raw_rx.changed() => {
                if res.is_err() { break; }
            }
        }

        let frame = match raw_rx.borrow_and_update().as_ref() {
            Some(f) => Arc::clone(f),
            None => continue,
        };

        // Compute source fps (Resolume sends 30/1 typically). Fall back to
        // TARGET_FPS if metadata is missing/zero — that means "emit every frame".
        let source_fps: u64 = if frame.frame_rate_d > 0 && frame.frame_rate_n > 0 {
            (frame.frame_rate_n as u64) / (frame.frame_rate_d as u64).max(1)
        } else {
            TARGET_FPS as u64
        };
        let source_fps = source_fps.max(TARGET_FPS as u64);

        phase += TARGET_FPS as u64;
        if phase < source_fps {
            continue;
        }
        phase -= source_fps;

        let (bgra, w, h) = if frame.fourcc == fourcc_bgra || frame.fourcc == fourcc_bgrx {
            (frame.data.clone(), frame.width, frame.height)
        } else if frame.fourcc == fourcc_uyvy {
            (
                uyvy_to_bgra(&frame.data, frame.width, frame.height),
                frame.width,
                frame.height,
            )
        } else {
            tracing::warn!(
                fourcc = format!("0x{:08x}", frame.fourcc),
                "unsupported fourcc; skipping"
            );
            continue;
        };

        match encoder.encode(&bgra, w, h) {
            Ok(jpeg) => {
                let _ = jpeg_tx.send(Bytes::from(jpeg));
            }
            Err(e) => {
                tracing::error!("JPEG encode error: {e}");
            }
        }
    }

    tracing::info!("NDI encode task stopped");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_frame(id: u32, fourcc: u32, w: u32, h: u32) -> VideoFrame {
        VideoFrame {
            width: w,
            height: h,
            data: vec![id as u8; (w * h * 4) as usize],
            stride: w * 4,
            fourcc,
            frame_rate_n: 30,
            frame_rate_d: 1,
        }
    }

    #[test]
    fn watch_newest_wins() {
        let (tx, mut rx) = watch::channel::<Option<Arc<VideoFrame>>>(None);
        tx.send(Some(Arc::new(make_frame(
            1,
            u32::from_le_bytes([b'B', b'G', b'R', b'A']),
            1,
            1,
        ))))
        .unwrap();
        tx.send(Some(Arc::new(make_frame(
            2,
            u32::from_le_bytes([b'B', b'G', b'R', b'A']),
            1,
            1,
        ))))
        .unwrap();
        // After multiple sends, watch holds only the newest (data filled with id=2).
        let snap = rx.borrow_and_update();
        assert!(snap.as_ref().unwrap().data.iter().all(|&b| b == 2));
    }

    #[test]
    fn watch_starts_empty() {
        let (_tx, rx) = watch::channel::<Option<Arc<VideoFrame>>>(None);
        assert!(rx.borrow().is_none());
    }

    /// 30 fps source → TARGET_FPS=20 should produce 2 emits per 3 raw frames.
    /// Pure-arithmetic check of the accumulator.
    #[test]
    fn frame_skip_accumulator_30_to_20_emits_2_of_3() {
        let mut phase: u64 = 0;
        let source_fps: u64 = 30;
        let target_fps: u64 = TARGET_FPS as u64;
        let mut emits = 0;
        let total = 30; // 30 raw frames = 1 second @ 30fps.
        for _ in 0..total {
            phase += target_fps;
            if phase < source_fps {
                continue;
            }
            phase -= source_fps;
            emits += 1;
        }
        assert_eq!(
            emits, 20,
            "30→20 fps accumulator should emit exactly 20 of 30 frames"
        );
    }

    /// 60 fps source → TARGET_FPS=20 should produce 1 emit per 3 raw frames.
    #[test]
    fn frame_skip_accumulator_60_to_20_emits_1_of_3() {
        let mut phase: u64 = 0;
        let source_fps: u64 = 60;
        let target_fps: u64 = TARGET_FPS as u64;
        let mut emits = 0;
        let total = 60;
        for _ in 0..total {
            phase += target_fps;
            if phase < source_fps {
                continue;
            }
            phase -= source_fps;
            emits += 1;
        }
        assert_eq!(
            emits, 20,
            "60→20 fps accumulator should emit exactly 20 of 60 frames"
        );
    }
}
```

- [ ] **Step 2: Update `crates/presenter-ndi/src/lib.rs`**

Overwrite with:

```rust
#![allow(non_camel_case_types)]

pub mod discovery;
pub mod encoder;
mod manager;
pub mod ndi_sdk;
pub mod receiver;

pub use discovery::SourceList;
pub use manager::NdiManager;
pub use manager::StatusCallback;
```

- [ ] **Step 3: Delete `tier.rs` and `tier_registry.rs`**

```bash
git rm crates/presenter-ndi/src/tier.rs crates/presenter-ndi/src/tier_registry.rs
```

- [ ] **Step 4: Overwrite `crates/presenter-server/src/router/integrations/ndi.rs`**

```rust
use axum::http::header;
use axum::{
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    response::IntoResponse,
    Json,
};
use bytes::Bytes;
use serde::Serialize;
use tokio::sync::broadcast::error::RecvError;
use tracing::instrument;

use super::super::AppError;
use crate::state::AppState;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NdiSourceDto {
    name: String,
}

#[instrument(skip_all)]
pub(crate) async fn discover_ndi_sources(
    State(state): State<AppState>,
) -> Result<Json<Vec<NdiSourceDto>>, AppError> {
    let manager = state
        .ndi_manager()
        .ok_or_else(|| AppError::service_unavailable("NDI SDK not available"))?;
    let sources = manager.discover_sources(0)?;
    Ok(Json(
        sources
            .into_iter()
            .map(|s| NdiSourceDto { name: s.name })
            .collect(),
    ))
}

#[instrument(skip_all)]
pub(crate) async fn ndi_status(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "available": state.ndi_manager().is_some() }))
}

/// WebSocket endpoint that streams JPEG frames from the single shared encoder.
pub(crate) async fn mjpeg_ws(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let manager = state
        .ndi_manager()
        .ok_or_else(|| AppError::service_unavailable("NDI SDK not available"))?;
    let rx = manager.subscribe_frames();
    Ok(ws.on_upgrade(move |socket| handle_mjpeg_ws(socket, rx)))
}

async fn handle_mjpeg_ws(
    mut socket: WebSocket,
    mut rx: tokio::sync::broadcast::Receiver<Bytes>,
) {
    loop {
        match rx.recv().await {
            Ok(jpeg) => {
                if socket
                    .send(Message::Binary(jpeg.to_vec().into()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            Err(RecvError::Lagged(n)) => {
                tracing::debug!(lag = n, "MJPEG WS client lagged");
            }
            Err(RecvError::Closed) => break,
        }
    }
}

/// HTTP MJPEG stream using multipart/x-mixed-replace.
///
/// Browsers render this natively in an `<img>` tag with no JS overhead.
/// Same idea IP cameras have used for streaming MJPEG for decades.
pub(crate) async fn mjpeg_http(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let manager = state
        .ndi_manager()
        .ok_or_else(|| AppError::service_unavailable("NDI SDK not available"))?;
    let rx = manager.subscribe_frames();
    let boundary = "mjpegboundary";
    let content_type = format!("multipart/x-mixed-replace; boundary={boundary}");

    let stream = async_stream::stream! {
        let mut rx = rx;
        loop {
            match rx.recv().await {
                Ok(jpeg) => {
                    let part_header = format!(
                        "--{boundary}\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
                        jpeg.len()
                    );
                    yield Ok::<Bytes, std::convert::Infallible>(Bytes::from(part_header));
                    yield Ok(jpeg);
                    yield Ok(Bytes::from("\r\n"));
                }
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            }
        }
    };

    let body = axum::body::Body::from_stream(stream);
    Ok((
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "no-cache".to_string()),
            (header::CONNECTION, "keep-alive".to_string()),
        ],
        body,
    ))
}
```

- [ ] **Step 5: Update `crates/presenter-server/src/main.rs`**

Find the line `mod adaptive_mjpeg;` (added in PR #263 near the other `mod` declarations) and delete it. Run:

```bash
grep -n "^mod adaptive_mjpeg" crates/presenter-server/src/main.rs
```

If a line is shown, remove just that one line. The other `mod ai;` etc. declarations are unchanged.

- [ ] **Step 6: Delete `adaptive_mjpeg.rs`**

```bash
git rm crates/presenter-server/src/adaptive_mjpeg.rs
```

- [ ] **Step 7: Build and run all workspace tests**

```bash
cargo build --workspace 2>&1 | tail -10
```

Expected: `Finished `dev` profile`. If errors:
- Errors in `crates/presenter-ndi/src/encoder.rs` mentioning `image::` → make sure Task 2 retargeted `encode_bgra_resized` correctly.
- Errors in `crates/presenter-server/src/state/mod.rs` referencing `subscribe_tier` → no, that file doesn't reference Tier; if it does, that's a Task 7 cleanup, fix here.
- Errors mentioning `adaptive_mjpeg` → make sure `mod adaptive_mjpeg;` was removed from main.rs.

Then:

```bash
cargo test --workspace 2>&1 | tail -20
```

Expected: all green. The Tier / TierRegistry / AdaptController / handler tests are gone (their files are deleted); 7 encoder tests + 4 manager tests + everything else from the workspace remain.

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
feat(ndi): replace adaptive tier ladder with single fixed 720p@20 encoder (#250)

Tear down PR #263's adaptive infrastructure that regressed all 4
production cheap TVs to a flapping L2/L3 floor. Replace with one
shared encoder at fixed 720p @ 20 fps quality 75, using the
ResizingEncoder added in the previous commits. mjpeg_http and
mjpeg_ws revert to thin subscribe-and-forward loops.

Deletes:
- crates/presenter-ndi/src/tier.rs (Tier enum, ladder, tests)
- crates/presenter-ndi/src/tier_registry.rs (lazy ref-counted encoders, tests)
- crates/presenter-server/src/adaptive_mjpeg.rs (AdaptController, slow-tick, tests)
- mod adaptive_mjpeg from main.rs
- pub mod tier / tier_registry from lib.rs
- handle_ok_frame / handle_lag / estimate_dropped / FrameDecision and 12 tests

Net diff: ~700 lines deleted, ~300 added. Server cost predicted to
return to ~17% load avg on N100 (was 69% with adaptive). All TVs
now get the same predictable 720p@20 stream — no per-connection
state, no lockstep flapping.
EOF
)"
```

---

## Task 5: Local fmt + clippy + tests

**Files:** None (verification step).

- [ ] **Step 1: Format**

```bash
cargo fmt --all
```

- [ ] **Step 2: Clippy zero-warnings across workspace**

```bash
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all 2>&1 | tail -20
```

Expected: clean. Common things to fix if they appear:
- `clippy::needless_borrow` on the new fast_image_resize calls — drop the `&` if flagged.
- `clippy::expect_fun_call` if any `.expect(format!(...))` slipped in — convert to `unwrap_or_else(|_| panic!(...))`.

- [ ] **Step 3: Full workspace tests**

```bash
cargo test --workspace 2>&1 | tail -10
```

Expected: all green. Total test count drops by ~25-30 vs PR #263 (the deleted Tier/Registry/AdaptController/handler tests).

- [ ] **Step 4: If any of Steps 2 or 3 produced fixes, commit**

```bash
git add -A
git commit -m "chore: fmt + clippy fixes for single-tier MJPEG (#250)"
```

If no diff after fmt/clippy, skip the commit.

---

## Task 6: Push to dev + monitor CI

**Files:** None.

- [ ] **Step 1: Sync with main, then push**

```bash
git fetch origin
git merge origin/main --no-edit 2>&1 | tail -3
git push origin dev 2>&1 | tail -3
```

If merge produces conflicts, stop and resolve manually. (Unlikely — main hasn't changed since the previous merge.)

- [ ] **Step 2: Identify the new pipeline run**

```bash
sleep 10
gh run list --branch dev --limit 3 --json databaseId,name,status,event --jq '.[] | "\(.databaseId)\t\(.name)\t\(.status)\t\(.event)"'
```

Capture the `databaseId` of the newest `Pipeline` row triggered by `push`.

- [ ] **Step 3: Monitor with single-sleep pattern**

```bash
RUN_ID=<paste databaseId>
sleep 1500 && gh run view $RUN_ID --json status,conclusion,jobs --jq '{status,conclusion,jobs:[.jobs[]|{name,conclusion,status}]}'
```

Run the above as a `run_in_background: true` Bash invocation. After it returns, if `Mutation Testing` or any job is still `in_progress`, schedule another `sleep 600 && gh run view $RUN_ID ...` background command. **Do NOT poll repeatedly. Do NOT use `gh run watch`.**

If any job fails, run `gh run view $RUN_ID --log-failed | tail -100`, fix in ONE commit, push, monitor again.

- [ ] **Step 4: Confirm deploy-dev succeeded**

After the run reaches `conclusion=success`:

```bash
gh run view $RUN_ID --json jobs --jq '.jobs[] | select(.name=="Deploy to Dev") | .conclusion'
```

Expected: `success`.

---

## Task 7: Verify on dev

**Files:** None (live check against `http://10.77.8.134:8080`).

- [ ] **Step 1: Confirm dev is on 0.4.35 and stream is alive**

```bash
curl -s http://10.77.8.134:8080/healthz; echo
```

Expected: `{"channel":"dev","status":"ok","version":"0.4.35"}`.

- [ ] **Step 2: Confirm cg-obs is the active source**

```bash
curl -s http://10.77.8.134:8080/integrations/video-sources | python3 -c "
import sys,json
d=json.load(sys.stdin)
for x in d:
    if x['isActive']:
        print(f\"active: {x['label']} ({x['ndiName']})\")"
```

If no active source, activate via the settings page or API before continuing.

- [ ] **Step 3: Measure FPS + bandwidth from a fast control client**

```bash
python3 - <<'PY'
import urllib.request, time
url='http://10.77.8.134:8080/ndi/mjpeg'
r=urllib.request.urlopen(url, timeout=5)
start=time.time(); buf=b''; frames=0; total=0
while time.time()-start<10:
    chunk=r.read(65536)
    if not chunk: break
    total+=len(chunk); buf+=chunk
    while True:
        i=buf.find(b'\xff\xd8\xff'); j=buf.find(b'\xff\xd9',i+3) if i>=0 else -1
        if i<0 or j<0: break
        frames+=1; buf=buf[j+2:]
print(f'fps={frames/10:.1f} kbps={total*8/1000/10:.0f} frame_size_avg_kb={total/max(frames,1)/1024:.0f}')
PY
```

Expected: `fps≈20`, `kbps≈5000-7000` (~6 Mbps), `frame_size≈30-40 KB`.

- [ ] **Step 4: Confirm dev journal shows new encoder, no tier/adaptive logs**

```bash
sshpass -p 'newlevel' ssh -o StrictHostKeyChecking=no newlevel@10.77.8.134 \
  "sudo journalctl -u presenter-dev --since '2 minutes ago' --no-pager 2>/dev/null" \
  | grep -E "encode task started|encoder started|tier|adapt|slow tick"
```

Expected: ONE line `NDI encode task started target_height=720 target_fps=20`. **No** lines mentioning `tier_registry`, `tier encoder`, `slow tick`, `adaptive`, or `demoting`/`promoting tier`. (Those code paths are deleted.)

- [ ] **Step 5: N100 dev2 load check**

```bash
uptime
```

Expected: `load average` reasonable (this machine runs lots of workloads; just confirm it didn't spike). The real comparison is on prod, after Task 9.

- [ ] **Step 6: Mark task done — no commit, this is observation only.**

---

## Task 8: Open PR dev → main + monitor PR CI

**Files:** None (PR creation).

- [ ] **Step 1: Verify state**

```bash
git fetch origin
git log origin/main..origin/dev --oneline
gh pr list --base main --head dev --state open
```

If a PR already exists for this branch, reuse it. Otherwise create.

- [ ] **Step 2: Create PR**

```bash
gh pr create --base main --head dev --title "feat(ndi): single fixed 720p@20 encoder, replaces adaptive tier ladder (#250)" --body "$(cat <<'EOF'
## Summary
- Replaces the adaptive tier ladder shipped in PR #263 (which regressed all 4 production cheap TVs to a flapping L2/L3 floor under N100 CPU pressure) with a single shared encoder at fixed **720p @ 20 fps**, quality 75.
- All MJPEG clients consume the same broadcast — no tier subscription, no per-connection adaptive state.
- SIMD-accelerated resize via \`fast_image_resize\` with reusable destination buffer (zero per-frame allocation in the hot path).
- Frame-rate accumulator throttles arbitrary source rates (30 / 60 fps) to TARGET_FPS exactly.
- Net deletion of ~700 lines of adaptive code (Tier, TierRegistry, AdaptController, per-connection handlers + their tests).
- **Last MJPEG iteration** before the WebRTC / low-latency video transport migration tracked separately.

## Test plan
- [x] Unit tests: \`encoder::tests\` (7 — encode_bgra_resized, ResizingEncoder buffer reuse, dst rebuild on dim change), \`manager::tests\` (4 — watch newest-wins, frame-skip accumulator 30→20 and 60→20).
- [x] \`cargo clippy --workspace --all-targets -D warnings\` clean.
- [x] Dev deploy verified: control client measures ~20 fps × ~6 Mbps, journal shows single encoder line, no tier/adaptive lines.
- [ ] Production verification on sd1l..sd4l (post-merge — ask user for visual confirmation, watch \`load avg\` return to ~1.0 on N100).

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 3: Monitor PR CI**

```bash
sleep 10
gh pr checks <new-PR-number> 2>&1 | head -30
```

Use the `sleep N && gh pr view <N> --json statusCheckRollup` background pattern from Task 6 to wait until terminal. ALL required checks must be green.

- [ ] **Step 4: Verify PR is mergeable + clean**

```bash
gh pr view <PR-number> --json number,mergeable,mergeStateStatus,url --jq '{number, mergeable, mergeStateStatus, url}'
```

Required: `mergeable: "MERGEABLE"`, `mergeStateStatus: "CLEAN"`. If `UNSTABLE`, investigate the failing check via `gh pr checks` and `gh run view --log-failed`. Fix the gate root-cause; do not propose admin-merge or "merge despite". The codecov/patch threshold may need attention since this PR mostly DELETES code — typically that's fine for codecov, but verify.

- [ ] **Step 5: Provide PR URL to user, wait for explicit merge instruction**

Per `pr-merge-policy.md` — never merge without the user saying "merge it" or equivalent. Output the full clickable URL.

---

## Task 9: Post-merge production verification + spec Findings

**Files:**
- Modify: `docs/superpowers/specs/2026-04-26-ndi-single-fixed-tier-design.md` (Findings section append)

Triggered after the user says "merge it" and the merge to main runs the Deploy workflow.

- [ ] **Step 1: Confirm main deploy succeeded**

```bash
gh run list --branch main --limit 3
```

Find the newest `Deploy` run. Wait for it to reach `conclusion=success` using the same monitor pattern as Task 6.

- [ ] **Step 2: Confirm production version + load avg**

```bash
curl -s http://10.77.9.205/healthz; echo
sshpass -p 'newlevel' ssh -o StrictHostKeyChecking=no newlevel@presenter.lan 'uptime'
```

Expected: `version: 0.4.35`. `load avg` should drop to roughly the pre-PR-263 baseline (~0.7) within 5 minutes of the new encoder taking over. Pre-fix prod was ~2.77.

- [ ] **Step 3: Confirm production logs show single encoder, no tier/adaptive**

```bash
sshpass -p 'newlevel' ssh -o StrictHostKeyChecking=no newlevel@presenter.lan \
  "sudo journalctl -u presenter --since '5 minutes ago' --no-pager 2>/dev/null" \
  | grep -E "encode task started|encoder started|tier|adaptive|slow tick"
```

Expected: ONE line `NDI encode task started target_height=720 target_fps=20`. Any line mentioning tier/adaptive would indicate a stale binary — re-trigger deploy.

- [ ] **Step 4: Ask user for visual confirmation on real TVs**

The user observes sd1l, sd2l, sd3l, sd4l. Pass criterion: all four are watchable, no visible flapping, text legible. Record the user's verdict.

- [ ] **Step 5: Append production results to spec Findings**

Open `docs/superpowers/specs/2026-04-26-ndi-single-fixed-tier-design.md` and append a new section after the existing content:

```markdown
## Production verification (YYYY-MM-DD, prod 0.4.35, post-merge)

Production deployed via the main-branch Deploy workflow. Within ~5 minutes of the new encoder taking over:

| Metric | Pre-fix (PR #263 adaptive) | Post-fix (this design) |
|---|---|---|
| N100 load avg | 2.77 / 4 cores (~69%) | <fill in from uptime> |
| Tier transitions in logs | ~16 demote + 4 promote / 15 min | 0 (no tier code anymore) |
| MJPEG frame rate to fast control client | varied (was demoting) | <fill in from control client> fps |
| MJPEG bandwidth | 3–24 Mbps (was bouncing) | <fill in> Mbps |
| Visual on sd1l (Tesla LEAP-S1) | flapping L2/L3 ≈ 10 fps, unwatchable | <user verdict> |
| Visual on sd2l/3/4 (Hyundai 1 GB) | flapping L2/L3 ≈ 10 fps, unwatchable | <user verdict> |

**Pass criteria:** server load avg returns to baseline AND user confirms all four TVs are watchable with no flapping. Result: **PASS / FAIL** (if FAIL, document exactly which TV and what symptom — drop to TARGET_FPS=15 in a follow-up commit).

**Decision:** issue #250 is closed by this fix. Next major iteration is the WebRTC / low-latency video migration in a separate issue.
```

Replace the `<fill in...>` placeholders with measured values.

- [ ] **Step 6: Commit and push the Findings update**

```bash
git fetch origin
git checkout dev
git merge origin/main --no-edit 2>&1 | tail -3
git add docs/superpowers/specs/2026-04-26-ndi-single-fixed-tier-design.md
git commit -m "docs(spec): record production verification of single-tier MJPEG (#250)"
git push origin dev 2>&1 | tail -3
```

Wait for the docs-only push to land green using the same single-sleep monitor pattern.

---

## Verification Summary

| Check | Where verified |
|---|---|
| `encode_bgra_resized` retargeted to fast_image_resize | `encoder::tests::encode_bgra_resized_*` (Task 2) |
| `ResizingEncoder` reuses dst buffer | `encoder::tests::resizing_encoder_reuses_destination_buffer_across_calls` (Task 3) |
| `ResizingEncoder` rebuilds dst on source dim change | `encoder::tests::resizing_encoder_rebuilds_dst_when_source_dims_change` (Task 3) |
| Frame-skip accumulator 30→20 = 2/3 | `manager::tests::frame_skip_accumulator_30_to_20_emits_2_of_3` (Task 4) |
| Frame-skip accumulator 60→20 = 1/3 | `manager::tests::frame_skip_accumulator_60_to_20_emits_1_of_3` (Task 4) |
| Single shared broadcast, no tier code | `cargo build --workspace` after Task 4 succeeds without referencing `Tier` or `TierRegistry` |
| `/ndi/mjpeg` serves valid 720p JPEG at ~20 fps on dev | Manual control client measurement (Task 7 Step 3) |
| N100 prod load returns to baseline | `uptime` on prod after deploy (Task 9 Step 2) |
| Real cheap TVs are watchable | User visual confirmation (Task 9 Step 4) |
