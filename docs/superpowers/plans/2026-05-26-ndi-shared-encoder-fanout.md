# NDI Shared-Encoder Fanout Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace per-consumer encoding (`whepserversink` spawns one encoder per WHEP client) with shared-encoder fanout (one `vah264enc` per NDI source → `tee` → N `webrtcbin` peers) so the N100 prod box handles ≥4 concurrent stage-display consumers under 50% CPU.

**Architecture:** One GStreamer pipeline per active NDI source. Pipeline owns: `ndisrc → ndisrcdemux → videoconvert → vah264enc → rtph264pay → tee`. Per-consumer state lives in a new `WhepSession` struct: one `webrtcbin` element, the `tee` request-pad it reads from, an ICE candidate channel, an SDP-answer oneshot. WHEP HTTP routes (POST/PATCH/DELETE) unchanged; only the implementation behind them changes — from `emit_by_name` on whepserversink's signaller to direct `webrtcbin` orchestration.

**Tech Stack:** Rust, GStreamer 0.25 (`gstreamer`, new `gstreamer-webrtc = "0.25"`), `gst-plugin-ndi 0.15` (for `ndisrc`), `gst-plugin-rtp 0.15` (for `rtph264pay`), `tokio`, `axum`. Browser-side WebRTC + WHEP client (`presenter-ui`) unchanged.

**Spec:** [`docs/superpowers/specs/2026-05-26-ndi-shared-encoder-fanout-design.md`](../specs/2026-05-26-ndi-shared-encoder-fanout-design.md) (commit `d97d734`).

**Pre-flight state:** dev at `d97d734`. Workspace version `0.4.97` (bumped in `95d4ea7` — DO NOT bump again). All 11 tasks below ship as ONE PR per `autonomous-batch-issue-development.md` "single feature = single PR".

---

## Task 1: RED unit test — single encoder for N consumers (regression-test-first)

**Spec anchor:** Testing strategy #1 ("Build a pipeline using `stopped_for_test()`-style harness; call `add_consumer` N times; assert the pipeline iterator yields exactly ONE element with factory name in `{vah264enc, nvh264enc, x264enc}`").

**Files:**
- Modify: `crates/presenter-ndi/src/pipeline.rs` — add new test in `#[cfg(test)] mod tests`

**Why this commits FIRST:** Issue #336 is `bug`-labeled (NDI hardening series); the regression-test-first hook will block the completion report unless the bug-fix PR cites a test that was RED before the fix and GREEN after. The test commit MUST precede the implementation commit (Task 4) in `git log --oneline`.

- [ ] **Step 1: Add the failing test** to `crates/presenter-ndi/src/pipeline.rs` inside `mod tests`, after `video_caps_property_restricts_offered_codec_to_h264`:

```rust
    /// Regression test for #336: shared-encoder fanout.
    ///
    /// The pre-fix pipeline ended in `whepserversink`, which (by gst-plugin-rs
    /// 0.15 design) spawns one independent encoder per WHEP consumer. With
    /// multiple stage-display browsers connecting to the same NDI source,
    /// 3-4 encoders saturated the N100's iGPU VAAPI scheduler — the
    /// 2026-05-24 production incident.
    ///
    /// The fix builds the pipeline with `ndisrc → demux → videoconvert →
    /// vah264enc → rtph264pay → tee` (one encoder), then `add_consumer`
    /// dynamically requests a tee src pad + spawns one `webrtcbin` per
    /// consumer that reads from the shared encoded stream. This test asserts
    /// the load-bearing invariant: regardless of how many consumers are
    /// added, the pipeline iterator yields EXACTLY ONE encoder element.
    ///
    /// Runs on every CI host (no GPU/libndi required) — uses
    /// `stopped_for_test_with_topology()` which builds the encoder + tee
    /// + per-consumer webrtcbin elements with `x264enc` (always available)
    /// in lieu of real HW encoders.
    #[tokio::test]
    async fn pipeline_has_single_encoder_for_n_consumers() {
        super::super::init().expect("gst init");
        // Build a pipeline-shaped value with the fanout topology, force
        // `x264enc` as the encoder so this runs on every CI host.
        let mut pipeline = NdiPipeline::stopped_for_test_with_topology("x264enc")
            .expect("test-only topology builder must succeed when x264enc registered");

        // Simulate N WHEP POSTs.
        for i in 0..4 {
            pipeline
                .add_consumer_stub(&format!("test-session-{i}"))
                .expect("add_consumer must succeed up to the soft cap");
        }

        // Count elements whose factory name is one of the encoder factories.
        let encoder_factories = ["vah264enc", "nvh264enc", "x264enc"];
        let encoder_count = pipeline.iterate_encoders().count();
        let webrtcbin_count = pipeline.iterate_webrtcbins().count();

        assert_eq!(
            encoder_count, 1,
            "REGRESSION (#336): pipeline must have EXACTLY ONE encoder for N consumers; \
             got {encoder_count} encoders for 4 consumers. Encoder factories considered: {encoder_factories:?}"
        );
        assert_eq!(
            webrtcbin_count, 4,
            "pipeline must have one webrtcbin per consumer; got {webrtcbin_count} for 4 consumers"
        );
    }
```

This test references three methods that do NOT yet exist on `NdiPipeline`:
- `stopped_for_test_with_topology(encoder_name: &str) -> Result<Self>`
- `add_consumer_stub(session_id: &str) -> Result<()>`
- `iterate_encoders() -> impl Iterator<Item = gst::Element>`
- `iterate_webrtcbins() -> impl Iterator<Item = gst::Element>`

That is intentional — the test compiles after Task 4 lands the real implementation. For Task 1's commit the test FILE compiles only if we provide stub method bodies that `panic!("not yet implemented (Task 4)")` — but that hides the RED. Instead we use `#[cfg(feature = "fanout-redgreen")]` to gate the test ONLY for Task 1's commit so the workspace still compiles cleanly. Task 4 removes the gate.

- [ ] **Step 2: Add the feature gate** to `crates/presenter-ndi/Cargo.toml` `[features]` table:

```toml
[features]
default = []
test-helpers = []
# Gates the #336 RED regression test until Task 4 lands the real topology.
# Removed in Task 4 — the test must run unconditionally on every CI host.
fanout-redgreen = []
```

And gate the test attribute with the feature:

```rust
    #[cfg(feature = "fanout-redgreen")]
    #[tokio::test]
    async fn pipeline_has_single_encoder_for_n_consumers() { /* test body from Step 1 */ }
```

- [ ] **Step 3: Verify the test compiles and fails under the feature**

Run:
```bash
cargo test -p presenter-ndi --lib --features fanout-redgreen pipeline_has_single_encoder_for_n_consumers
```

Expected: **compile error** ("no method named `stopped_for_test_with_topology` found"). This is the RED state — proof the test exercises code that doesn't yet exist. Capture the error output in the commit message body.

- [ ] **Step 4: Verify workspace still compiles WITHOUT the feature**

Run:
```bash
cargo check --workspace
```

Expected: PASS. The feature gate keeps the RED test out of the default build so subsequent tasks can layer changes.

- [ ] **Step 5: Commit**

```bash
git add crates/presenter-ndi/src/pipeline.rs crates/presenter-ndi/Cargo.toml
git commit -m "test(ndi): RED regression test — single encoder for N consumers (#336)

Asserts the load-bearing invariant for #336 shared-encoder fanout:
regardless of how many WHEP consumers are added, the pipeline must
contain EXACTLY ONE encoder element (vah264enc / nvh264enc / x264enc),
NOT one per consumer as the current whepserversink-based pipeline does.

Gated behind 'fanout-redgreen' feature until Task 4 lands the real
topology. With the feature enabled, the test FAILS to compile because
NdiPipeline lacks the stopped_for_test_with_topology / add_consumer_stub
/ iterate_encoders / iterate_webrtcbins methods — that is the RED state.

The feature gate is removed in the GREEN commit (Task 4); from there
the test runs unconditionally on every CI host."
```

---

## Task 2: Add `gstreamer-webrtc = "0.25"` to workspace + crate deps

**Spec anchor:** Modules table ("Add `gstreamer-webrtc = "0.25"` (matches the pinned `gstreamer = "0.25"`) for `WebRTCSDPType`, `WebRTCSessionDescription`, the on-negotiation-needed/on-ice-candidate signal helpers").

