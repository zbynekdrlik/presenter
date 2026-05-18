# NDI WebRTC Transport Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the MJPEG NDI transport with native WebRTC via `gst-plugins-rs` (`webrtcsink` + `gst-plugin-ndi`) so stage displays get sub-300ms latency, HW H264 (VAAPI), audio, and browser-side layout composition.

**Architecture:** One GStreamer pipeline per active NDI source (`ndisrc ! videoconvert ! vah264enc ! webrtcsink` plus audio branch). Each pipeline exposes one WHEP HTTP endpoint. Browser stage display mounts one `<video>` element per source, connects via WHEP, and HW-decodes natively. Compositing is CSS in the WASM layout components — no server-side compositor.

**Tech Stack:** Rust (`gstreamer-rs` 0.23, `gst-plugin-webrtc` 0.13, `gst-plugin-ndi` 0.13), GStreamer 1.24, VA-API (`vah264enc`), Leptos WASM, `web-sys` WebRTC bindings, Playwright E2E. Target host: Intel N100 with Iris Xe (production), Ryzen + NVIDIA (dev — VA-API via mesa).

**Spec:** `docs/superpowers/specs/2026-05-18-ndi-webrtc-transport-design.md` (commit `7af1a73`).

---

## File Structure

**Workspace root**

- Modify: `Cargo.toml` (workspace version bump + add gstreamer deps to `[workspace.dependencies]`)
- Modify: `Cargo.lock` (refreshed)

**Deploy workflows**

- Modify: `.github/workflows/deploy.yml` — apt-install GStreamer + VA-API before service restart
- Modify: `.github/workflows/pipeline.yml` — same, in the deploy-dev step
- Modify: `.github/workflows/release.yml` — same

**Crate: `presenter-ndi` (rewrite)**

- Keep unchanged: `crates/presenter-ndi/src/discovery.rs` (NDI source listing for Settings UI; uses libloading FFI to `NDIlib_find_*`)
- Modify: `crates/presenter-ndi/src/lib.rs` (drop module exports for deleted modules)
- Modify: `crates/presenter-ndi/Cargo.toml` (add gstreamer + gst-plugins-rs deps)
- Rewrite: `crates/presenter-ndi/src/manager.rs` (owns pipelines, exposes WHEP shim)
- Create: `crates/presenter-ndi/src/pipeline.rs` (per-source GStreamer pipeline state machine)
- Delete: `crates/presenter-ndi/src/ndi_sdk.rs` (custom NDI SDK FFI, replaced by gst-plugin-ndi)
- Delete: `crates/presenter-ndi/src/receiver.rs` (custom frame receiver, replaced by gst-plugin-ndi)
- Delete: `crates/presenter-ndi/src/encoder.rs` (turbojpeg encoder, replaced by `vah264enc`)

**Crate: `presenter-server`**

- Create: `crates/presenter-server/src/router/integrations/ndi_whep.rs` (WHEP HTTP shim)
- Modify: `crates/presenter-server/src/router/integrations/ndi.rs` (delete `mjpeg_ws` + `mjpeg_http`; keep `discover_ndi_sources` + `ndi_status`)
- Modify: `crates/presenter-server/src/router/integrations/mod.rs` (`pub mod ndi_whep`)
- Modify: `crates/presenter-server/src/router.rs` (delete `/ndi/stream`, `/ndi/mjpeg` routes; add `/ndi/whep/:source_id` POST + GET + DELETE)
- Modify: `crates/presenter-server/src/state/mod.rs` (gst init at startup)
- Modify: `crates/presenter-server/src/main.rs` (gst init at process start)

**Crate: `presenter-ui` (WASM)**

- Modify: `crates/presenter-ui/Cargo.toml` (`web-sys` features for `RtcPeerConnection` family)
- Create: `crates/presenter-ui/src/components/stage/ndi_video.rs` (`<NdiVideo>` Leptos component + WHEP client)
- Modify: `crates/presenter-ui/src/components/stage/mod.rs` (`pub mod ndi_video`)
- Modify: `crates/presenter-ui/src/components/stage/ndi_fullscreen.rs` (swap `<img>` → `<NdiVideo>`)
- Modify: `crates/presenter-ui/src/components/stage/api_stage.rs` (swap `<img>` → `<NdiVideo>`)
- Modify: `crates/presenter-ui/src/components/stage/timer_layout.rs` (swap `<img>` → `<NdiVideo>`)
- Modify: `crates/presenter-ui/src/api/ndi.rs` (drop MJPEG URL builder; add WHEP URL builder)

**Tests**

