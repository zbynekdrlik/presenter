# NDI Cheap-TV Adaptive Streaming Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the `/ndi/mjpeg` stream auto-adapt per-connection — tiered shared encoders + lag-driven demote/promote — so cheap Android TVs become usable on the `ndi-fullscreen` and `api` stage layouts without operator configuration.

**Architecture:** Replace the single global JPEG encoder thread in `presenter-ndi::manager` with a `TierRegistry` of up to four lazy, ref-counted tier encoders (L0=1080@30, L1=1080@15, L2=720@15, L3=720@10). Each MJPEG HTTP/WebSocket connection holds a `TierSubscription` and runs a small adaptive controller that demotes one tier on backpressure (`broadcast::RecvError::Lagged` ≥5 in 30 s) and promotes after 60 s of clean reception. No DB migration, no settings UI, no client changes.

**Tech Stack:** Rust 1.95 (workspace local builds allowed), `tokio::sync::broadcast`, `image` crate (resize), libjpeg-turbo via `turbojpeg` (already in deps), axum (server), Playwright (E2E). Manual ADB+chrome://inspect on sd1l.lan and sd2l.lan for verification.

**Spec:** `docs/superpowers/specs/2026-04-25-ndi-cheap-tv-adaptive-design.md`

---

## File Structure

| File | Change |
|---|---|
| `Cargo.toml` (workspace) | Bump version 0.4.33 → 0.4.34. Add `image` workspace dep. |
| `crates/presenter-ndi/Cargo.toml` | Add `image = { workspace = true }`. |
| `crates/presenter-ndi/src/tier.rs` | **NEW.** `Tier` enum, `TierSpec`, `Tier::demote/promote/initial`. Pure value types + transitions. |
| `crates/presenter-ndi/src/encoder.rs` | Add `JpegEncoder::encode_bgra_resized(...)` and `encode_uyvy_resized(...)`. |
| `crates/presenter-ndi/src/tier_registry.rs` | **NEW.** `TierRegistry` (refcount lifecycle, lazy spawn), `TierSubscription` (drop-guarded receiver), `run_tier_encoder(...)` thread fn. |
| `crates/presenter-ndi/src/manager.rs` | Replace the single `frame_tx` + `run_encode_thread` with a `TierRegistry`. Add `subscribe_tier(Tier) -> TierSubscription`. Keep `subscribe_frames()` as a thin wrapper that returns L0 (back-compat for any unmodified callers). |
| `crates/presenter-ndi/src/lib.rs` | Re-export `Tier`, `TierSubscription`. |
| `crates/presenter-server/src/router/integrations/ndi.rs` | Replace `mjpeg_http` and `mjpeg_ws` bodies: subscribe at L0, run adaptive controller in the stream future, swap tiers via `TierRegistry::subscribe_tier` on demote/promote conditions. |
| `crates/presenter-ndi/src/manager.rs` (tests module) | Add tests for TierRegistry refcount + tier ladder. |
| `crates/presenter-ndi/src/tier_registry.rs` (tests module) | Tests for adaptive controller transitions. (Controller lives here as a free fn so it can be unit-tested without an HTTP body stream.) |
| `crates/presenter-server/src/router/integrations/ndi.rs` (tests module) | Integration test: artificially slow consumer demotes, then recovers. |

**Spec doc updates (in this same PR, before merge):**
- `docs/superpowers/specs/2026-04-25-ndi-cheap-tv-adaptive-design.md` Findings section — populated in two waves (baseline before code, post-fix after code).

---

## Phase 1: Baseline Profiling (no code yet)

### Task 1: Baseline profile of sd1l.lan (Tesla LEAP-S1, 2 GB)

**Files:**
- Modify: `docs/superpowers/specs/2026-04-25-ndi-cheap-tv-adaptive-design.md` (Findings section)

- [ ] **Step 1: Confirm prerequisites**

```bash
adb -s sd1l.lan:5555 shell getprop ro.product.model
# Expected: LEAP-S1
adb -s sd1l.lan:5555 shell dumpsys activity activities | grep mResumedActivity
# Expected: com.fullykiosk.videokiosk/de.ozerov.fully.FullyActivity
curl -s http://10.77.8.134:8080/integrations/video-sources | python3 -c "import json,sys; print([s for s in json.load(sys.stdin) if s['isActive']])"
# Expected: one isActive=True entry, e.g. RESOLUME-SNV (cg-obs)
```

- [ ] **Step 2: Force ndi-fullscreen layout on dev server**

```bash
curl -s -X POST -H 'Content-Type: application/json' \
  -d '{"code":"ndi-fullscreen"}' \
  http://10.77.8.134:8080/stage/layout
```

- [ ] **Step 3: Set up reverse port-forward and enable WebView debugging on sd1l**

```bash
adb -s sd1l.lan:5555 reverse tcp:8080 tcp:8080
# Fully Kiosk respects this intent extra to enable WebView debugging:
adb -s sd1l.lan:5555 shell am force-stop com.fullykiosk.videokiosk
adb -s sd1l.lan:5555 shell am start -n com.fullykiosk.videokiosk/de.ozerov.fully.MainActivity \
  -d "http://127.0.0.1:8080/stage" --es WEBVIEW_DEBUG true
sleep 3
adb -s sd1l.lan:5555 shell dumpsys activity activities | grep mResumedActivity
# Expected: FullyActivity is again the resumed activity
```

- [ ] **Step 4: Attach DevTools and capture a 10 s Performance trace**

In the dev machine's Chromium, open `chrome://inspect/#devices`. The sd1l webview hosting `127.0.0.1:8080/stage` should appear. Click "inspect". In DevTools:

1. Performance tab → Record for 10 s while NDI is streaming.
2. Network tab → keep all entries, observe MJPEG payload sizes and timing.
3. Console → run `JSON.stringify(performance.memory)` — record `usedJSHeapSize`.

Export the Performance profile (3-dot menu → Save profile…) to `/tmp/sd1l-baseline-L0.json` for later reference (do not commit binary artifacts).

- [ ] **Step 5: Tabulate measurements**

From the trace, read off:

- `decode_p50`, `decode_p95` — median and 95th-percentile of `Image Decode` event durations (Performance summary or individual events).
- `paint_p50` — median of `Paint` event durations.
- `fps_sustained` — count `Image Decode` events in the trace, divide by trace duration.
- Network: `kbps_avg`, `kb_per_frame_avg`.

- [ ] **Step 6: Append to spec Findings section**

Edit `docs/superpowers/specs/2026-04-25-ndi-cheap-tv-adaptive-design.md`. Replace the placeholder paragraph under "Findings" with:

```markdown
## Findings

### Baseline (before fix), 2026-04-25, source RESOLUME-SNV (cg-obs) at 1920×1080 @ 30 fps

| TV | Tier | decode_p50 | decode_p95 | paint_p50 | fps_sustained | kbps |
|---|---|---|---|---|---|---|
| sd1l (Tesla LEAP-S1, 2 GB) | L0 (1080@30) | <fill> ms | <fill> ms | <fill> ms | <fill> | <fill> |
```

(Replace `<fill>` with actual numbers. The table will grow in Tasks 2 and 13.)