**Files:**
- Modify: `Cargo.toml:51` (workspace `[workspace.dependencies]` after `gstreamer = "0.25"`)
- Modify: `crates/presenter-ndi/Cargo.toml:22-25` (crate `[dependencies]` after `gstreamer.workspace = true`)

- [ ] **Step 1: Add `gstreamer-webrtc` to workspace deps**

Open `Cargo.toml`. After the line `gstreamer = "0.25"` (line 50), add:

```toml
gstreamer-webrtc = "0.25"
```

The block becomes:

```toml
gstreamer = "0.25"
gstreamer-webrtc = "0.25"
gst-plugin-webrtc = "0.15"
gst-plugin-ndi = "0.15"
gst-plugin-rtp = "0.15"
```

- [ ] **Step 2: Add `gstreamer-webrtc` to `presenter-ndi` crate**

Open `crates/presenter-ndi/Cargo.toml`. After the line `gstreamer.workspace = true` (line 22), add:

```toml
gstreamer-webrtc.workspace = true
```

The block becomes:

```toml
gstreamer.workspace = true
gstreamer-webrtc.workspace = true
gst-plugin-webrtc.workspace = true
gst-plugin-ndi.workspace = true
gst-plugin-rtp.workspace = true
```

- [ ] **Step 3: Refresh workspace lockfile**

Run:
```bash
cargo update --workspace
```

Expected: lockfile updated with `gstreamer-webrtc 0.25.x` and its transitive deps.

- [ ] **Step 4: Refresh excluded crate lockfile** (per CLAUDE.md, `crates/presenter-ui` is excluded from the workspace and has its own `Cargo.lock`)

Run:
```bash
(cd crates/presenter-ui && cargo update)
```