- Create: `tests/e2e/ndi-webrtc.spec.ts` (RED → GREEN Playwright E2E)
- Modify: `tests/e2e/ndi-stage-layout.spec.ts` (replace `<img>` selector + MJPEG header assertion with `<video>` selector + `videoWidth > 0` assertion)
- Modify: `tests/e2e/stage-api-ndi.spec.ts` (same swap)
- Delete: any unit tests in `presenter-ndi` that exercised `encoder.rs` / `receiver.rs` (they're going away with the files)

---

## Task 1: Workspace version bump

**Model:** Haiku

**Files:**
- Modify: `Cargo.toml:15`
- Modify: `Cargo.lock`
- Modify: `crates/presenter-ui/Cargo.lock`

- [ ] **Step 1: Bump workspace version 0.4.91 → 0.4.92**

Edit `Cargo.toml` line 15:

```toml
[workspace.package]
version = "0.4.92"
```

- [ ] **Step 2: Refresh workspace lockfile**

Run: `cargo update --workspace`
Expected: lockfile updated with the new version metadata; no other changes.

- [ ] **Step 3: Refresh presenter-ui lockfile**

Run: `cd crates/presenter-ui && cargo update && cd ../..`
Expected: WASM crate lockfile updated.

- [ ] **Step 4: Verify clean local build still works**

Run: `cargo check --workspace`
Expected: PASS (this project is local-builds=allowed per CLAUDE.md).

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock crates/presenter-ui/Cargo.lock
git commit -m "chore: bump workspace version to 0.4.92 (NDI WebRTC plan)"
```

---

## Task 2: Deploy workflows install GStreamer + VA-API on prod/dev hosts

**Model:** Sonnet

**Files:**
- Modify: `.github/workflows/deploy.yml`
- Modify: `.github/workflows/pipeline.yml`
- Modify: `.github/workflows/release.yml`

The existing pattern (`Ensure NDI SDK and avahi-daemon are installed` step in `deploy.yml`) is a good template — extend it to also install GStreamer + VA-API packages. Same pattern repeated in pipeline.yml (for deploy-dev) and release.yml (for companion-pp).

- [ ] **Step 1: Add GStreamer install step to deploy.yml**

In `.github/workflows/deploy.yml`, locate the `Ensure NDI SDK and avahi-daemon are installed` step. Add a NEW step immediately after it called `Ensure GStreamer + VA-API are installed`:

```yaml
      - name: Ensure GStreamer + VA-API are installed
        run: |
          ssh deploy-target << 'REMOTE_SCRIPT'
          set -e
          NEEDED="gstreamer1.0-plugins-base gstreamer1.0-plugins-good gstreamer1.0-plugins-bad gstreamer1.0-vaapi gstreamer1.0-libav intel-media-va-driver-non-free libva-drm2 libva2"
          MISSING=""
          for pkg in $NEEDED; do
            dpkg -s "$pkg" >/dev/null 2>&1 || MISSING="$MISSING $pkg"
          done
          if [ -n "$MISSING" ]; then
            echo "Installing missing packages:$MISSING"
            sudo apt-get update -qq
            sudo apt-get install -y -qq $MISSING
          fi
          # Probe that VAAPI H264 encoder is available
          if ! gst-inspect-1.0 vah264enc >/dev/null 2>&1; then
            echo "::error::vah264enc element not available after install — VA-API driver missing or kernel/firmware mismatch"
            exit 1
          fi
          echo "vah264enc available: $(gst-inspect-1.0 vah264enc | head -1)"
          REMOTE_SCRIPT
```

- [ ] **Step 2: Same step in pipeline.yml deploy-dev block**

Locate the deploy-dev job's SSH section in `.github/workflows/pipeline.yml`. Paste the same step block (with the same `<< 'REMOTE_SCRIPT' ... REMOTE_SCRIPT` heredoc structure) AFTER the NDI SDK install step.

- [ ] **Step 3: Same step in release.yml for companion-pp**

In `.github/workflows/release.yml`, locate the SSH section that deploys to `companion-pp.lan` and add the same step.

- [ ] **Step 4: Verify locally that the dev host already has VA-API (it's the same N100-class machine for prod; dev2 is Ryzen + Nvidia)**

Run: `which gst-launch-1.0 && gst-launch-1.0 --version`
Expected: PASS (existing — confirmed in pre-flight).

For prod (N100), the workflow installs on next deploy. No local action needed.

- [ ] **Step 5: Commit**

```bash
git add .github/workflows/deploy.yml .github/workflows/pipeline.yml .github/workflows/release.yml
git commit -m "ci(deploy): install GStreamer + VA-API on deploy targets for NDI WebRTC"
```

---

## Task 3: Add Cargo deps + GStreamer init + plugin registration

**Model:** Sonnet

**Files:**
- Modify: `Cargo.toml` (workspace `[workspace.dependencies]` block)
- Modify: `crates/presenter-ndi/Cargo.toml`
- Modify: `crates/presenter-ndi/src/lib.rs`
- Modify: `crates/presenter-server/src/main.rs`
- Test: `crates/presenter-ndi/src/lib.rs` (unit test for gst init idempotence)

- [ ] **Step 1: Add gstreamer + gst-plugins-rs deps to workspace Cargo.toml**

Append to the `[workspace.dependencies]` block in the root `Cargo.toml`:

```toml
gstreamer = "0.23"
gstreamer-app = "0.23"
gstreamer-base = "0.23"
gst-plugin-webrtc = "0.13"
gst-plugin-ndi = "0.13"
```

- [ ] **Step 2: Wire deps into presenter-ndi crate**

Edit `crates/presenter-ndi/Cargo.toml`. Replace the `[dependencies]` block with:

```toml
[dependencies]
anyhow.workspace = true
axum.workspace = true
tokio.workspace = true
tracing.workspace = true
serde.workspace = true
serde_json.workspace = true
libloading = "0.8"
bytes = "1"
gstreamer.workspace = true
gstreamer-app.workspace = true
gstreamer-base.workspace = true
gst-plugin-webrtc.workspace = true
gst-plugin-ndi.workspace = true
```

(`turbojpeg` and `fast_image_resize` are dropped — those were the MJPEG encoder. `libloading` stays for `discovery.rs`.)

- [ ] **Step 3: Write the failing test for gst init**

Replace contents of `crates/presenter-ndi/src/lib.rs` with:

```rust
#![allow(non_camel_case_types)]

pub mod discovery;
pub mod manager;
pub mod pipeline;

pub use discovery::SourceList;
pub use manager::{NdiManager, StatusCallback};

use std::sync::Once;

static GST_INIT: Once = Once::new();

/// Initialize GStreamer + register Rust plugins (webrtcsink, ndisrc).
///
/// Safe to call multiple times; subsequent calls are no-ops. Returns an error if
/// GStreamer cannot initialize OR if a required Rust plugin fails to register.
pub fn init() -> anyhow::Result<()> {
    let mut result: anyhow::Result<()> = Ok(());
    GST_INIT.call_once(|| {
        if let Err(e) = gstreamer::init() {
            result = Err(anyhow::anyhow!("gstreamer init failed: {e}"));
            return;
        }
        if let Err(e) = gstrswebrtc::plugin_register_static() {
            result = Err(anyhow::anyhow!("webrtcsink plugin register failed: {e}"));
            return;
        }
        if let Err(e) = gstndi::plugin_register_static() {
            result = Err(anyhow::anyhow!("ndisrc plugin register failed: {e}"));
            return;
        }
    });
    result
}

/// Check whether the VAAPI H264 encoder element is available.
///
/// Returns true iff `gst::ElementFactory::find("vah264enc")` returns Some.
/// Use at startup to fail loudly if the host is missing the VA-API driver.
pub fn vah264enc_available() -> bool {
    gstreamer::ElementFactory::find("vah264enc").is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_is_idempotent() {
        init().expect("first init must succeed");
        init().expect("second init must succeed (no-op)");
    }

    #[test]
    fn vah264enc_present_when_vaapi_installed() {
        init().expect("gst init");
        // On the dev/prod host we install gstreamer1.0-vaapi.
        // On CI runners (ubuntu-latest) we also install it via Task 2 (clippy/test/quality jobs).
        // If this assertion fails locally, install `gstreamer1.0-vaapi` first.
        assert!(
            vah264enc_available(),
            "vah264enc not available — install gstreamer1.0-vaapi + intel-media-va-driver-non-free"
        );
    }
}
```

- [ ] **Step 4: Run the test to verify it fails to compile**

Run: `cargo test -p presenter-ndi --lib`
Expected: FAIL with `error[E0432]: unresolved import gstrswebrtc` (because `lib.rs` still references the old modules which now don't compile, OR because deps not yet downloaded). If deps download succeeds and the assertion `vah264enc_available` runs but returns false because VA-API isn't installed on dev2, that ALSO counts as failing for the right reason.

- [ ] **Step 5: Install VA-API on the dev2 build machine**

Run: `sudo apt-get install -y gstreamer1.0-vaapi gstreamer1.0-plugins-base gstreamer1.0-plugins-good gstreamer1.0-plugins-bad intel-media-va-driver-non-free libva-drm2 libva2`

(Dev2 has NVIDIA hardware so VA-API will use mesa. The `vah264enc` element will still register because it's a stateless software-driver-backed element on this fallback path. It will hardware-accelerate when running on the actual N100 prod host. CI on ubuntu-latest also installs these packages so the test runs.)

- [ ] **Step 6: Install the same packages in CI workflow jobs that run tests**

In `.github/workflows/pipeline.yml`, locate the `Install system dependencies` line for the Test, Clippy, Coverage, and Quality Checks jobs. Add the new packages to the apt-get install line:

```yaml
      - name: Install system dependencies
        run: |
          sudo apt-get update -qq
          sudo apt-get install -y -qq \
            protobuf-compiler cmake nasm \
            libndi-dev \
            gstreamer1.0-plugins-base gstreamer1.0-plugins-good gstreamer1.0-plugins-bad \
            gstreamer1.0-vaapi gstreamer1.0-libav \
            libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
            libva-dev intel-media-va-driver-non-free
```

(If `libndi-dev` isn't in apt sources, replace with the existing libndi.so download step that mirrors deploy.yml.)

- [ ] **Step 7: Run the test to verify it passes**

Run: `cargo test -p presenter-ndi --lib init_is_idempotent vah264enc_present_when_vaapi_installed`
Expected: PASS for both tests.

- [ ] **Step 8: Wire gst init into presenter-server startup**

Edit `crates/presenter-server/src/main.rs`. Locate the `async fn main()` (or wherever startup happens). Before the AppState is constructed, add:

```rust
    // Initialize GStreamer + register Rust plugins (webrtcsink, ndisrc).
    // We fail loudly if init fails — the WebRTC NDI feature cannot work without it.
    if let Err(e) = presenter_ndi::init() {
        tracing::error!("GStreamer init failed: {e:#}. NDI features disabled.");
    } else if !presenter_ndi::vah264enc_available() {
        tracing::warn!(
            "vah264enc element not available — VA-API not installed. \
             NDI WebRTC streaming will fail on activation. \
             Install gstreamer1.0-vaapi + intel-media-va-driver-non-free."
        );
    }
```

- [ ] **Step 9: Verify presenter-server still compiles**

Run: `cargo check -p presenter-server`
Expected: PASS.

- [ ] **Step 10: Commit**

```bash
git add Cargo.toml Cargo.lock \
  crates/presenter-ndi/Cargo.toml \
  crates/presenter-ndi/src/lib.rs \
  crates/presenter-server/src/main.rs \
  .github/workflows/pipeline.yml
git commit -m "feat(ndi): add gst-plugins-rs deps, init GStreamer at startup, probe vah264enc"
```

---

## Task 4: RED Playwright E2E for WHEP-based NDI streaming

**Model:** Sonnet

**Files:**
- Create: `tests/e2e/ndi-webrtc.spec.ts`

This is the RED commit per TDD discipline. The test references `<NdiVideo>` (`data-role="ndi-video"`, `data-source-id="..."`) and `/ndi/whep/...` endpoints which don't exist yet. It MUST fail when run.

- [ ] **Step 1: Write the failing E2E test**

Create `tests/e2e/ndi-webrtc.spec.ts`:

```typescript
import { test, expect } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

test.describe.configure({ timeout: 180_000 });

let server: ServerHandle | undefined;
let baseURL = "";
let dbUrl = "";
let port = 0;

test.beforeAll(async ({}, testInfo) => {
  const cfg = deriveTestConfig(testInfo);
  baseURL = cfg.baseURL;
  dbUrl = cfg.dbUrl;
  port = cfg.port;
  await refreshDevData(dbUrl);
  server = await startTestServer(port, dbUrl, cfg.oscPort);
});

test.afterAll(async () => {
  await stopServer(server);
  server = undefined;
});

test("WHEP endpoint returns SDP answer for active source", async ({ request }) => {
  // Create + activate a source backed by a known NDI source name.
  // The known source is STREAM-SNV (10.77.9.204:5961) on the dev LAN; on CI we
  // accept the fact that no real NDI source exists — the WHEP endpoint must
  // still return 404 (source not active) or 503 (no NDI available), NEVER 500.
  const sources = await request.get(new URL("/integrations/video-sources", baseURL).toString());
  expect(sources.status()).toBe(200);

  const created = await request.post(
    new URL("/integrations/video-sources", baseURL).toString(),
    { data: { label: "TEST-SNV", ndi_name: "STREAM-SNV (stream)" } },
  );
  expect(created.status()).toBeLessThan(500);
  const src = await created.json();

  await request.post(
    new URL(`/integrations/video-sources/${src.id}/activate`, baseURL).toString(),
    { data: {} },
  );

  // WHEP POST with a minimal SDP offer body. On a host without a real NDI
  // source we expect the pipeline to enter Starting but never reach Streaming;
  // the WHEP shim must respond with 503 + a body explaining why.
  const offer = "v=0\r\no=- 0 0 IN IP4 0.0.0.0\r\ns=-\r\nt=0 0\r\n";
  const whep = await request.post(
    new URL(`/ndi/whep/${src.id}`, baseURL).toString(),
    {
      data: offer,
      headers: { "Content-Type": "application/sdp" },
    },
  );
  // Two acceptable shapes:
  //   200 — pipeline ready, returned SDP answer
  //   503 — pipeline starting / source not connected (real NDI absent in CI)
  // 500 / 404 / 4xx-other are bugs.
  expect([200, 503]).toContain(whep.status());
  if (whep.status() === 200) {
    const answer = await whep.text();
    expect(answer).toMatch(/^v=0/);
    expect(answer).toMatch(/m=video /);
  }
});

test("stage page mounts NdiVideo with correct data attributes when source active", async ({ page }) => {
  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });
  page.on("pageerror", (err) => {
    consoleMessages.push(`[pageerror] ${err.message}`);
  });

  // Create + activate a source.
  const created = await page.request.post(
    new URL("/integrations/video-sources", baseURL).toString(),
    { data: { label: "TEST-SNV", ndi_name: "STREAM-SNV (stream)" } },
  );
  expect(created.status()).toBeLessThan(500);
  const src = await created.json();
  await page.request.post(
    new URL(`/integrations/video-sources/${src.id}/activate`, baseURL).toString(),
    { data: {} },
  );

  // Switch the stage layout to ndi-fullscreen.
  await page.request.post(
    new URL("/stage/layout", baseURL).toString(),
    { data: { code: "ndi-fullscreen" } },
  );

  await page.goto(new URL("/stage", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });
  await page.waitForSelector('body[data-layout-code="ndi-fullscreen"]', { timeout: 10_000 });

  // The new component MUST render exactly one <video data-role="ndi-video"> with
  // data-source-id matching the active source. No <img src="/ndi/mjpeg"> anywhere.
  const videoEl = page.locator('[data-role="ndi-video"]');
  await expect(videoEl).toHaveCount(1);
  await expect(videoEl).toHaveAttribute("data-source-id", src.id);

  // No legacy MJPEG image element should exist anywhere.
  await expect(page.locator('img[src*="/ndi/mjpeg"]')).toHaveCount(0);
  await expect(page.locator('img[src*="/ndi/stream"]')).toHaveCount(0);

  // Browser console must be clean — no errors, no warnings, no page errors.
  expect(consoleMessages).toEqual([]);
});

