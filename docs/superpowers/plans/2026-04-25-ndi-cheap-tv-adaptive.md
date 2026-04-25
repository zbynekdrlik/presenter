# NDI Cheap-TV Adaptive Streaming Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `/ndi/mjpeg` self-tune per-connection so cheap Android TVs (Hyundai 1 GB, Tesla LEAP-S1 2 GB) can render NDI fullscreen without manual config, while fast clients stay native and the N100 production server's CPU cost stays bounded.

**Architecture:** Replace the single global JPEG encoder with a `TierRegistry` that runs at most 4 lazy ref-counted tier encoders (1080@30, 1080@15, 720@15, 720@10), each consuming the same `tokio::sync::watch` raw-frame source. Each MJPEG HTTP/WS connection holds a per-tier `broadcast::Receiver<Bytes>` plus an `AdaptController` that demotes one tier on 5+ `RecvError::Lagged` events in 30 s and promotes one tier after 60 s of zero lag. No DB, no UI, no client changes.

**Tech Stack:** Rust (tokio, axum, broadcast, watch, async-stream), turbojpeg, image (new dep), Playwright for existing E2E only.

**Spec:** `docs/superpowers/specs/2026-04-25-ndi-cheap-tv-adaptive-design.md` (commit `07cbae6`)

---

## Context