- [ ] **Step 7: Commit**

```bash
git add docs/superpowers/specs/2026-04-25-ndi-cheap-tv-adaptive-design.md
git commit -m "docs(spec): record sd1l baseline NDI profiling (#250)"
```

---

### Task 2: Baseline profile of sd2l.lan (Hyundai Android TV, 1 GB)

**Files:**
- Modify: `docs/superpowers/specs/2026-04-25-ndi-cheap-tv-adaptive-design.md` (Findings section)

- [ ] **Step 1: Repeat Task 1 Steps 1–5 for sd2l.lan**

Use `sd2l.lan:5555` instead of `sd1l.lan:5555` everywhere. Save trace to `/tmp/sd2l-baseline-L0.json`.

- [ ] **Step 2: Append sd2l row to the Findings table**

Add one more row to the table from Task 1 Step 6:

```markdown
| sd2l (Hyundai, 1 GB) | L0 (1080@30) | <fill> | <fill> | <fill> | <fill> | <fill> |
```

- [ ] **Step 3: Commit**

```bash
git add docs/superpowers/specs/2026-04-25-ndi-cheap-tv-adaptive-design.md
git commit -m "docs(spec): record sd2l baseline NDI profiling (#250)"
```

**If baseline shows even L1 (1080@15) is unusable on sd2l** (decode_p95 + paint_p50 > 66 ms), STOP and re-evaluate the tier ladder with the user. The whole spec assumes L0 is ambitious-but-plausible; if it's catastrophic on the Hyundai, we likely need to start clients at L2 by default rather than L0. Open a discussion before continuing to Task 3.

---

## Phase 2: Implementation

### Task 3: Workspace prep — version bump and `image` dep

**Files:**
- Modify: `Cargo.toml` (workspace root)
- Modify: `crates/presenter-ndi/Cargo.toml`
- Modify: `Cargo.lock` (auto)

- [ ] **Step 1: Bump workspace version**

In `Cargo.toml`, change:

```toml
[workspace.package]
version = "0.4.33"
```

to:

```toml
[workspace.package]
version = "0.4.34"
```

- [ ] **Step 2: Add `image` to workspace deps**

In `Cargo.toml` under `[workspace.dependencies]`, add:

```toml
image = { version = "0.25", default-features = false, features = ["jpeg"] }
```

- [ ] **Step 3: Add the dep to `presenter-ndi`**

In `crates/presenter-ndi/Cargo.toml`, under `[dependencies]`, add:

```toml
image = { workspace = true }
```

- [ ] **Step 4: Update Cargo.lock**

```bash
cargo build -p presenter-ndi --quiet
cargo build -p presenter-ui --manifest-path crates/presenter-ui/Cargo.toml --quiet
```

(presenter-ui is a workspace-excluded crate with its own Cargo.lock — version bump must propagate there too.)

- [ ] **Step 5: Verify**

```bash
grep '^version = "0.4.34"' Cargo.lock | head -3
grep '^version = "0.4.34"' crates/presenter-ui/Cargo.lock | head -3
```

Both must show the presenter-* crates at the new version.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock crates/presenter-ndi/Cargo.toml crates/presenter-ui/Cargo.lock
git commit -m "chore: bump version to 0.4.34, add image crate (#250)"
```

---

### Task 4: `Tier` enum and transitions

**Files:**
- Create: `crates/presenter-ndi/src/tier.rs`
- Modify: `crates/presenter-ndi/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/presenter-ndi/src/tier.rs` with:

```rust
//! Tier definitions for adaptive MJPEG streaming.
//!
//! A `Tier` is one (resolution, framerate) target. Tiers form a totally-
//! ordered ladder where L0 is the most demanding and L3 is the floor.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tier {
    L0,
    L1,
    L2,
    L3,
}

#[derive(Debug, Clone, Copy)]
pub struct TierSpec {
    /// Maximum output height in pixels. Source dims are preserved if
    /// already smaller; aspect ratio is preserved during downscale.
    pub max_height: u32,
    /// Target output framerate.
    pub target_fps: u32,
}

impl Tier {
    pub const fn spec(self) -> TierSpec {
        match self {
            Tier::L0 => TierSpec { max_height: 1080, target_fps: 30 },
            Tier::L1 => TierSpec { max_height: 1080, target_fps: 15 },
            Tier::L2 => TierSpec { max_height: 720,  target_fps: 15 },
            Tier::L3 => TierSpec { max_height: 720,  target_fps: 10 },
        }
    }

    /// Initial tier for a new connection.
    pub const fn initial() -> Self {
        Tier::L0
    }

    /// Returns the next-lower tier, or `None` if already at the floor.
    pub const fn demote(self) -> Option<Self> {
        match self {
            Tier::L0 => Some(Tier::L1),
            Tier::L1 => Some(Tier::L2),
            Tier::L2 => Some(Tier::L3),
            Tier::L3 => None,
        }
    }