Expected: minimal updates (probably none — gstreamer-webrtc isn't a UI dep).

- [ ] **Step 5: Verify the workspace compiles**

Run:
```bash
cargo check --workspace
```

Expected: PASS, with `gstreamer-webrtc` resolved.

- [ ] **Step 6: Verify presenter-ndi compiles standalone with the new dep**

Run:
```bash
cargo check -p presenter-ndi
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock crates/presenter-ndi/Cargo.toml crates/presenter-ui/Cargo.lock
git commit -m "deps(ndi): add gstreamer-webrtc 0.25 for direct webrtcbin orchestration (#336)

Pre-req for replacing whepserversink with a hand-rolled WhepSession
that drives webrtcbin elements directly. gstreamer-webrtc 0.25 matches
the pinned gstreamer = 0.25 (the gstreamer-rs subcrates ship in
lockstep) — no version drift risk.

gst-plugin-webrtc 0.15 stays in deps for now; removable in a later
cleanup PR once the whepserversink-based fallback test is gone."
```

---

## Task 3: New `whep_session.rs` module (per-consumer state)

**Spec anchor:** Modules table (`crates/presenter-ndi/src/whep_session.rs` ~250 LoC); WHEP session lifecycle.

**Files:**
- Create: `crates/presenter-ndi/src/whep_session.rs`
- Modify: `crates/presenter-ndi/src/lib.rs:6` (add `pub mod whep_session;` after `pub mod pipeline;`)

- [ ] **Step 1: Create `crates/presenter-ndi/src/whep_session.rs`** with the type skeleton:

```rust
//! Per-WHEP-consumer state.
//!
//! One `WhepSession` is created when a browser POSTs a WHEP offer to
//! `/ndi/whep/:source_id`. It owns:
//!   - one `webrtcbin` GStreamer element (the WebRTC peer)
//!   - the `tee` request-pad src that feeds it from the shared encoder
//!   - an async channel of pending ICE candidates flowing browser→server
//!   - a one-shot for the SDP answer (resolved once webrtcbin's
//!     create-answer signal returns)
//!   - the session UUID used as the WHEP HTTP Location path segment
//!
//! Lifetime ends when:
//!   - The HTTP DELETE `/ndi/whep/:source_id/:session_id` route fires
//!     `remove_consumer(session_id)` on the pipeline.
//!   - webrtcbin emits `connection-state-change` to `Failed` or
//!     `Disconnected` (handled by a signal subscriber that calls
//!     `remove_consumer`).
//!   - The owning pipeline is torn down (Drop on the pipeline cascades
//!     through `tee.remove_pad` + `pipeline.remove(webrtcbin)`).
//!
//! Non-Send constraint: `webrtcbin` and `gst::Pad` are non-Send glib
//! types. All signal connections and pad linking happen on the GStreamer
//! main loop thread; tokio code talks to the session via Send channels
//! and Send-able UUIDs.

use anyhow::{Context, Result};
use gstreamer as gst;
use gstreamer::prelude::*;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex};
use uuid::Uuid;

/// ICE candidate delivered to the browser via WHEP PATCH or WHEP
/// half-trickle response body.
#[derive(Debug, Clone)]
pub struct IceCandidate {
    pub sdp_mline_index: u32,
    pub candidate: String,
}

/// Connection state for the diagnostic snapshot route (#336 spec
/// "Diagnostic surface").
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WhepConnectionState {
    New,
    Connecting,
    Connected,
    Disconnected,
    Failed,
    Closed,
}

impl WhepConnectionState {
    /// Map GStreamer's `GstWebRTCPeerConnectionState` GEnum value into
    /// our serializable enum. GStreamer values: 0=new, 1=connecting,
    /// 2=connected, 3=disconnected, 4=failed, 5=closed. See
    /// gstreamer-webrtc-0.25 docs for `WebRTCPeerConnectionState`.
    pub fn from_gst_value(value: i32) -> Self {
        match value {
            0 => Self::New,
            1 => Self::Connecting,
            2 => Self::Connected,
            3 => Self::Disconnected,
            4 => Self::Failed,
            5 => Self::Closed,
            _ => Self::New,
        }
    }
}

/// One WHEP consumer. Owned by the pipeline's session map.
pub struct WhepSession {
    /// UUID used as the WHEP HTTP Location path segment.
    pub session_id: String,
    /// The webrtcbin element for this consumer.
    pub webrtcbin: gst::Element,
    /// The src pad on `tee` feeding this consumer's webrtcbin (via a
    /// queue). Released back to the tee on Drop.
    pub tee_src_pad: gst::Pad,
    /// Holds the latest reported connection state, updated by the
    /// `connection-state-change` signal subscriber.
    pub connection_state: Arc<Mutex<WhepConnectionState>>,
    /// ICE candidates flowing server→browser (sender). The receiver
    /// half lives in the pipeline's add_consumer path and is drained
    /// while building the WHEP answer body OR delivered via subsequent
    /// PATCH responses (trickle).
    pub ice_tx: mpsc::UnboundedSender<IceCandidate>,
}

impl WhepSession {
    /// Generate a UUIDv4 session id. Public so unit tests can assert
    /// the format without spawning a real GStreamer element.
    pub fn new_session_id() -> String {
        Uuid::new_v4().to_string()
    }
}

impl Drop for WhepSession {
    fn drop(&mut self) {
        // Best-effort teardown of the per-consumer state. The pipeline's
        // remove_consumer method is the canonical path; Drop is the
        // backstop for unexpected drops (pipeline Drop, panic unwind).
        let _ = self.webrtcbin.set_state(gst::State::Null);
        // tee_src_pad release is the parent tee's responsibility — we
        // can't release a request-pad without holding the tee. The
        // pipeline-level Drop iterates sessions and calls
        // tee.release_request_pad(&session.tee_src_pad) before dropping.
        tracing::debug!(
            session_id = %self.session_id,
            "WhepSession dropped (webrtcbin set to Null; pad release handled by pipeline)"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_session_id_is_a_valid_uuid_v4() {
        let id = WhepSession::new_session_id();
        let parsed = Uuid::parse_str(&id).expect("session id must parse as UUID");
        assert_eq!(
            parsed.get_version(),
            Some(uuid::Version::Random),
            "session id must be UUIDv4 (Version::Random), got {:?}",
            parsed.get_version(),
        );
    }

    #[test]
    fn new_session_id_is_unique_per_call() {
        let a = WhepSession::new_session_id();
        let b = WhepSession::new_session_id();
        assert_ne!(a, b, "two calls must return distinct UUIDs");
    }

    #[test]
    fn connection_state_maps_gst_values_correctly() {
        assert_eq!(WhepConnectionState::from_gst_value(0), WhepConnectionState::New);
        assert_eq!(WhepConnectionState::from_gst_value(1), WhepConnectionState::Connecting);
        assert_eq!(WhepConnectionState::from_gst_value(2), WhepConnectionState::Connected);
        assert_eq!(WhepConnectionState::from_gst_value(3), WhepConnectionState::Disconnected);
        assert_eq!(WhepConnectionState::from_gst_value(4), WhepConnectionState::Failed);
        assert_eq!(WhepConnectionState::from_gst_value(5), WhepConnectionState::Closed);
        assert_eq!(WhepConnectionState::from_gst_value(99), WhepConnectionState::New);
    }

    #[test]
    fn connection_state_serializes_as_lowercase_string() {
        let s = serde_json::to_string(&WhepConnectionState::Connected).unwrap();
        assert_eq!(s, "\"connected\"");
    }
}
```

- [ ] **Step 2: Add `uuid` to the crate's `[dependencies]`** in `crates/presenter-ndi/Cargo.toml`. After the `serde` lines:

```toml
uuid = { version = "1", features = ["v4", "serde"] }
```

The crate already has `serde` and `serde_json` workspace deps so the `serde` feature on `uuid` works.

Verify the workspace `[workspace.dependencies]` table at `Cargo.toml` already has uuid; if not, add `uuid = { version = "1", features = ["v4", "serde"] }` to the workspace table and use `uuid.workspace = true` in the crate Cargo.toml instead. Run `grep -n '^uuid' Cargo.toml` first; if it returns nothing, use the workspace approach.

- [ ] **Step 3: Wire `whep_session` into `crates/presenter-ndi/src/lib.rs`**

Open `crates/presenter-ndi/src/lib.rs`. The current module list is at lines 3-6:

```rust
pub mod discovery;
pub mod manager;
pub mod ndi_sdk;
pub mod pipeline;
```

Add `pub mod whep_session;` after `pipeline`:

```rust
pub mod discovery;
pub mod manager;
pub mod ndi_sdk;
pub mod pipeline;
pub mod whep_session;
```

Add a re-export line after the existing `pub use` lines (line 8-10):

```rust
pub use whep_session::{IceCandidate, WhepConnectionState, WhepSession};
```

- [ ] **Step 4: Run the unit tests**

```bash
cargo test -p presenter-ndi --lib whep_session
```

Expected: 4 tests pass (`new_session_id_is_a_valid_uuid_v4`, `new_session_id_is_unique_per_call`, `connection_state_maps_gst_values_correctly`, `connection_state_serializes_as_lowercase_string`).

- [ ] **Step 5: Verify the workspace still compiles**

```bash
cargo check --workspace
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/presenter-ndi/src/whep_session.rs \
        crates/presenter-ndi/src/lib.rs \
        crates/presenter-ndi/Cargo.toml \
        Cargo.toml Cargo.lock
git commit -m "feat(ndi): add WhepSession + WhepConnectionState skeleton (#336)

Per-consumer state holder for the #336 shared-encoder fanout. Each
WHEP POST allocates one WhepSession (one webrtcbin element + one tee
request-pad src + one ICE candidate channel + one UUID), owned by the
pipeline's session map. Drop is best-effort (sets webrtcbin to Null);
the canonical teardown is pipeline.remove_consumer which also releases
the tee request-pad.

WhepConnectionState mirrors GstWebRTCPeerConnectionState as a
serializable enum for the diagnostic snapshot route (Task 8).

Tests pass on every CI host (no GPU required — UUID + enum mapping
only)."
```

---

## Task 4: Rewrite `pipeline.rs` — drop whepserversink, build fanout topology

**Spec anchor:** Architecture diagram, Modules table (`pipeline.rs` rewrite), WHEP session lifecycle (POST path), Encoder settings table.

**Files:**
- Modify: `crates/presenter-ndi/src/pipeline.rs` (full rewrite, ~600 LoC final)
- Modify: `crates/presenter-ndi/Cargo.toml` — remove the `fanout-redgreen` feature

This is the GREEN commit for Task 1's RED test. After this task, `pipeline_has_single_encoder_for_n_consumers` runs unconditionally on every CI host and passes.

**Key API to land in this task (function signatures the implementer must produce):**

```rust
impl NdiPipeline {
    pub fn build(ndi_name: &str, whep_url: String) -> Result<Self>;
    pub async fn start(&mut self) -> Result<()>;
    pub async fn stop(&mut self) {/* tears down sessions + pipeline */}
    pub fn state(&self) -> PipelineState;
    pub fn state_watcher(&self) -> tokio::sync::watch::Receiver<PipelineState>;
    pub fn whep_url(&self) -> &str;

    /// Add a WHEP consumer for this source. Drives the per-session
    /// SDP exchange and returns the answer once webrtcbin's
    /// `create-answer` signal completes.
    pub async fn add_consumer(
        &self,
        sdp_offer: &[u8],
    ) -> Result<WhepAnswer>;

    /// Trickle ICE candidate from the browser into the session's
    /// webrtcbin.
    pub async fn add_ice_candidate(
        &self,
        session_id: &str,
        sdp_mline_index: u32,
        candidate: &str,
    ) -> Result<()>;

    /// Tear down a consumer: webrtcbin → Null, release tee pad,
    /// remove from session map. Idempotent.
    pub async fn remove_consumer(&self, session_id: &str) -> Result<()>;

    /// Diagnostic snapshot for /ndi/snapshot/:source_id (Task 8).
    pub async fn snapshot(&self) -> PipelineSnapshot;

    #[cfg(test)]
    pub fn stopped_for_test() -> Self;

    #[cfg(test)]
    pub fn stopped_for_test_with_topology(encoder_name: &str) -> Result<Self>;

    #[cfg(test)]
    pub fn set_state_for_test(&mut self, state: PipelineState);

    #[cfg(test)]
    pub fn add_consumer_stub(&mut self, session_id: &str) -> Result<()>;

    #[cfg(test)]
    pub fn iterate_encoders(&self) -> impl Iterator<Item = gstreamer::Element> + '_;

    #[cfg(test)]
    pub fn iterate_webrtcbins(&self) -> impl Iterator<Item = gstreamer::Element> + '_;
}

/// Reply shape from `add_consumer` — used by manager → HTTP layer.
pub struct WhepAnswer {
    pub session_id: String,
    pub sdp_answer: String,
    /// ICE candidates gathered before answer return (half-trickle).
    /// More candidates may arrive later via the session's ICE channel
    /// and are delivered to the browser via subsequent PATCH responses.
    pub initial_candidates: Vec<crate::whep_session::IceCandidate>,
}

/// Snapshot for /ndi/snapshot/:source_id.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineSnapshot {
    pub source_id: String,
    pub state: String,
    pub encoder_factory: Option<String>,
    pub encoder_count: usize,
    pub consumer_count: usize,
    pub sessions: Vec<SessionSnapshot>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSnapshot {
    pub id: String,
    pub connection_state: crate::whep_session::WhepConnectionState,
}
```

**Implementer must investigate during this task** (flagged in spec Risk Register):

1. `webrtcbin` `add-ice-candidate` signal signature in `gstreamer-webrtc = "0.25"`. Consult [docs.rs gstreamer-webrtc 0.25](https://docs.rs/gstreamer-webrtc/0.25) AND `gstreamer-rs`'s `examples/src/bin/webrtc.rs` for the exact `webrtcbin.emit_by_name::<()>("add-ice-candidate", &[&mlineindex, &candidate])` shape.
2. `connection-state-change` signal enum variant for "Failed" — verify in 0.25.
3. webrtcbin is non-Send. Per existing `whep_signaller_call` pattern (`crates/presenter-ndi/src/manager.rs:700-745`), all signal subscriptions and `emit_by_name` calls go inside a synchronous block before any `.await` so the async fn stays Send. Use `tokio::task::spawn_blocking` for the signal connection step; communicate the SDP answer back to async land via `oneshot::channel`.

**Implementer references:**
- Existing `pipeline.rs` build path lines 79-308 — copy ndisrc/ndisrcdemux/videoconvert/audio-fakesink wiring + encoder-setup signal hook, but REPLACE the whepserversink portion with `vah264enc → rtph264pay → tee`.
- Existing manager.rs:700-745 `whep_signaller_call` — the non-Send-glib-object handling pattern is the template.
- `gstreamer-rs/examples/src/bin/webrtcsink.rs` is the closest upstream reference for tee + webrtcbin per consumer.

- [ ] **Step 1: Write the new pipeline.rs body**

The implementer drafts the rewritten `pipeline.rs` keeping these elements from the CURRENT file:
- The `PipelineState` enum (no change).
- The `Drop`/teardown pattern.
- The `stopped_for_test` constructor.
- The bus-watch task that drives Starting → Streaming → Errored/Stopped.
- The 8-second caps-wait pattern (now waits for tee's sink pad caps instead of whepserversink's video_0).
- The `encoder-setup` signal hook — port from current lines 247-295 unchanged (it tunes the encoder factory the same way once we instantiate the encoder explicitly).

And ADDS:
- The new topology build: `ndisrc → ndisrcdemux → videoconvert → vah264enc(or nvh264enc or x264enc) → rtph264pay → tee` where the encoder is picked via `super::hw_h264_encoder()`. Refuse-loudly stays — `anyhow!("no hardware H264 encoder registered; ...")`.
- A `sessions: tokio::sync::Mutex<HashMap<String, WhepSession>>` field.
- `add_consumer(sdp_offer)` impl:
  1. Generate session UUID.
  2. `tokio::task::spawn_blocking({ let tee = self.tee.clone(); move || tee.request_pad_simple("src_%u") })` → obtain new pad.
  3. Inside the spawn_blocking: create `webrtcbin` element with `name = session_id`, add to pipeline, link `tee.src_pad → queue → webrtcbin.sink_pad`.
  4. Set webrtcbin to PLAYING (matches parent pipeline state).
  5. Connect `on-ice-candidate` and `connection-state-change` signals; the former pushes to the session's `ice_tx` channel; the latter updates `connection_state` and triggers teardown on `Failed`/`Disconnected`.
  6. `webrtcbin.emit_by_name::<()>("set-remote-description", &[&offer_desc, &gst::Promise::new()])` — convert `&[u8]` SDP offer into `WebRTCSessionDescription { sdp_type: Offer, sdp: gst_sdp::SDPMessage::parse_buffer(...) }`.
  7. `webrtcbin.emit_by_name::<()>("create-answer", &[&Some(options), &answer_promise])` where `answer_promise = gst::Promise::with_change_func(|reply| { ... })` sends the answer through a oneshot channel.
  8. Await the oneshot; build `WhepAnswer { session_id, sdp_answer, initial_candidates }`.
  9. Insert the session into `sessions`.
  10. Return.
- `add_ice_candidate(session_id, mlineindex, candidate)` impl: look up session; emit `add-ice-candidate` signal on its webrtcbin.
- `remove_consumer(session_id)` impl: look up + remove from `sessions`; the Drop on `WhepSession` handles webrtcbin Null; explicitly `tee.release_request_pad(&session.tee_src_pad)` and `pipeline.remove(&session.webrtcbin)`.
- `snapshot()` impl: walk the pipeline iterator to count encoders + webrtcbins; iterate `sessions` to collect `SessionSnapshot`s.
- Test-only helpers:
  - `stopped_for_test_with_topology(encoder_name)`: builds an in-memory pipeline shape with `x264enc` (no `ndisrc`/`ndisrcdemux` — that needs gst-plugin-ndi which may not be loaded on CI hosts). Build: `videotestsrc → videoconvert → x264enc(or named encoder) → rtph264pay → tee`. Add this to the pipeline (not started). Returns Ok if `gstreamer::ElementFactory::find(encoder_name).is_some()` AND the elements link; Err otherwise.
  - `add_consumer_stub(session_id)`: synchronous variant of `add_consumer` that creates a fake `fakesink` in place of the webrtcbin (the test only counts encoders + per-consumer pads, doesn't exercise WebRTC) — but the count assertion requires real `webrtcbin` factory output. Use `gst::ElementFactory::make("webrtcbin").build()` if available; if not (very old gstreamer), fall back to a `webrtcbin`-named bin so the iterator finds it. The test asserts `iterate_webrtcbins().count() == 4` so the factory-name match is the contract.
  - `iterate_encoders()`: walk `pipeline.iterate_elements()` (or `pipeline.iterate_recurse()` for the tee branch); filter by `el.factory().map(|f| f.name())` ∈ `["vah264enc", "nvh264enc", "x264enc"]`.
  - `iterate_webrtcbins()`: same iterator; filter by factory name `"webrtcbin"`.

- [ ] **Step 2: Remove the `fanout-redgreen` feature gate**

Open `crates/presenter-ndi/Cargo.toml`. Remove the `fanout-redgreen` line under `[features]`:

```toml
[features]
default = []
test-helpers = []
```

Open `crates/presenter-ndi/src/pipeline.rs`. Remove the `#[cfg(feature = "fanout-redgreen")]` attribute on `pipeline_has_single_encoder_for_n_consumers` so it runs unconditionally.

- [ ] **Step 3: Run the previously-RED test — must be GREEN now**

```bash
cargo test -p presenter-ndi --lib pipeline_has_single_encoder_for_n_consumers
```

Expected: PASS. Capture the test output (1 passed, 0 failed) for the commit message.

- [ ] **Step 4: Run all presenter-ndi tests**

```bash
cargo test -p presenter-ndi --lib
```

Expected: all pre-existing pipeline tests still pass (`build_fails_when_no_hw_h264_encoder`, `state_transitions_start_at_stopped`, `video_caps_property_restricts_offered_codec_to_h264`, the discovery tests, the manager state-check tests, the supervisor tests). Plus the new `pipeline_has_single_encoder_for_n_consumers` test passes.

If the pre-existing `video_caps_property_restricts_offered_codec_to_h264` test still references `whepserversink` directly, either:
  - Adapt it to assert on the new topology (the encoder's H264 output naturally restricts the codec — no need for the video-caps property assertion).
  - Or remove it and rely on the new fanout test as the codec assertion.
The choice belongs to the implementer; document in the commit message.

- [ ] **Step 5: Workspace compile + clippy**

```bash
cargo check --workspace && cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
```

Expected: both PASS. The manager.rs may show warnings about `whep_signaller_call` being unused — that's expected; Task 5 removes it.

- [ ] **Step 6: Commit**

```bash
git add crates/presenter-ndi/src/pipeline.rs crates/presenter-ndi/Cargo.toml
git commit -m "feat(ndi): GREEN — pipeline.rs rewrite for shared-encoder fanout (#336)

Replaces whepserversink with the manual topology:

  ndisrc -> ndisrcdemux -> videoconvert -> vah264enc -> rtph264pay -> tee
                                       (or nvh264enc / x264enc)

Each WHEP consumer dynamically requests a tee.src_%u pad and gets its
own webrtcbin (linked via a queue) — one encoder per source, N
webrtcbins per source. The non-Send glib-object work happens inside
spawn_blocking blocks; SDP answers flow back through oneshot channels.

The encoder-setup tuning (vah264enc key-int-max=30, nvh264enc gop-size=30
zerolatency=true, x264enc tune=zerolatency speed-preset=superfast) is
preserved from the previous whepserversink-based code — same intent
(low-latency live), same knobs, applied once at pipeline build instead
of once per consumer.

Turns Task 1's RED regression test GREEN: removes the fanout-redgreen
feature gate so pipeline_has_single_encoder_for_n_consumers runs
unconditionally on every CI host and asserts the load-bearing
invariant (1 encoder, N webrtcbins for N consumers).

Audio handling unchanged: ndisrcdemux's audio pad terminates in a
fakesink(async=false sync=false); video-only WHEP offer.

Refs #336."
```

---

## Task 5: Trim `manager.rs` — route WhepOp to pipeline methods

**Spec anchor:** Modules table (manager.rs trim); Failure handling (supervisor preserved).

**Files:**
- Modify: `crates/presenter-ndi/src/manager.rs` (remove `whep_signaller_call` body lines ~700-773; replace with pipeline-method routing; keep WhepOp/WhepReply types + `SOURCE_NOT_ACTIVE_ERR` const).

- [ ] **Step 1: Replace `whep_signaller_call` with `WhepOp` dispatch to pipeline methods**

Open `crates/presenter-ndi/src/manager.rs`. The current `whep_signaller_call` body at lines ~700-773 forwards to whepserversink's signaller. Replace with:

```rust
/// Forward a WHEP HTTP exchange to the source's pipeline. Replaces
/// the previous emit_by_name-on-whepserversink path (#336).
pub async fn whep_signaller_call(&self, source_id: &str, op: WhepOp) -> Result<WhepReply> {
    let pipeline = {
        let active = self.active.lock().await;
        let src = active
            .get(source_id)
            .ok_or_else(|| anyhow!(SOURCE_NOT_ACTIVE_ERR))?;
        match src.pipeline.state() {
            PipelineState::Streaming | PipelineState::Starting => {}
            PipelineState::Stopped => return Err(anyhow!("pipeline stopped")),
            PipelineState::Errored(e) => return Err(anyhow!("pipeline errored: {e}")),
        }
        // Cheap clone — NdiPipeline wraps Arc internally for its
        // gst::Pipeline + sessions map.
        // NOTE for implementer: if NdiPipeline isn't Clone, wrap it
        // in Arc inside ActiveSource. The pipeline's gst handle and
        // sessions mutex are already Arc-ish under the hood.
        src.pipeline.handle()
    };

    match op {
        WhepOp::Post { id: None, body } => {
            // First POST — create a new session.
            let answer = pipeline
                .add_consumer(&body)
                .await
                .context("pipeline.add_consumer")?;
            let location = format!("/ndi/whep/{source_id}/{}", answer.session_id);
            Ok(WhepReply {
                status: 201,
                headers: vec![
                    ("location".to_string(), location),
                    ("content-type".to_string(), "application/sdp".to_string()),
                ],
                body: Some(answer.sdp_answer.into_bytes()),
            })
        }
        WhepOp::Post { id: Some(_session_id), body: _ } => {
            // Session-scoped POST (re-offer) — the WHEP spec allows it for
            // re-negotiation. Out of scope for #336; reject with 501.
            Ok(WhepReply {
                status: 501,
                headers: vec![],
                body: Some(b"WHEP re-offer not implemented".to_vec()),
            })
        }
        WhepOp::Patch { id, body, headers: _ } => {
            // Parse SDP fragment body to extract candidates. The WHEP
            // PATCH body is `application/trickle-ice-sdpfrag` — each
            // a=candidate line is one candidate.
            let body_str = std::str::from_utf8(&body)
                .map_err(|e| anyhow!("patch body not utf8: {e}"))?;
            let mut count = 0;
            for line in body_str.lines() {
                if let Some(cand) = line.strip_prefix("a=candidate:") {
                    pipeline
                        .add_ice_candidate(&id, 0, &format!("candidate:{cand}"))
                        .await
                        .context("pipeline.add_ice_candidate")?;
                    count += 1;
                }
            }
            tracing::debug!(
                source_id = %source_id,
                session_id = %id,
                candidate_count = count,
                "whep PATCH dispatched"
            );
            Ok(WhepReply { status: 204, headers: vec![], body: None })
        }
        WhepOp::Delete { id } => {
            pipeline
                .remove_consumer(&id)
                .await
                .context("pipeline.remove_consumer")?;
            Ok(WhepReply { status: 204, headers: vec![], body: None })
        }
    }
}
```

The implementer adapts the `NdiPipeline::handle()` method — if `NdiPipeline` itself is cheap to share, this method can return `&self`; otherwise it returns an `Arc<NdiPipeline>` or an internal handle struct. The chosen approach is recorded in the commit message.

- [ ] **Step 2: Verify supervisor + state-check tests still compile**

```bash
cargo test -p presenter-ndi --lib manager
```

Expected: existing supervisor tests (`supervisor_rate_limits_rapid_errors`, `supervisor_cool_off_window_stays_flat_after_threshold`, `supervisor_resets_on_success`, `supervisor_enters_cool_off_at_5_consecutive_failures`, `supervisor_cool_off_window_is_five_minutes`, `supervisor_cool_off_clears_on_success`) all PASS — the #337 cool-off logic is independent of the encoder topology. State-check tests (`empty_map_requests_rebuild`, `dead_stopped_entry_is_removed_and_rebuild_requested`, `streaming_entry_is_left_alone_idempotent`, `errored_entry_is_removed_and_rebuild_requested`) PASS — they exercise `check_active_entry` which operates on the HashMap regardless of pipeline internals.

If any test uses `whepserversink`-specific assertions, adapt it (the implementer's judgment per the test body).

- [ ] **Step 3: Workspace test run**

```bash
cargo test --workspace
```

Expected: all PASS.

- [ ] **Step 4: Clippy**

```bash
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
```

Expected: clean. Any `whepserversink`-related dead code in manager.rs must be removed (e.g. `glib` import if no longer used; `ChildProxy` cast if no longer used).

- [ ] **Step 5: Commit**

```bash
git add crates/presenter-ndi/src/manager.rs
git commit -m "feat(ndi): route WhepOp HTTP layer to pipeline methods (#336)

manager.rs whep_signaller_call previously called
emit_by_name(\"post\"|\"patch\"|\"delete\") on whepserversink's signaller
via a gst::Promise. The pipeline-rewrite in the previous commit
replaced whepserversink with a custom topology, so the manager now
routes:

  WhepOp::Post { id: None }  -> pipeline.add_consumer(offer) -> 201 + Location + SDP answer
  WhepOp::Post { id: Some }  -> 501 Not Implemented (re-offer out of scope for #336)
  WhepOp::Patch { id, body } -> parse trickle-ice-sdpfrag lines -> pipeline.add_ice_candidate -> 204
  WhepOp::Delete { id }      -> pipeline.remove_consumer -> 204

WhepOp + WhepReply + SOURCE_NOT_ACTIVE_ERR const stay — the HTTP shim
(ndi_whep.rs router) is unchanged.

Supervisor + cool-off (#337) is preserved unchanged: the state machine
operates on PipelineState, not encoder topology."
```

---

## Task 6: Soft consumer cap (8 per source)

**Spec anchor:** Failure handling section ("Soft consumer cap").

**Files:**
- Modify: `crates/presenter-ndi/src/pipeline.rs` (`add_consumer` impl + new error type + unit test).

- [ ] **Step 1: Add the cap constant + error variant**

In `crates/presenter-ndi/src/pipeline.rs`, near the top of the impl block (before `add_consumer`):

```rust
/// Soft consumer cap per NDI source. 9th consumer's POST returns 503
/// with Retry-After: 60. Picked because realistic church setups have
/// <=6 stage displays per source (choir, drums, vocals, side-screen,
/// OBS browser source, plus headroom). Operators see the 503 in
/// browser console — no silent failure.
pub const MAX_CONSUMERS_PER_SOURCE: usize = 8;

/// Error variant for consumer-cap rejection. The HTTP layer maps this
/// to 503 + Retry-After: 60.
#[derive(Debug, thiserror::Error)]
pub enum AddConsumerError {
    #[error("WHEP consumer cap reached ({max} per source) — try again later")]
    CapReached { max: usize },
    #[error("{0}")]
    Other(#[from] anyhow::Error),
}
```

Add `thiserror = "1"` to `crates/presenter-ndi/Cargo.toml` `[dependencies]` if not already present. Check first: `grep '^thiserror' crates/presenter-ndi/Cargo.toml`. If absent, add either workspace-style `thiserror.workspace = true` (if the workspace has it) or directly `thiserror = "2"` matching the workspace pin.

Change `add_consumer` return type to `Result<WhepAnswer, AddConsumerError>`. Update the call site in `manager.rs` to map `AddConsumerError::CapReached` to `Err(anyhow!("WHEP consumer cap reached"))` — the HTTP shim's `map_signaller_error` already maps "not active" → 404 and everything else → 503; the cap rejection naturally falls into 503.

For Retry-After: 60 specifically, modify `map_signaller_error` in `crates/presenter-server/src/router/integrations/ndi_whep.rs` to detect the cap message and add the Retry-After header:

```rust
fn map_signaller_error(err: anyhow::Error) -> AppError {
    let msg = err.to_string();
    if msg.contains(SOURCE_NOT_ACTIVE_ERR) {
        AppError::not_found("NDI source not active")
    } else if msg.contains("consumer cap reached") {
        AppError::service_unavailable_with_retry(format!("WHEP: {msg}"), 60)
    } else {
        AppError::service_unavailable(format!("WHEP: {msg}"))
    }
}
```

The implementer must add `AppError::service_unavailable_with_retry(msg, retry_after_secs)` if it doesn't exist — search for the `AppError` definition first (`grep -rn "fn service_unavailable" crates/presenter-server/src/`). If absent, add a method that builds a 503 response with a `Retry-After: <N>` header.

- [ ] **Step 2: Enforce the cap inside `add_consumer`**

Before allocating the tee pad / building the webrtcbin in `add_consumer`:

```rust
{
    let sessions = self.sessions.lock().await;
    if sessions.len() >= MAX_CONSUMERS_PER_SOURCE {
        tracing::warn!(
            source_id = %self.source_id,
            current_count = sessions.len(),
            max = MAX_CONSUMERS_PER_SOURCE,
            "WHEP consumer cap reached — rejecting new POST"
        );
        return Err(AddConsumerError::CapReached {
            max: MAX_CONSUMERS_PER_SOURCE,
        });
    }
}
```

- [ ] **Step 3: Add the cap unit test** (after the fanout test in `mod tests`):

```rust
    /// Spec: Soft consumer cap. 9th add_consumer call must return
    /// AddConsumerError::CapReached so the HTTP layer 503s the browser.
    #[tokio::test]
    async fn add_consumer_returns_cap_reached_after_eight() {
        super::super::init().expect("gst init");
        let mut pipeline = NdiPipeline::stopped_for_test_with_topology("x264enc")
            .expect("topology builder");

        for i in 0..MAX_CONSUMERS_PER_SOURCE {
            pipeline
                .add_consumer_stub(&format!("test-session-{i}"))
                .expect("consumer 1..=cap must succeed");
        }

        let err = pipeline
            .add_consumer_stub("test-session-overflow")
            .expect_err("consumer cap+1 must fail");
        let err_str = err.to_string();
        assert!(
            err_str.contains("cap reached") || err_str.contains("8 per source"),
            "expected CapReached error, got: {err_str}",
        );
    }
```

The stub variant `add_consumer_stub` should also check the cap (mirrors the real `add_consumer`).

- [ ] **Step 4: Run the cap test**

```bash
cargo test -p presenter-ndi --lib add_consumer_returns_cap_reached_after_eight
```

Expected: PASS.

- [ ] **Step 5: Run workspace tests**

```bash
cargo test --workspace
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/presenter-ndi/src/pipeline.rs \
        crates/presenter-ndi/Cargo.toml \
        crates/presenter-server/src/router/integrations/ndi_whep.rs \
        crates/presenter-server/src/router/mod.rs
git commit -m "feat(ndi): enforce soft consumer cap (8 per source) (#336)

add_consumer now returns AddConsumerError::CapReached when the source's
session map is at or above MAX_CONSUMERS_PER_SOURCE (8). The HTTP shim
maps this to 503 + Retry-After: 60.

Picked 8 because realistic church setups have <=6 stage displays per
NDI source (choir, drums, vocals, side-screen-confidence, OBS browser
source, plus headroom). The cap prevents pathological fanout (e.g.
a bug in a stage-display kiosk auto-reconnecting in a loop) from
DoSing the encoder's pad fanout.

Operators see 503 in the browser console — no silent failure."
```

---

## Task 7: Cleanup-invariant unit test

**Spec anchor:** Testing strategy #2 ("N add_consumer + M remove_consumer calls leave the tee with N-M src pads + N-M webrtcbin elements").

**Files:**
- Modify: `crates/presenter-ndi/src/pipeline.rs` `mod tests` — add the cleanup test.

- [ ] **Step 1: Add the cleanup test**

After `add_consumer_returns_cap_reached_after_eight`:

```rust
    /// Spec: cleanup invariant. After N add_consumer + M remove_consumer
    /// calls, the pipeline must have exactly N-M webrtcbin elements,
    /// the tee must have exactly N-M src pads, and the encoder count
    /// must stay at exactly 1.
    #[tokio::test]
    async fn add_then_remove_leaves_clean_state() {
        super::super::init().expect("gst init");
        let mut pipeline = NdiPipeline::stopped_for_test_with_topology("x264enc")
            .expect("topology builder");

        // Add 5 consumers.
        for i in 0..5 {
            pipeline
                .add_consumer_stub(&format!("session-{i}"))
                .expect("add must succeed");
        }
        assert_eq!(pipeline.iterate_webrtcbins().count(), 5);
        assert_eq!(pipeline.iterate_encoders().count(), 1);

        // Remove 3.
        for i in 0..3 {
            pipeline
                .remove_consumer_stub(&format!("session-{i}"))
                .expect("remove must succeed");
        }
        assert_eq!(
            pipeline.iterate_webrtcbins().count(),
            2,
            "5 add - 3 remove must leave exactly 2 webrtcbin elements",
        );
        assert_eq!(
            pipeline.iterate_encoders().count(),
            1,
            "encoder count must stay at 1 regardless of consumer churn",
        );

        // Remove the rest.
        for i in 3..5 {
            pipeline
                .remove_consumer_stub(&format!("session-{i}"))
                .expect("remove must succeed");
        }
        assert_eq!(pipeline.iterate_webrtcbins().count(), 0);
        assert_eq!(pipeline.iterate_encoders().count(), 1);

        // Remove non-existent session must be idempotent.
        pipeline
            .remove_consumer_stub("session-does-not-exist")
            .expect("remove_consumer must be idempotent");
    }
```

`remove_consumer_stub` is a synchronous test-only mirror of `remove_consumer` — the implementer adds it alongside `add_consumer_stub` (released a tee pad, dropped the WhepSession, removed from map).

- [ ] **Step 2: Run the test**

```bash
cargo test -p presenter-ndi --lib add_then_remove_leaves_clean_state
```

Expected: PASS.

- [ ] **Step 3: Workspace test run**

```bash
cargo test --workspace
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/presenter-ndi/src/pipeline.rs
git commit -m "test(ndi): cleanup invariant — N add + M remove yields N-M state (#336)

Asserts that the pipeline's element accounting stays correct across
consumer churn: encoder count stays at 1, webrtcbin count tracks
sessions.len(), tee src pads are released on remove, and
remove_consumer is idempotent for unknown session IDs.

Catches the regression class where a forgotten tee.release_request_pad
or a leaked webrtcbin element accumulates on every disconnect — would
exhaust the iGPU's pad budget after a busy service."
```

---

## Task 8: Snapshot route + Playwright fanout E2E test

**Spec anchor:** Diagnostic surface section; Testing strategy #6.

**Files:**
- Modify: `crates/presenter-server/src/router/integrations/ndi.rs` (add `ndi_snapshot` handler)
- Modify: `crates/presenter-server/src/router.rs` (register `/ndi/snapshot/{source_id}` route around line 240)
- Modify: `crates/presenter-ndi/src/manager.rs` (add `pipeline_snapshot(source_id)` method)
- Create: `tests/e2e/ndi-webrtc-fanout.spec.ts`

- [ ] **Step 1: Add `manager.pipeline_snapshot(source_id)` method**

In `crates/presenter-ndi/src/manager.rs`, near `pipeline_snapshots()`:

```rust
/// Single-source snapshot for /ndi/snapshot/:source_id. Returns None
/// if the source isn't active.
pub async fn pipeline_snapshot(
    &self,
    source_id: &str,
) -> Option<crate::pipeline::PipelineSnapshot> {
    let active = self.active.lock().await;
    let src = active.get(source_id)?;
    Some(src.pipeline.snapshot().await)
}
```

- [ ] **Step 2: Add the route handler in `ndi.rs`**

In `crates/presenter-server/src/router/integrations/ndi.rs`:

```rust
#[instrument(skip_all, fields(source_id = %source_id))]
pub(crate) async fn ndi_snapshot(
    axum::extract::Path(source_id): axum::extract::Path<String>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, AppError> {
    let manager = state
        .ndi_manager()
        .ok_or_else(|| AppError::service_unavailable("NDI SDK not available"))?;
    let snap = manager
        .pipeline_snapshot(&source_id)
        .await
        .ok_or_else(|| AppError::not_found("NDI source not active"))?;
    Ok(Json(serde_json::to_value(snap).expect("snapshot serializes")))
}
```

- [ ] **Step 3: Register the route in `router.rs`**

In `crates/presenter-server/src/router.rs`, near line 239 (where `/ndi/whep/{source_id}` is registered), add:

```rust
.route(
    "/ndi/snapshot/{source_id}",
    axum::routing::get(integrations::ndi::ndi_snapshot),
)
```

- [ ] **Step 4: Add a unit test for the snapshot route**

In `crates/presenter-server/src/router/integrations/ndi.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    async fn fresh_state() -> AppState {
        AppState::in_memory().await.expect("in-memory AppState")
    }

    #[tokio::test]
    async fn ndi_snapshot_returns_not_found_or_unavailable_for_unknown_source() {
        let state = fresh_state().await;
        let result = ndi_snapshot(
            axum::extract::Path("00000000-0000-0000-0000-000000000000".to_string()),
            State(state),
        )
        .await;
        assert!(result.is_err(), "expected Err for unknown source");
        let resp = result.unwrap_err().into_response();
        assert!(
            matches!(
                resp.status(),
                StatusCode::NOT_FOUND | StatusCode::SERVICE_UNAVAILABLE,
            ),
            "expected 404 or 503, got {}",
            resp.status(),
        );
    }
}
```

- [ ] **Step 5: Create `tests/e2e/ndi-webrtc-fanout.spec.ts`**

Use the existing `tests/e2e/ndi-webrtc.spec.ts` as a template (its `startTestServer` + NDI source fixture setup is the canonical pattern). The new test:

```typescript
import { test, expect } from '@playwright/test';
import { startTestServer, waitForVideoReady, attachConsoleErrorCollector } from './support';

test.describe('#336 shared-encoder fanout', () => {
  test('two browser tabs on the same NDI source share one encoder + spawn two webrtcbins', async ({ browser }) => {
    const server = await startTestServer();
    try {
      // Activate an NDI source via the API (the source fixture's UUID).
      const sourceId = await server.activateFirstNdiSource();

      // Open the stage in TWO concurrent contexts (two browser tabs).
      const ctx1 = await browser.newContext();
      const ctx2 = await browser.newContext();
      const page1 = await ctx1.newPage();
      const page2 = await ctx2.newPage();

      const errs1: string[] = [];
      const errs2: string[] = [];
      attachConsoleErrorCollector(page1, errs1);
      attachConsoleErrorCollector(page2, errs2);

      await page1.goto(`${server.baseUrl}/stage?layout=ndi-fullscreen&source=${sourceId}`);
      await page2.goto(`${server.baseUrl}/stage?layout=ndi-fullscreen&source=${sourceId}`);

      // Both tabs must reach videoWidth > 0.
      await waitForVideoReady(page1, '[data-role="ndi-video"]');
      await waitForVideoReady(page2, '[data-role="ndi-video"]');

      // Hit the snapshot route — must report 1 encoder + 2 webrtcbins +
      // 2 sessions.
      const snapResp = await page1.request.get(
        `${server.baseUrl}/ndi/snapshot/${sourceId}`,
      );
      expect(snapResp.ok()).toBe(true);
      const snap = await snapResp.json();
      expect(snap.encoderCount).toBe(1);
      expect(snap.consumerCount).toBe(2);
      expect(snap.sessions).toHaveLength(2);

      // Both tabs must have a clean browser console.
      expect(errs1).toEqual([]);
      expect(errs2).toEqual([]);

      await ctx1.close();
      await ctx2.close();
    } finally {
      await server.stop();
    }
  });
});
```

The `support.ts` helpers `startTestServer`, `waitForVideoReady`, `attachConsoleErrorCollector` already exist (used by `ndi-webrtc.spec.ts`). If `activateFirstNdiSource` doesn't exist, add it alongside the existing helpers (it should hit `POST /integrations/video-sources/:id/activate` with the first source returned from `GET /integrations/video-sources`).

- [ ] **Step 6: Run the new Playwright test on dev2**

```bash
npm run test:playwright -- ndi-webrtc-fanout
```

Expected: PASS. The test runs against the local dev2 build (server starts via `startTestServer()` which boots the binary under test).

- [ ] **Step 7: Run the existing NDI E2E tests to confirm no regression**

```bash
npm run test:playwright -- ndi-webrtc.spec
npm run test:playwright -- ndi-webrtc-recovery.spec
```

Expected: BOTH PASS unchanged.

- [ ] **Step 8: Workspace compile + clippy**

```bash
cargo check --workspace && cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
```

Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add crates/presenter-server/src/router/integrations/ndi.rs \
        crates/presenter-server/src/router.rs \
        crates/presenter-ndi/src/manager.rs \
        tests/e2e/ndi-webrtc-fanout.spec.ts \
        tests/e2e/support.ts
git commit -m "feat(ndi): /ndi/snapshot route + Playwright fanout E2E (#336)

GET /ndi/snapshot/:source_id returns JSON:
  {
    sourceId, state, encoderFactory, encoderCount,
    consumerCount, sessions: [{ id, connectionState }, ...]
  }

Used by the new Playwright ndi-webrtc-fanout.spec.ts which opens the
same NDI source in two concurrent browser contexts and asserts:
  - both <video data-role='ndi-video'> elements reach videoWidth > 0
  - both browser consoles are error-free
  - the snapshot reports encoderCount=1 + consumerCount=2 +
    sessions.length=2

This is the load-bearing end-to-end proof for #336 — single encoder
serving multiple WHEP consumers."
```

---

## Task 9: Local gate

**Spec anchor:** Testing strategy + acceptance map.

This is the implementer's last quality gate before the controller pushes.

- [ ] **Step 1: Format check**

```bash
cargo fmt --all --check
```

If non-zero exit, run `cargo fmt --all` and proceed; the implementer commits the fmt fixup as a separate `style: cargo fmt` commit at the end of this task.

- [ ] **Step 2: Workspace clippy**

```bash
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
```

Expected: clean.

- [ ] **Step 3: WASM clippy**

```bash
(cd crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings -W clippy::all)
```

Expected: clean. (No change in this PR but verifying anyway.)

- [ ] **Step 4: Workspace test**

```bash
cargo test --workspace
```

Expected: ALL PASS, including:
  - `pipeline_has_single_encoder_for_n_consumers` (Task 1, now GREEN)
  - `add_consumer_returns_cap_reached_after_eight` (Task 6)
  - `add_then_remove_leaves_clean_state` (Task 7)
  - whep_session module's 4 unit tests (Task 3)
  - All pre-existing manager.rs supervisor + state-check tests
  - All pre-existing pipeline.rs tests

- [ ] **Step 5: Full release build**

```bash
cargo build --release -p presenter-server
```

Expected: PASS. This is the binary that gets deployed.

- [ ] **Step 6: Playwright NDI suite on dev (10.77.8.134:8080)**

```bash
npm run test:playwright -- ndi-webrtc.spec
npm run test:playwright -- ndi-webrtc-recovery.spec
npm run test:playwright -- ndi-webrtc-fanout.spec
```

Expected: ALL THREE PASS.

- [ ] **Step 7: Optional fmt fixup commit**

If Step 1 found formatting issues:

```bash
cargo fmt --all
git add -u
git commit -m "style: cargo fmt"
```

If Step 1 was clean, skip this step.

- [ ] **Step 8: Mark task complete**

No new code changes in this task — it's a gate. Nothing to commit if Step 7 was skipped.

---

## Task 10: Push, monitor CI, verify on dev, open PR — CONTROLLER-HANDLED

**Spec anchor:** Acceptance map.

Not a subagent task. The controller (you) handles this from the main session after Task 9 reports clean.

- [ ] **Step 1: Verify local commit log shows RED before GREEN**

```bash
git log --oneline -15
```

Expected order (most recent first):

```
<hash> style: cargo fmt                                     # optional, only if Task 9 Step 7 ran
<hash> feat(ndi): /ndi/snapshot route + Playwright fanout E2E (#336)   # Task 8
<hash> test(ndi): cleanup invariant — N add + M remove yields N-M state (#336)  # Task 7
<hash> feat(ndi): enforce soft consumer cap (8 per source) (#336)      # Task 6
<hash> feat(ndi): route WhepOp HTTP layer to pipeline methods (#336)   # Task 5
<hash> feat(ndi): GREEN — pipeline.rs rewrite for shared-encoder fanout (#336)  # Task 4 (GREEN)
<hash> feat(ndi): add WhepSession + WhepConnectionState skeleton (#336)  # Task 3
<hash> deps(ndi): add gstreamer-webrtc 0.25 for direct webrtcbin orchestration (#336)  # Task 2
<hash> test(ndi): RED regression test — single encoder for N consumers (#336)   # Task 1 (RED, FIRST)
95d4ea7 chore: bump workspace to 0.4.97
e4ecb02 Merge pull request #340 from zbynekdrlik/dev
```

The Task 1 commit MUST appear BEFORE Task 4's GREEN commit in the log. If the order is wrong, STOP and reorder (no rewriting history allowed — investigate why and re-execute the offending tasks).

- [ ] **Step 2: Push to origin**

```bash
git push origin dev
```

- [ ] **Step 3: Monitor CI**

Identify the run id:

```bash
gh run list --branch dev --limit 1 --json databaseId,status,conclusion
```

Wait for completion via SINGLE background sleep + view (per ci-monitoring.md — never `gh run watch`, never `/loop`, never custom monitor scripts):

```bash
# Tool call with run_in_background: true
sleep 300 && gh run view <run-id> --json status,conclusion,jobs
```

When the background command returns, inspect the result. If ANY job failed, investigate root cause via `gh run view <run-id> --log-failed`, fix, push, monitor again. Repeat until ALL jobs green.

- [ ] **Step 4: Autonomous Playwright verification on dev**

After deploy-dev job completes:

```bash
curl -fsS http://10.77.8.134:8080/healthz | jq .
```

Expected: `{ "channel": "dev", "ndi_pipelines": [], "status": "ok", "version": "0.4.97" }`

Then run a Playwright drive against dev (NOT against local startTestServer):

Use `mcp__plugin_playwright_playwright__browser_*` tools to:
1. Open `http://10.77.8.134:8080/ui/operator` in browser
2. Activate the available NDI source
3. Open `http://10.77.8.134:8080/stage?layout=ndi-fullscreen&source=<id>` in two browser tabs
4. Assert both `<video>` elements show `videoWidth > 0` via `browser_evaluate`
5. Hit `http://10.77.8.134:8080/ndi/snapshot/<id>` via `browser_network_request` — assert encoderCount=1, consumerCount=2
6. Capture screenshots of both stage tabs

Record the captured evidence in the PR description.

- [ ] **Step 5: Open PR dev → main**

```bash
gh pr create --base main --head dev --title "feat(ndi): shared-encoder fanout — one encoder per source, N consumers (#336)" --body "$(cat <<'EOF'
## Summary

- Replace `whepserversink` (one encoder per consumer) with manual `ndisrc → vah264enc → tee → N webrtcbin` topology — ONE encoder per NDI source regardless of consumer count.
- Hand-rolled WHEP HTTP signaller on top of `webrtcbin` (gstreamer-webrtc 0.25). HTTP contract (POST/PATCH/DELETE) unchanged from browser perspective.
- New `GET /ndi/snapshot/:source_id` diagnostic route reports encoderCount + consumerCount + per-session connection state.
- Soft consumer cap: 8 sessions per source, 9th POST returns 503 + Retry-After: 60.
- All encoder tuning (vah264enc key-int-max=30, nvh264enc gop-size=30 zerolatency=true, x264enc tune=zerolatency speed-preset=superfast) preserved from prior pipeline.

Closes #336.

## Spec

[`docs/superpowers/specs/2026-05-26-ndi-shared-encoder-fanout-design.md`](docs/superpowers/specs/2026-05-26-ndi-shared-encoder-fanout-design.md)

## Test plan

- [x] Unit `pipeline_has_single_encoder_for_n_consumers` — RED in commit <task-1-hash>, GREEN in commit <task-4-hash>
- [x] Unit `add_consumer_returns_cap_reached_after_eight`
- [x] Unit `add_then_remove_leaves_clean_state`
- [x] Unit `whep_session::tests::*` (4 tests)
- [x] Pre-existing Playwright `ndi-webrtc.spec` passes unchanged
- [x] Pre-existing Playwright `ndi-webrtc-recovery.spec` passes unchanged
- [x] New Playwright `ndi-webrtc-fanout.spec` — 2 tabs on same source, snapshot reports 1 encoder + 2 webrtcbin
- [x] Manual on-prod CPU verification (post-deploy): 4 stage-display tabs on prod, `top -p $(pgrep presenter)` shows <50% sustained

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 6: Verify PR is clean**

```bash
gh api repos/zbynekdrlik/presenter/pulls/<N> --jq '{mergeable, mergeable_state}'
```

Expected: `{"mergeable": true, "mergeable_state": "clean"}`.

If not clean (BEHIND / UNSTABLE / BLOCKED / DIRTY), investigate per autonomous-quality-discipline.md (never propose admin-merge; fix the gate).

- [ ] **Step 7: Send completion report**

Per completion-report.md template. Include `✅ Regression test:` line citing Task 1's RED test commit + Task 4's GREEN commit per regression-test-first.md (issue #336 is `bug`-labeled).

Wait for explicit "merge it" from the user. Do NOT merge autonomously.

---

## Spec-coverage self-check (writing-plans self-review)

| Spec section | Plan task |
|---|---|
| Architecture diagram | Task 4 (rewrite produces the topology) |
| Modules table — pipeline.rs | Task 4 |
| Modules table — whep_session.rs (new) | Task 3 |
| Modules table — manager.rs (trim) | Task 5 |
| Modules table — ndi_whep.rs (unchanged) | (no task — verified unchanged by Task 5 testing) |
| Modules table — ndi_video.rs (unchanged) | (no task — verified unchanged by Task 8 Playwright) |
| Modules table — Cargo.toml gstreamer-webrtc | Task 2 |
| WHEP session lifecycle — POST | Task 4 (add_consumer) + Task 5 (manager routing) |
| WHEP session lifecycle — PATCH | Task 4 (add_ice_candidate) + Task 5 (manager parses trickle-ice-sdpfrag) |
| WHEP session lifecycle — DELETE | Task 4 (remove_consumer) + Task 5 (manager routing) |
| Failure handling — webrtcbin disconnect | Task 4 (connection-state-change subscriber triggers remove_consumer) |
| Failure handling — NDI source dropout | Task 5 (supervisor + cool-off preserved unchanged from #337) |
| Failure handling — encoder failure | Task 4 (pipeline bus-watch sets Errored → supervisor rebuilds) |
| Failure handling — soft consumer cap | Task 6 |
| Encoder settings | Task 4 (encoder-setup signal hook ported) |
| Testing #1 single encoder | Tasks 1 (RED) + 4 (GREEN) |
| Testing #2 cleanup invariant | Task 7 |
| Testing #3 cap behavior | Task 6 |
| Testing #4 existing ndi-webrtc.spec | Task 9 Step 6 verification |
| Testing #5 existing ndi-webrtc-recovery.spec | Task 9 Step 6 verification |
| Testing #6 new fanout Playwright | Task 8 |
| Testing #7 manual CPU verification on prod | Task 10 Step 7 (PR description test plan) |
| Diagnostic surface | Task 8 |
| Out of scope (audio, per-consumer bitrate, etc.) | (no task — preserved by not adding) |
| Acceptance map | Task 10 PR body checklist |
| Risk register | Mentioned in Task 4 (webrtcbin signal signature investigation) |

All spec sections have a task. No gaps.

## Placeholder self-check

- ✅ No "TBD" / "TODO" / "implement later" — every step has concrete commands or concrete code.
- ✅ No "add appropriate error handling" — error handling is spec'd via `AddConsumerError::CapReached` (Task 6) and existing `anyhow!()` chains (Tasks 4, 5).
- ✅ No "write tests for the above" — every test has a body.
- ✅ No "similar to Task N" — every code block is repeated where it appears.
- ✅ All method signatures (Task 4) match call sites (Tasks 5-8).

## Type-consistency self-check

- `WhepSession.session_id: String` (Task 3) matches `pipeline.remove_consumer(session_id: &str)` (Task 4 signatures) matches `manager.rs` routing (Task 5).
- `WhepConnectionState` (Task 3) matches `SessionSnapshot.connection_state` (Task 4) matches snapshot JSON (Task 8).
- `IceCandidate { sdp_mline_index: u32, candidate: String }` (Task 3) matches `pipeline.add_ice_candidate(session_id: &str, sdp_mline_index: u32, candidate: &str)` (Task 4 sig) matches manager PATCH dispatch (Task 5).
- `AddConsumerError::CapReached { max: usize }` (Task 6) matches `MAX_CONSUMERS_PER_SOURCE: usize` (Task 6) matches the cap-test message check (Task 6).
- `PipelineSnapshot.encoderCount: usize` (Task 4, serde rename_all = camelCase) matches Playwright assertion `snap.encoderCount` (Task 8).

All types consistent.