Issue [#250](https://github.com/zbynekdrlik/presenter/issues/250). Today's pipeline: `presenter-ndi::manager::run_capture_thread` writes the newest `VideoFrame` to a `FrameSlot` (`Arc<Mutex<Option<VideoFrame>>>`) and notifies a `Condvar`. `run_encode_thread` waits, takes the frame, calls `JpegEncoder::encode_bgra` (or `encode_uyvy`), and broadcasts JPEG `Bytes` on a `broadcast::channel(8)`. `mjpeg_http` and `mjpeg_ws` in `presenter-server::router::integrations::ndi` subscribe and forward.

Measured 2026-04-25 against `RESOLUME-SNV (cg-obs)`: 1920×1080 @ ~29.6 fps, ~115 KB/frame, ~24.5 Mbps. Cheap TVs (Amlogic SoC, software JPEG decode) cannot keep pace. Production server is Intel N100 (4 cores), so per-connection encoding does not scale.

**Image-handling constraint for profiling tasks (Task 1, 2, 12, 14):** Save screenshots and DevTools profile exports to `/tmp/ndi-profiling/` only. Do **NOT** open captured PNGs/JPEGs with the Read tool — recent API error `req_011CaQjH9cNLofQDkTHDg9XX` was triggered by image content. Read the DevTools `.json` profile exports (text), and report numbers extracted from the JSON. Screenshots exist only as paths the human user can open, never as Read inputs.

---

## File Structure

| File | Status | Responsibility |
|---|---|---|
| `Cargo.toml` (workspace) | Modify | Bump `version = "0.4.34"`; add workspace dep `image = "0.25"` if used as workspace dep, otherwise per-crate. |
| `crates/presenter-ndi/Cargo.toml` | Modify | Add `image = "0.25"` (default features off, only `std`). |
| `crates/presenter-ndi/src/tier.rs` | Create | `Tier` enum (L0..L3), `TierSpec` (height + fps + frame_skip_modulus), demote/promote helpers, tests. |
| `crates/presenter-ndi/src/tier_registry.rs` | Create | `TierRegistry` (lazy ref-counted spawn), `TierSubscription` (RAII), spawn loop reading from raw watch. |
| `crates/presenter-ndi/src/encoder.rs` | Modify | Add `encode_bgra_resized(bgra, src_w, src_h, target_h) -> Vec<u8>` and `convert_uyvy_to_bgra` pub fn. Tests. |
| `crates/presenter-ndi/src/manager.rs` | Modify | Replace `frame_tx: broadcast::Sender<Bytes>` with `raw_frame_tx: watch::Sender<Option<Arc<VideoFrame>>>` + a `tier_registry: Arc<TierRegistry>`. Capture thread sends raw frames to watch. `subscribe_frames()` removed; new `subscribe_tier(Tier) -> TierSubscription`. `run_encode_thread` deleted. |
| `crates/presenter-ndi/src/lib.rs` | Modify | `pub mod tier; pub mod tier_registry;` and re-export `Tier`, `TierRegistry`, `TierSubscription`. |
| `crates/presenter-server/src/adaptive_mjpeg.rs` | Create | `AdaptController` state machine (sliding lag window, demote/promote rules) + tests. |
| `crates/presenter-server/src/router.rs` | Modify | `pub mod adaptive_mjpeg;` (or add the `mod` declaration in `main.rs`/`lib.rs` — match existing pattern). |
| `crates/presenter-server/src/main.rs` | Modify | `mod adaptive_mjpeg;` (current style — see existing `mod ai;` etc.). |
| `crates/presenter-server/src/router/integrations/ndi.rs` | Modify | `mjpeg_http` and `mjpeg_ws`: subscribe to `Tier::L0`, drive `AdaptController`, swap subscriptions on transitions. |
| `docs/superpowers/specs/2026-04-25-ndi-cheap-tv-adaptive-design.md` | Modify | Fill in the Findings section in Tasks 1, 2, 12. |
| `/tmp/ndi-profiling/` | Create at runtime | Profiling artifacts (DevTools `.json` exports, `screenshot-*.png`). Not committed. |

---

## Task 1: Baseline profile sd1l.lan (Tesla LEAP-S1)

**Files:** None (data collection). Output appended to `docs/superpowers/specs/2026-04-25-ndi-cheap-tv-adaptive-design.md` Findings section in Task 12.

This task captures the **before** numbers — current native pipeline (1080p @ 30) on the Tesla TV. Implementation comes later; we measure first so the post-deploy numbers in Task 12 have a reference point.

- [ ] **Step 1: Prepare the dev server stream**

```bash
# Confirm dev server is running and the cg-obs source is active
curl -s http://10.77.8.134:8080/integrations/video-sources | python3 -c "import sys,json; d=json.load(sys.stdin); print([(x['label'], x['isActive']) for x in d])"
```

Expected: `cg` row shows `isActive: True`. If not, activate it via the settings UI before continuing.

- [ ] **Step 2: Set up artifact directory + ADB reverse**

```bash
mkdir -p /tmp/ndi-profiling/sd1l-baseline
adb -s sd1l.lan:5555 reverse tcp:8080 tcp:8080
adb -s sd1l.lan:5555 reverse --list
```

Expected: `host-9 tcp:8080 tcp:8080`.

- [ ] **Step 3: Restart Fully Kiosk with WebView debugging**

```bash
adb -s sd1l.lan:5555 shell am force-stop com.fullykiosk.videokiosk
adb -s sd1l.lan:5555 shell am start -n com.fullykiosk.videokiosk/de.ozerov.fully.MainActivity --es WEBVIEW_DEBUG true
sleep 5
adb -s sd1l.lan:5555 shell "dumpsys activity activities | grep -E 'mResumedActivity|topResumedActivity'" | head -2
```

Expected: `mResumedActivity` shows `com.fullykiosk.videokiosk/de.ozerov.fully.FullyActivity`.

- [ ] **Step 4: Switch dev stage to ndi-fullscreen**

```bash
curl -s -X POST http://10.77.8.134:8080/stage/layout -H 'content-type: application/json' -d '{"code":"ndi-fullscreen"}'
curl -s http://10.77.8.134:8080/stage/layout
```

Expected: response contains `"code":"ndi-fullscreen"`.

- [ ] **Step 5: Attach DevTools and record 10 s Performance trace**

Open `chrome://inspect/#devices` on the dev machine. The Fully Kiosk webview should be listed under sd1l.lan. Click "inspect". In the DevTools Performance tab, click Record, wait 10 s, click Stop.

Save the trace as `/tmp/ndi-profiling/sd1l-baseline/trace.json` via DevTools "Save profile…".

Capture a Network tab summary screenshot to `/tmp/ndi-profiling/sd1l-baseline/network.png`.

**Do not read the PNG with the Read tool. Only report file paths.**

- [ ] **Step 6: Extract numbers from trace.json (text only)**

```bash
python3 - <<'PY'
import json, statistics
with open('/tmp/ndi-profiling/sd1l-baseline/trace.json') as f:
    data = json.load(f)
events = data.get('traceEvents', data) if isinstance(data, dict) else data
def durs(name):
    return [e['dur']/1000.0 for e in events
            if e.get('ph')=='X' and e.get('name')==name and 'dur' in e]
decode = durs('Decode Image')
paint = durs('Paint')
def stats(label, xs):
    if not xs: print(f'{label}: n=0'); return
    xs.sort()
    p50 = xs[len(xs)//2]
    p95 = xs[int(len(xs)*0.95)]
    print(f'{label}: n={len(xs)} p50={p50:.2f}ms p95={p95:.2f}ms max={xs[-1]:.2f}ms')
stats('decode', decode)
stats('paint', paint)
PY
```

Record p50/p95 for decode and paint. Save the printout as `/tmp/ndi-profiling/sd1l-baseline/stats.txt`.

- [ ] **Step 7: Sustained-FPS measurement from the server side**

```bash
python3 - <<'PY' > /tmp/ndi-profiling/sd1l-baseline/server-fps.txt
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
elapsed=10
print(f'frames={frames} fps={frames/elapsed:.1f} kbps={total*8/1000/elapsed:.0f}')
PY
cat /tmp/ndi-profiling/sd1l-baseline/server-fps.txt
```

Note: this measures what the SERVER pushes, not what the TV decodes. The TV's effective FPS is in the trace (count of `Decode Image` events / 10 s).

- [ ] **Step 8: Save a parseable summary**

```bash
cat <<EOF > /tmp/ndi-profiling/sd1l-baseline/SUMMARY.md
# sd1l.lan baseline (Tesla LEAP-S1, current native pipeline)
- decode_p50: <fill from stats.txt>
- decode_p95: <fill from stats.txt>
- paint_p50: <fill from stats.txt>
- fps_sustained_browser: <Decode Image count / 10>
- server_fps: <from server-fps.txt>
- server_kbps: <from server-fps.txt>
EOF
```

Manually edit the angle-bracket placeholders in `SUMMARY.md` with the values just measured. This file is the input for Task 12 when filling the spec.

- [ ] **Step 9: Mark task done**

No commit yet — these artifacts live under `/tmp` and are not committed; the spec gets updated in Task 12 along with post-deploy numbers.

---

## Task 2: Baseline profile sd2l.lan (Hyundai)

**Files:** None (data collection). Output saved to `/tmp/ndi-profiling/sd2l-baseline/`.

The Hyundai TVs have only 1 GB RAM and Android 11 — expect substantially worse numbers than sd1l. If decode_p95 already exceeds 33 ms at native 1080p (very likely), that confirms the hypothesis: the issue is software JPEG decode on cheap chips, not network or paint.

- [ ] **Step 1: Prepare artifact dir + ADB reverse for sd2l**

```bash
mkdir -p /tmp/ndi-profiling/sd2l-baseline
adb -s sd2l.lan:5555 reverse tcp:8080 tcp:8080
adb -s sd2l.lan:5555 reverse --list
```

Expected: `host-9 tcp:8080 tcp:8080`.

- [ ] **Step 2: Restart Fully Kiosk with WebView debugging on sd2l**

```bash
adb -s sd2l.lan:5555 shell am force-stop com.fullykiosk.videokiosk
adb -s sd2l.lan:5555 shell am start -n com.fullykiosk.videokiosk/de.ozerov.fully.MainActivity --es WEBVIEW_DEBUG true
sleep 5
adb -s sd2l.lan:5555 shell "dumpsys activity activities | grep -E 'mResumedActivity|topResumedActivity'" | head -2
```

Expected: `mResumedActivity` shows `com.fullykiosk.videokiosk/de.ozerov.fully.FullyActivity`.

- [ ] **Step 3: Confirm dev stage is still on ndi-fullscreen**

```bash
curl -s http://10.77.8.134:8080/stage/layout
```

Expected: `"code":"ndi-fullscreen"`. If not, repost as in Task 1 Step 4.

- [ ] **Step 4: Attach DevTools and record 10 s Performance trace**

Open `chrome://inspect/#devices`, attach to the sd2l.lan kiosk webview, Performance tab → Record 10 s → Stop → Save profile to `/tmp/ndi-profiling/sd2l-baseline/trace.json`.

Save Network tab summary screenshot to `/tmp/ndi-profiling/sd2l-baseline/network.png` — **do not Read this PNG**.

- [ ] **Step 5: Extract numbers from trace.json**

```bash
python3 - <<'PY'
import json, statistics
with open('/tmp/ndi-profiling/sd2l-baseline/trace.json') as f:
    data = json.load(f)
events = data.get('traceEvents', data) if isinstance(data, dict) else data
def durs(name):
    return [e['dur']/1000.0 for e in events
            if e.get('ph')=='X' and e.get('name')==name and 'dur' in e]
decode = durs('Decode Image')
paint = durs('Paint')
def stats(label, xs):
    if not xs: print(f'{label}: n=0'); return
    xs.sort()
    p50 = xs[len(xs)//2]
    p95 = xs[int(len(xs)*0.95)]
    print(f'{label}: n={len(xs)} p50={p50:.2f}ms p95={p95:.2f}ms max={xs[-1]:.2f}ms')
stats('decode', decode)
stats('paint', paint)
PY
```

Save the printout to `/tmp/ndi-profiling/sd2l-baseline/stats.txt`.

- [ ] **Step 6: Sustained-FPS measurement from server side (same source check, just file under sd2l-baseline)**

```bash
python3 - <<'PY' > /tmp/ndi-profiling/sd2l-baseline/server-fps.txt
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
elapsed=10
print(f'frames={frames} fps={frames/elapsed:.1f} kbps={total*8/1000/elapsed:.0f}')
PY
cat /tmp/ndi-profiling/sd2l-baseline/server-fps.txt
```

- [ ] **Step 7: Save parseable summary**

```bash
cat <<EOF > /tmp/ndi-profiling/sd2l-baseline/SUMMARY.md
# sd2l.lan baseline (Hyundai 1 GB, current native pipeline)
- decode_p50: <fill from stats.txt>
- decode_p95: <fill from stats.txt>
- paint_p50: <fill from stats.txt>
- fps_sustained_browser: <Decode Image count / 10>
- server_fps: <from server-fps.txt>
- server_kbps: <from server-fps.txt>
EOF
```

Manually fill the placeholders in `SUMMARY.md`. This file feeds Task 12.

- [ ] **Step 8: Mark task done**

No commit yet — artifacts live under `/tmp` and are not committed.

---

## Task 3: Workspace prep — version bump 0.4.34 + image dep

**Files:**
- Modify: `Cargo.toml:15`
- Modify: `crates/presenter-ndi/Cargo.toml:9-19`

- [ ] **Step 1: Bump workspace version**

In `Cargo.toml`, change line 15:

```toml
# Old:
version = "0.4.33"

# New:
version = "0.4.34"
```

- [ ] **Step 2: Add image crate to presenter-ndi**

In `crates/presenter-ndi/Cargo.toml`, append after line 19 (last existing dep):

```toml
image = { version = "0.25", default-features = false }
```

- [ ] **Step 3: Verify build still compiles**

```bash
cargo build -p presenter-ndi 2>&1 | tail -5
```

Expected: `Finished `dev` profile`.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock crates/presenter-ndi/Cargo.toml
git commit -m "chore: bump version to 0.4.34 and add image dep for tier resize (#250)"
```

---

## Task 4: Tier enum and transitions

**Files:**
- Create: `crates/presenter-ndi/src/tier.rs`
- Modify: `crates/presenter-ndi/src/lib.rs`

- [ ] **Step 1: Write the failing tier.rs with tests first**

Create `crates/presenter-ndi/src/tier.rs`:

```rust
//! Adaptive streaming tier ladder for `/ndi/mjpeg`.
//!
//! Four tiers chosen to keep text readable (floor at 720p) while degrading
//! framerate first (Resolume composed graphics don't move much).
//!
//! L0 (native): 1080p @ 30 fps  ~24 Mbps
//! L1:          1080p @ 15 fps  ~12 Mbps
//! L2:          720p  @ 15 fps  ~6 Mbps
//! L3 (floor):  720p  @ 10 fps  ~4 Mbps

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tier {
    L0,
    L1,
    L2,
    L3,
}

#[derive(Debug, Clone, Copy)]
pub struct TierSpec {
    pub target_height: u32,
    pub target_fps: u32,
    pub frame_skip_modulus: u32,
}

impl Tier {
    pub const ALL: [Tier; 4] = [Tier::L0, Tier::L1, Tier::L2, Tier::L3];

    pub fn spec(self) -> TierSpec {
        match self {
            Tier::L0 => TierSpec { target_height: 1080, target_fps: 30, frame_skip_modulus: 1 },
            Tier::L1 => TierSpec { target_height: 1080, target_fps: 15, frame_skip_modulus: 2 },
            Tier::L2 => TierSpec { target_height: 720,  target_fps: 15, frame_skip_modulus: 2 },
            Tier::L3 => TierSpec { target_height: 720,  target_fps: 10, frame_skip_modulus: 3 },
        }
    }

    /// One step worse. Returns `None` at the floor.
    pub fn demote(self) -> Option<Tier> {
        match self {
            Tier::L0 => Some(Tier::L1),
            Tier::L1 => Some(Tier::L2),
            Tier::L2 => Some(Tier::L3),
            Tier::L3 => None,
        }
    }

    /// One step better. Returns `None` at native.
    pub fn promote(self) -> Option<Tier> {
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
    fn l0_is_native_1080p_30fps() {
        let s = Tier::L0.spec();
        assert_eq!(s.target_height, 1080);
        assert_eq!(s.target_fps, 30);
        assert_eq!(s.frame_skip_modulus, 1);
    }

    #[test]
    fn l3_is_floor_720p_10fps() {
        let s = Tier::L3.spec();
        assert_eq!(s.target_height, 720);
        assert_eq!(s.target_fps, 10);
        assert_eq!(s.frame_skip_modulus, 3);
    }

    #[test]
    fn demote_walks_l0_to_l3_then_none() {
        assert_eq!(Tier::L0.demote(), Some(Tier::L1));
        assert_eq!(Tier::L1.demote(), Some(Tier::L2));
        assert_eq!(Tier::L2.demote(), Some(Tier::L3));
        assert_eq!(Tier::L3.demote(), None);
    }

    #[test]
    fn promote_walks_l3_to_l0_then_none() {
        assert_eq!(Tier::L3.promote(), Some(Tier::L2));
        assert_eq!(Tier::L2.promote(), Some(Tier::L1));
        assert_eq!(Tier::L1.promote(), Some(Tier::L0));
        assert_eq!(Tier::L0.promote(), None);
    }

    #[test]
    fn all_lists_every_tier() {
        assert_eq!(Tier::ALL.len(), 4);
        for t in Tier::ALL {
            // every tier round-trips through spec()
            let _ = t.spec();
        }
    }
}
```

- [ ] **Step 2: Wire into lib.rs**

In `crates/presenter-ndi/src/lib.rs`, add after line 7 (existing `mod` lines):

```rust
pub mod tier;
```

And add a re-export after existing `pub use` lines:

```rust
pub use tier::{Tier, TierSpec};
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p presenter-ndi tier:: 2>&1 | tail -15
```

Expected: 5 passed.

- [ ] **Step 4: Commit**

```bash
git add crates/presenter-ndi/src/tier.rs crates/presenter-ndi/src/lib.rs
git commit -m "feat(ndi): add Tier enum with promote/demote ladder (#250)"
```

---

## Task 5: JpegEncoder resize variants

**Files:**
- Modify: `crates/presenter-ndi/src/encoder.rs`

- [ ] **Step 1: Make uyvy_to_bgra public (used by tier encoders)**

In `crates/presenter-ndi/src/encoder.rs`, change line 42:

```rust
// Old:
fn uyvy_to_bgra(uyvy: &[u8], width: u32, height: u32) -> Vec<u8> {

// New:
pub fn uyvy_to_bgra(uyvy: &[u8], width: u32, height: u32) -> Vec<u8> {
```

- [ ] **Step 2: Add encode_bgra_resized method with TDD**

Append the following to `crates/presenter-ndi/src/encoder.rs` (replace the existing `#[cfg(test)]` block if present, or append if not):

```rust
impl JpegEncoder {
    /// Resize BGRA pixel data to `target_height` (preserving aspect) and JPEG-encode.
    ///
    /// If `src_height == target_height`, this is a fast path that skips resize.
    /// Otherwise uses `image::imageops::resize` with the `Triangle` filter,
    /// chosen for cheap CPU cost over Lanczos quality (the difference is
    /// imperceptible at typical NDI-display sizes).
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

        let img = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(src_width, src_height, bgra.to_vec())
            .ok_or_else(|| anyhow::anyhow!("BGRA buffer size mismatch: {} bytes for {}x{}", bgra.len(), src_width, src_height))?;
        let resized = image::imageops::resize(&img, target_width, target_height, image::imageops::FilterType::Triangle);
        self.encode_bgra(resized.as_raw(), target_width, target_height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_bgra(w: u32, h: u32) -> Vec<u8> {
        // Simple gradient so resize has something to interpolate
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                out.push((x % 256) as u8);    // B
                out.push((y % 256) as u8);    // G
                out.push(((x + y) % 256) as u8); // R
                out.push(255);                // A
            }
        }
        out
    }

    #[test]
    fn encode_bgra_resized_passthrough_when_target_equals_source() {
        let bgra = make_bgra(64, 64);
        let enc = JpegEncoder::new(75);
        let jpeg = enc.encode_bgra_resized(&bgra, 64, 64, 64).unwrap();
        assert!(jpeg.starts_with(&[0xff, 0xd8, 0xff]), "JPEG SOI marker missing");
    }

    #[test]
    fn encode_bgra_resized_downscales_aspect_preserved() {
        // Resize 1920x1080 → 720 height. Width must scale to 1280 (preserving 16:9).
        let bgra = make_bgra(1920, 1080);
        let enc = JpegEncoder::new(75);
        let jpeg = enc.encode_bgra_resized(&bgra, 1920, 1080, 720).unwrap();
        assert!(jpeg.starts_with(&[0xff, 0xd8, 0xff]));

        // Decode and check dims
        let img = turbojpeg::decompress(&jpeg, turbojpeg::PixelFormat::BGRA).unwrap();
        assert_eq!(img.height, 720);
        assert_eq!(img.width, 1280);
    }

    #[test]
    fn encode_bgra_resized_rejects_wrong_buffer_size() {
        let bgra = vec![0u8; 16]; // way too small
        let enc = JpegEncoder::new(75);
        let err = enc.encode_bgra_resized(&bgra, 100, 100, 50).unwrap_err();
        assert!(err.to_string().contains("buffer size mismatch"));
    }

    #[test]
    fn uyvy_to_bgra_produces_4bytes_per_pixel() {
        // 4x2 dummy UYVY frame
        let uyvy = vec![128u8; 4 * 2 * 2]; // 2 bytes per pixel
        let bgra = uyvy_to_bgra(&uyvy, 4, 2);
        assert_eq!(bgra.len(), 4 * 2 * 4);
    }
}
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p presenter-ndi encoder:: 2>&1 | tail -15
```

Expected: 4 passed.

- [ ] **Step 4: Commit**

```bash
git add crates/presenter-ndi/src/encoder.rs
git commit -m "feat(ndi): add encode_bgra_resized for tier downscale (#250)"
```

---

## Task 6: TierRegistry + TierSubscription

**Files:**
- Create: `crates/presenter-ndi/src/tier_registry.rs`
- Modify: `crates/presenter-ndi/src/lib.rs`

- [ ] **Step 1: Create tier_registry.rs**

```rust
//! Lazy ref-counted registry of per-tier JPEG broadcasters.
//!
//! Each `Tier` has at most one running encoder task; the task is spawned
//! when a subscriber registers and stopped when the last subscriber drops.
//! This decouples server CPU cost from client count: 4 clients on the same
//! tier share one encoder.

use std::collections::HashMap;
use std::sync::Arc;

use bytes::Bytes;
use tokio::sync::{broadcast, watch, Mutex};
use tokio::task::JoinHandle;

use crate::encoder::{uyvy_to_bgra, JpegEncoder};
use crate::receiver::VideoFrame;
use crate::tier::Tier;

const JPEG_BROADCAST_CAPACITY: usize = 4;

/// Newest-wins raw-frame channel. `None` means no active stream.
pub type RawFrameRx = watch::Receiver<Option<Arc<VideoFrame>>>;
pub type RawFrameTx = watch::Sender<Option<Arc<VideoFrame>>>;

struct TierEntry {
    jpeg_tx: broadcast::Sender<Bytes>,
    refcount: usize,
    stop_tx: watch::Sender<bool>,
    handle: JoinHandle<()>,
}

/// Handle held by an MJPEG connection. Drop = unsubscribe + decrement refcount.
pub struct TierSubscription {
    tier: Tier,
    pub rx: broadcast::Receiver<Bytes>,
    registry: Arc<TierRegistry>,
}

impl TierSubscription {
    pub fn tier(&self) -> Tier {
        self.tier
    }
}

impl Drop for TierSubscription {
    fn drop(&mut self) {
        let registry = Arc::clone(&self.registry);
        let tier = self.tier;
        // We can't .await in Drop; spawn a release task.
        tokio::spawn(async move {
            registry.release(tier).await;
        });
    }
}

pub struct TierRegistry {
    entries: Mutex<HashMap<Tier, TierEntry>>,
    raw_rx: RawFrameRx,
}

impl TierRegistry {
    pub fn new(raw_rx: RawFrameRx) -> Arc<Self> {
        Arc::new(Self {
            entries: Mutex::new(HashMap::new()),
            raw_rx,
        })
    }

    pub async fn subscribe(self: &Arc<Self>, tier: Tier) -> TierSubscription {
        let mut guard = self.entries.lock().await;
        let entry = guard.entry(tier).or_insert_with(|| {
            let (jpeg_tx, _) = broadcast::channel(JPEG_BROADCAST_CAPACITY);
            let (stop_tx, stop_rx) = watch::channel(false);
            let handle = tokio::spawn(run_tier_encoder(
                tier,
                self.raw_rx.clone(),
                jpeg_tx.clone(),
                stop_rx,
            ));
            TierEntry { jpeg_tx, refcount: 0, stop_tx, handle }
        });
        entry.refcount += 1;
        let rx = entry.jpeg_tx.subscribe();
        TierSubscription {
            tier,
            rx,
            registry: Arc::clone(self),
        }
    }

    pub async fn release(self: &Arc<Self>, tier: Tier) {
        let mut guard = self.entries.lock().await;
        if let Some(entry) = guard.get_mut(&tier) {
            entry.refcount = entry.refcount.saturating_sub(1);
            if entry.refcount == 0 {
                let entry = guard.remove(&tier).unwrap();
                let _ = entry.stop_tx.send(true);
                entry.handle.abort();
            }
        }
    }

    #[cfg(test)]
    pub async fn active_tier_count(&self) -> usize {
        self.entries.lock().await.len()
    }
}

async fn run_tier_encoder(
    tier: Tier,
    mut raw_rx: RawFrameRx,
    jpeg_tx: broadcast::Sender<Bytes>,
    mut stop_rx: watch::Receiver<bool>,
) {
    let fourcc_uyvy = u32::from_le_bytes([b'U', b'Y', b'V', b'Y']);
    let fourcc_bgra = u32::from_le_bytes([b'B', b'G', b'R', b'A']);
    let fourcc_bgrx = u32::from_le_bytes([b'B', b'G', b'R', b'X']);
    let encoder = JpegEncoder::new(75);
    let spec = tier.spec();

    let mut frame_index: u32 = 0;
    tracing::info!(?tier, target_height = spec.target_height, target_fps = spec.target_fps, "tier encoder started");

    loop {
        tokio::select! {
            _ = stop_rx.changed() => {
                if *stop_rx.borrow() { break; }
            }
            res = raw_rx.changed() => {
                if res.is_err() { break; }
            }
        }

        let frame = match raw_rx.borrow().as_ref() {
            Some(f) => Arc::clone(f),
            None => continue,
        };

        // Frame skip
        frame_index = frame_index.wrapping_add(1);
        if frame_index % spec.frame_skip_modulus != 0 {
            continue;
        }

        // Resolve BGRA bytes (convert UYVY if needed)
        let (bgra, w, h) = if frame.fourcc == fourcc_bgra || frame.fourcc == fourcc_bgrx {
            (frame.data.clone(), frame.width, frame.height)
        } else if frame.fourcc == fourcc_uyvy {
            (uyvy_to_bgra(&frame.data, frame.width, frame.height), frame.width, frame.height)
        } else {
            tracing::warn!(?tier, fourcc = format!("0x{:08x}", frame.fourcc), "unsupported fourcc; skipping");
            continue;
        };

        match encoder.encode_bgra_resized(&bgra, w, h, spec.target_height) {
            Ok(jpeg) => {
                let _ = jpeg_tx.send(Bytes::from(jpeg));
            }
            Err(e) => {
                tracing::error!(?tier, "tier encode error: {e}");
            }
        }
    }

    tracing::info!(?tier, "tier encoder stopped");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::receiver::VideoFrame;

    fn fake_bgra_frame(w: u32, h: u32) -> Arc<VideoFrame> {
        Arc::new(VideoFrame {
            width: w,
            height: h,
            data: vec![128u8; (w * h * 4) as usize],
            stride: (w * 4) as i32,
            fourcc: u32::from_le_bytes([b'B', b'G', b'R', b'A']),
            frame_rate_n: 30,
            frame_rate_d: 1,
        })
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn subscribe_spawns_one_encoder_per_tier() {
        let (raw_tx, raw_rx) = watch::channel(None);
        let registry = TierRegistry::new(raw_rx);
        assert_eq!(registry.active_tier_count().await, 0);

        let _s1 = registry.subscribe(Tier::L0).await;
        assert_eq!(registry.active_tier_count().await, 1);

        let _s2 = registry.subscribe(Tier::L0).await;
        assert_eq!(registry.active_tier_count().await, 1, "second L0 sub must reuse encoder");

        let _s3 = registry.subscribe(Tier::L2).await;
        assert_eq!(registry.active_tier_count().await, 2);

        drop(raw_tx);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dropping_last_subscription_stops_encoder() {
        let (raw_tx, raw_rx) = watch::channel(None);
        let registry = TierRegistry::new(raw_rx);

        let s1 = registry.subscribe(Tier::L0).await;
        let s2 = registry.subscribe(Tier::L0).await;
        assert_eq!(registry.active_tier_count().await, 1);

        drop(s1);
        // Drop spawns an async release; give it a turn
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(registry.active_tier_count().await, 1, "still 1 sub left");

        drop(s2);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(registry.active_tier_count().await, 0);

        drop(raw_tx);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tier_encoder_emits_jpeg_for_each_passing_frame() {
        let (raw_tx, raw_rx) = watch::channel(None);
        let registry = TierRegistry::new(raw_rx);
        let mut sub = registry.subscribe(Tier::L0).await;

        // L0 has frame_skip_modulus = 1, so every frame should pass.
        // Push 3 frames, expect 3 JPEGs.
        for i in 0..3 {
            raw_tx.send(Some(fake_bgra_frame(64, 64))).unwrap();
            // Give encoder a turn
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            let jpeg = tokio::time::timeout(
                std::time::Duration::from_millis(200),
                sub.rx.recv(),
            ).await.expect(&format!("timed out waiting for jpeg #{i}")).unwrap();
            assert!(jpeg.starts_with(&[0xff, 0xd8, 0xff]), "frame #{i} not a JPEG");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tier_l3_frame_skip_emits_one_third_of_frames() {
        // L3 has frame_skip_modulus = 3, so 1 of every 3 frames should pass.
        let (raw_tx, raw_rx) = watch::channel(None);
        let registry = TierRegistry::new(raw_rx);
        let mut sub = registry.subscribe(Tier::L3).await;

        // Push 9 frames slowly so each is processed
        for _ in 0..9 {
            raw_tx.send(Some(fake_bgra_frame(64, 64))).unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(15)).await;
        }

        // Drain receiver
        let mut got = 0;
        while let Ok(Ok(_)) = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            sub.rx.recv(),
        ).await {
            got += 1;
        }
        // Allow off-by-one (depending on which frame triggers the modulus)
        assert!((2..=4).contains(&got), "expected ~3 frames, got {got}");
    }
}
```

- [ ] **Step 2: Wire into lib.rs**

In `crates/presenter-ndi/src/lib.rs`, after `pub mod tier;`:

```rust
pub mod tier_registry;
```

And after the existing re-exports:

```rust
pub use tier_registry::{TierRegistry, TierSubscription};
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p presenter-ndi tier_registry:: 2>&1 | tail -20
```

Expected: 4 passed.

- [ ] **Step 4: Commit**

```bash
git add crates/presenter-ndi/src/tier_registry.rs crates/presenter-ndi/src/lib.rs
git commit -m "feat(ndi): add TierRegistry with lazy ref-counted tier encoders (#250)"
```

---

## Task 7: Replace single encoder with TierRegistry in NdiManager

**Files:**
- Modify: `crates/presenter-ndi/src/manager.rs`

This is the biggest single change. The capture thread is reworked to publish raw `Arc<VideoFrame>` via a `watch` channel; the old `frame_tx` broadcast and `run_encode_thread` are deleted; `subscribe_frames()` is replaced by `subscribe_tier`.

- [ ] **Step 1: Replace the entire manager.rs file**

Overwrite `crates/presenter-ndi/src/manager.rs` with:

```rust
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{watch, Mutex};

use crate::discovery::{self, FinderShutdown, SourceList};
use crate::ndi_sdk::NdiLib;
use crate::receiver::{NdiReceiver, VideoFrame};
use crate::tier::Tier;
use crate::tier_registry::{TierRegistry, TierSubscription};

/// Callback for reporting NDI connection status changes.
pub type StatusCallback = Arc<dyn Fn(String) + Send + Sync>;

struct ActiveStream {
    stop_signal: watch::Sender<bool>,
    capture_thread: Option<std::thread::JoinHandle<()>>,
}

/// Orchestrates NDI discovery, capture, and adaptive MJPEG encoding.
///
/// Discovery runs in a persistent background thread — sources accumulate
/// over time via mDNS. Capture runs in an OS thread that publishes the
/// newest raw frame to a `tokio::sync::watch` channel; per-tier JPEG
/// encoders subscribe via `TierRegistry`.
pub struct NdiManager {
    sdk: Arc<NdiLib>,
    source_list: SourceList,
    _finder_shutdown: FinderShutdown,
    active_stream: Mutex<Option<ActiveStream>>,
    raw_frame_tx: watch::Sender<Option<Arc<VideoFrame>>>,
    tier_registry: Arc<TierRegistry>,
}

impl NdiManager {
    /// Try to create a new manager by loading the NDI SDK.
    ///
    /// Returns `None` if the NDI runtime is not available on this system.
    /// Immediately starts a persistent finder thread for source discovery.
    pub fn try_new() -> Option<Self> {
        let sdk = NdiLib::load().ok()?;
        let sdk = Arc::new(sdk);
        let (source_list, finder_shutdown) = discovery::spawn_persistent_finder(Arc::clone(&sdk));
        let (raw_frame_tx, raw_frame_rx) = watch::channel(None);
        let tier_registry = TierRegistry::new(raw_frame_rx);
        Some(Self {
            sdk,
            source_list,
            _finder_shutdown: finder_shutdown,
            active_stream: Mutex::new(None),
            raw_frame_tx,
            tier_registry,
        })
    }

    pub fn is_available(&self) -> bool {
        true
    }

    pub fn discover_sources(&self, _timeout_ms: u32) -> Result<Vec<discovery::NdiSourceInfo>> {
        Ok(self.source_list.read())
    }

    /// Subscribe to a JPEG broadcast for a given adaptive tier.
    pub async fn subscribe_tier(&self, tier: Tier) -> TierSubscription {
        self.tier_registry.subscribe(tier).await
    }

    /// Start capturing from the named NDI source.
    ///
    /// Spawns one OS thread for frame capture; tier encoders are spawned
    /// lazily by `TierRegistry` as subscribers register.
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
            .spawn(move || {
                run_capture_thread(sdk, source_name, raw_tx, stop_rx, status_cb);
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
            // Capture thread checks stop_rx every iteration; clear the frame so subscribers see "stream gone"
            let _ = self.raw_frame_tx.send(None);
            if let Some(h) = stream.capture_thread.take() {
                let _ = h.join();
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
                    let period = (1000 * frame.frame_rate_d as u64) / frame.frame_rate_n as u64;
                    capture_timeout_ms = (period as u32).clamp(16, 200);
                }

                if !connected {
                    connected = true;
                    tracing::info!(
                        "NDI connected: {}x{} @ {}/{}fps",
                        frame.width, frame.height, frame.frame_rate_n, frame.frame_rate_d
                    );
                    if let Some(cb) = &status_cb {
                        cb("connected".to_string());
                    }
                }
                last_frame_time = std::time::Instant::now();

                // Publish to watch — newest replaces previous; `Arc` so consumers don't copy data.
                let _ = raw_tx.send(Some(Arc::new(frame)));
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
    fn watch_newest_wins() {
        let (tx, mut rx) = watch::channel::<Option<Arc<VideoFrame>>>(None);
        tx.send(Some(Arc::new(make_frame(1)))).unwrap();
        tx.send(Some(Arc::new(make_frame(2)))).unwrap();
        // After multiple sends, watch holds only the newest
        assert_eq!(rx.borrow_and_update().as_ref().unwrap().width, 2);
    }

    #[test]
    fn watch_starts_empty() {
        let (_tx, rx) = watch::channel::<Option<Arc<VideoFrame>>>(None);
        assert!(rx.borrow().is_none());
    }
}
```

- [ ] **Step 2: Update lib.rs to drop the old re-exports**

In `crates/presenter-ndi/src/lib.rs`, ensure the file looks like:

```rust
#![allow(non_camel_case_types)]

pub mod discovery;
pub mod encoder;
mod manager;
pub mod ndi_sdk;
pub mod receiver;
pub mod tier;
pub mod tier_registry;

pub use discovery::SourceList;
pub use manager::NdiManager;
pub use manager::StatusCallback;
pub use tier::{Tier, TierSpec};
pub use tier_registry::{TierRegistry, TierSubscription};
```

- [ ] **Step 3: Build presenter-ndi to confirm**

```bash
cargo build -p presenter-ndi 2>&1 | tail -10
```

Expected: `Finished `dev` profile`. If `subscribe_frames` callers in `presenter-server` fail to compile yet, that's expected — they get rewritten in Task 9.

- [ ] **Step 4: Run presenter-ndi tests**

```bash
cargo test -p presenter-ndi 2>&1 | tail -15
```

Expected: all `tier::tests`, `tier_registry::tests`, `manager::tests`, `encoder::tests` pass.

- [ ] **Step 5: Commit**

```bash
git add crates/presenter-ndi/src/manager.rs crates/presenter-ndi/src/lib.rs
git commit -m "refactor(ndi): replace single encode thread with TierRegistry (#250)"
```

---

## Task 8: AdaptController state machine

**Files:**
- Create: `crates/presenter-server/src/adaptive_mjpeg.rs`
- Modify: `crates/presenter-server/src/main.rs`

- [ ] **Step 1: Create adaptive_mjpeg.rs**

```rust
//! Per-connection adaptive controller for `/ndi/mjpeg`.
//!
//! Keeps a sliding 30-second window of `broadcast::RecvError::Lagged`
//! events. Demotes one tier when the window holds 5+ events; promotes
//! one tier after 60 seconds of zero lag at the current tier.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use presenter_ndi::Tier;

const LAG_WINDOW: Duration = Duration::from_secs(30);
const LAG_DEMOTE_THRESHOLD: usize = 5;
const PROMOTE_AFTER: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdaptDecision {
    Stay,
    Demote(Tier),
    Promote(Tier),
}

pub struct AdaptController {
    tier: Tier,
    lag_events: VecDeque<Instant>,
    last_lag_at: Option<Instant>,
    entered_tier_at: Instant,
}

impl AdaptController {
    pub fn new(initial: Tier) -> Self {
        let now = Instant::now();
        Self {
            tier: initial,
            lag_events: VecDeque::new(),
            last_lag_at: None,
            entered_tier_at: now,
        }
    }

    pub fn tier(&self) -> Tier {
        self.tier
    }

    /// Called when a successful frame is received. Returns Promote if conditions met.
    pub fn on_frame(&mut self, now: Instant) -> AdaptDecision {
        self.trim_window(now);
        if self.tier != Tier::L0 && self.entered_tier_at.elapsed() >= PROMOTE_AFTER {
            // 60 s smooth at this tier and we have a higher tier to try.
            if self.last_lag_at.map_or(true, |t| now.duration_since(t) >= PROMOTE_AFTER) {
                if let Some(next) = self.tier.promote() {
                    self.tier = next;
                    self.entered_tier_at = now;
                    self.lag_events.clear();
                    return AdaptDecision::Promote(next);
                }
            }
        }
        AdaptDecision::Stay
    }

    /// Called when broadcast::RecvError::Lagged is observed.
    pub fn on_lag(&mut self, now: Instant) -> AdaptDecision {
        self.lag_events.push_back(now);
        self.last_lag_at = Some(now);
        self.trim_window(now);
        if self.lag_events.len() >= LAG_DEMOTE_THRESHOLD {
            if let Some(next) = self.tier.demote() {
                self.tier = next;
                self.entered_tier_at = now;
                self.lag_events.clear();
                return AdaptDecision::Demote(next);
            }
        }
        AdaptDecision::Stay
    }

    fn trim_window(&mut self, now: Instant) {
        while let Some(front) = self.lag_events.front() {
            if now.duration_since(*front) > LAG_WINDOW {
                self.lag_events.pop_front();
            } else {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add_lags(c: &mut AdaptController, t0: Instant, count: usize, spacing_ms: u64) -> Vec<AdaptDecision> {
        let mut out = Vec::new();
        for i in 0..count {
            out.push(c.on_lag(t0 + Duration::from_millis(i as u64 * spacing_ms)));
        }
        out
    }

    #[test]
    fn five_lags_in_30s_demotes() {
        let t0 = Instant::now();
        let mut c = AdaptController::new(Tier::L0);
        let decisions = add_lags(&mut c, t0, 5, 1000);
        assert_eq!(decisions[..4], [AdaptDecision::Stay; 4]);
        assert_eq!(decisions[4], AdaptDecision::Demote(Tier::L1));
        assert_eq!(c.tier(), Tier::L1);
    }

    #[test]
    fn lags_outside_window_dont_count() {
        let t0 = Instant::now();
        let mut c = AdaptController::new(Tier::L0);
        // 4 lags at the start
        add_lags(&mut c, t0, 4, 1000);
        // 1 lag 60 seconds later — first 4 are now outside window, so total in window is 1
        let d = c.on_lag(t0 + Duration::from_secs(60));
        assert_eq!(d, AdaptDecision::Stay);
        assert_eq!(c.tier(), Tier::L0);
    }

    #[test]
    fn promote_after_60s_clean_at_l1() {
        let t0 = Instant::now();
        let mut c = AdaptController::new(Tier::L1);
        // No lag events; pass time by reporting frames
        let d1 = c.on_frame(t0 + Duration::from_secs(30));
        assert_eq!(d1, AdaptDecision::Stay);
        let d2 = c.on_frame(t0 + Duration::from_secs(61));
        assert_eq!(d2, AdaptDecision::Promote(Tier::L0));
        assert_eq!(c.tier(), Tier::L0);
    }

    #[test]
    fn promote_blocked_by_recent_lag() {
        let t0 = Instant::now();
        let mut c = AdaptController::new(Tier::L1);
        // Lag at +5s — resets entered_tier_at? Actually NO: lag at L1 doesn't change tier (it's fewer than 5 in window).
        c.on_lag(t0 + Duration::from_secs(5));
        // At +61s, window holds zero events (30s window), but last_lag_at was 56s ago — less than 60s.
        let d = c.on_frame(t0 + Duration::from_secs(61));
        assert_eq!(d, AdaptDecision::Stay);
        // At +66s (61s after the lag), promote allowed.
        let d2 = c.on_frame(t0 + Duration::from_secs(66));
        assert_eq!(d2, AdaptDecision::Promote(Tier::L0));
    }

    #[test]
    fn floor_l3_cannot_demote() {
        let t0 = Instant::now();
        let mut c = AdaptController::new(Tier::L3);
        // 5 rapid lags
        let decisions = add_lags(&mut c, t0, 5, 100);
        assert_eq!(decisions[4], AdaptDecision::Stay, "L3 has no demote target");
        assert_eq!(c.tier(), Tier::L3);
    }

    #[test]
    fn ceiling_l0_cannot_promote() {
        let t0 = Instant::now();
        let mut c = AdaptController::new(Tier::L0);
        let d = c.on_frame(t0 + Duration::from_secs(120));
        assert_eq!(d, AdaptDecision::Stay);
    }
}
```

- [ ] **Step 2: Register module**

In `crates/presenter-server/src/main.rs` (or `lib.rs`), add `mod adaptive_mjpeg;` near the other `mod` declarations. Match existing visibility/ordering pattern (search for `mod ai;`).

- [ ] **Step 3: Run tests**

```bash
cargo test -p presenter-server adaptive_mjpeg:: 2>&1 | tail -20
```

Expected: 6 passed.

- [ ] **Step 4: Commit**

```bash
git add crates/presenter-server/src/adaptive_mjpeg.rs crates/presenter-server/src/main.rs
git commit -m "feat(server): add AdaptController state machine (#250)"
```

---

## Task 9: Wire controller into mjpeg_http + mjpeg_ws

**Files:**
- Modify: `crates/presenter-server/src/router/integrations/ndi.rs`

The `mjpeg_http` and `mjpeg_ws` handlers must:
1. Acquire an initial `TierSubscription` at `Tier::L0`.
2. Wrap the recv loop with an `AdaptController`.
3. On `RecvError::Lagged(n)`, call `controller.on_lag(now)` and swap subscriptions if a demote is returned.
4. On `Ok(jpeg)`, call `controller.on_frame(now)` and swap if promote returned.

- [ ] **Step 1: Replace mjpeg_http and mjpeg_ws**

Overwrite `crates/presenter-server/src/router/integrations/ndi.rs` (replacing the existing `handle_mjpeg_ws`, `mjpeg_ws`, and `mjpeg_http` functions; keep `discover_ndi_sources` and `ndi_status` unchanged):

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
use presenter_ndi::{Tier, TierSubscription};
use serde::Serialize;
use std::time::Instant;
use tokio::sync::broadcast::error::RecvError;
use tracing::instrument;

use super::super::AppError;
use crate::adaptive_mjpeg::{AdaptController, AdaptDecision};
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
    Ok(Json(sources.into_iter().map(|s| NdiSourceDto { name: s.name }).collect()))
}

#[instrument(skip_all)]
pub(crate) async fn ndi_status(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "available": state.ndi_manager().is_some() }))
}