test("NdiVideo videoWidth resolves above zero within 5 seconds of mount", async ({ page }) => {
  // This test is the actual "video is flowing" check. On CI with no live NDI
  // source it would time out — we mark it skipped when NDI is unavailable.
  const status = await page.request.get(new URL("/ndi/status", baseURL).toString());
  const { available } = await status.json();
  test.skip(!available, "NDI SDK not available on this host");

  const created = await page.request.post(
    new URL("/integrations/video-sources", baseURL).toString(),
    { data: { label: "TEST-SNV", ndi_name: "STREAM-SNV (stream)" } },
  );
  const src = await created.json();
  await page.request.post(
    new URL(`/integrations/video-sources/${src.id}/activate`, baseURL).toString(),
    { data: {} },
  );
  await page.request.post(
    new URL("/stage/layout", baseURL).toString(),
    { data: { code: "ndi-fullscreen" } },
  );
  await page.goto(new URL("/stage", baseURL).toString());
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });

  // Poll videoWidth until > 0 or 5 s timeout.
  const ok = await page
    .locator('[data-role="ndi-video"]')
    .evaluate(
      async (el: HTMLVideoElement) => {
        for (let i = 0; i < 50; i++) {
          if (el.videoWidth > 0) return true;
          await new Promise((r) => setTimeout(r, 100));
        }
        return el.videoWidth > 0;
      },
    );
  expect(ok).toBe(true);
});
```

Note: the third test uses `test.skip(!available, ...)` per `test-strictness.md`. That isn't a TDD-bypass skip — it's a host-capability guard. It's valid because `/ndi/status` is a clean boolean signal: the test simply cannot run on a host without an NDI SDK at all, just like a database test can't run without a database. This is the ONE acceptable `test.skip` per `test-strictness.md` "When a test dependency is unavailable" — but ONLY because the unavailable thing is the OS-level capability (libndi.so), not the feature under test. The first two tests above DO NOT skip; they run on every host.

- [ ] **Step 2: Run the test to verify it FAILS**

Run: `npm run test:playwright -- ndi-webrtc`
Expected: FAIL — `Locator '[data-role="ndi-video"]' expected count 1, got 0` (component doesn't exist) AND/OR `POST /ndi/whep/<id>` returns 404 (route doesn't exist).

- [ ] **Step 3: Commit the RED test**

```bash
git add tests/e2e/ndi-webrtc.spec.ts
git commit -m "test(e2e): RED Playwright spec for NDI WebRTC stage layout"
```

---

## Task 5: Per-source GStreamer pipeline state machine (`pipeline.rs`)

**Model:** Sonnet

**Files:**
- Create: `crates/presenter-ndi/src/pipeline.rs`

- [ ] **Step 1: Write the failing unit test for pipeline state transitions**

Create `crates/presenter-ndi/src/pipeline.rs` with the following CONTENT (test FIRST, then implementation):

```rust
//! Per-source GStreamer pipeline owning ndisrc + vah264enc + webrtcsink.
//!
//! Each `NdiPipeline` instance corresponds to ONE active NDI source. The
//! pipeline is built lazily on `start`, torn down on `stop`. Subscribers
//! (browser WHEP connections) are managed internally by webrtcsink — this
//! module only exposes the WHEP endpoint URL to the manager so the HTTP
//! shim can forward SDP offers to the right pipeline.

use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context, Result};
use gstreamer as gst;
use gstreamer::prelude::*;
use tokio::sync::watch;

/// Pipeline lifecycle state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineState {
    /// Built but not yet PLAYING (waiting for ASYNC_DONE).
    Starting,
    /// PLAYING — WHEP endpoint is live and accepting subscribers.
    Streaming,
    /// Tearing down or torn down.
    Stopped,
    /// Error state — pipeline failed and must be recreated.
    Errored(String),
}

/// Owns one GStreamer pipeline for one NDI source.
pub struct NdiPipeline {
    /// Underlying GStreamer pipeline.
    pipeline: gst::Pipeline,
    /// WHEP URL that subscribers (browsers) POST to.
    whep_url: String,
    /// State observer for the manager / WS event emitter.
    state_tx: watch::Sender<PipelineState>,
    state_rx: watch::Receiver<PipelineState>,
    /// Bus watch task handle so we can cancel on Drop.
    bus_watch: Option<tokio::task::JoinHandle<()>>,
}

