# NDI Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix unstable NDI source discovery and video stuttering by implementing persistent finder, separated capture/encode pipeline, canvas rendering, and connection status events.

**Architecture:** Persistent background finder thread accumulates NDI sources via mDNS. Capture and JPEG encoding split into two OS threads with shared frame slot. Browser switches from Blob URL + `<img>` to `<canvas>` + `createImageBitmap()`. Connection status events sent on state changes.

**Tech Stack:** Rust (NDI FFI, tokio, turbojpeg), Leptos WASM (web-sys canvas API), Playwright (E2E)

**Spec:** `docs/superpowers/specs/2026-04-05-ndi-hardening-design.md`

---

## Context

Real-world testing at church revealed: (1) NDI source scanning returns different counts each time (2→6→11→2→8) because discovery creates/destroys a new finder per scan, losing mDNS state; (2) video is unwatchable — stuttering and freezing because JPEG encoding blocks the capture thread (6 stutters in 10s, max 89ms gap measured on production); (3) "Connecting..." overlay never clears because server never sends connection status events.

---

## File Structure

### Modified Files
| File | Change |
|------|--------|
| `crates/presenter-ndi/src/discovery.rs` | Persistent finder thread with `SourceList` shared state |
| `crates/presenter-ndi/src/manager.rs` | Split capture/encode, status callback, use persistent finder |
| `crates/presenter-ndi/src/lib.rs` | Export `SourceList` |
| `crates/presenter-server/src/router/integrations/ndi.rs` | `discover_ndi_sources` reads from persistent list (non-blocking) |
| `crates/presenter-server/src/state/integrations.rs` | Wire status callback from `live_hub` into `NdiManager` |
| `crates/presenter-ui/src/components/stage/ndi_fullscreen.rs` | `<canvas>` + `createImageBitmap()` rendering |
| `crates/presenter-ui/Cargo.toml` | Add web-sys features: `HtmlCanvasElement`, `CanvasRenderingContext2d`, `ImageBitmap` |
| `crates/presenter-ui/styles/stage.css` | No change needed — `object-fit: contain` works on `<canvas>` |
| `tests/e2e/video-source-api.spec.ts` | Discovery stability test |
| `tests/e2e/ndi-stage-layout.spec.ts` | Canvas element test, frame delivery test |

---

## Task 1: Persistent NDI Finder

**Files:**
- Modify: `crates/presenter-ndi/src/discovery.rs`
- Modify: `crates/presenter-ndi/src/lib.rs`

- [ ] **Step 1: Write tests for SourceList shared state**

Add at bottom of `crates/presenter-ndi/src/discovery.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, RwLock};

    #[test]
    fn source_list_read_returns_current_snapshot() {
        let list = SourceList(Arc::new(RwLock::new(vec![
            NdiSourceInfo { name: "SRC-A".into() },
            NdiSourceInfo { name: "SRC-B".into() },
        ])));
        let snapshot = list.read();
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[0].name, "SRC-A");
    }

    #[test]
    fn source_list_update_replaces_contents() {
        let inner = Arc::new(RwLock::new(vec![
            NdiSourceInfo { name: "OLD".into() },
        ]));
        let list = SourceList(Arc::clone(&inner));

        // Simulate finder thread updating
        {
            let mut w = inner.write().unwrap();
            *w = vec![
                NdiSourceInfo { name: "NEW-1".into() },
                NdiSourceInfo { name: "NEW-2".into() },
                NdiSourceInfo { name: "NEW-3".into() },
            ];
        }

        let snapshot = list.read();
        assert_eq!(snapshot.len(), 3);
        assert_eq!(snapshot[0].name, "NEW-1");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p presenter-ndi --lib source_list`
Expected: FAIL — `SourceList` type does not exist yet.

- [ ] **Step 3: Implement persistent finder**

Rewrite `crates/presenter-ndi/src/discovery.rs`:

```rust
use crate::ndi_sdk::{NDIlib_find_create_t, NdiLib};
use anyhow::Result;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use tracing::{debug, info, warn};

/// A discovered NDI source on the network.
#[derive(Debug, Clone, Serialize)]
pub struct NdiSourceInfo {
    pub name: String,
}

/// Thread-safe handle to the accumulated NDI source list.
///
/// Cheap to clone — internally an `Arc<RwLock<Vec>>`.
#[derive(Clone)]
pub struct SourceList(pub(crate) Arc<RwLock<Vec<NdiSourceInfo>>>);

impl SourceList {
    /// Read a snapshot of all currently known NDI sources.
    pub fn read(&self) -> Vec<NdiSourceInfo> {
        self.0.read().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// Handle that stops the persistent finder thread on drop.
pub struct FinderShutdown {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Drop for FinderShutdown {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Spawn a persistent finder thread that continuously discovers NDI sources.
///
/// The finder runs in a background OS thread (not tokio) since NDI FFI calls
/// are blocking. Sources accumulate via mDNS and the list stabilizes over time.
///
/// Returns a `SourceList` for reading discovered sources and a `FinderShutdown`
/// handle that stops the thread when dropped.
pub fn spawn_persistent_finder(sdk: Arc<NdiLib>) -> (SourceList, FinderShutdown) {
    let sources = Arc::new(RwLock::new(Vec::new()));
    let source_list = SourceList(Arc::clone(&sources));
    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = Arc::clone(&stop);

    let handle = std::thread::Builder::new()
        .name("ndi-finder".into())
        .spawn(move || {
            run_finder_loop(sdk, sources, stop_clone);
        })
        .expect("failed to spawn NDI finder thread");

    let shutdown = FinderShutdown {
        stop,
        handle: Some(handle),
    };
    (source_list, shutdown)
}

fn run_finder_loop(
    sdk: Arc<NdiLib>,
    sources: Arc<RwLock<Vec<NdiSourceInfo>>>,
    stop: Arc<AtomicBool>,
) {
    unsafe {
        let create_settings = NDIlib_find_create_t {
            show_local_sources: true,
            p_groups: std::ptr::null(),
            p_extra_ips: std::ptr::null(),
        };

        let finder = (sdk.find_create_v2)(&create_settings);
        if finder.is_null() {
            warn!("NDIlib_find_create_v2 returned null — finder disabled");
            return;
        }

        info!("NDI persistent finder started");

        while !stop.load(Ordering::SeqCst) {
            let changed = (sdk.find_wait_for_sources)(finder, 5000);

            if stop.load(Ordering::SeqCst) {
                break;
            }

            // Always read current sources (SDK returns full list each call)
            let mut num_sources: u32 = 0;
            let sources_ptr = (sdk.find_get_current_sources)(finder, &mut num_sources);

            let mut new_list = Vec::new();
            if !sources_ptr.is_null() && num_sources > 0 {
                let raw = std::slice::from_raw_parts(sources_ptr, num_sources as usize);
                for src in raw {
                    if let Ok(name) = crate::ndi_sdk::cstr_to_string(src.p_ndi_name) {
                        new_list.push(NdiSourceInfo { name });
                    }
                }
            }

            if changed {
                debug!("NDI sources updated: {} found", new_list.len());
            }

            // Replace the source list atomically
            if let Ok(mut w) = sources.write() {
                *w = new_list;
            }
        }

        (sdk.find_destroy)(finder);
        info!("NDI persistent finder stopped");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, RwLock};

    #[test]
    fn source_list_read_returns_current_snapshot() {
        let list = SourceList(Arc::new(RwLock::new(vec![
            NdiSourceInfo { name: "SRC-A".into() },
            NdiSourceInfo { name: "SRC-B".into() },
        ])));
        let snapshot = list.read();
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[0].name, "SRC-A");
    }

    #[test]
    fn source_list_update_replaces_contents() {
        let inner = Arc::new(RwLock::new(vec![
            NdiSourceInfo { name: "OLD".into() },
        ]));
        let list = SourceList(Arc::clone(&inner));
        {
            let mut w = inner.write().unwrap();
            *w = vec![
                NdiSourceInfo { name: "NEW-1".into() },
                NdiSourceInfo { name: "NEW-2".into() },
                NdiSourceInfo { name: "NEW-3".into() },
            ];
        }
        let snapshot = list.read();
        assert_eq!(snapshot.len(), 3);
        assert_eq!(snapshot[0].name, "NEW-1");
    }
}
```

- [ ] **Step 4: Export SourceList from lib.rs**

In `crates/presenter-ndi/src/lib.rs`, add after `pub use manager::NdiManager;`:

```rust
pub use discovery::SourceList;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p presenter-ndi --lib source_list`
Expected: 2 tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/presenter-ndi/src/discovery.rs crates/presenter-ndi/src/lib.rs
git commit -m "feat(ndi): implement persistent NDI finder thread