/// WebSocket endpoint that streams JPEG frames; tier adapts per-connection.
pub(crate) async fn mjpeg_ws(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let manager = state
        .ndi_manager()
        .ok_or_else(|| AppError::service_unavailable("NDI SDK not available"))?;
    let sub = manager.subscribe_tier(Tier::L0).await;
    Ok(ws.on_upgrade(move |socket| handle_mjpeg_ws(socket, sub, state)))
}

async fn handle_mjpeg_ws(mut socket: WebSocket, mut sub: TierSubscription, state: AppState) {
    let mut controller = AdaptController::new(Tier::L0);
    let manager = match state.ndi_manager() {
        Some(m) => m,
        None => return,
    };
    loop {
        match sub.rx.recv().await {
            Ok(jpeg) => {
                let decision = controller.on_frame(Instant::now());
                if let AdaptDecision::Promote(next) = decision {
                    sub = manager.subscribe_tier(next).await;
                }
                if socket.send(Message::Binary(jpeg.to_vec().into())).await.is_err() {
                    break;
                }
            }
            Err(RecvError::Lagged(n)) => {
                tracing::debug!(lag = n, tier = ?controller.tier(), "MJPEG WS client lagged");
                let decision = controller.on_lag(Instant::now());
                if let AdaptDecision::Demote(next) = decision {
                    tracing::info!(from = ?controller.tier(), to = ?next, "MJPEG WS demoting tier");
                    sub = manager.subscribe_tier(next).await;
                }
            }
            Err(RecvError::Closed) => break,
        }
    }
}