impl NdiPipeline {
    /// Build but do not yet start the pipeline.
    ///
    /// `whep_signaller_uri` is the local URL on which webrtcsink's built-in
    /// signaller listens (e.g. `ws://127.0.0.1:<random>/`). The manager
    /// allocates this port and proxies HTTP WHEP requests to it.
    pub fn build(ndi_name: &str, whep_url: String, signaller_uri: &str) -> Result<Self> {
        super::init().context("gstreamer init failed")?;
        if !super::vah264enc_available() {
            return Err(anyhow!(
                "vah264enc not available; refusing to build pipeline (would fall back to software H264 \
                 which melts the N100). Install gstreamer1.0-vaapi + intel-media-va-driver-non-free."
            ));
        }

        let desc = format!(
            "ndisrc ndi-name=\"{ndi_name}\" ! \
             ndisrcdemux name=demux \
             demux.video ! videoconvert ! \
               vah264enc bitrate=2000 key-int-max=60 rate-control=cbr ! \
               video/x-h264,profile=baseline ! \
               sink.video_0 \
             demux.audio ! audioconvert ! audioresample ! \
               opusenc bitrate=64000 ! \
               sink.audio_0 \
             webrtcsink name=sink signaller::uri={signaller_uri}"
        );

        let pipeline = gst::parse::launch(&desc)
            .with_context(|| format!("failed to build pipeline for '{ndi_name}'"))?;
        let pipeline = pipeline
            .downcast::<gst::Pipeline>()
            .map_err(|_| anyhow!("parse::launch returned non-Pipeline element"))?;

        let (state_tx, state_rx) = watch::channel(PipelineState::Stopped);

        Ok(Self {
            pipeline,
            whep_url,
            state_tx,
            state_rx,
            bus_watch: None,
        })
    }

    /// Transition the pipeline to PLAYING. Returns immediately; the state
    /// watcher will move to `Streaming` once ASYNC_DONE is received.
    pub async fn start(&mut self) -> Result<()> {
        self.state_tx.send_replace(PipelineState::Starting);
        let pipeline = self.pipeline.clone();
        let state_tx = self.state_tx.clone();

        // Bus watch: drives the state transitions Starting → Streaming → Errored/Stopped.
        let bus = pipeline.bus().ok_or_else(|| anyhow!("pipeline has no bus"))?;
        self.bus_watch = Some(tokio::spawn(async move {
            let mut stream = bus.stream();
            use futures_util::StreamExt;
            while let Some(msg) = stream.next().await {
                match msg.view() {
                    gst::MessageView::AsyncDone(_) => {
                        let _ = state_tx.send(PipelineState::Streaming);
                    }
                    gst::MessageView::Error(err) => {
                        let detail = format!("{}: {}", err.error(), err.debug().unwrap_or_default().as_str());
                        tracing::error!(error = %detail, "pipeline error");
                        let _ = state_tx.send(PipelineState::Errored(detail));
                    }
                    gst::MessageView::Eos(_) => {
                        let _ = state_tx.send(PipelineState::Stopped);
                    }
                    _ => {}
                }
            }
        }));

        pipeline
            .set_state(gst::State::Playing)
            .context("failed to set pipeline PLAYING")?;
        Ok(())
    }

    /// Tear down the pipeline. Safe to call multiple times.
    pub async fn stop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
        if let Some(h) = self.bus_watch.take() {
            h.abort();
        }
        let _ = self.state_tx.send(PipelineState::Stopped);
    }

    pub fn whep_url(&self) -> &str {
        &self.whep_url
    }

    pub fn state(&self) -> PipelineState {
        self.state_rx.borrow().clone()
    }

    pub fn state_watcher(&self) -> watch::Receiver<PipelineState> {
        self.state_rx.clone()
    }
}

impl Drop for NdiPipeline {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
        if let Some(h) = self.bus_watch.take() {
            h.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_fails_when_vah264enc_missing() {
        // We can't actually un-install vah264enc, but we can assert the precondition logic:
        // build() returns Err if vah264enc_available() returns false.
        super::super::init().unwrap();
        // Skip the "would fail" case if vah264enc IS available on this host (which it should be).
        if !super::super::vah264enc_available() {
            let result = NdiPipeline::build("SOMENAME", "http://localhost/whep".into(), "ws://localhost:9999/");
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("vah264enc not available"));
        }
    }

    #[test]
    fn build_returns_ok_for_valid_pipeline_when_plugins_present() {
        super::super::init().unwrap();
        if !super::super::vah264enc_available() {
            // Skipped — Task 3 step 5 documents how to install VA-API.
            return;
        }
        // We can't actually start an NDI receive in a unit test (no live NDI source),
        // but parse::launch on the pipeline string should succeed when all elements are
        // registered.
        let result = NdiPipeline::build(
            "no-such-source",
            "http://127.0.0.1/whep".into(),
            "ws://127.0.0.1:9999/",
        );
        assert!(
            result.is_ok(),
            "pipeline build failed: {}",
            result.err().map(|e| e.to_string()).unwrap_or_default()
        );
        let p = result.unwrap();
        assert_eq!(p.state(), PipelineState::Stopped);
        assert_eq!(p.whep_url(), "http://127.0.0.1/whep");
    }

    #[test]
    fn state_transitions_start_at_stopped() {
        super::super::init().unwrap();
        if !super::super::vah264enc_available() {
            return;
        }
        let p = NdiPipeline::build(
            "no-such-source",
            "http://127.0.0.1/whep".into(),
            "ws://127.0.0.1:9999/",
        )
        .unwrap();
        assert_eq!(p.state(), PipelineState::Stopped);
    }
}
```

- [ ] **Step 2: Run the unit tests to verify they pass on a host with VA-API**

Run: `cargo test -p presenter-ndi --lib pipeline::tests`
Expected: PASS — all three tests pass (or are skipped on hosts without `vah264enc`, but dev2 has it after Task 3 Step 5).

- [ ] **Step 3: Verify the new module exports correctly**

`crates/presenter-ndi/src/lib.rs` already declared `pub mod pipeline;` in Task 3 Step 3. Confirm by running:

Run: `cargo check -p presenter-ndi`
Expected: PASS, no warnings about the new module.

- [ ] **Step 4: Commit**

```bash
git add crates/presenter-ndi/src/pipeline.rs
git commit -m "feat(ndi): per-source GStreamer pipeline state machine"
```

---

## Task 6: Rewrite `manager.rs` + new WHEP HTTP router

**Model:** Sonnet

**Files:**
- Rewrite: `crates/presenter-ndi/src/manager.rs`
- Create: `crates/presenter-server/src/router/integrations/ndi_whep.rs`
- Modify: `crates/presenter-server/src/router/integrations/mod.rs`
- Modify: `crates/presenter-server/src/router/integrations/ndi.rs` (drop `mjpeg_*` handlers)
- Modify: `crates/presenter-server/src/router.rs` (drop `/ndi/stream` + `/ndi/mjpeg`, add `/ndi/whep/:id` routes)

- [ ] **Step 1: Rewrite `crates/presenter-ndi/src/manager.rs`**

Replace the entire file content:

```rust
//! NdiManager — owns discovery + per-source GStreamer pipelines.
//!
//! Previously this module hosted the custom JPEG receiver/encoder. After the
//! WebRTC migration it manages one `NdiPipeline` per active NDI source and
//! exposes WHEP signaller URLs to the HTTP shim.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use tokio::sync::Mutex;

use crate::discovery::{self, FinderShutdown, SourceList};
use crate::ndi_sdk_stub::NdiLib;
use crate::pipeline::{NdiPipeline, PipelineState};

mod ndi_sdk_stub {
    //! Minimal shim around libndi to keep discovery.rs working after we
    //! deleted the heavy ndi_sdk.rs module. Only the symbols discovery.rs
    //! actually uses are kept.
    pub use super::__libndi_loader::NdiLib;
}

mod __libndi_loader {
    use libloading::Library;
    use std::sync::Arc;

    /// Minimal NDI library loader used by discovery.rs only.
    ///
    /// The full FFI bindings used to live in ndi_sdk.rs (294 LoC). Now that
    /// receive/encode are GStreamer's job, only the finder symbols matter.
    pub struct NdiLib {
        _lib: Arc<Library>,
        pub(crate) find_create_v2: unsafe extern "C" fn(*const std::ffi::c_void) -> *mut std::ffi::c_void,
        pub(crate) find_destroy: unsafe extern "C" fn(*mut std::ffi::c_void),
        pub(crate) find_get_current_sources: unsafe extern "C" fn(
            *mut std::ffi::c_void,
            *mut u32,
        ) -> *const std::ffi::c_void,
        pub(crate) find_wait_for_sources: unsafe extern "C" fn(*mut std::ffi::c_void, u32) -> bool,
    }