Replace create-scan-destroy discovery pattern with a persistent finder
that accumulates sources via mDNS. SourceList provides cheap reads of
the current source snapshot. Fixes unstable source counts across scans."
```

---

## Task 2: Separated Capture/Encode Pipeline

**Files:**
- Modify: `crates/presenter-ndi/src/manager.rs`

- [ ] **Step 1: Write tests for frame slot behavior**

Add at bottom of `crates/presenter-ndi/src/manager.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::receiver::VideoFrame;

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
        let frame = slot.lock().unwrap().take();
        assert_eq!(frame.unwrap().width, 2);
    }

    #[test]
    fn frame_slot_empty_read() {
        let slot: FrameSlot = Arc::new(std::sync::Mutex::new(None));
        let frame = slot.lock().unwrap().take();
        assert!(frame.is_none());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p presenter-ndi --lib frame_slot`
Expected: FAIL — `FrameSlot` type does not exist yet.

- [ ] **Step 3: Implement separated capture/encode pipeline**

Rewrite `crates/presenter-ndi/src/manager.rs`:

```rust
use std::sync::Arc;

use anyhow::Result;
use bytes::Bytes;
use tokio::sync::{broadcast, Mutex};

use crate::discovery::{self, FinderShutdown, SourceList};
use crate::encoder::JpegEncoder;
use crate::ndi_sdk::NdiLib;
use crate::receiver::{NdiReceiver, VideoFrame};

type FrameSlot = Arc<std::sync::Mutex<Option<VideoFrame>>>;

/// Callback for reporting NDI connection status changes.
pub type StatusCallback = Arc<dyn Fn(String) + Send + Sync>;

struct ActiveStream {
    stop_signal: tokio::sync::watch::Sender<bool>,
    capture_thread: Option<std::thread::JoinHandle<()>>,
    encode_thread: Option<std::thread::JoinHandle<()>>,
}

/// Orchestrates NDI discovery, capture, and MJPEG encoding.
///
/// Discovery runs in a persistent background thread — sources accumulate
/// over time via mDNS. Capture and encode run in separate OS threads
/// connected by a shared frame slot (newest frame wins).
pub struct NdiManager {
    sdk: Arc<NdiLib>,
    source_list: SourceList,
    _finder_shutdown: FinderShutdown,
    active_stream: Mutex<Option<ActiveStream>>,
    frame_tx: broadcast::Sender<Bytes>,
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
        let (frame_tx, _) = broadcast::channel(8);
        Some(Self {
            sdk,
            source_list,
            _finder_shutdown: finder_shutdown,
            active_stream: Mutex::new(None),
            frame_tx,
        })
    }

    /// Whether the NDI SDK is loaded and available.
    pub fn is_available(&self) -> bool {
        true
    }

    /// Read currently known NDI sources from the persistent finder.
    ///
    /// Returns instantly — no network scan is performed. The `timeout_ms`
    /// parameter is kept for API compatibility but is ignored.
    pub fn discover_sources(&self, _timeout_ms: u32) -> Result<Vec<discovery::NdiSourceInfo>> {
        Ok(self.source_list.read())
    }

    /// Subscribe to the JPEG frame stream.
    pub fn subscribe_frames(&self) -> broadcast::Receiver<Bytes> {
        self.frame_tx.subscribe()
    }

    /// Start capturing from the named NDI source and encoding to JPEG.
    ///
    /// Spawns two OS threads: one for frame capture, one for JPEG encoding.
    /// The optional `status_cb` is called with "connected" on first frame
    /// and "disconnected" after 3 seconds without frames.
    pub async fn start_stream(
        &self,
        ndi_name: &str,
        status_cb: Option<StatusCallback>,
    ) -> Result<()> {
        self.stop_stream().await;

        let sdk = Arc::clone(&self.sdk);
        let frame_tx = self.frame_tx.clone();
        let source_name = ndi_name.to_string();
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        let frame_slot: FrameSlot = Arc::new(std::sync::Mutex::new(None));
        let condvar = Arc::new(std::sync::Condvar::new());

        // Capture thread
        let slot_c = Arc::clone(&frame_slot);
        let condvar_c = Arc::clone(&condvar);
        let stop_rx_c = stop_rx.clone();
        let status_cb_c = status_cb;
        let capture_thread = std::thread::Builder::new()
            .name("ndi-capture".into())
            .spawn(move || {
                run_capture_thread(sdk, source_name, slot_c, condvar_c, stop_rx_c, status_cb_c);
            })?;

        // Encode thread
        let slot_e = Arc::clone(&frame_slot);
        let condvar_e = Arc::clone(&condvar);
        let encode_thread = std::thread::Builder::new()
            .name("ndi-encode".into())
            .spawn(move || {
                run_encode_thread(slot_e, condvar_e, frame_tx, stop_rx);
            })?;

        let mut active = self.active_stream.lock().await;
        *active = Some(ActiveStream {
            stop_signal: stop_tx,
            capture_thread: Some(capture_thread),
            encode_thread: Some(encode_thread),
        });

        Ok(())
    }

    /// Stop the active NDI capture stream, if any.
    pub async fn stop_stream(&self) {
        let mut active = self.active_stream.lock().await;
        if let Some(mut stream) = active.take() {
            let _ = stream.stop_signal.send(true);
            if let Some(h) = stream.capture_thread.take() {
                let _ = h.join();
            }
            if let Some(h) = stream.encode_thread.take() {
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
    frame_slot: FrameSlot,
    condvar: Arc<std::sync::Condvar>,
    stop_rx: tokio::sync::watch::Receiver<bool>,
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
    let mut capture_timeout_ms: u32 = 50; // fallback before first frame

    tracing::info!("NDI capture thread started for '{source_name}'");

    loop {
        if *stop_rx.borrow() {
            break;
        }

        match receiver.capture_video(capture_timeout_ms) {
            Ok(Some(frame)) => {
                // Adapt timeout to actual frame rate
                if frame.frame_rate_d > 0 && frame.frame_rate_n > 0 {
                    let period =
                        (1000 * frame.frame_rate_d as u64) / frame.frame_rate_n as u64;
                    capture_timeout_ms = (period as u32).clamp(16, 200);
                }

                if !connected {
                    connected = true;
                    tracing::info!(
                        "NDI connected: {}x{} @ {}/{}fps",
                        frame.width, frame.height,
                        frame.frame_rate_n, frame.frame_rate_d
                    );
                    if let Some(cb) = &status_cb {
                        cb("connected".to_string());
                    }
                }
                last_frame_time = std::time::Instant::now();

                // Write to shared slot — newest frame wins
                {
                    let mut slot = frame_slot.lock().unwrap_or_else(|e| e.into_inner());
                    *slot = Some(frame);
                }
                condvar.notify_one();
            }
            Ok(None) => {
                // Timeout — check for disconnect
                if connected
                    && last_frame_time.elapsed() > std::time::Duration::from_secs(3)
                {
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

// ---------------------------------------------------------------------------
// Encode thread
// ---------------------------------------------------------------------------

fn run_encode_thread(
    frame_slot: FrameSlot,
    condvar: Arc<std::sync::Condvar>,
    frame_tx: broadcast::Sender<Bytes>,
    stop_rx: tokio::sync::watch::Receiver<bool>,
) {
    let fourcc_uyvy = u32::from_le_bytes([b'U', b'Y', b'V', b'Y']);
    let fourcc_bgra = u32::from_le_bytes([b'B', b'G', b'R', b'A']);
    let fourcc_bgrx = u32::from_le_bytes([b'B', b'G', b'R', b'X']);
    let encoder = JpegEncoder::new(75);

    tracing::info!("NDI encode thread started");

    loop {
        if *stop_rx.borrow() {
            break;
        }

        // Wait for a frame (with timeout so we can check stop signal)
        let frame = {
            let slot = frame_slot.lock().unwrap_or_else(|e| e.into_inner());
            let (mut slot, _) = condvar
                .wait_timeout(slot, std::time::Duration::from_millis(100))
                .unwrap_or_else(|e| e.into_inner());
            slot.take()
        };

        let frame = match frame {
            Some(f) => f,
            None => continue,
        };

        let jpeg = if frame.fourcc == fourcc_bgra || frame.fourcc == fourcc_bgrx {
            encoder.encode_bgra(&frame.data, frame.width, frame.height)
        } else if frame.fourcc == fourcc_uyvy {
            encoder.encode_uyvy(&frame.data, frame.width, frame.height)
        } else {
            tracing::warn!("unsupported fourcc: 0x{:08x}", frame.fourcc);
            continue;
        };

        match jpeg {
            Ok(data) => {
                let _ = frame_tx.send(Bytes::from(data));
            }
            Err(e) => {
                tracing::error!("JPEG encode error: {e}");
            }
        }
    }

    tracing::info!("NDI encode thread stopped");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::receiver::VideoFrame;

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
        let frame = slot.lock().unwrap().take();
        assert_eq!(frame.unwrap().width, 2);
    }

    #[test]
    fn frame_slot_empty_read() {
        let slot: FrameSlot = Arc::new(std::sync::Mutex::new(None));
        let frame = slot.lock().unwrap().take();
        assert!(frame.is_none());
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p presenter-ndi --lib`
Expected: 5 tests pass (2 discovery + 2 frame_slot + 1 sdk_load).

- [ ] **Step 5: Commit**

```bash
git add crates/presenter-ndi/src/manager.rs
git commit -m "feat(ndi): separate capture and encode into two threads

Capture thread drains NDI frames into a shared slot without blocking
on encoding. Encode thread picks the latest frame and compresses to
JPEG. Eliminates stuttering caused by sequential capture+encode.

Also adds status callback for connection state reporting."
```

---

## Task 3: Wire Connection Status Events

**Files:**
- Modify: `crates/presenter-server/src/state/integrations.rs`

- [ ] **Step 1: Update activate_video_source to pass status callback**

In `crates/presenter-server/src/state/integrations.rs`, change `activate_video_source` (lines 136-147).

Replace:
```rust
        // Start NDI stream if manager is available
        if let Some(manager) = &self.ndi_manager {
            manager.start_stream(&source.ndi_name).await?;
        }
```

With:
```rust
        // Start NDI stream with status callback
        if let Some(manager) = &self.ndi_manager {
            let hub = self.live_hub.clone();
            let status_cb: presenter_ndi::StatusCallback =
                Arc::new(move |status: String| {
                    hub.publish(LiveEvent::NdiConnectionStatus { status });
                });
            manager
                .start_stream(&source.ndi_name, Some(status_cb))
                .await?;
        }
```

Also add `use std::sync::Arc;` to the imports at the top of the file if not already present.

- [ ] **Step 2: Verify compilation**

Run: `cargo fmt --all --check`
Expected: Clean.

- [ ] **Step 3: Commit**

```bash
git add crates/presenter-server/src/state/integrations.rs
git commit -m "feat(ndi): wire connection status events through live hub

Capture thread now publishes NdiConnectionStatus('connected') on first
frame and NdiConnectionStatus('disconnected') after 3s without frames.
Stage display shows/hides overlay accordingly."
```

---

## Task 4: Canvas Rendering in Browser

**Files:**
- Modify: `crates/presenter-ui/Cargo.toml`
- Modify: `crates/presenter-ui/src/components/stage/ndi_fullscreen.rs`

- [ ] **Step 1: Add web-sys features for canvas**

In `crates/presenter-ui/Cargo.toml`, add these features to the `web-sys` features list (after `"WebSocket",` on line 65):

```toml
    "HtmlCanvasElement",
    "CanvasRenderingContext2d",
    "ImageBitmap",
```

- [ ] **Step 2: Rewrite ndi_fullscreen.rs with canvas rendering**

Replace `crates/presenter-ui/src/components/stage/ndi_fullscreen.rs` entirely:

```rust
use leptos::prelude::*;
use wasm_bindgen::prelude::*;

use crate::components::stage::status_bar::StatusBar;
use crate::state::stage::StageContext;
use crate::ws::stage::StageWsState;

#[component]
pub fn NdiFullscreen(
    ws_state: ReadSignal<StageWsState>,
    latency_ms: ReadSignal<Option<f64>>,
) -> impl IntoView {
    let ctx = use_context::<StageContext>().expect("StageContext not provided");
    let ndi_active = ctx.ndi_active;
    let ndi_status = ctx.ndi_status;
    let canvas_ref = NodeRef::<leptos::html::Canvas>::new();

    // When ndi_active becomes true, connect to MJPEG WebSocket stream
    {
        let canvas_ref = canvas_ref.clone();
        Effect::new(move |_| {
            let active = ndi_active.get();
            if active {
                let canvas_ref = canvas_ref.clone();
                leptos::task::spawn_local(async move {
                    connect_mjpeg_ws(canvas_ref);
                });
            }
        });
    }

    view! {
        <div class="stage-ndi">
            <canvas
                node_ref=canvas_ref
                class="stage-ndi__video"
            />

            <Show when=move || !ndi_active.get()>
                <div class="stage-ndi__placeholder">
                    "No video source configured"
                </div>
            </Show>

            <Show when=move || {
                let status = ndi_status.get();
                status == "disconnected" || status == "connecting"
            }>
                <div class="stage-ndi__overlay">
                    {move || {
                        let status = ndi_status.get();
                        if status == "disconnected" {
                            "Signal Lost — Reconnecting..."
                        } else if status == "connecting" {
                            "Connecting..."
                        } else {
                            ""
                        }
                    }}
                </div>
            </Show>

            <StatusBar ws_state=ws_state latency_ms=latency_ms />
        </div>
    }
}

/// Connect to the MJPEG WebSocket stream and render frames to a <canvas>.
///
/// Uses `createImageBitmap()` for off-main-thread JPEG decoding, then
/// draws to canvas with `drawImage()`. This avoids the Blob URL overhead
/// of the previous <img>-based approach.
fn connect_mjpeg_ws(canvas_ref: NodeRef<leptos::html::Canvas>) {
    let window = match web_sys::window() {
        Some(w) => w,
        None => return,
    };
    let location = window.location();
    let protocol = location.protocol().unwrap_or_default();
    let host = location.host().unwrap_or_default();
    let ws_protocol = if protocol == "https:" { "wss:" } else { "ws:" };
    let ws_url = format!("{ws_protocol}//{host}/ndi/stream");

    let ws = match web_sys::WebSocket::new(&ws_url) {
        Ok(ws) => ws,
        Err(e) => {
            web_sys::console::error_1(&format!("NDI WS connect failed: {e:?}").into());
            return;
        }
    };
    ws.set_binary_type(web_sys::BinaryType::Arraybuffer);

    let canvas_ref_msg = canvas_ref.clone();
    let onmessage =
        Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |ev: web_sys::MessageEvent| {
            let data = ev.data();
            if let Ok(buf) = data.dyn_into::<js_sys::ArrayBuffer>() {
                let array = js_sys::Uint8Array::new(&buf);
                let blob_parts = js_sys::Array::new();
                blob_parts.push(&array);

                let mut options = web_sys::BlobPropertyBag::new();
                options.type_("image/jpeg");

                if let Ok(blob) = web_sys::Blob::new_with_buffer_source_sequence_and_options(
                    &blob_parts,
                    &options,
                ) {
                    let canvas_ref = canvas_ref_msg.clone();
                    // createImageBitmap decodes JPEG off the main thread
                    if let Some(window) = web_sys::window() {
                        if let Ok(promise) = window.create_image_bitmap_with_blob(&blob) {
                            let future = wasm_bindgen_futures::JsFuture::from(promise);
                            wasm_bindgen_futures::spawn_local(async move {
                                if let Ok(bitmap_js) = future.await {
                                    let bitmap: web_sys::ImageBitmap =
                                        bitmap_js.unchecked_into();
                                    if let Some(canvas_el) = canvas_ref.get() {
                                        let html_canvas: &web_sys::HtmlCanvasElement =
                                            canvas_el.as_ref();
                                        let bw = bitmap.width();
                                        let bh = bitmap.height();

                                        // Match canvas internal resolution to source
                                        if html_canvas.width() != bw
                                            || html_canvas.height() != bh
                                        {
                                            html_canvas.set_width(bw);
                                            html_canvas.set_height(bh);
                                        }

                                        if let Ok(Some(ctx)) = html_canvas.get_context("2d")
                                        {
                                            let ctx: web_sys::CanvasRenderingContext2d =
                                                ctx.unchecked_into();
                                            let _ = ctx
                                                .draw_image_with_image_bitmap(
                                                    &bitmap, 0.0, 0.0,
                                                );
                                        }
                                        bitmap.close();
                                    }
                                }
                            });
                        }
                    }
                }
            }
        });
    ws.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
    onmessage.forget();

    let onopen = Closure::<dyn FnMut()>::new(move || {
        web_sys::console::log_1(&"NDI: MJPEG WebSocket connected".into());
    });
    ws.set_onopen(Some(onopen.as_ref().unchecked_ref()));
    onopen.forget();

    let onerror = Closure::<dyn FnMut()>::new(move || {
        web_sys::console::error_1(&"NDI: MJPEG WebSocket error".into());
    });
    ws.set_onerror(Some(onerror.as_ref().unchecked_ref()));
    onerror.forget();

    let canvas_ref_reconnect = canvas_ref.clone();
    let onclose = Closure::<dyn FnMut()>::new(move || {
        web_sys::console::log_1(
            &"NDI: MJPEG WebSocket closed, reconnecting in 2s...".into(),
        );
        let canvas_ref = canvas_ref_reconnect.clone();
        let _ = gloo_timers::callback::Timeout::new(2000, move || {
            connect_mjpeg_ws(canvas_ref);
        });
    });
    ws.set_onclose(Some(onclose.as_ref().unchecked_ref()));
    onclose.forget();
}
```

- [ ] **Step 3: Verify formatting**

Run: `cargo fmt --all --check`
Fix if needed: `cargo fmt --all`

- [ ] **Step 4: Commit**

```bash
git add crates/presenter-ui/Cargo.toml crates/presenter-ui/src/components/stage/ndi_fullscreen.rs
git commit -m "feat(ndi): switch stage rendering to canvas + createImageBitmap

Replace Blob URL + <img> approach with <canvas> + createImageBitmap().
JPEG decoding happens off the main thread via the browser's built-in
decoder. No more Blob URL creation/revocation overhead per frame."
```

---

## Task 5: Update Server NDI Router (non-blocking discovery)

**Files:**
- Modify: `crates/presenter-server/src/router/integrations/ndi.rs`

- [ ] **Step 1: Make discover_ndi_sources async-clean**

The `discover_sources` method is now non-blocking (reads from persistent list), so the handler no longer blocks the async runtime. The code structurally stays the same — the change is in what `manager.discover_sources()` does internally. But update the `timeout` parameter:

In `crates/presenter-server/src/router/integrations/ndi.rs`, change line 28 from:
```rust
    let sources = manager.discover_sources(3000)?;
```
to:
```rust
    let sources = manager.discover_sources(0)?;
```

The timeout is now ignored by the persistent finder, but passing 0 makes the intent clear.

- [ ] **Step 2: Commit**

```bash
git add crates/presenter-server/src/router/integrations/ndi.rs
git commit -m "refactor(ndi): update discovery endpoint for persistent finder

Discovery now reads from the persistent finder's accumulated list.
The timeout parameter is no longer meaningful — pass 0 to signal this."
```

---

## Task 6: E2E Tests

**Files:**
- Modify: `tests/e2e/video-source-api.spec.ts`
- Modify: `tests/e2e/ndi-stage-layout.spec.ts`

- [ ] **Step 1: Add discovery stability test**

In `tests/e2e/video-source-api.spec.ts`, add after the last test (inside the `describe` block):

```typescript
  test("NDI discovery returns stable source count (persistent finder)", async ({
    request,
  }) => {
    const statusResp = await request.get(
      new URL("/ndi/status", baseURL).toString()
    );
    const { available } = await statusResp.json();
    test.skip(!available, "NDI SDK not available");

    // Wait for persistent finder to accumulate sources via mDNS
    await new Promise((r) => setTimeout(r, 6000));

    const counts: number[] = [];
    for (let i = 0; i < 5; i++) {
      const resp = await request.get(
        new URL("/ndi/sources", baseURL).toString()
      );
      expect(resp.status()).toBe(200);
      const sources = await resp.json();
      counts.push(sources.length);
      await new Promise((r) => setTimeout(r, 1000));
    }

    // All 5 scans should return the same count
    const unique = [...new Set(counts)];
    expect(unique).toHaveLength(1);
  });
```

- [ ] **Step 2: Add canvas rendering test**

In `tests/e2e/ndi-stage-layout.spec.ts`, add after the last test:

```typescript
  test("uses canvas element for NDI rendering", async ({ page }) => {
    const consoleMessages: string[] = [];
    page.on("console", (msg) => {
      if (msg.type() === "error" || msg.type() === "warning") {
        consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
      }
    });

    await page.request.post(new URL("/stage/layout", baseURL).toString(), {
      data: { code: "ndi-fullscreen" },
    });

    await page.goto(new URL("/stage", baseURL).toString());
    await page.waitForSelector('body[data-wasm-ready="true"]', {
      timeout: 30_000,
    });
    await page.waitForSelector('body[data-layout-code="ndi-fullscreen"]', {
      timeout: 10_000,
    });

    // Verify canvas element exists (not img)
    const canvas = page.locator("canvas.stage-ndi__video");
    await expect(canvas).toBeAttached();

    // Verify no img element for video
    const img = page.locator("img.stage-ndi__video");
    await expect(img).not.toBeAttached();

    expect(
      consoleMessages.filter((m) => !m.includes("favicon"))
    ).toEqual([]);
  });
```

- [ ] **Step 3: Add frame delivery test (NDI SDK required)**

In `tests/e2e/ndi-stage-layout.spec.ts`, add:

```typescript
  test("frame delivery is smooth (requires NDI source)", async ({
    page,
    request,
  }) => {
    const statusResp = await request.get(
      new URL("/ndi/status", baseURL).toString()
    );
    const { available } = await statusResp.json();
    test.skip(!available, "NDI SDK not available");

    // Wait for finder to discover sources
    await new Promise((r) => setTimeout(r, 6000));
    const sourcesResp = await request.get(
      new URL("/ndi/sources", baseURL).toString()
    );
    const sources = await sourcesResp.json();
    test.skip(sources.length === 0, "No NDI sources on network");

    // Create and activate a video source
    const createResp = await request.post(
      new URL("/integrations/video-sources", baseURL).toString(),
      { data: { label: "E2E Test", ndiName: sources[0].name } }
    );
    const source = await createResp.json();
    await request.post(
      new URL(
        `/integrations/video-sources/${source.id}/activate`,
        baseURL
      ).toString()
    );

    // Set layout and navigate
    await request.post(new URL("/stage/layout", baseURL).toString(), {
      data: { code: "ndi-fullscreen" },
    });
    await page.goto(new URL("/stage", baseURL).toString());
    await page.waitForSelector('body[data-wasm-ready="true"]', {
      timeout: 30_000,
    });

    // Measure frame delivery in the browser
    const metrics = await page.evaluate(() => {
      return new Promise<{
        frames: number;
        fps: string;
        maxIntervalMs: string;
        stutters: number;
      }>((resolve) => {
        const canvas = document.querySelector(
          "canvas.stage-ndi__video"
        ) as HTMLCanvasElement | null;
        if (!canvas) return resolve({ frames: 0, fps: "0", maxIntervalMs: "0", stutters: 0 });

        let frameCount = 0;
        let firstTime: number | null = null;
        let lastTime: number | null = null;
        const intervals: number[] = [];

        const observer = new MutationObserver(() => {
          // Canvas doesn't fire mutation events on draw, so we use a
          // different approach: monitor the WebSocket directly
        });

        // Monitor canvas draws via a proxy on getContext
        const origGetContext = canvas.getContext.bind(canvas);
        let ctx2d: CanvasRenderingContext2D | null = null;

        // Actually, we can't easily intercept drawImage. Instead,
        // monitor the WebSocket messages which trigger canvas draws.
        // Each binary WS message = one frame drawn.
        // We'll use performance observer or just count via WS.

        // Simpler approach: poll canvas pixel data changes
        const checkInterval = 10; // ms
        let lastPixelHash = "";
        const timer = setInterval(() => {
          try {
            const c = canvas.getContext("2d");
            if (!c || canvas.width === 0) return;
            const pixel = c.getImageData(
              Math.floor(canvas.width / 2),
              Math.floor(canvas.height / 2),
              1, 1
            ).data;
            const hash = `${pixel[0]},${pixel[1]},${pixel[2]}`;
            if (hash !== lastPixelHash && hash !== "0,0,0") {
              lastPixelHash = hash;
              const now = performance.now();
              frameCount++;
              if (firstTime === null) {
                firstTime = now;
              } else {
                intervals.push(now - lastTime!);
              }
              lastTime = now;
            }
          } catch { /* ignore */ }
        }, checkInterval);

        setTimeout(() => {
          clearInterval(timer);
          if (frameCount < 2) {
            return resolve({ frames: frameCount, fps: "0", maxIntervalMs: "0", stutters: 0 });
          }
          const elapsed = (lastTime! - firstTime!) / 1000;
          const fps = frameCount / elapsed;
          const avgInterval =
            intervals.reduce((a, b) => a + b, 0) / intervals.length;
          const maxInterval = Math.max(...intervals);
          const stutters = intervals.filter(
            (i) => i > avgInterval * 2
          ).length;
          resolve({
            frames: frameCount,
            fps: fps.toFixed(1),
            maxIntervalMs: maxInterval.toFixed(1),
            stutters,
          });
        }, 5000);
      });
    });

    // Assertions
    expect(metrics.frames).toBeGreaterThan(50); // expect ~150 at 30fps
    expect(parseFloat(metrics.fps)).toBeGreaterThan(20);
    expect(metrics.stutters).toBeLessThanOrEqual(2); // allow minor jitter

    // Cleanup
    await request.post(
      new URL("/integrations/video-sources/deactivate", baseURL).toString()
    );
    await request.delete(
      new URL(
        `/integrations/video-sources/${source.id}`,
        baseURL
      ).toString()
    );
  });
```

- [ ] **Step 4: Run E2E tests locally**

Run: `npx playwright test tests/e2e/ndi-stage-layout.spec.ts tests/e2e/video-source-api.spec.ts --reporter=line`
Expected: All tests pass (NDI-requiring tests run since SDK is available on dev machine).

- [ ] **Step 5: Commit**

```bash
git add tests/e2e/video-source-api.spec.ts tests/e2e/ndi-stage-layout.spec.ts
git commit -m "test(e2e): add NDI discovery stability, canvas, and frame delivery tests

Discovery stability: 5 scans must return same count (persistent finder).
Canvas: verify <canvas> element used instead of <img>.
Frame delivery: measure fps/stutters over 5s, assert smooth playback."
```

---

## Task 7: Version Bump, Format Check, Push, Monitor CI

- [ ] **Step 1: Check and bump version**

```bash
git fetch origin
DEV_VER=$(grep -m1 '^version = ' Cargo.toml | sed 's/version = "\(.*\)"/\1/')
MAIN_VER=$(git show origin/main:Cargo.toml | grep -m1 '^version = ' | sed 's/version = "\(.*\)"/\1/')
echo "dev=$DEV_VER main=$MAIN_VER"
```

If dev version is not higher than main, bump patch in `Cargo.toml` workspace `[workspace.package].version`.

- [ ] **Step 2: Format check**

Run: `cargo fmt --all --check`
Fix if needed: `cargo fmt --all`

- [ ] **Step 3: Push to dev**

```bash
git push origin dev
```

- [ ] **Step 4: Monitor CI**

```bash
gh run list --branch dev --limit 3
```

Wait for all jobs to reach terminal state. If any fail, investigate and fix.

---

## Verification

After CI deploys to dev (http://10.77.8.134:8080):

1. **Discovery stability**: Run `for i in 1 2 3 4 5; do curl -s http://10.77.8.134:8080/ndi/sources | python3 -c "import sys,json; d=json.load(sys.stdin); print(len(d))"; sleep 1; done` — all 5 should show the same count.

2. **Frame delivery**: Open stage in Playwright, activate NDI source, measure frame timing — expect 30fps with <3ms jitter, zero stutters.

3. **Canvas rendering**: Open http://10.77.8.134:8080/stage in Playwright, verify `<canvas>` element (not `<img>`), verify frames render without console errors.

4. **Connection status**: Activate NDI source, verify stage overlay clears ("connected"). Deactivate, verify "Signal Lost" overlay appears.

5. **E2E in CI**: All NDI tests pass on self-hosted runner (SDK available). GitHub-hosted runner tests skip gracefully.