/// HTTP MJPEG stream using multipart/x-mixed-replace.
pub(crate) async fn mjpeg_http(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let manager = state
        .ndi_manager()
        .ok_or_else(|| AppError::service_unavailable("NDI SDK not available"))?;

    let initial_sub = manager.subscribe_tier(Tier::L0).await;
    let manager_clone = state.clone();
    let boundary = "mjpegboundary";
    let content_type = format!("multipart/x-mixed-replace; boundary={boundary}");

    let stream = async_stream::stream! {
        let mut sub = initial_sub;
        let mut controller = AdaptController::new(Tier::L0);
        let manager = match manager_clone.ndi_manager() {
            Some(m) => m,
            None => return,
        };
        loop {
            match sub.rx.recv().await {
                Ok(jpeg) => {
                    let decision = controller.on_frame(Instant::now());
                    if let AdaptDecision::Promote(next) = decision {
                        sub = manager.subscribe_tier(next).await;
                    }
                    let part_header = format!(
                        "--{boundary}\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
                        jpeg.len()
                    );
                    yield Ok::<Bytes, std::convert::Infallible>(Bytes::from(part_header));
                    yield Ok(jpeg);
                    yield Ok(Bytes::from("\r\n"));
                }
                Err(RecvError::Lagged(n)) => {
                    tracing::debug!(lag = n, tier = ?controller.tier(), "MJPEG HTTP client lagged");
                    let decision = controller.on_lag(Instant::now());
                    if let AdaptDecision::Demote(next) = decision {
                        tracing::info!(from = ?controller.tier(), to = ?next, "MJPEG HTTP demoting tier");
                        sub = manager.subscribe_tier(next).await;
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

- [ ] **Step 2: Build full workspace**

```bash
cargo build --workspace 2>&1 | tail -10
```

Expected: `Finished `dev` profile`.

- [ ] **Step 3: Run server-side tests**

```bash
cargo test -p presenter-server -- --test-threads=4 2>&1 | tail -15
```

Expected: all green. Pre-existing tests must still pass.

- [ ] **Step 4: Manual smoke test**

```bash
# Start dev server in background (or rely on already-running presenter-dev.service)
# In one terminal:
curl -sN http://10.77.8.134:8080/ndi/mjpeg --output - 2>/dev/null | head -c 50000 > /tmp/smoke-mjpeg.bin
ls -l /tmp/smoke-mjpeg.bin
python3 -c "
data=open('/tmp/smoke-mjpeg.bin','rb').read()
soi=data.find(b'\xff\xd8\xff')
eoi=data.find(b'\xff\xd9', soi)
print('SOI@', soi, 'EOI@', eoi, 'first JPEG bytes:', eoi-soi)
"
```

Expected: a JPEG SOI is found, frame size > 30 KB.

- [ ] **Step 5: Commit**

```bash
git add crates/presenter-server/src/router/integrations/ndi.rs
git commit -m "feat(server): wire AdaptController into mjpeg_http and mjpeg_ws (#250)"
```

---

## Task 10: Local fmt + clippy + tests

**Files:** None (verification step).

- [ ] **Step 1: Format**

```bash
cargo fmt --all
```

- [ ] **Step 2: Clippy zero-warnings**

```bash
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all 2>&1 | tail -25
```

Expected: clean. Fix any warnings before continuing — do not push with warnings.

- [ ] **Step 3: All tests**

```bash
cargo test --workspace 2>&1 | tail -20
```

Expected: all green.

- [ ] **Step 4: If any of Steps 2 or 3 produced fixes, commit**

```bash
git add -A
git commit -m "chore: fmt + clippy fixes for tier adaptive (#250)"
```

If no changes, skip.

---

## Task 11: Push to dev + monitor CI

**Files:** None.

- [ ] **Step 1: Push**

```bash
git fetch origin
git push origin dev
```

- [ ] **Step 2: Identify the latest run and monitor in background**

```bash
sleep 10
gh run list --branch dev --limit 3 --json databaseId,name,status,conclusion,event,createdAt
```

Capture the `databaseId` for the just-triggered `Pipeline` run.

- [ ] **Step 3: Wait for terminal state (sleep + view, do not poll)**

```bash
RUN_ID=<paste databaseId>
sleep 1500 && gh run view $RUN_ID --json status,conclusion,jobs --jq '{status,conclusion,jobs:[.jobs[]|{name,conclusion,status}]}'
```

If still running after one wake, sleep again (300–600 s). If failed, `gh run view $RUN_ID --log-failed | tail -100`, fix in ONE commit, push, and monitor again.

- [ ] **Step 4: Confirm deploy-dev job succeeded**

```bash
gh run view $RUN_ID --json jobs --jq '.jobs[] | select(.name=="Deploy to Dev") | {name, conclusion}'
```

Expected: `conclusion=success`.

---

## Task 12: Post-deploy profiling on sd1l + sd2l

**Files:**
- Modify: `docs/superpowers/specs/2026-04-25-ndi-cheap-tv-adaptive-design.md` (Findings section)

This is the validation pass. After dev is deployed with the adaptive code, we re-profile both TVs and observe:
1. The connection starts at L0 and downgrades automatically if the TV can't keep pace.
2. The final settled tier produces decode_p95 + paint_p50 < frame_interval.

- [ ] **Step 1: Profile sd1l.lan after the dev deploy lands**

Run a 60-second trace this time so we observe at least one demote+stabilize cycle. The longer window lets us see the initial L0 frames (high decode time, lag events), the transition (less frequent JPEGs as a smaller tier kicks in), and the settled tier (decode_p95 < tier's frame_interval).

In a separate terminal, watch dev server logs to confirm transitions:

```bash
sshpass -p 'newlevel' ssh newlevel@10.77.8.134 "sudo journalctl -u presenter-dev -f" | grep -i tier
```

Then drive the profiling on sd1l:

```bash
mkdir -p /tmp/ndi-profiling/sd1l-postdeploy
adb -s sd1l.lan:5555 reverse tcp:8080 tcp:8080
adb -s sd1l.lan:5555 shell am force-stop com.fullykiosk.videokiosk
adb -s sd1l.lan:5555 shell am start -n com.fullykiosk.videokiosk/de.ozerov.fully.MainActivity --es WEBVIEW_DEBUG true
sleep 5
curl -s -X POST http://10.77.8.134:8080/stage/layout -H 'content-type: application/json' -d '{"code":"ndi-fullscreen"}'
```

Open `chrome://inspect/#devices`, attach to the sd1l.lan webview, Performance tab → Record 60 s → Stop → Save profile to `/tmp/ndi-profiling/sd1l-postdeploy/trace.json`.

Extract numbers, breaking the trace into "first 15s" (initial tier) and "last 15s" (settled tier):

```bash
python3 - <<'PY'
import json
with open('/tmp/ndi-profiling/sd1l-postdeploy/trace.json') as f:
    data = json.load(f)
events = data.get('traceEvents', data) if isinstance(data, dict) else data
xs = [(e['ts'], e['dur']/1000.0) for e in events
      if e.get('ph')=='X' and e.get('name')=='Decode Image' and 'dur' in e]
if not xs:
    print('no decode events'); exit()
t0 = xs[0][0]
def stats(label, sub):
    if not sub: print(f'{label}: n=0'); return
    durs = sorted([d for _,d in sub])
    p50 = durs[len(durs)//2]
    p95 = durs[int(len(durs)*0.95)]
    print(f'{label}: n={len(durs)} p50={p50:.2f}ms p95={p95:.2f}ms')
stats('initial(0-15s)', [x for x in xs if x[0]-t0 < 15_000_000])
stats('settled(45-60s)', [x for x in xs if x[0]-t0 > 45_000_000])
PY
```

Save the printout to `/tmp/ndi-profiling/sd1l-postdeploy/stats.txt`. Capture the journalctl output during the trace and save the relevant tier-transition lines to `/tmp/ndi-profiling/sd1l-postdeploy/journal.txt`.

- [ ] **Step 2: Profile sd2l.lan with the same 60 s methodology**

```bash
mkdir -p /tmp/ndi-profiling/sd2l-postdeploy
adb -s sd2l.lan:5555 reverse tcp:8080 tcp:8080
adb -s sd2l.lan:5555 shell am force-stop com.fullykiosk.videokiosk
adb -s sd2l.lan:5555 shell am start -n com.fullykiosk.videokiosk/de.ozerov.fully.MainActivity --es WEBVIEW_DEBUG true
sleep 5
```

Attach DevTools to sd2l, record 60 s, save trace as `/tmp/ndi-profiling/sd2l-postdeploy/trace.json`. Run the same Python extractor (substitute path), save to `/tmp/ndi-profiling/sd2l-postdeploy/stats.txt`. Save the corresponding journal lines to `/tmp/ndi-profiling/sd2l-postdeploy/journal.txt`.

- [ ] **Step 3: Fill in Findings section in spec**

Open `docs/superpowers/specs/2026-04-25-ndi-cheap-tv-adaptive-design.md`. Replace the placeholder "Findings (filled in during Phase 1, before merging)" content with two tables:

```markdown
## Findings (2026-04-25 dev deploy of 0.4.34)

### sd1l.lan — Tesla LEAP-S1, 2 GB, Android 12

| Phase | Tier | decode_p50 | decode_p95 | paint_p50 | fps_browser | server_kbps |
|---|---|---|---|---|---|---|
| baseline | L0 (forced) | <ms> | <ms> | <ms> | <fps> | <kbps> |
| post-deploy initial | L0 | <ms> | <ms> | <ms> | <fps> | <kbps> |
| post-deploy settled | <Lx> | <ms> | <ms> | <ms> | <fps> | <kbps> |

Settled tier: **Lx**. Pass criterion (decode_p95 + paint_p50 < frame_interval): **PASS / FAIL**.

### sd2l.lan — Hyundai, 1 GB, Android 11

(same table)
```

Replace every `<>` placeholder with the measured value.

- [ ] **Step 4: Decide pass/fail per TV**

If sd2l (Hyundai) cannot pass even at L3 (720p @ 10 fps), add a follow-up note in the spec:

```markdown
### Follow-up
Hyundai cannot sustain L3. File a follow-up issue to investigate H.264 codec
or resolution floor below 720p — out of scope for this PR.
```

If both TVs pass, the spec stands as-is.

- [ ] **Step 5: Commit**

```bash
git add docs/superpowers/specs/2026-04-25-ndi-cheap-tv-adaptive-design.md
git commit -m "docs(spec): record NDI cheap-TV adaptive Findings from dev deploy (#250)"
git push origin dev
```

Wait for the docs-only push to land (no functional CI impact, but keep the same monitoring discipline as Task 11).

---

## Task 13: Open PR dev → main + monitor PR CI

**Files:** None (PR creation).

- [ ] **Step 1: Verify mergeable state**

```bash
git fetch origin
git log origin/main..origin/dev --oneline
gh pr list --base main --head dev --state open
```

If a PR already exists (unlikely in a fresh branch), reuse it. Otherwise create one.

- [ ] **Step 2: Create PR**

```bash
gh pr create --base main --head dev --title "feat(ndi): adaptive MJPEG tiered streaming for cheap Android TVs (#250)" --body "$(cat <<'EOF'
## Summary
- Replaces single global JPEG encoder with `TierRegistry` running up to 4 lazy ref-counted tier encoders (1080@30 / 1080@15 / 720@15 / 720@10).
- Each `/ndi/mjpeg` (HTTP and WS) connection auto-tunes via `AdaptController`: 5+ `RecvError::Lagged` in 30s → demote one tier; 60s clean → promote.
- No DB / UI / client changes. Cheap TVs degrade automatically; fast clients stay native; server cost scales with active tiers, not client count.
- Profiled on sd1l.lan (Tesla LEAP-S1, 2 GB) and sd2l.lan (Hyundai, 1 GB) before and after deploy — see Findings section in the design spec.

## Test plan
- [x] Unit tests: `Tier`, `JpegEncoder::encode_bgra_resized`, `TierRegistry` ref-count, frame-skip, `AdaptController` window/threshold.
- [x] Manual MJPEG HTTP smoke (curl extracts a JPEG).
- [x] Live profiling on sd1l + sd2l, results in `docs/superpowers/specs/2026-04-25-ndi-cheap-tv-adaptive-design.md`.
- [x] Existing Playwright `stage-api-ndi.spec.ts` still passes.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 2: Monitor PR CI**

```bash
PR_RUN=$(gh pr checks --json name,status,conclusion,detailsUrl | head -1)
echo "$PR_RUN"
```

Use the same `sleep N && gh run view $RUN_ID` pattern as Task 11 to wait until terminal. ALL jobs must be green — pipeline, e2e shards, mutation testing, version-check.

- [ ] **Step 3: Verify mergeable**

```bash
gh pr view --json number,mergeable,mergeable_state --jq '{number, mergeable, mergeable_state}'
```

Expected: `{number: NN, mergeable: "MERGEABLE", mergeable_state: "clean"}`. If `behind`, sync; if `dirty`, resolve.

- [ ] **Step 4: Provide the PR URL to the user and STOP**

Per PR merge policy: never merge without explicit user instruction. Provide the full PR URL and wait.

---

## Task 14: After merge, verify production on all 4 TVs

**Files:** None (post-merge verification).

This task only starts after the user explicitly says "merge it" and the merge to main has run + deployed to production.

- [ ] **Step 1: Confirm main deploy succeeded**

```bash
gh run list --branch main --limit 3
sleep 1200 && gh run view <main-deploy-run-id> --json status,conclusion,jobs --jq '{status, conclusion}'
```

Expected: `conclusion=success`.

- [ ] **Step 2: Confirm production version**

```bash
curl -s http://10.77.9.205/healthz
```

Expected: `{"status":"ok","version":"0.4.34","channel":"release"}`.

- [ ] **Step 3: Verify each TV holds its connection through the cg-obs source**

For each of `sd1l.lan`, `sd2l.lan`, `sd3l.lan`, `sd4l.lan`:

```bash
# Confirm Fully Kiosk is foreground
adb -s <tv>:5555 shell "dumpsys activity activities | grep -E 'mResumedActivity'" | head -1

# Re-attach DevTools quickly to confirm NO console errors and stable frame loop
# (Manual step: chrome://inspect → attach → check Console tab; report file paths only, do not Read screenshots)

# Read production server log for tier transitions
sshpass -p 'newlevel' ssh newlevel@presenter.lan "sudo journalctl -u presenter --since '5 minutes ago' | grep -i tier" | tail -20
```

Note: each TV should appear at least once in the log lines, with either no transition (stayed L0) or a `demoting tier` line followed by a stable settled tier.

- [ ] **Step 4: Update memory note**

```bash
cat > /home/newlevel/.claude/projects/-home-newlevel-devel-presenter-presenter-dev2/memory/project_ndi_adaptive.md <<'EOF'
---
name: NDI adaptive tiered streaming
description: Per-connection tier ladder (L0..L3) on /ndi/mjpeg auto-degrades for cheap TVs
type: project
---

`/ndi/mjpeg` is now adaptive. Per-connection state machine in `crates/presenter-server/src/adaptive_mjpeg.rs`. Tier encoders in `crates/presenter-ndi/src/tier_registry.rs` are lazy + ref-counted, so server cost scales with active tiers, not client count. Floor is L3 = 720p @ 10 fps — text quality decision, not technical.

**Why:** PR #<N> from issue #250. Cheap Android TVs (Hyundai 1 GB, Tesla LEAP-S1 2 GB) couldn't keep up with native 1080p @ 30 software JPEG decode.

**How to apply:** When debugging NDI latency on a TV, look at production server log for `tier` lines (`journalctl -u presenter | grep tier`) — that tells you which tier the connection settled on.
EOF
```

Add entry to `MEMORY.md`:

```bash
# Manually edit /home/newlevel/.claude/projects/.../memory/MEMORY.md to add:
# - [NDI adaptive tiered streaming](project_ndi_adaptive.md) — /ndi/mjpeg auto-tunes per connection across L0..L3 tiers
```

- [ ] **Step 5: Send completion report**

Per the user's completion-report template (with full URLs, all ✅ lines, no ⏳/❌). Cite the PR URL, both run URLs (dev + main pipelines), and the production verification details.

---

## Verification Summary

| Check | Where verified |
|---|---|
| Tier ladder correct | `tier::tests` (Task 4) |
| Resize preserves aspect, JPEG decodable | `encoder::tests::encode_bgra_resized_downscales_aspect_preserved` (Task 5) |
| Tier encoders are lazy + ref-counted | `tier_registry::tests::subscribe_spawns_one_encoder_per_tier` + `dropping_last_subscription_stops_encoder` (Task 6) |
| Frame skip works | `tier_registry::tests::tier_l3_frame_skip_emits_one_third_of_frames` (Task 6) |
| AdaptController demote/promote | `adaptive_mjpeg::tests` (Task 8) |
| `/ndi/mjpeg` still serves valid JPEG | Manual smoke (Task 9 Step 4) |
| Tier transitions in production | `journalctl | grep tier` (Task 14 Step 3) |
| Cheap TVs converge to a sustainable tier | Findings tables in spec (Task 12) |