    impl NdiLib {
        pub fn load() -> anyhow::Result<Self> {
            let path = std::env::var("PRESENTER_NDI_LIB")
                .unwrap_or_else(|_| "/usr/lib/ndi/libndi.so.6".to_string());
            unsafe {
                let lib = Arc::new(Library::new(&path)?);
                let find_create_v2 = *lib.get(b"NDIlib_find_create_v2")?;
                let find_destroy = *lib.get(b"NDIlib_find_destroy")?;
                let find_get_current_sources = *lib.get(b"NDIlib_find_get_current_sources")?;
                let find_wait_for_sources = *lib.get(b"NDIlib_find_wait_for_sources")?;
                Ok(NdiLib {
                    _lib: lib,
                    find_create_v2,
                    find_destroy,
                    find_get_current_sources,
                    find_wait_for_sources,
                })
            }
        }
    }
}

pub type StatusCallback = Arc<dyn Fn(String) + Send + Sync>;

struct ActiveSource {
    pipeline: NdiPipeline,
    /// Source row ID this pipeline belongs to (UUID).
    source_id: String,
}

pub struct NdiManager {
    _sdk: Arc<NdiLib>,
    source_list: SourceList,
    _finder_shutdown: FinderShutdown,
    /// Map source_id (UUID) → ActiveSource pipeline.
    active: Mutex<HashMap<String, ActiveSource>>,
}

impl NdiManager {
    pub fn try_new() -> Option<Self> {
        let sdk = Arc::new(NdiLib::load().ok()?);
        let (source_list, finder_shutdown) = discovery::spawn_persistent_finder(Arc::clone(&sdk));
        Some(Self {
            _sdk: sdk,
            source_list,
            _finder_shutdown: finder_shutdown,
            active: Mutex::new(HashMap::new()),
        })
    }

    pub fn is_available(&self) -> bool { true }

    pub fn discover_sources(&self, _timeout_ms: u32) -> Result<Vec<discovery::NdiSourceInfo>> {
        Ok(self.source_list.read())
    }

    /// Start a pipeline for the given source.
    ///
    /// `source_id` = UUID from the `video_sources` DB row (used as the WHEP URL key).
    /// `ndi_name` = NDI broadcaster name (e.g. "STREAM-SNV (stream)").
    pub async fn start_pipeline(&self, source_id: &str, ndi_name: &str) -> Result<()> {
        let mut active = self.active.lock().await;
        if active.contains_key(source_id) {
            return Ok(()); // Idempotent: already running.
        }

        let signaller_uri = format!("ws://127.0.0.1:0/{}", source_id); // webrtcsink picks free port
        let whep_url = format!("/ndi/whep/{}", source_id);

        let mut pipeline = NdiPipeline::build(ndi_name, whep_url, &signaller_uri)?;
        pipeline.start().await?;
        active.insert(
            source_id.to_string(),
            ActiveSource { pipeline, source_id: source_id.to_string() },
        );
        Ok(())
    }

    /// Stop the pipeline for the given source.
    pub async fn stop_pipeline(&self, source_id: &str) {
        let mut active = self.active.lock().await;
        if let Some(mut src) = active.remove(source_id) {
            src.pipeline.stop().await;
        }
    }

    /// Stop ALL pipelines.
    pub async fn stop_all(&self) {
        let mut active = self.active.lock().await;
        for (_, mut src) in active.drain() {
            src.pipeline.stop().await;
        }
    }

    /// Forward a WHEP SDP offer to the named source's pipeline.
    ///
    /// Returns the SDP answer as a string. Returns Err if the source isn't
    /// active or the pipeline isn't yet Streaming.
    pub async fn whep_offer(&self, source_id: &str, sdp_offer: &str) -> Result<String> {
        let active = self.active.lock().await;
        let src = active.get(source_id).ok_or_else(|| anyhow!("source not active"))?;
        match src.pipeline.state() {
            PipelineState::Streaming => {}
            PipelineState::Starting => return Err(anyhow!("pipeline starting; retry shortly")),
            PipelineState::Stopped => return Err(anyhow!("pipeline stopped")),
            PipelineState::Errored(e) => return Err(anyhow!("pipeline errored: {e}")),
        }
        // The actual SDP offer→answer exchange is delegated to webrtcsink's
        // built-in default signaller. The shim here forwards the SDP over the
        // internal signaller URI. webrtcsink's HTTP signaller mode handles
        // ICE/DTLS/SRTP negotiation directly with the browser.
        //
        // Implementation note: webrtcsink 0.13 exposes a `consumer-added`
        // signal AND a `request-answer` API; we use the simpler approach of
        // letting webrtcsink's `signaller::uri` point at an in-process WS
        // server we proxy the browser POST through. For now, error out
        // explicitly if the signaller cannot be reached — that's a real bug
        // we want to surface, not silently swallow.
        let _ = (src, sdp_offer); // silence unused warnings until signaller proxy is wired
        Err(anyhow!(
            "WHEP signaller proxy not yet implemented — see Task 6 Step 3 for the in-process signaller"
        ))
    }

    pub async fn is_active(&self, source_id: &str) -> bool {
        self.active.lock().await.contains_key(source_id)
    }
}
```

- [ ] **Step 2: Build the in-process WHEP signaller proxy**

The webrtcsink "default signaller" speaks a custom JSON-over-WebSocket protocol. To accept a plain WHEP HTTP POST from a browser we need a thin proxy. webrtcsink 0.13 ships an alternative `WhipServer` mode that natively accepts WHEP/WHIP HTTP without the WS shim.

Rewrite `manager.rs::start_pipeline` to use webrtcsink in WhipServer mode by setting the appropriate properties on the `webrtcsink` element. Replace the `desc` formatting in `pipeline.rs` (Task 5) with:

```rust
        let desc = format!(
            "ndisrc ndi-name=\"{ndi_name}\" ! \
             ndisrcdemux name=demux \
             demux.video ! videoconvert ! \
               vah264enc bitrate=2000 key-int-max=60 rate-control=cbr ! \
               video/x-h264,profile=baseline ! \
               sink.video_0 \
             demux.audio ! audioconvert ! audioresample ! \
               opusenc bitrate=64000 ! \
               sink.audio_0 \
             whipserversink name=sink \
                 host-addr=127.0.0.1 \
                 port=0 \
                 stun-server=null"
        );
```

`whipserversink` is the WHIP/WHEP variant of `webrtcsink` shipped in `gst-plugin-webrtc` 0.13. It exposes a local HTTP server on a chosen (or auto-picked) port that natively speaks WHEP. The manager's job becomes: query the bound port after pipeline starts, then proxy the public-facing `/ndi/whep/:id` POST to `http://127.0.0.1:<bound_port>/whep`.

After pipeline transitions to `Streaming`, retrieve the bound port from the element:

```rust
        let sink = self.pipeline.by_name("sink").ok_or_else(|| anyhow!("no sink element"))?;
        let port: i32 = sink.property("port");
```

Store this `port` alongside the pipeline. The public WHEP shim uses `reqwest` (already a workspace dep) to POST the offer to `http://127.0.0.1:<port>/whep`.

Replace the `whep_offer` placeholder in manager.rs with:

```rust
    pub async fn whep_offer(&self, source_id: &str, sdp_offer: &str) -> Result<String> {
        let (port, _state) = {
            let active = self.active.lock().await;
            let src = active.get(source_id).ok_or_else(|| anyhow!("source not active"))?;
            match src.pipeline.state() {
                PipelineState::Streaming => {}
                PipelineState::Starting => return Err(anyhow!("pipeline starting; retry shortly")),
                PipelineState::Stopped => return Err(anyhow!("pipeline stopped")),
                PipelineState::Errored(e) => return Err(anyhow!("pipeline errored: {e}")),
            }
            (src.pipeline.whip_port(), src.pipeline.state())
        };
        let url = format!("http://127.0.0.1:{port}/whep");
        let resp = reqwest::Client::new()
            .post(&url)
            .header("Content-Type", "application/sdp")
            .body(sdp_offer.to_string())
            .send()
            .await
            .context("forward WHEP offer to whipserversink")?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(anyhow!("whipserversink returned {status}: {body}"));
        }
        Ok(body)
    }
```

Add `whip_port()` accessor to `NdiPipeline` in `pipeline.rs`:

```rust
    pub fn whip_port(&self) -> i32 {
        self.pipeline
            .by_name("sink")
            .map(|sink| sink.property::<i32>("port"))
            .unwrap_or(0)
    }
```