    /// Returns the next-higher tier, or `None` if already at the top.
    pub const fn promote(self) -> Option<Self> {
        match self {
            Tier::L0 => None,
            Tier::L1 => Some(Tier::L0),
            Tier::L2 => Some(Tier::L1),
            Tier::L3 => Some(Tier::L2),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ladder_demotes_to_floor() {
        let mut t = Tier::initial();
        for expected in [Tier::L1, Tier::L2, Tier::L3] {
            t = t.demote().expect("not at floor yet");
            assert_eq!(t, expected);
        }
        assert_eq!(t.demote(), None, "L3 is the floor");
    }

    #[test]
    fn ladder_promotes_to_top() {
        let mut t = Tier::L3;
        for expected in [Tier::L2, Tier::L1, Tier::L0] {
            t = t.promote().expect("not at top yet");
            assert_eq!(t, expected);
        }
        assert_eq!(t.promote(), None, "L0 is the top");
    }

    #[test]
    fn specs_match_design() {
        assert_eq!(Tier::L0.spec().max_height, 1080);
        assert_eq!(Tier::L0.spec().target_fps, 30);
        assert_eq!(Tier::L3.spec().max_height, 720);
        assert_eq!(Tier::L3.spec().target_fps, 10);
    }
}
```

- [ ] **Step 2: Wire into the crate**

In `crates/presenter-ndi/src/lib.rs`, add:

```rust
pub mod tier;
```

right after `pub mod encoder;`. Then add the re-exports near the bottom:

```rust
pub use tier::{Tier, TierSpec};
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p presenter-ndi tier::tests --quiet
```

Expected: 3 passed.

- [ ] **Step 4: Commit**

```bash
git add crates/presenter-ndi/src/tier.rs crates/presenter-ndi/src/lib.rs
git commit -m "feat(ndi): add Tier ladder (L0..L3) for adaptive MJPEG (#250)"
```

---

### Task 5: Resize-and-encode in `JpegEncoder`

**Files:**
- Modify: `crates/presenter-ndi/src/encoder.rs`

- [ ] **Step 1: Write the failing test**

Append to `crates/presenter-ndi/src/encoder.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn make_bgra(width: u32, height: u32) -> Vec<u8> {
        // Solid red BGRA frame so resize is well-defined.
        let mut v = Vec::with_capacity((width * height * 4) as usize);
        for _ in 0..(width * height) {
            v.extend_from_slice(&[0, 0, 255, 255]); // BGRA red
        }
        v
    }

    #[test]
    fn encode_bgra_resized_caps_height_and_preserves_aspect() {
        let enc = JpegEncoder::new(75);
        // 1920x1080 -> max_height 720 should produce 1280x720
        let bgra = make_bgra(1920, 1080);
        let jpeg = enc.encode_bgra_resized(&bgra, 1920, 1080, 720).unwrap();
        let img = image::load_from_memory(&jpeg).unwrap();
        assert_eq!(img.height(), 720);
        assert_eq!(img.width(), 1280);
    }

    #[test]
    fn encode_bgra_resized_passthrough_when_source_smaller() {
        let enc = JpegEncoder::new(75);
        // 640x480 source with max_height 720 should NOT upscale: stays 640x480.
        let bgra = make_bgra(640, 480);
        let jpeg = enc.encode_bgra_resized(&bgra, 640, 480, 720).unwrap();
        let img = image::load_from_memory(&jpeg).unwrap();
        assert_eq!(img.width(), 640);
        assert_eq!(img.height(), 480);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p presenter-ndi encoder::tests --quiet
```

Expected: compile error — `encode_bgra_resized` does not exist.

- [ ] **Step 3: Implement**

Replace the contents of `crates/presenter-ndi/src/encoder.rs` with:

```rust
use anyhow::{Context, Result};

/// JPEG encoder using libjpeg-turbo for minimal latency.
pub struct JpegEncoder {
    quality: i32,
}

impl JpegEncoder {
    /// Create a new JPEG encoder with the given quality (1-100).
    pub fn new(quality: i32) -> Self {
        Self {
            quality: quality.clamp(1, 100),
        }
    }

    /// Encode BGRA/BGRX pixel data to JPEG.
    ///
    /// Returns the compressed JPEG bytes.
    pub fn encode_bgra(&self, bgra: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
        let image = turbojpeg::Image {
            pixels: bgra,
            width: width as usize,
            pitch: width as usize * 4,
            height: height as usize,
            format: turbojpeg::PixelFormat::BGRA,
        };
        let buf = turbojpeg::compress(image, self.quality, turbojpeg::Subsamp::Sub2x2)
            .context("JPEG encode failed")?;
        Ok(buf.to_vec())
    }

    /// Encode UYVY pixel data to JPEG.
    ///
    /// Converts UYVY → BGRA first, then encodes.
    pub fn encode_uyvy(&self, uyvy: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
        let bgra = uyvy_to_bgra(uyvy, width, height);
        self.encode_bgra(&bgra, width, height)
    }

    /// Encode BGRA pixel data to JPEG, capped at `max_height`.
    ///
    /// If the source is already at or below `max_height`, no resize is done.
    /// Aspect ratio is preserved.
    pub fn encode_bgra_resized(
        &self,
        bgra: &[u8],
        width: u32,
        height: u32,
        max_height: u32,
    ) -> Result<Vec<u8>> {
        if height <= max_height {
            return self.encode_bgra(bgra, width, height);
        }
        let (out_w, out_h) = scaled_dims(width, height, max_height);
        let resized = resize_bgra(bgra, width, height, out_w, out_h);
        self.encode_bgra(&resized, out_w, out_h)
    }

    /// Encode UYVY pixel data to JPEG, capped at `max_height`.
    pub fn encode_uyvy_resized(
        &self,
        uyvy: &[u8],
        width: u32,
        height: u32,
        max_height: u32,
    ) -> Result<Vec<u8>> {
        let bgra = uyvy_to_bgra(uyvy, width, height);
        self.encode_bgra_resized(&bgra, width, height, max_height)
    }
}

/// Compute resized dims preserving aspect, capped at `max_height`.
fn scaled_dims(w: u32, h: u32, max_height: u32) -> (u32, u32) {
    let new_h = max_height;
    // Round to even for chroma-subsampled JPEG safety.
    let new_w = ((w as u64 * new_h as u64 + (h as u64 / 2)) / h as u64) as u32;
    let new_w = (new_w + 1) & !1;
    (new_w, new_h)
}

/// Resize BGRA via the `image` crate. Triangle filter (a.k.a. bilinear) — fast,
/// good enough for compositing-heavy NDI sources.
fn resize_bgra(bgra: &[u8], in_w: u32, in_h: u32, out_w: u32, out_h: u32) -> Vec<u8> {
    let img: image::ImageBuffer<image::Bgra<u8>, &[u8]> =
        image::ImageBuffer::from_raw(in_w, in_h, bgra)
            .expect("BGRA buffer length matches dims");
    let resized = image::imageops::resize(&img, out_w, out_h, image::imageops::FilterType::Triangle);
    resized.into_raw()
}

/// Convert UYVY to BGRA for JPEG encoding.
fn uyvy_to_bgra(uyvy: &[u8], width: u32, height: u32) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let mut bgra = vec![0u8; w * h * 4];

    for y in 0..h {
        for x in (0..w).step_by(2) {
            let uyvy_offset = (y * w + x) * 2;
            let u = uyvy[uyvy_offset] as f32 - 128.0;
            let y0 = uyvy[uyvy_offset + 1] as f32;
            let v = uyvy[uyvy_offset + 2] as f32 - 128.0;
            let y1 = uyvy[uyvy_offset + 3] as f32;

            // YUV to RGB
            let r0 = (y0 + 1.402 * v).clamp(0.0, 255.0) as u8;
            let g0 = (y0 - 0.344 * u - 0.714 * v).clamp(0.0, 255.0) as u8;
            let b0 = (y0 + 1.772 * u).clamp(0.0, 255.0) as u8;

            let r1 = (y1 + 1.402 * v).clamp(0.0, 255.0) as u8;
            let g1 = (y1 - 0.344 * u - 0.714 * v).clamp(0.0, 255.0) as u8;
            let b1 = (y1 + 1.772 * u).clamp(0.0, 255.0) as u8;

            let idx0 = (y * w + x) * 4;
            bgra[idx0] = b0;
            bgra[idx0 + 1] = g0;
            bgra[idx0 + 2] = r0;
            bgra[idx0 + 3] = 255;

            let idx1 = (y * w + x + 1) * 4;
            bgra[idx1] = b1;
            bgra[idx1 + 1] = g1;
            bgra[idx1 + 2] = r1;
            bgra[idx1 + 3] = 255;
        }
    }

    bgra
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_bgra(width: u32, height: u32) -> Vec<u8> {
        let mut v = Vec::with_capacity((width * height * 4) as usize);
        for _ in 0..(width * height) {
            v.extend_from_slice(&[0, 0, 255, 255]);
        }
        v
    }

    #[test]
    fn encode_bgra_resized_caps_height_and_preserves_aspect() {
        let enc = JpegEncoder::new(75);
        let bgra = make_bgra(1920, 1080);
        let jpeg = enc.encode_bgra_resized(&bgra, 1920, 1080, 720).unwrap();
        let img = image::load_from_memory(&jpeg).unwrap();
        assert_eq!(img.height(), 720);
        assert_eq!(img.width(), 1280);
    }

    #[test]
    fn encode_bgra_resized_passthrough_when_source_smaller() {
        let enc = JpegEncoder::new(75);
        let bgra = make_bgra(640, 480);
        let jpeg = enc.encode_bgra_resized(&bgra, 640, 480, 720).unwrap();
        let img = image::load_from_memory(&jpeg).unwrap();
        assert_eq!(img.width(), 640);
        assert_eq!(img.height(), 480);
    }

    #[test]
    fn scaled_dims_rounds_to_even() {
        // 1921 width with height halve to a non-even number — must round to even
        let (w, h) = scaled_dims(1921, 1081, 540);
        assert_eq!(h, 540);
        assert_eq!(w % 2, 0);
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test -p presenter-ndi encoder::tests --quiet
```

Expected: 3 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/presenter-ndi/src/encoder.rs
git commit -m "feat(ndi): add resize-aware JPEG encode (#250)"
```

---

### Task 6: `TierRegistry` and `TierSubscription`

**Files:**
- Create: `crates/presenter-ndi/src/tier_registry.rs`
- Modify: `crates/presenter-ndi/src/lib.rs`

- [ ] **Step 1: Write the registry skeleton + tests**

Create `crates/presenter-ndi/src/tier_registry.rs`:

```rust
//! Lazy ref-counted registry of tier encoders.
//!
//! At most one encoder runs per `Tier`. An encoder is spawned the first
//! time a subscriber registers for that tier and shut down when the last
//! subscriber drops its `TierSubscription`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytes::Bytes;
use tokio::sync::broadcast;

use crate::encoder::JpegEncoder;
use crate::receiver::VideoFrame;
use crate::tier::Tier;

/// Capacity of each tier's broadcast channel. Small so backpressure
/// surfaces quickly as `RecvError::Lagged`.
const TIER_CHANNEL_CAPACITY: usize = 4;

/// Shared upstream raw frame slot — the tier encoder reads the latest
/// frame here and applies its tier-specific transform.
pub type FrameSlot = Arc<Mutex<Option<VideoFrame>>>;

struct TierEntry {
    tx: broadcast::Sender<Bytes>,
    refcount: usize,
    stop_tx: tokio::sync::watch::Sender<bool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

/// A subscription handle tying a `broadcast::Receiver` to a refcount.
/// Dropping it decrements the refcount and possibly stops the tier encoder.
pub struct TierSubscription {
    pub rx: broadcast::Receiver<Bytes>,
    tier: Tier,
    registry: Arc<TierRegistryInner>,
}

impl TierSubscription {
    pub fn tier(&self) -> Tier {
        self.tier
    }
}

impl Drop for TierSubscription {
    fn drop(&mut self) {
        self.registry.unsubscribe(self.tier);
    }
}

pub struct TierRegistry {
    inner: Arc<TierRegistryInner>,
}

pub(crate) struct TierRegistryInner {
    frame_slot: FrameSlot,
    condvar: Arc<std::sync::Condvar>,
    entries: Mutex<HashMap<Tier, TierEntry>>,
}

impl TierRegistry {
    pub fn new(frame_slot: FrameSlot, condvar: Arc<std::sync::Condvar>) -> Self {
        Self {
            inner: Arc::new(TierRegistryInner {
                frame_slot,
                condvar,
                entries: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Subscribe to a tier — spawns the encoder if not already running.
    pub fn subscribe(&self, tier: Tier) -> TierSubscription {
        self.inner.subscribe(tier)
    }

    /// Stop all tier encoders (called when the upstream NDI stream stops).
    pub fn stop_all(&self) {
        self.inner.stop_all();
    }

    /// Test/inspection: how many encoders are currently running.
    pub fn active_tiers(&self) -> usize {
        self.inner.entries.lock().unwrap().len()
    }
}

impl TierRegistryInner {
    fn subscribe(self: &Arc<Self>, tier: Tier) -> TierSubscription {
        let mut entries = self.entries.lock().unwrap();
        let entry = entries.entry(tier).or_insert_with(|| {
            let (tx, _) = broadcast::channel::<Bytes>(TIER_CHANNEL_CAPACITY);
            let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
            let frame_slot = Arc::clone(&self.frame_slot);
            let condvar = Arc::clone(&self.condvar);
            let tx_clone = tx.clone();
            let handle = std::thread::Builder::new()
                .name(format!("ndi-tier-{:?}", tier))
                .spawn(move || run_tier_encoder(tier, frame_slot, condvar, tx_clone, stop_rx))
                .expect("spawn tier encoder thread");
            TierEntry {
                tx,
                refcount: 0,
                stop_tx,
                handle: Some(handle),
            }
        });
        entry.refcount += 1;
        let rx = entry.tx.subscribe();
        TierSubscription {
            rx,
            tier,
            registry: Arc::clone(self),
        }
    }

    fn unsubscribe(&self, tier: Tier) {
        let mut entries = self.entries.lock().unwrap();
        let should_stop = if let Some(entry) = entries.get_mut(&tier) {
            entry.refcount -= 1;
            entry.refcount == 0
        } else {
            false
        };
        if should_stop {
            if let Some(mut entry) = entries.remove(&tier) {
                let _ = entry.stop_tx.send(true);
                if let Some(h) = entry.handle.take() {
                    drop(entries); // release before joining (encoder may want condvar)
                    let _ = h.join();
                }
            }
        }
    }

    fn stop_all(&self) {
        let mut entries = self.entries.lock().unwrap();
        for (_tier, mut entry) in entries.drain() {
            let _ = entry.stop_tx.send(true);
            if let Some(h) = entry.handle.take() {
                let _ = h.join();
            }
        }
    }
}

/// Encoder loop for a single tier. Reads the shared frame slot via
/// condvar, applies the tier's resize+frame-skip, JPEG-encodes,
/// broadcasts.
fn run_tier_encoder(
    tier: Tier,
    frame_slot: FrameSlot,
    condvar: Arc<std::sync::Condvar>,
    tx: broadcast::Sender<Bytes>,
    mut stop_rx: tokio::sync::watch::Receiver<bool>,
) {
    let fourcc_uyvy = u32::from_le_bytes([b'U', b'Y', b'V', b'Y']);
    let fourcc_bgra = u32::from_le_bytes([b'B', b'G', b'R', b'A']);
    let fourcc_bgrx = u32::from_le_bytes([b'B', b'G', b'R', b'X']);
    let encoder = JpegEncoder::new(75);
    let spec = tier.spec();
    let frame_interval = Duration::from_secs_f64(1.0 / spec.target_fps as f64);
    let mut next_emit = Instant::now();

    tracing::info!(?tier, "tier encoder started");

    loop {
        if *stop_rx.borrow() {
            break;
        }

        let frame = {
            let slot = frame_slot.lock().unwrap_or_else(|e| e.into_inner());
            let (mut slot, _) = condvar
                .wait_timeout(slot, Duration::from_millis(100))
                .unwrap_or_else(|e| e.into_inner());
            slot.clone()
        };

        let frame = match frame {
            Some(f) => f,
            None => continue,
        };

        let now = Instant::now();
        if now < next_emit {
            continue; // frame skip — drop this one
        }
        next_emit = now + frame_interval;

        let jpeg = if frame.fourcc == fourcc_bgra || frame.fourcc == fourcc_bgrx {
            encoder.encode_bgra_resized(&frame.data, frame.width, frame.height, spec.max_height)
        } else if frame.fourcc == fourcc_uyvy {
            encoder.encode_uyvy_resized(&frame.data, frame.width, frame.height, spec.max_height)
        } else {
            tracing::warn!(?tier, "unsupported fourcc: 0x{:08x}", frame.fourcc);
            continue;
        };

        match jpeg {
            Ok(data) => {
                let _ = tx.send(Bytes::from(data));
            }
            Err(e) => {
                tracing::error!(?tier, "JPEG encode error: {e}");
            }
        }
    }

    tracing::info!(?tier, "tier encoder stopped");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_slot() -> (FrameSlot, Arc<std::sync::Condvar>) {
        (
            Arc::new(Mutex::new(None)),
            Arc::new(std::sync::Condvar::new()),
        )
    }

    #[test]
    fn subscribe_then_drop_stops_encoder() {
        let (slot, cv) = empty_slot();
        let reg = TierRegistry::new(slot, cv);
        assert_eq!(reg.active_tiers(), 0);
        {
            let _sub = reg.subscribe(Tier::L0);
            assert_eq!(reg.active_tiers(), 1);
        }
        // Give the worker thread up to 200 ms to observe its stop signal
        // and the registry to clean up.
        for _ in 0..20 {
            if reg.active_tiers() == 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(reg.active_tiers(), 0);
    }

    #[test]
    fn two_subscribers_share_one_encoder() {
        let (slot, cv) = empty_slot();
        let reg = TierRegistry::new(slot, cv);
        let _a = reg.subscribe(Tier::L1);
        let _b = reg.subscribe(Tier::L1);
        assert_eq!(reg.active_tiers(), 1, "shared tier counts once");
    }

    #[test]
    fn distinct_tiers_run_independently() {
        let (slot, cv) = empty_slot();
        let reg = TierRegistry::new(slot, cv);
        let _a = reg.subscribe(Tier::L0);
        let _b = reg.subscribe(Tier::L2);
        assert_eq!(reg.active_tiers(), 2);
    }
}
```

- [ ] **Step 2: Wire into the crate**

In `crates/presenter-ndi/src/lib.rs`, add:

```rust
pub mod tier_registry;
```

after `pub mod tier;`. Add re-export:

```rust
pub use tier_registry::{TierRegistry, TierSubscription};
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p presenter-ndi tier_registry --quiet
```

Expected: 3 passed.

- [ ] **Step 4: Commit**

```bash
git add crates/presenter-ndi/src/tier_registry.rs crates/presenter-ndi/src/lib.rs
git commit -m "feat(ndi): TierRegistry with refcount lifecycle + per-tier encoder (#250)"
```

---

### Task 7: Replace single encoder with TierRegistry in `NdiManager`

**Files:**
- Modify: `crates/presenter-ndi/src/manager.rs`

- [ ] **Step 1: Replace manager.rs content**

Replace the contents of `crates/presenter-ndi/src/manager.rs` with:

```rust
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{watch, Mutex};

use crate::discovery::{self, FinderShutdown, SourceList};
use crate::ndi_sdk::NdiLib;
use crate::receiver::{NdiReceiver, VideoFrame};
use crate::tier::Tier;
use crate::tier_registry::{FrameSlot, TierRegistry, TierSubscription};

/// Callback for reporting NDI connection status changes.
pub type StatusCallback = Arc<dyn Fn(String) + Send + Sync>;

struct ActiveStream {
    stop_signal: watch::Sender<bool>,
    capture_thread: Option<std::thread::JoinHandle<()>>,
}

/// Orchestrates NDI discovery, capture, and tiered MJPEG encoding.
pub struct NdiManager {
    sdk: Arc<NdiLib>,
    source_list: SourceList,
    _finder_shutdown: FinderShutdown,
    active_stream: Mutex<Option<ActiveStream>>,
    frame_slot: FrameSlot,
    condvar: Arc<std::sync::Condvar>,
    tier_registry: TierRegistry,
}

impl NdiManager {
    pub fn try_new() -> Option<Self> {
        let sdk = NdiLib::load().ok()?;
        let sdk = Arc::new(sdk);
        let (source_list, finder_shutdown) = discovery::spawn_persistent_finder(Arc::clone(&sdk));
        let frame_slot: FrameSlot = Arc::new(std::sync::Mutex::new(None));
        let condvar = Arc::new(std::sync::Condvar::new());
        let tier_registry = TierRegistry::new(Arc::clone(&frame_slot), Arc::clone(&condvar));
        Some(Self {
            sdk,
            source_list,
            _finder_shutdown: finder_shutdown,
            active_stream: Mutex::new(None),
            frame_slot,
            condvar,
            tier_registry,
        })
    }

    pub fn is_available(&self) -> bool {
        true
    }

    pub fn discover_sources(&self, _timeout_ms: u32) -> Result<Vec<discovery::NdiSourceInfo>> {
        Ok(self.source_list.read())
    }

    /// Subscribe to a specific tier.
    pub fn subscribe_tier(&self, tier: Tier) -> TierSubscription {
        self.tier_registry.subscribe(tier)
    }

    /// Back-compat: subscribe at the initial tier (L0).
    pub fn subscribe_frames(&self) -> TierSubscription {
        self.tier_registry.subscribe(Tier::initial())
    }

    pub async fn start_stream(
        &self,
        ndi_name: &str,
        status_cb: Option<StatusCallback>,
    ) -> Result<()> {
        self.stop_stream().await;

        let sdk = Arc::clone(&self.sdk);
        let source_name = ndi_name.to_string();
        let (stop_tx, stop_rx) = watch::channel(false);
        let frame_slot = Arc::clone(&self.frame_slot);
        let condvar = Arc::clone(&self.condvar);

        let capture_thread = std::thread::Builder::new()
            .name("ndi-capture".into())
            .spawn(move || {
                run_capture_thread(sdk, source_name, frame_slot, condvar, stop_rx, status_cb);
            })?;

        let mut active = self.active_stream.lock().await;
        *active = Some(ActiveStream {
            stop_signal: stop_tx,
            capture_thread: Some(capture_thread),
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
            if let Some(h) = stream.capture_thread.take() {
                let _ = h.join();
            }
        }
        // Tier encoders run only while there are subscribers; they will
        // observe an empty slot indefinitely after capture stops, but
        // that's harmless. Stop them too so they don't busy-spin if
        // there are no subscribers.
        self.tier_registry.stop_all();
    }
}

// ---------------------------------------------------------------------------
// Capture thread (unchanged from previous implementation)
// ---------------------------------------------------------------------------

fn run_capture_thread(
    sdk: Arc<NdiLib>,
    source_name: String,
    frame_slot: FrameSlot,
    condvar: Arc<std::sync::Condvar>,
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
                    let period = (1000 * frame.frame_rate_d as u64) / frame.frame_rate_n as u64;
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
                {
                    let mut slot = frame_slot.lock().unwrap_or_else(|e| e.into_inner());
                    *slot = Some(frame);
                }
                condvar.notify_all();
            }
            Ok(None) => {
                if connected && last_frame_time.elapsed() > std::time::Duration::from_secs(3) {
                    connected = false;
                    tracing::warn!("NDI signal lost for '{source_name}'");
                    if let Some(cb) = &status_cb {
                        cb("disconnected".to_string());
                    }
                }
                if stop_rx.has_changed().unwrap_or(false) && *stop_rx.borrow_and_update() {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_frame(id: u32) -> VideoFrame {
        VideoFrame {
            width: id,
            height: 1,
            data: vec![0u8; 4],
            stride: 4,
            fourcc: 0,
            frame_rate_n: 30,
            frame_rate_d: 1,
        }
    }

    #[test]
    fn frame_slot_newest_wins() {
        let slot: FrameSlot = Arc::new(std::sync::Mutex::new(None));
        {
            let mut s = slot.lock().unwrap();
            *s = Some(make_frame(1));
        }
        {
            let mut s = slot.lock().unwrap();
            *s = Some(make_frame(2));
        }
        let frame = slot.lock().unwrap().clone();
        assert_eq!(frame.unwrap().width, 2);
    }
}
```

**Important:** the capture thread now calls `condvar.notify_all()` (was `notify_one`) so all active tier encoders wake on each new frame.

- [ ] **Step 2: Run tests**

```bash
cargo test -p presenter-ndi --quiet
```

Expected: all tests pass (manager + tier + tier_registry + encoder).

- [ ] **Step 3: Commit**

```bash
git add crates/presenter-ndi/src/manager.rs
git commit -m "refactor(ndi): replace single encoder with TierRegistry (#250)"
```

---

### Task 8: Adaptive controller — pure logic + tests

**Files:**
- Modify: `crates/presenter-ndi/src/tier_registry.rs` (add controller types and tests)

- [ ] **Step 1: Append controller logic**

Append to `crates/presenter-ndi/src/tier_registry.rs` (above the `#[cfg(test)]` block):

```rust
// ---------------------------------------------------------------------------
// Adaptive controller — pure state machine, decoupled from any I/O.
// ---------------------------------------------------------------------------

/// Number of `Lagged` events within `LAG_WINDOW` that triggers a demote.
pub const LAG_THRESHOLD: u32 = 5;
/// Sliding window for counting `Lagged` events.
pub const LAG_WINDOW: Duration = Duration::from_secs(30);
/// How long a connection must run without any `Lagged` event before promoting.
pub const PROMOTE_AFTER: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy)]
pub enum AdaptEvent {
    /// A frame was successfully delivered.
    FrameOk,
    /// The broadcast subscriber lagged (one or more frames were dropped).
    Lagged,
}

#[derive(Debug, Clone, Copy)]
pub enum AdaptDecision {
    /// Stay on the current tier.
    Stay,
    /// Demote (or stay if already at floor).
    Demote,
    /// Promote (or stay if already at top).
    Promote,
}

/// Per-connection adaptive state. The controller is fed `(now, event)`
/// pairs and produces a decision. The caller applies the decision by
/// re-subscribing on the registry.
#[derive(Debug)]
pub struct AdaptController {
    lag_times: std::collections::VecDeque<Instant>,
    last_lag_at: Option<Instant>,
    started_at: Instant,
    last_promote_check: Instant,
}

impl AdaptController {
    pub fn new(now: Instant) -> Self {
        Self {
            lag_times: std::collections::VecDeque::new(),
            last_lag_at: None,
            started_at: now,
            last_promote_check: now,
        }
    }

    /// Returns the decision for the next step.
    pub fn observe(&mut self, now: Instant, event: AdaptEvent) -> AdaptDecision {
        // Drop expired lag entries.
        while let Some(t) = self.lag_times.front() {
            if now.duration_since(*t) > LAG_WINDOW {
                self.lag_times.pop_front();
            } else {
                break;
            }
        }

        match event {
            AdaptEvent::Lagged => {
                self.lag_times.push_back(now);
                self.last_lag_at = Some(now);
                if self.lag_times.len() as u32 >= LAG_THRESHOLD {
                    // Reset window after acting so we don't rapid-fire demote.
                    self.lag_times.clear();
                    self.last_promote_check = now;
                    AdaptDecision::Demote
                } else {
                    AdaptDecision::Stay
                }
            }
            AdaptEvent::FrameOk => {
                let reference = self.last_lag_at.unwrap_or(self.started_at);
                if now.duration_since(reference) >= PROMOTE_AFTER
                    && now.duration_since(self.last_promote_check) >= PROMOTE_AFTER
                {
                    self.last_promote_check = now;
                    AdaptDecision::Promote
                } else {
                    AdaptDecision::Stay
                }
            }
        }
    }
}
```

Then append tests inside the existing `#[cfg(test)] mod tests` block:

```rust
    #[test]
    fn controller_demotes_after_threshold_lags() {
        let t0 = Instant::now();
        let mut c = AdaptController::new(t0);
        for i in 0..(LAG_THRESHOLD - 1) {
            assert!(matches!(
                c.observe(t0 + Duration::from_secs(i as u64), AdaptEvent::Lagged),
                AdaptDecision::Stay
            ));
        }
        let last = c.observe(t0 + Duration::from_secs(LAG_THRESHOLD as u64), AdaptEvent::Lagged);
        assert!(matches!(last, AdaptDecision::Demote));
    }

    #[test]
    fn controller_window_expires_old_lags() {
        let t0 = Instant::now();
        let mut c = AdaptController::new(t0);
        // 4 lags very early
        for i in 0..4 {
            c.observe(t0 + Duration::from_secs(i), AdaptEvent::Lagged);
        }
        // Far in the future — old lags should not count any more.
        let later = t0 + LAG_WINDOW + Duration::from_secs(60);
        // One more lag — count should reset to 1, not threshold.
        let d = c.observe(later, AdaptEvent::Lagged);
        assert!(matches!(d, AdaptDecision::Stay));
    }

    #[test]
    fn controller_promotes_after_clean_period() {
        let t0 = Instant::now();
        let mut c = AdaptController::new(t0);
        // Inject a single lag so promotion is measured from "after the lag".
        c.observe(t0, AdaptEvent::Lagged);
        // FrameOk shortly after: must NOT promote yet.
        let early = c.observe(t0 + Duration::from_secs(10), AdaptEvent::FrameOk);
        assert!(matches!(early, AdaptDecision::Stay));
        // Past the cooldown: must promote.
        let late = c.observe(t0 + PROMOTE_AFTER + Duration::from_secs(1), AdaptEvent::FrameOk);
        assert!(matches!(late, AdaptDecision::Promote));
    }
```

- [ ] **Step 2: Re-export from lib**

In `crates/presenter-ndi/src/lib.rs`, extend the `tier_registry` re-export:

```rust
pub use tier_registry::{AdaptController, AdaptDecision, AdaptEvent, TierRegistry, TierSubscription};
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p presenter-ndi tier_registry --quiet
```

Expected: 6 passed (3 from Task 6 + 3 new).

- [ ] **Step 4: Commit**

```bash
git add crates/presenter-ndi/src/tier_registry.rs crates/presenter-ndi/src/lib.rs
git commit -m "feat(ndi): adaptive controller state machine (#250)"
```

---

### Task 9: Wire adaptive controller into MJPEG endpoints

**Files:**
- Modify: `crates/presenter-server/src/router/integrations/ndi.rs`

- [ ] **Step 1: Replace `mjpeg_http` and `mjpeg_ws`**

Replace the contents of `crates/presenter-server/src/router/integrations/ndi.rs` with:

```rust
use std::time::Instant;

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
use presenter_ndi::{AdaptController, AdaptDecision, AdaptEvent, TierSubscription};
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
    let payload = sources
        .into_iter()
        .map(|s| NdiSourceDto { name: s.name })
        .collect();
    Ok(Json(payload))
}

#[instrument(skip_all)]
pub(crate) async fn ndi_status(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "available": state.ndi_manager().is_some() }))
}

/// WebSocket endpoint that streams MJPEG frames. Adaptive: starts at L0
/// and demotes / promotes based on subscriber backpressure.
pub(crate) async fn mjpeg_ws(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let manager = state
        .ndi_manager()
        .ok_or_else(|| AppError::service_unavailable("NDI SDK not available"))?;
    let sub = manager.subscribe_frames(); // L0
    let manager = manager.clone();
    Ok(ws.on_upgrade(move |socket| handle_mjpeg_ws(socket, sub, manager)))
}

async fn handle_mjpeg_ws(
    mut socket: WebSocket,
    mut sub: TierSubscription,
    manager: std::sync::Arc<presenter_ndi::NdiManager>,
) {
    let mut ctrl = AdaptController::new(Instant::now());
    loop {
        match sub.rx.recv().await {
            Ok(jpeg) => {
                if socket.send(Message::Binary(jpeg.to_vec().into())).await.is_err() {
                    break;
                }
                if let AdaptDecision::Promote = ctrl.observe(Instant::now(), AdaptEvent::FrameOk) {
                    if let Some(target) = sub.tier().promote() {
                        sub = manager.subscribe_tier(target);
                    }
                }
            }
            Err(RecvError::Lagged(_)) => {
                if let AdaptDecision::Demote = ctrl.observe(Instant::now(), AdaptEvent::Lagged) {
                    if let Some(target) = sub.tier().demote() {
                        sub = manager.subscribe_tier(target);
                    }
                }
            }
            Err(RecvError::Closed) => break,
        }
    }
}

/// HTTP MJPEG stream using multipart/x-mixed-replace. Adaptive: the
/// stream future demotes/promotes its tier subscription based on
/// backpressure observed via `RecvError::Lagged`.
pub(crate) async fn mjpeg_http(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let manager = state
        .ndi_manager()
        .ok_or_else(|| AppError::service_unavailable("NDI SDK not available"))?;
    let sub = manager.subscribe_frames();
    let manager = manager.clone();

    let boundary = "mjpegboundary";
    let content_type = format!("multipart/x-mixed-replace; boundary={boundary}");

    let stream = async_stream::stream! {
        let mut sub = sub;
        let mut ctrl = AdaptController::new(Instant::now());
        loop {
            match sub.rx.recv().await {
                Ok(jpeg) => {
                    let part_header = format!(
                        "--{boundary}\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
                        jpeg.len()
                    );
                    yield Ok::<Bytes, std::convert::Infallible>(Bytes::from(part_header));
                    yield Ok(jpeg);
                    yield Ok(Bytes::from("\r\n"));
                    if let AdaptDecision::Promote = ctrl.observe(Instant::now(), AdaptEvent::FrameOk) {
                        if let Some(target) = sub.tier().promote() {
                            sub = manager.subscribe_tier(target);
                            tracing::info!(?target, "MJPEG client promoted");
                        }
                    }
                }
                Err(RecvError::Lagged(_)) => {
                    if let AdaptDecision::Demote = ctrl.observe(Instant::now(), AdaptEvent::Lagged) {
                        if let Some(target) = sub.tier().demote() {
                            sub = manager.subscribe_tier(target);
                            tracing::warn!(?target, "MJPEG client demoted on backpressure");
                        }
                    }
                }
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

- [ ] **Step 2: Build to verify compile**

```bash
cargo build -p presenter-server --quiet
```

Expected: clean build.

- [ ] **Step 3: Commit**

```bash
git add crates/presenter-server/src/router/integrations/ndi.rs
git commit -m "feat(server): adaptive MJPEG controller on /ndi/mjpeg + ws (#250)"
```

---

### Task 10: Local check — fmt + clippy + full test suite

**Files:** none (verification only)

- [ ] **Step 1: Format**

```bash
cargo fmt --all
git diff --stat
```

If there are unstaged formatting changes, stage and commit them as `style: cargo fmt`.

- [ ] **Step 2: Clippy on workspace**

```bash
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
```

Expected: zero warnings.

- [ ] **Step 3: Clippy on the WASM frontend (excluded from workspace)**

```bash
( cd crates/presenter-ui && cargo clippy --all-targets -- -D warnings -W clippy::all )
```

Expected: zero warnings. (No code change in this PR for the WASM crate, but the version bump and workspace touched it; clippy must still pass.)

- [ ] **Step 4: Tests**

```bash
cargo test -p presenter-ndi --quiet
cargo test -p presenter-server --quiet
```

Expected: all green.

- [ ] **Step 5: If anything failed, fix in ONE batched commit**

Do not push partial fixes. Address ALL failures and stage them together.

```bash
git add -A
git commit -m "fix: address fmt/clippy/test issues (#250)"
```

---

### Task 11: Push to dev, monitor CI

**Files:** none (CI / deploy)

- [ ] **Step 1: Push**

```bash
git push origin dev
```

- [ ] **Step 2: Identify the run**

```bash
gh run list --branch dev --limit 3
```

Note the latest run ID for the push.

- [ ] **Step 3: Monitor in background**

```bash
sleep 600 && gh run view <run-id> --json status,conclusion,jobs
```

(Use `run_in_background: true` via the Bash tool. Do NOT poll in a tight loop.)

- [ ] **Step 4: When the run completes**

- If green: proceed to Task 12.
- If red: `gh run view <run-id> --log-failed`. Fix ALL failures in ONE commit, push once more.

---

### Task 12: Post-deploy profiling on dev (sd1l + sd2l with the fix)

**Files:**
- Modify: `docs/superpowers/specs/2026-04-25-ndi-cheap-tv-adaptive-design.md`

- [ ] **Step 1: Confirm dev deploy succeeded**

```bash
curl -s http://10.77.8.134:8080/healthz | python3 -m json.tool
```

Expected: `version=0.4.34`, `channel=dev`.

- [ ] **Step 2: Re-run the chrome://inspect Performance trace on sd1l**

Same procedure as Task 1 Step 4. Let the connection run for ~2 minutes BEFORE recording — the adaptive controller needs to settle. Capture the steady-state tier (look at server logs `tracing::info!("MJPEG client promoted/demoted ...")` to confirm the tier the connection lands on).

```bash
sshpass -p 'newlevel' ssh newlevel@10.77.8.134 'sudo journalctl -u presenter-dev -n 200 --no-pager' | grep -E 'MJPEG client|tier encoder'
```

Record the steady-state tier reached by sd1l. Capture decode/paint/fps numbers at that tier.

- [ ] **Step 3: Repeat for sd2l**

Same procedure. Different TV, possibly different settled tier.

- [ ] **Step 4: Update spec Findings**

Add a sub-table under Findings:

```markdown
### Post-fix (with adaptive controller), 2026-04-25

| TV | Settled tier | decode_p50 | decode_p95 | paint_p50 | fps_sustained | kbps |
|---|---|---|---|---|---|---|
| sd1l | <fill> | <fill> | <fill> | <fill> | <fill> | <fill> |
| sd2l | <fill> | <fill> | <fill> | <fill> | <fill> | <fill> |
```

- [ ] **Step 5: User acceptance gate**

Decode_p95 + paint_p50 must be `< frame_interval(target_fps)` for the settled tier on each TV. If sd2l is still struggling at L3 (>100 ms), return to design — the floor is too high.

- [ ] **Step 6: Commit**

```bash
git add docs/superpowers/specs/2026-04-25-ndi-cheap-tv-adaptive-design.md
git commit -m "docs(spec): record post-fix NDI profiling on sd1l + sd2l (#250)"
git push origin dev
```

Wait for the second CI run to go green (Task 11 monitoring routine).

---

### Task 13: Open PR dev → main

**Files:** none (workflow)

- [ ] **Step 1: Verify branch is clean and ahead of main**

```bash
git fetch origin
git status
git log --oneline origin/main..dev | head -20
```

- [ ] **Step 2: Open PR**

```bash
gh pr create --base main --head dev --title "feat(ndi): adaptive MJPEG with tiered shared encoders (#250)" --body "$(cat <<'EOF'
## Summary

- Per-connection adaptive MJPEG: each `<img src="/ndi/mjpeg">` connection auto-degrades to the highest tier its consumer can sustain.
- Server-side: tiered shared encoders (L0=1080@30, L1=1080@15, L2=720@15, L3=720@10) replace the single global encoder. Lazy + ref-counted — N connections on the same tier cost the same as one.
- No DB migration, no UI changes, no client changes. Operator surface is unchanged.

## Spec & plan
- Spec: `docs/superpowers/specs/2026-04-25-ndi-cheap-tv-adaptive-design.md`
- Plan: `docs/superpowers/plans/2026-04-25-ndi-cheap-tv-adaptive.md`

## Findings
See "Findings" section of the spec — baseline + post-fix profiling on sd1l.lan (Tesla LEAP-S1) and sd2l.lan (Hyundai 1 GB).

## Test plan
- [ ] All unit tests in `presenter-ndi` pass (Tier ladder, registry refcount, controller transitions, encoder resize).
- [ ] All integration / E2E tests pass.
- [ ] Mutation testing job passes.
- [ ] Clippy clean (workspace + presenter-ui).
- [ ] Manual on sd1l.lan: post-fix decode + paint < frame_interval at settled tier.
- [ ] Manual on sd2l.lan: same.
- [ ] After merge, manual on production sd1l-sd4l.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 3: Monitor PR CI**

```bash
gh pr view --json url,number
```

Note the PR URL. Then:

```bash
sleep 600 && gh pr checks <pr-number>
```

Run via `run_in_background: true`. Wait for terminal state. Fix any failures in one commit, push.

- [ ] **Step 4: Verify mergeable state**

```bash
gh api "repos/zbynekdrlik/presenter/pulls/<pr-number>" --jq '{mergeable, mergeable_state}'
```

Required: `mergeable=true`, `mergeable_state="clean"`.

- [ ] **Step 5: Post the green PR URL to the user**

DO NOT merge. The merge happens only on explicit user instruction.

---

### Task 14: After merge — production deploy verification

**Files:** none (workflow)

- [ ] **Step 1: After user says "merge it", merge**

```bash
gh pr merge <pr-number> --merge
```

(Plain merge — no squash, no rebase. Per project commit convention.)

- [ ] **Step 2: Monitor the deploy.yml workflow on main**

```bash
gh run list --branch main --limit 3
```

Identify the deploy run, then `sleep 600 && gh run view <run-id> --json status,conclusion,jobs` in background. Wait for terminal state.

- [ ] **Step 3: Verify production version**

```bash
curl -s http://10.77.9.205/healthz | python3 -m json.tool
```

Expected: `version=0.4.34`, `channel=release`.

- [ ] **Step 4: Production functional verification on all four TVs**

For each of sd1l-sd4l:

```bash
# Force ndi-fullscreen layout in production
curl -s -X POST -H 'Content-Type: application/json' -d '{"code":"ndi-fullscreen"}' http://10.77.9.205/stage/layout

# Confirm the kiosk is foreground and pointing at production
adb -s <tv>:5555 shell dumpsys activity activities | grep mResumedActivity
```

For each TV, attach `chrome://inspect`, run a 30-second steady-state observation, confirm:
- The connection settles on a tier (server logs show tier promote/demote events).
- `decode_p95 + paint_p50 < frame_interval` at the settled tier.
- Browser console has zero errors / warnings on the kiosk webview.

- [ ] **Step 5: Restore the production stage layout to its previous setting**

If the church-default layout is `worship-snv` (or whatever was active before), restore it:

```bash
curl -s -X POST -H 'Content-Type: application/json' -d '{"code":"worship-snv"}' http://10.77.9.205/stage/layout
```

- [ ] **Step 6: Send completion report**

Per `airuleset/modules/core/completion-report.md`. Include:
- E2E test coverage table referencing `tests/e2e/stage-api-ndi.spec.ts` and `tests/e2e/ndi-stage-layout.spec.ts` (existing — no new E2E needed; adaptive logic is server-internal and unit-tested).
- Production verification details for all 4 TVs.
- Findings table excerpt with concrete numbers.

---

## Verification Summary

| Check | How verified |
|---|---|
| Tier ladder values match spec | Unit test `specs_match_design` in `tier.rs` |
| Tier ladder transitions correct | Unit tests in `tier.rs` |
| Encoder resize preserves aspect | Unit test `encode_bgra_resized_caps_height_and_preserves_aspect` |
| Registry refcount lifecycle | Unit tests in `tier_registry.rs` |
| Adaptive controller demote/promote | Unit tests in `tier_registry.rs` |
| Server compile + clippy clean | Task 10 |
| CI green | Tasks 11, 13 |
| Cheap TV settles at usable tier (dev) | Task 12 (manual, recorded in spec) |
| Cheap TV settles at usable tier (prod) | Task 14 (manual, recorded in completion report) |
| No regressions on api / ndi-fullscreen layouts | Existing E2E suite (Task 11 / 13 CI) |
| Clean browser console | E2E + manual chrome://inspect on prod TVs |