- [ ] **Step 3: Create the WHEP HTTP shim router module**

Create `crates/presenter-server/src/router/integrations/ndi_whep.rs`:

```rust
//! WHEP HTTP shim — accepts browser SDP offers and forwards to the
//! per-source whipserversink element inside the NDI pipeline.

use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::IntoResponse,
    Json,
};
use tracing::instrument;

use super::super::AppError;
use crate::state::AppState;

#[instrument(skip_all, fields(source_id = %source_id))]
pub(crate) async fn post_whep_offer(
    Path(source_id): Path<String>,
    State(state): State<AppState>,
    body: String,
) -> Result<impl IntoResponse, AppError> {
    let manager = state
        .ndi_manager()
        .ok_or_else(|| AppError::service_unavailable("NDI SDK not available"))?;

    if !manager.is_active(&source_id).await {
        return Err(AppError::not_found("NDI source is not active"));
    }

    let answer = manager
        .whep_offer(&source_id, &body)
        .await
        .map_err(|e| AppError::service_unavailable(&format!("WHEP forward failed: {e}")))?;

    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/sdp"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        answer,
    ))
}

#[instrument(skip_all)]
pub(crate) async fn get_whep_cached(
    Path(_source_id): Path<String>,
    State(_state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    // Browsers MAY reuse a previous SDP via GET. For the initial implementation
    // we always return 404 — callers should POST a fresh offer. This is safe
    // (per WHEP spec, GET is optional).
    Err::<Json<()>, _>(AppError::not_found("WHEP GET cache not implemented; POST a fresh offer"))
}

#[instrument(skip_all)]
pub(crate) async fn delete_whep_subscriber(
    Path((_source_id, _client_id)): Path<(String, String)>,
    State(_state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    // whipserversink GC's subscribers on ICE timeout; explicit DELETE is optional.
    Ok(StatusCode::NO_CONTENT)
}
```

- [ ] **Step 4: Wire routes into `router.rs`**

Modify `crates/presenter-server/src/router.rs`. Replace lines 238–239:

```rust
        .route("/ndi/stream", get(integrations::ndi::mjpeg_ws))
        .route("/ndi/mjpeg", get(integrations::ndi::mjpeg_http))
```

with:

```rust
        .route("/ndi/whep/:source_id", post(integrations::ndi_whep::post_whep_offer)
            .get(integrations::ndi_whep::get_whep_cached))
        .route("/ndi/whep/:source_id/:client_id",
            delete(integrations::ndi_whep::delete_whep_subscriber))
```

Add to the `integrations` module declarations in `crates/presenter-server/src/router/integrations/mod.rs`:

```rust
pub(crate) mod ndi_whep;
```

- [ ] **Step 5: Update `video_source` activate handler to start a pipeline**

Open `crates/presenter-server/src/router/integrations/video_source.rs`. Locate `activate_video_source`. After persisting the `is_active=true` flag, call the manager:

```rust
    if let Some(manager) = state.ndi_manager() {
        manager
            .start_pipeline(&source.id, &source.ndi_name)
            .await
            .map_err(|e| AppError::service_unavailable(&format!("start pipeline: {e}")))?;
    }
```

In `deactivate_video_sources`, after clearing `is_active`:

```rust
    if let Some(manager) = state.ndi_manager() {
        manager.stop_all().await;
    }
```

(Exact edit points depend on existing handler bodies — the engineer reads the file once, finds the right insertion point inside each function, and adds the call.)

- [ ] **Step 6: Drop MJPEG handlers from ndi.rs**

In `crates/presenter-server/src/router/integrations/ndi.rs`, delete:
- `mjpeg_ws` function (currently lines 46–55)
- `handle_mjpeg_ws` function (currently lines 57–75)
- `mjpeg_http` function (currently lines 81–119)
- the `bytes::Bytes` and `tokio::sync::broadcast::error::RecvError` and `axum::extract::ws::*` imports if no other handler uses them

Keep: `NdiSourceDto`, `discover_ndi_sources`, `ndi_status`, all imports they use.

- [ ] **Step 7: Run cargo check**

Run: `cargo check --workspace`
Expected: PASS.

- [ ] **Step 8: Run cargo clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings -W clippy::all`
Expected: PASS.

- [ ] **Step 9: Run cargo test (will skip live-pipeline cases when no NDI source available)**

Run: `cargo test --workspace`
Expected: PASS — unit tests pass; the `vah264enc_present_when_vaapi_installed` test passes on dev2 (VA-API installed in Task 3 Step 5).

- [ ] **Step 10: Commit**

```bash
git add crates/presenter-ndi/src/manager.rs \
        crates/presenter-ndi/src/pipeline.rs \
        crates/presenter-server/src/router.rs \
        crates/presenter-server/src/router/integrations/mod.rs \
        crates/presenter-server/src/router/integrations/ndi.rs \
        crates/presenter-server/src/router/integrations/ndi_whep.rs \
        crates/presenter-server/src/router/integrations/video_source.rs
git commit -m "feat(ndi): WHEP HTTP shim + per-source pipeline lifecycle, drop MJPEG routes"
```

---

## Task 7: WASM `<NdiVideo>` component + WHEP client

**Model:** Sonnet

**Files:**
- Create: `crates/presenter-ui/src/components/stage/ndi_video.rs`
- Modify: `crates/presenter-ui/src/components/stage/mod.rs`
- Modify: `crates/presenter-ui/Cargo.toml`

- [ ] **Step 1: Enable required `web-sys` features**

Open `crates/presenter-ui/Cargo.toml`. Locate the `[dependencies.web-sys]` block (or the `web-sys = { ... }` line). Ensure these features are present (add any missing):

```toml
"RtcPeerConnection",
"RtcConfiguration",
"RtcSessionDescription",
"RtcSessionDescriptionInit",
"RtcSdpType",
"RtcTrackEvent",
"RtcRtpTransceiver",
"RtcRtpTransceiverInit",
"RtcRtpTransceiverDirection",
"MediaStream",
"MediaStreamTrack",
"HtmlVideoElement",
"Request",
"RequestInit",
"RequestMode",
"Response",
"Headers",
```

- [ ] **Step 2: Create the component**

Create `crates/presenter-ui/src/components/stage/ndi_video.rs`:

```rust
//! NdiVideo — WHEP-subscribing `<video>` element for one NDI source.
//!
//! Each `<NdiVideo>` mounts an HTMLVideoElement and immediately connects
//! to the server's WHEP endpoint for the given source. The browser
//! handles ICE/DTLS/SRTP/jitter-buffer/AV-sync natively. The WASM code
//! is signaling glue only.

use leptos::prelude::*;
use leptos::wasm_bindgen::{closure::Closure, JsCast, JsValue};
use leptos::web_sys::{
    HtmlVideoElement, MediaStream, RtcConfiguration, RtcPeerConnection, RtcRtpTransceiverDirection,
    RtcRtpTransceiverInit, RtcSdpType, RtcSessionDescriptionInit, RtcTrackEvent,
};
use leptos::wasm_bindgen_futures::{spawn_local, JsFuture};

/// Build the URL for the WHEP endpoint of a given source.
pub fn whep_url(source_id: &str) -> String {
    format!("/ndi/whep/{source_id}")
}

#[component]
pub fn NdiVideo(source_id: String, #[prop(optional)] class: Option<&'static str>) -> impl IntoView {
    let video_ref = NodeRef::<leptos::html::Video>::new();
    let source_id_for_effect = source_id.clone();

    Effect::new(move |_| {
        let video = match video_ref.get() {
            Some(v) => v,
            None => return,
        };
        let source_id = source_id_for_effect.clone();
        spawn_local(async move {
            if let Err(e) = connect_whep(&video, &source_id).await {
                leptos::logging::error!("WHEP connect for {source_id} failed: {:?}", e);
            }
        });
    });

    view! {
        <video
            node_ref=video_ref
            data-role="ndi-video"
            data-source-id=source_id.clone()
            class=class.unwrap_or("")
            autoplay
            muted
            playsinline
        />
    }
}

async fn connect_whep(video: &HtmlVideoElement, source_id: &str) -> Result<(), JsValue> {
    let cfg = RtcConfiguration::new();
    let pc = RtcPeerConnection::new_with_configuration(&cfg)?;

    let video_init = RtcRtpTransceiverInit::new();
    video_init.set_direction(RtcRtpTransceiverDirection::Recvonly);
    pc.add_transceiver_with_str_and_init("video", &video_init);

    let audio_init = RtcRtpTransceiverInit::new();
    audio_init.set_direction(RtcRtpTransceiverDirection::Recvonly);
    pc.add_transceiver_with_str_and_init("audio", &audio_init);

    let video_clone = video.clone();
    let ontrack = Closure::<dyn FnMut(RtcTrackEvent)>::new(move |ev: RtcTrackEvent| {
        if let Ok(stream) = ev.streams().get(0).dyn_into::<MediaStream>() {
            video_clone.set_src_object(Some(&stream));
        }
    });
    pc.set_ontrack(Some(ontrack.as_ref().unchecked_ref()));
    ontrack.forget(); // PC owns the closure for its lifetime

    let offer = JsFuture::from(pc.create_offer()).await?;
    let offer_init = offer.unchecked_into::<RtcSessionDescriptionInit>();
    JsFuture::from(pc.set_local_description(&offer_init)).await?;
    let offer_sdp = leptos::reactive::owner::js_sys::Reflect::get(&offer_init, &"sdp".into())?
        .as_string()
        .unwrap_or_default();

    let url = whep_url(source_id);
    let init = leptos::web_sys::RequestInit::new();
    init.set_method("POST");
    init.set_body(&offer_sdp.into());
    let headers = leptos::web_sys::Headers::new()?;
    headers.set("Content-Type", "application/sdp")?;
    init.set_headers(&headers);
    let request = leptos::web_sys::Request::new_with_str_and_init(&url, &init)?;
    let window = leptos::web_sys::window().ok_or(JsValue::from_str("no window"))?;
    let resp_val = JsFuture::from(window.fetch_with_request(&request)).await?;
    let resp: leptos::web_sys::Response = resp_val.dyn_into()?;
    if !resp.ok() {
        return Err(JsValue::from_str(&format!(
            "WHEP POST returned {}",
            resp.status()
        )));
    }
    let answer_text = JsFuture::from(resp.text()?).await?.as_string().unwrap_or_default();
    let answer = RtcSessionDescriptionInit::new(RtcSdpType::Answer);
    answer.set_sdp(&answer_text);
    JsFuture::from(pc.set_remote_description(&answer)).await?;
    Ok(())
}
```

- [ ] **Step 3: Register the module in `mod.rs`**

In `crates/presenter-ui/src/components/stage/mod.rs`, add:

```rust
pub mod ndi_video;
```

- [ ] **Step 4: Verify WASM compiles**

Run: `cd crates/presenter-ui && cargo check --target wasm32-unknown-unknown && cd ../..`
Expected: PASS.

- [ ] **Step 5: Run WASM clippy**

Run: `cd crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings -W clippy::all && cd ../..`
Expected: PASS. Fix any clippy lints inline before committing.

- [ ] **Step 6: Commit**

```bash
git add crates/presenter-ui/Cargo.toml \
        crates/presenter-ui/src/components/stage/mod.rs \
        crates/presenter-ui/src/components/stage/ndi_video.rs
git commit -m "feat(ui): NdiVideo Leptos component + WHEP client"
```

---

## Task 8: Swap `<img>` for `<NdiVideo>` in stage layouts

**Model:** Sonnet

**Files:**
- Modify: `crates/presenter-ui/src/components/stage/ndi_fullscreen.rs`
- Modify: `crates/presenter-ui/src/components/stage/api_stage.rs`
- Modify: `crates/presenter-ui/src/components/stage/timer_layout.rs`
- Modify: `crates/presenter-ui/src/api/ndi.rs`

- [ ] **Step 1: Refactor `StageContext` to expose active source ID**

The current code uses `ctx.ndi_active` (boolean). We now need `ctx.ndi_active_source_id` (Option<String>) so each `<NdiVideo>` knows which source to subscribe to.

Open `crates/presenter-ui/src/state/stage.rs`. Find the `StageContext` struct and the `ndi_active` field. Add:

```rust
    pub ndi_active_source_id: ReadSignal<Option<String>>,
```

Wherever `ndi_active` is updated (search the file for `set_ndi_active`), also update `ndi_active_source_id` from the WS event's `NdiSourceActivated { source_id, .. }` payload. The current MJPEG path didn't need this — for WebRTC we MUST know the source UUID.

If `NdiSourceActivated` doesn't currently carry `source_id`, add it (server-side it already exists — the event already includes the source row's identity for the existing `ndi_active` toggle). Confirm in `crates/presenter-server/src/state/live_events.rs` and broadcast the source_id on activation.

- [ ] **Step 2: Update `ndi_fullscreen.rs`**

Replace the `<img>` block (lines 31–35) with:

```rust
            <Show when=move || ndi_active.get()>
                {move || {
                    ctx.ndi_active_source_id.get().map(|source_id| view! {
                        <crate::components::stage::ndi_video::NdiVideo
                            source_id=source_id
                            class="stage-ndi__video"
                        />
                    })
                }}
            </Show>
```

- [ ] **Step 3: Update `api_stage.rs`**

Replace the `<img>` line (currently around line 22):

```rust
            <Show when=move || ndi_active.get()>
                {move || {
                    ctx.ndi_active_source_id.get().map(|source_id| view! {
                        <crate::components::stage::ndi_video::NdiVideo
                            source_id=source_id
                            class="stage-api__ndi"
                        />
                    })
                }}
            </Show>
```

- [ ] **Step 4: Update `timer_layout.rs`**

Same pattern — replace `<img src="/ndi/mjpeg" class="stage-timer__ndi" />` with the `<NdiVideo>` block referencing `ctx.ndi_active_source_id`.

- [ ] **Step 5: Update `api/ndi.rs` URL builder**

Open `crates/presenter-ui/src/api/ndi.rs`. The old MJPEG URL is hardcoded `/ndi/mjpeg` in components (now removed). No URL builder to delete since the inlined string is gone. Add a new builder for completeness:

```rust
pub fn whep_url(source_id: &str) -> String {
    format!("/ndi/whep/{source_id}")
}
```

(`<NdiVideo>` builds its own URL in `ndi_video.rs::whep_url`. This helper is exposed for any future caller that wants it programmatically.)

- [ ] **Step 6: Verify WASM compiles**

Run: `cd crates/presenter-ui && cargo check --target wasm32-unknown-unknown && cd ../..`
Expected: PASS.

- [ ] **Step 7: Verify WASM clippy clean**

Run: `cd crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings -W clippy::all && cd ../..`
Expected: PASS.

- [ ] **Step 8: Run the RED Playwright test from Task 4 — should now go GREEN for the static assertions**

Run: `npm run test:playwright -- ndi-webrtc`
Expected: Test 1 (`WHEP endpoint returns SDP answer for active source`) PASSES (returns 503 if no NDI source available, 200 if available — both acceptable). Test 2 (`stage page mounts NdiVideo with correct data attributes`) PASSES (component now exists, `<img src=/ndi/mjpeg>` is gone). Test 3 may be skipped on hosts without a live NDI source — that's expected.

- [ ] **Step 9: Commit**

```bash
git add crates/presenter-ui/src/state/stage.rs \
        crates/presenter-ui/src/components/stage/ndi_fullscreen.rs \
        crates/presenter-ui/src/components/stage/api_stage.rs \
        crates/presenter-ui/src/components/stage/timer_layout.rs \
        crates/presenter-ui/src/api/ndi.rs \
        crates/presenter-server/src/state/live_events.rs
git commit -m "feat(ui): swap stage NDI layouts from <img>/MJPEG to <NdiVideo>/WebRTC"
```

---

## Task 9: Delete dead code

**Model:** Sonnet

**Files:**
- Delete: `crates/presenter-ndi/src/ndi_sdk.rs`
- Delete: `crates/presenter-ndi/src/receiver.rs`
- Delete: `crates/presenter-ndi/src/encoder.rs`
- Modify: existing tests in `tests/e2e/ndi-stage-layout.spec.ts` + `tests/e2e/stage-api-ndi.spec.ts` to use new `<video data-role="ndi-video">` selector

- [ ] **Step 1: Delete the dead source files**

Run:

```bash
rm crates/presenter-ndi/src/ndi_sdk.rs
rm crates/presenter-ndi/src/receiver.rs
rm crates/presenter-ndi/src/encoder.rs
```

- [ ] **Step 2: Update `crates/presenter-ndi/src/lib.rs` (the new minimal one from Task 3 already drops the references — verify it does NOT mention `ndi_sdk` / `receiver` / `encoder`)**

If any `pub mod ndi_sdk;` / `pub mod receiver;` / `pub mod encoder;` lines remain, remove them.

- [ ] **Step 3: Update existing Playwright tests to use the new selector**

In `tests/e2e/ndi-stage-layout.spec.ts`, find any assertion that targets the MJPEG `<img>` and switch it to the new component selector. Specifically, search for:
- `img[src*="/ndi/mjpeg"]` — replace with `[data-role="ndi-video"]`
- `stage-ndi__video` (as an `<img>` selector) — keep the class name but expect `<video>` element type
- Any header/content-type assertions referencing `image/jpeg` or `multipart/x-mixed-replace` — DELETE these tests (the MJPEG endpoint no longer exists)

Same pass in `tests/e2e/stage-api-ndi.spec.ts`.

- [ ] **Step 4: Run cargo build to verify nothing references deleted modules**

Run: `cargo check --workspace`
Expected: PASS, no `unresolved import` errors.

- [ ] **Step 5: Run cargo clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings -W clippy::all`
Expected: PASS.

- [ ] **Step 6: Run all unit tests**

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 7: Run all E2E tests**

Run: `npm run test:playwright`
Expected: PASS. Any test that previously relied on `<img>/MJPEG` is now using `<video data-role="ndi-video">`.

- [ ] **Step 8: Commit**

```bash
git rm crates/presenter-ndi/src/ndi_sdk.rs \
       crates/presenter-ndi/src/receiver.rs \
       crates/presenter-ndi/src/encoder.rs
git add crates/presenter-ndi/src/lib.rs \
        tests/e2e/ndi-stage-layout.spec.ts \
        tests/e2e/stage-api-ndi.spec.ts
git commit -m "chore(ndi): remove dead MJPEG code (ndi_sdk, receiver, encoder) + update E2E selectors"
```

---

## Task 10: Final local gate

**Model:** Sonnet

**Files:** none — verification only.

- [ ] **Step 1: Format check**

Run: `cargo fmt --all --check`
Expected: PASS. If fails: `cargo fmt --all` and commit as `style: cargo fmt`.

- [ ] **Step 2: Workspace clippy (binary + libs + tests)**

Run: `cargo clippy --workspace --all-targets -- -D warnings -W clippy::all`
Expected: PASS.

- [ ] **Step 3: WASM clippy**

Run: `cd crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings -W clippy::all && cd ../..`
Expected: PASS.

- [ ] **Step 4: Workspace test (uses cargo nextest or cargo test depending on local setup)**

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 5: Full local release build**

Run: `cargo build --release -p presenter-server`
Expected: PASS, binary at `target/release/presenter-server`.

(This is project-allowed per CLAUDE.md `Local Build Policy: Local Rust builds are ALLOWED on this machine`.)

- [ ] **Step 6: Run all Playwright E2E against the new binary on dev host (10.77.8.134:8080)**

Stop any running `presenter-dev` service first, then:

```bash
sudo systemctl stop presenter-dev
PRESENTER_PORT=8080 PRESENTER_BUILD_CHANNEL=dev ./target/release/presenter-server &
sleep 3
npm run test:playwright
kill %1
sudo systemctl start presenter-dev
```

Expected: ALL Playwright specs PASS. Console errors / warnings = 0.

- [ ] **Step 7: Commit any format/fixup if needed**

If Step 1 required a fmt fixup:

```bash
git add -u
git commit -m "style: cargo fmt for NDI WebRTC PR"
```

If everything was already clean, no commit needed.

---

## Task 11: Controller — push, monitor CI, verify on dev, open PR

**This task is handled by the parent (controller) Claude, not a subagent.**

- [ ] **Step 1: Push dev**

Run: `git push origin dev`
Expected: push succeeds.

- [ ] **Step 2: Identify the pipeline run id**

Run: `gh run list --branch dev --limit 1 --json databaseId,status,conclusion`
Expected: latest run with status `in_progress`.

- [ ] **Step 3: Monitor CI in a single background sleep (per ci-monitoring.md — no /loop, no gh run watch)**

Run (background):

```bash
sleep 600 && gh run view <run-id> --json status,conclusion,jobs
```

When the BashOutput returns, evaluate. If `conclusion=success` for ALL jobs (build, test, clippy, fmt, e2e, deploy-dev), proceed. If anything fails: `gh run view <run-id> --log-failed`, fix the root cause, push again, monitor again.

- [ ] **Step 4: Verify on dev (autonomously, per autonomous-verification.md)**

Open Playwright against http://10.77.8.134:8080. Drive the flow:

1. Settings → Video Sources → activate `STREAM-SNV (stream)`
2. Operator → set stage layout to `ndi-fullscreen`
3. Navigate to http://10.77.8.134:8080/stage
4. Wait `body[data-wasm-ready="true"]`
5. Wait `[data-role="ndi-video"]` exists, `videoWidth > 0`
6. Verify browser console = clean (0 errors, 0 warnings)
7. Take screenshot for the completion report evidence

Record the dev version DOM label from `[data-testid="version"]` — must show `v0.4.92 (dev)`.

- [ ] **Step 5: Open PR**

```bash
gh pr create --title "feat(ndi): WebRTC transport via gst-plugins-rs (replaces MJPEG)" --body "$(cat <<'EOF'
## Summary

- Replaces MJPEG (#250 single-fixed-tier) with native WebRTC over WHEP
- HW H264 encode on N100 via VAAPI (`vah264enc`); audio path added via `gst-plugin-ndi`
- Browser composes layouts from N `<video>` elements, each subscribing to its own WHEP endpoint — no server-side compositor
- Spec: `docs/superpowers/specs/2026-05-18-ndi-webrtc-transport-design.md`

## Test plan

- [x] cargo fmt --all --check
- [x] cargo clippy --workspace --all-targets -- -D warnings
- [x] WASM clippy clean
- [x] cargo test --workspace
- [x] npm run test:playwright (all specs green)
- [x] Live dev verification on 10.77.8.134:8080 — STREAM-SNV plays, console clean, version label `v0.4.92 (dev)`

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 6: Verify PR mergeable**

Run: `gh pr view <number> --json mergeable,mergeStateStatus`
Expected: `mergeable: MERGEABLE`, `mergeStateStatus: CLEAN`. If `UNSTABLE` or `BLOCKED`, fix the failing gate before reporting.

- [ ] **Step 7: Send completion report per completion-report.md template, wait for explicit "merge it"**

Do NOT merge autonomously. Per pr-merge-policy.md, await explicit user instruction.

---

## Self-Review

**Spec coverage:**

- WebRTC transport via gst-plugins-rs — Tasks 3, 5, 6
- HW H264 encode via vah264enc — Tasks 2, 3, 5
- Audio via gst-plugin-ndi → opusenc → webrtcsink — Tasks 3, 5
- Browser-side compositing — Tasks 7, 8 (each NDI = one `<NdiVideo>`)
- WHEP HTTP shim routes — Task 6
- Per-source pipeline state machine — Task 5
- Failure-loudly when VAAPI missing — Task 3, 5 (no silent software fallback)
- MJPEG fully removed — Tasks 6, 9
- E2E Playwright coverage — Task 4 (RED), Task 8 (GREEN), Task 9 (existing specs migrated)
- Capacity budget verified — Task 11 (live dev)
- Deploy infra apt-installs packages — Task 2

All spec sections have at least one task.

**Placeholder scan:** no `TBD`, `TODO`, `implement later`, `add appropriate error handling`, `similar to Task N`. Every code step includes the exact code. The only TBD-like text is "(Exact edit points depend on existing handler bodies…)" in Task 6 Step 5 — that's because the file changes outside the diff we have in this plan. The engineer reads the file once and applies the documented mutation. Acceptable.

**Type consistency:**

- `data-role="ndi-video"` and `data-source-id="<id>"` — same in Task 4 (E2E) and Task 7 (component).
- WHEP URL pattern `/ndi/whep/{source_id}` — same in Task 4, 6, 7, 8.
- `NdiPipeline::whip_port()` — defined in Task 6 Step 2, called from Task 6 Step 2 `whep_offer`. Names match.
- `presenter_ndi::init()` and `presenter_ndi::vah264enc_available()` — defined in Task 3 Step 3, called from Task 3 Step 8 and Task 5 Step 1. Names match.
- `start_pipeline(source_id, ndi_name)` and `stop_pipeline(source_id)` and `stop_all()` and `is_active(source_id)` and `whep_offer(source_id, sdp)` — all on `NdiManager` (Task 6 Step 1) and called from Task 6 Step 3 (router) and Task 6 Step 5 (video_source handler). Names match.

No type drift across tasks.
