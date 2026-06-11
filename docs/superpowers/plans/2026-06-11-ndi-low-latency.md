# NDI Low-Latency Package Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make NDI → stage-display video latency low (sub-300 ms) and STABLE over hours (no drift growth, no keyframe-pulse choppiness), with observability and a CI latency guard.

**Architecture:** Tune the existing shared-encoder + per-consumer-pipeline fanout (do NOT change its topology or its three load-bearing invariants). Decouple PTS from the NDI sender clock, stop holding frames at the producer appsink, replace 1 s keyframe pulses with GOP=240 + force-keyunit on join, hint the browser jitter buffer to stay minimal, and bake a clock strip into the synthetic E2E sender so CI measures true glass-to-glass latency.

**Tech Stack:** Rust (gstreamer-rs 0.25, gst-plugin-ndi 0.15, gstreamer-utils StreamProducer, axum), Leptos/WASM stage UI, Playwright E2E.

**Spec:** `docs/superpowers/specs/2026-06-11-ndi-low-latency-design.md`

**Version note:** dev is 0.4.111, main is 0.4.110 — dev already strictly higher, NO bump needed.

**Build policy:** This machine (dev2) allows local builds. Run `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test -p presenter-ndi` locally per task; full workspace test + e2e before push. ONE push at the end (ci-push-discipline).

---

### Task 1: Encoder-pipeline tuning — unit property tests (RED)

**Files:**
- Modify: `crates/presenter-ndi/src/pipeline/tests.rs` (append after `state_transitions_start_at_stopped`)

- [ ] **Step 1: Write the failing test**

Append to `crates/presenter-ndi/src/pipeline/tests.rs`:

```rust
/// Low-latency regression locks (2026-06-11 design): every knob below was a
/// measured latency/stability mechanism — see
/// docs/superpowers/specs/2026-06-11-ndi-low-latency-design.md.
#[test]
fn pipeline_tuning_properties_are_low_latency() {
    super::super::init().unwrap();
    if super::super::hw_h264_encoder().is_none() {
        return;
    }
    let p = NdiPipeline::build("no-such-source", "http://127.0.0.1/whep".into()).unwrap();

    // 1. PTS from server receive time — zero sender-clock coupling, no drift
    //    DISCONT jumps ("lag builds, then jumps").
    let ndisrc = p.pipeline.by_name("ndisrc").expect("ndisrc element must be named 'ndisrc'");
    assert_eq!(
        ndisrc.property::<gst_plugin_ndi::TimestampMode>("timestamp-mode"),
        gst_plugin_ndi::TimestampMode::ReceiveTime,
        "ndisrc must use pure receive-time timestamps"
    );

    // 2. Relay forwards frames immediately (sync=false saves ~40ms measured);
    //    small bounded backlog (5 frames, drop=true).
    let appsink = p
        .pipeline
        .by_name("enc_appsink")
        .expect("appsink named enc_appsink")
        .downcast::<gst_app::AppSink>()
        .expect("enc_appsink is an AppSink");
    assert!(
        !appsink.property::<bool>("sync"),
        "producer appsink must be sync=false (StreamProducer::with ProducerSettings)"
    );
    assert_eq!(appsink.max_buffers(), 5, "appsink backlog must be 5 frames");

    // 3. GOP 240 (8s): no 1s IDR pulses; joins use force-keyunit instead.
    let encoder = p.pipeline.by_name("encoder").expect("encoder named");
    let factory = encoder.factory().expect("factory").name().to_string();
    let gop: i64 = match factory.as_str() {
        "nvh264enc" => encoder.property::<i32>("gop-size") as i64,
        "vah264enc" | "x264enc" => encoder.property::<u32>("key-int-max") as i64,
        other => panic!("unexpected encoder factory {other}"),
    };
    assert_eq!(gop, 240, "GOP must be 240 frames");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p presenter-ndi pipeline_tuning_properties_are_low_latency`
Expected: FAIL — `ndisrc element must be named 'ndisrc'` (the element is currently unnamed), and after naming it would fail on timestamp-mode `Auto != ReceiveTime`.

(If the host had no encoder plugins the test would early-return — dev2 and CI both have them; verify the test actually ran by its assert message on failure.)

### Task 2: Encoder-pipeline tuning — implementation (GREEN)

**Files:**
- Modify: `crates/presenter-ndi/src/pipeline/build.rs`

- [ ] **Step 1: ndisrc — name + receive-time timestamps**

In `NdiPipeline::build`, change:

```rust
        let ndisrc = gst::ElementFactory::make("ndisrc")
            .property("ndi-name", ndi_name)
            .build()
            .context("build ndisrc")?;
```

to:

```rust
        // timestamp-mode=receive-time: PTS purely from this server's clock at
        // frame arrival. The default ("auto") follows the NDI sender's
        // (Resolume's) timecode with windowed drift correction — accumulated
        // sender-clock drift is then corrected via DISCONT, which the browser
        // sees as "latency builds up, then the picture jumps". Receive-time
        // has ZERO sender-clock coupling; arrival jitter (10-60ms measured)
        // is absorbed by the browser's jitter buffer.
        let ndisrc = gst::ElementFactory::make("ndisrc")
            .name("ndisrc")
            .property("ndi-name", ndi_name)
            .property_from_str("timestamp-mode", "receive-time")
            .build()
            .context("build ndisrc")?;
```

- [ ] **Step 2: appsink — 5-frame backlog**

In the same function change `.max_buffers(30)` to `.max_buffers(5)` and update the comment above it:

```rust
        // `max-buffers`+`drop` bound the appsink so a momentarily-slow fanout
        // can never back-pressure (and stall) the shared encoder. 5 frames
        // (~170ms) — a bigger backlog would replay stale frames late after a
        // transient stall (latency spike); drop(true) keeps the newest.
        let appsink = gst_app::AppSink::builder()
            .name("enc_appsink")
            .caps(&consumer_h264_caps())
            .max_buffers(5)
            .drop(true)
            .build();
```

- [ ] **Step 3: StreamProducer — sync=false**

Change:

```rust
        let producer = StreamProducer::from(&appsink);
```

to:

```rust
        // sync=false: forward every encoded frame to consumers IMMEDIATELY.
        // The StreamProducer default (sync=true) holds each frame on the
        // appsink until its clock deadline (full pipeline latency budget,
        // ~40ms measured) — correct for a rendering sink, wrong for a relay.
        let producer = StreamProducer::with(
            &appsink,
            gstreamer_utils::streamproducer::ProducerSettings { sync: false },
        );
```

- [ ] **Step 4: encoder GOP 240 + vah264enc target-usage**

In `build_encoder`, change the match arms to:

```rust
    match encoder_name {
        "vah264enc" => {
            // key-int-max=240 (8s GOP): no 1s IDR pulses — large keyframes
            // made low-end TVs choppy and inflated their jitter buffers.
            // Consumer joins get an immediate IDR via force-keyunit (see
            // consumers::request_keyframe); loss recovery stays PLI-driven.
            // target-usage=6: faster encode on the prod N100 (default 4).
            encoder_builder = encoder_builder
                .property("key-int-max", 240u32)
                .property("target-usage", 6u32)
                .property("bitrate", 2500u32);
        }
        "nvh264enc" => {
            encoder_builder = encoder_builder
                .property("gop-size", 240i32)
                .property("zerolatency", true)
                .property("bitrate", 2500u32);
        }
        "x264enc" => {
            encoder_builder = encoder_builder
                .property_from_str("tune", "zerolatency")
                .property_from_str("speed-preset", "superfast")
                .property("key-int-max", 240u32)
                .property("bitrate", 2500u32);
        }
        _ => {}
    }
```

Also update the stale comment in `build` above `h264parse` that says keyframes arrive "≈1s at gop-size 30" — replace that sentence with: "a consumer that joins mid-stream gets an IDR immediately via `consumers::request_keyframe` (GOP itself is 240 frames)".

- [ ] **Step 5: Run tests**

Run: `cargo test -p presenter-ndi`
Expected: `pipeline_tuning_properties_are_low_latency` PASSES; all existing tests stay green.

- [ ] **Step 6: Commit**

```bash
git add crates/presenter-ndi/src/pipeline/tests.rs crates/presenter-ndi/src/pipeline/build.rs
git commit -m "test(ndi): lock low-latency pipeline properties [red]" -- crates/presenter-ndi/src/pipeline/tests.rs
git commit -m "feat(ndi): receive-time PTS, sync=false producer, GOP 240, 5-frame backlog

timestamp-mode=receive-time removes Resolume-clock drift DISCONTs (lag-then-
jump); ProducerSettings{sync:false} stops holding encoded frames to clock
deadlines (-40ms measured); GOP 30->240 removes the 1s IDR pulses that made
low-end TVs choppy; appsink backlog 30->5 bounds stale-frame replay.
Per docs/superpowers/specs/2026-06-11-ndi-low-latency-design.md" crates/presenter-ndi/src/pipeline/build.rs
```

(Two commits: test commit FIRST, then the implementation commit — regression-test-first order. `git commit <path>` commits only the staged path listed.)

### Task 3: Payloader zero-latency aggregation (RED → GREEN)

**Files:**
- Modify: `crates/presenter-ndi/src/pipeline/tests.rs`
- Modify: `crates/presenter-ndi/src/pipeline/consumers.rs:595-600`

- [ ] **Step 1: Write the failing test** (append to tests.rs)

```rust
/// webrtcsink parity: rtph264pay must aggregate in zero-latency mode
/// (default "none" can hold NALs; webrtcsink sets this on every payloader).
#[test]
fn consumer_payloader_uses_zero_latency_aggregation() {
    super::super::init().unwrap();
    let (_appsrc, payloader, _webrtcbin) =
        super::consumers::build_consumer_elements("test-agg", Some(102))
            .expect("consumer elements build");
    let value = payloader.property_value("aggregate-mode");
    let (_, enum_value) = gstreamer::glib::EnumValue::from_value(&value)
        .expect("aggregate-mode is an enum");
    assert_eq!(enum_value.nick(), "zero-latency");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p presenter-ndi consumer_payloader_uses_zero_latency_aggregation`
Expected: FAIL — nick is `"none"`.

- [ ] **Step 3: Implement** — in `build_consumer_elements` (consumers.rs), add one property to the payloader builder:

```rust
    let payloader = gst::ElementFactory::make("rtph264pay")
        .name(format!("pay_{session_id}"))
        .property("config-interval", -1i32)
        .property("pt", offer_h264_pt.unwrap_or(96))
        // webrtcsink parity: aggregate NALs only until a VCL unit is complete —
        // never hold a frame's data back for packing efficiency.
        .property_from_str("aggregate-mode", "zero-latency")
        .build()
        .context("build rtph264pay")?;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p presenter-ndi consumer_payloader_uses_zero_latency_aggregation`
Expected: PASS.

- [ ] **Step 5: Commit (test first, then impl)**

```bash
git add crates/presenter-ndi/src/pipeline/tests.rs
git commit -m "test(ndi): lock rtph264pay aggregate-mode=zero-latency [red]"
git add crates/presenter-ndi/src/pipeline/consumers.rs
git commit -m "feat(ndi): rtph264pay aggregate-mode=zero-latency (webrtcsink parity)"
```

### Task 4: Force-keyunit on consumer join (RED → GREEN)

**Files:**
- Modify: `Cargo.toml` (workspace deps)
- Modify: `crates/presenter-ndi/Cargo.toml`
- Modify: `crates/presenter-ndi/src/pipeline/consumers.rs`
- Modify: `crates/presenter-ndi/src/pipeline/tests.rs`

- [ ] **Step 1: Add gstreamer-video dependency**

In root `Cargo.toml` `[workspace.dependencies]`, next to the existing `gstreamer-app = "0.25"` line, add:

```toml
gstreamer-video = "0.25"
```

In `crates/presenter-ndi/Cargo.toml` `[dependencies]`, add:

```toml
gstreamer-video.workspace = true
```

- [ ] **Step 2: Write the failing test** (append to tests.rs)

```rust
/// With GOP=240 a joining consumer MUST trigger an immediate IDR — otherwise
/// it would wait up to 8s for the next scheduled keyframe (black join).
#[test]
fn request_keyframe_sends_force_key_unit_upstream() {
    use std::sync::atomic::{AtomicBool, Ordering};
    super::super::init().unwrap();
    if super::super::hw_h264_encoder().is_none() {
        return;
    }
    let p = NdiPipeline::build("no-such-source", "http://127.0.0.1/whep".into()).unwrap();
    let appsink_pad = p
        .pipeline
        .by_name("enc_appsink")
        .unwrap()
        .static_pad("sink")
        .unwrap();
    let seen = std::sync::Arc::new(AtomicBool::new(false));
    let seen_probe = std::sync::Arc::clone(&seen);
    appsink_pad.add_probe(gst::PadProbeType::EVENT_UPSTREAM, move |_, info| {
        if let Some(gst::PadProbeData::Event(ev)) = &info.data {
            if gstreamer_video::UpstreamForceKeyUnitEvent::parse(ev).is_ok() {
                seen_probe.store(true, Ordering::SeqCst);
            }
        }
        gst::PadProbeReturn::Ok
    });
    super::consumers::request_keyframe(&p.producer);
    assert!(
        seen.load(Ordering::SeqCst),
        "ForceKeyUnit must be pushed upstream from the producer appsink"
    );
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p presenter-ndi request_keyframe_sends_force_key_unit_upstream`
Expected: COMPILE FAIL — `request_keyframe` not found (that is the RED state; commit the test only after the GREEN step compiles — see Step 6 note).

- [ ] **Step 4: Implement** — append to `crates/presenter-ndi/src/pipeline/consumers.rs`:

```rust
/// Ask the shared encoder for an immediate IDR (all-headers=true so SPS/PPS
/// precede it). Pushed upstream from the producer appsink's sink pad — the
/// same path StreamProducer uses to forward browser PLIs. REQUIRED for
/// consumer join with GOP=240: without it a new consumer waits up to 8s for
/// the next scheduled keyframe.
pub(super) fn request_keyframe(producer: &StreamProducer) {
    let event = gstreamer_video::UpstreamForceKeyUnitEvent::builder()
        .all_headers(true)
        .build();
    if let Some(pad) = producer.appsink().static_pad("sink") {
        if !pad.push_event(event) {
            tracing::warn!("force-keyunit event was not handled upstream");
        }
    }
}
```

And in `build_consumer_pipeline_blocking`, right after the `branch.link = Some(...)` block (the `producer.add_consumer` call), add:

```rust
    // GOP is 240 frames — explicitly request an IDR so this consumer starts
    // decoding immediately instead of waiting for the GOP boundary.
    request_keyframe(producer);
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p presenter-ndi`
Expected: all PASS, including `request_keyframe_sends_force_key_unit_upstream`.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock crates/presenter-ndi/Cargo.toml crates/presenter-ndi/src/pipeline/tests.rs
git commit -m "test(ndi): joining consumer must trigger immediate force-keyunit [red]"
git add crates/presenter-ndi/src/pipeline/consumers.rs
git commit -m "feat(ndi): request_keyframe on consumer join (GOP 240 join path)"
```

(The RED here is a compile failure, so the test commit includes the dep additions; the impl commit follows. Order in history: test before fix.)

### Task 5: Clock strip in the synthetic sender (RED → GREEN)

**Files:**
- Create: `crates/presenter-ndi/src/test_strip.rs`
- Modify: `crates/presenter-ndi/src/lib.rs` (module registration)
- Modify: `crates/presenter-ndi/src/bin/ndi_test_sender.rs`
- Delete: `crates/presenter-ndi/src/bin/ndi_clock_sender.rs` (untracked diagnostic leftover — `rm` it)

- [ ] **Step 1: Write the failing roundtrip test** — create `crates/presenter-ndi/src/test_strip.rs`:

```rust
//! Clock-strip painting/decoding for the synthetic E2E sender.
//!
//! 26 blocks of 48×48 px at (48,48) in a 2560-px-wide UYVY frame:
//! block 0 = always white, block 1 = always black (threshold calibration),
//! blocks 2..=25 = 24-bit big-endian `unix_millis % 2^24` (white=1, black=0).
//! Geometry survives the 1280×720 downscale and 2.5 Mbps H264 encode
//! (verified live 2026-06-11). The Playwright latency e2e decodes the strip
//! from a canvas and computes glass-to-glass latency = Date.now() − value
//! (sender and browser run on the same CI machine → same clock).

pub const STRIP_BLOCK_PX: usize = 48;
pub const STRIP_X0: usize = 48;
pub const STRIP_Y0: usize = 48;
pub const STRIP_DATA_BITS: usize = 24;
pub const STRIP_MODULUS: u64 = 1 << STRIP_DATA_BITS;

/// Paint one block (UYVY: byte pairs [U Y V Y]; luma set, chroma neutral).
fn paint_block(data: &mut [u8], stride: usize, idx: usize, white: bool) {
    let y_val: u8 = if white { 235 } else { 16 };
    let x_px = STRIP_X0 + idx * STRIP_BLOCK_PX;
    for row in STRIP_Y0..(STRIP_Y0 + STRIP_BLOCK_PX) {
        let base = row * stride + x_px * 2;
        for p in (0..STRIP_BLOCK_PX * 2).step_by(2) {
            data[base + p] = 128;
            data[base + p + 1] = y_val;
        }
    }
}

/// Paint the full strip encoding `now_ms % 2^24` into a UYVY frame.
pub fn paint_strip(data: &mut [u8], stride: usize, now_ms: u64) {
    let val = (now_ms % STRIP_MODULUS) as u32;
    paint_block(data, stride, 0, true);
    paint_block(data, stride, 1, false);
    for bit in 0..STRIP_DATA_BITS {
        paint_block(data, stride, 2 + bit, (val >> (STRIP_DATA_BITS - 1 - bit)) & 1 == 1);
    }
}

/// Decode the strip from a UYVY frame (inverse of `paint_strip`; test-side).
pub fn decode_strip(data: &[u8], stride: usize) -> Option<u32> {
    let luma = |idx: usize| -> u32 {
        let x_px = STRIP_X0 + idx * STRIP_BLOCK_PX + STRIP_BLOCK_PX / 2;
        let y = STRIP_Y0 + STRIP_BLOCK_PX / 2;
        data[y * stride + x_px * 2 + 1] as u32
    };
    let white = luma(0);
    let black = luma(1);
    if white <= black + 50 {
        return None;
    }
    let thr = (white + black) / 2;
    let mut val: u32 = 0;
    for bit in 0..STRIP_DATA_BITS {
        val = (val << 1) | u32::from(luma(2 + bit) > thr);
    }
    Some(val)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_roundtrip_paints_and_decodes() {
        let stride = 2560 * 2;
        let mut frame = vec![100u8; stride * 1440];
        paint_strip(&mut frame, stride, 1_781_179_287_123);
        assert_eq!(
            decode_strip(&frame, stride),
            Some((1_781_179_287_123u64 % STRIP_MODULUS) as u32)
        );
    }

    #[test]
    fn strip_decode_returns_none_without_strip() {
        let stride = 2560 * 2;
        let frame = vec![100u8; stride * 1440];
        assert_eq!(decode_strip(&frame, stride), None);
    }
}
```

Register in `crates/presenter-ndi/src/lib.rs` (next to the other module declarations):

```rust
#[cfg(feature = "test-helpers")]
pub mod test_strip;
```

- [ ] **Step 2: Run the tests**

Run: `cargo test -p presenter-ndi --features test-helpers test_strip`
Expected: PASS (pure-function module lands with its tests in one commit — there is no pre-existing behavior to lock RED here).

- [ ] **Step 3: Wire into the sender** — in `crates/presenter-ndi/src/bin/ndi_test_sender.rs`, after the `capsfilter` is built and linked (after the `combiner.link(&sink)` line), add a buffer probe (mirror the existing element setup style):

```rust
    // Paint the clock strip into every outgoing frame so the latency e2e can
    // measure true glass-to-glass latency (see presenter_ndi::test_strip).
    let probe_pad = capsfilter
        .static_pad("src")
        .ok_or_else(|| anyhow!("capsfilter has no src pad (probe)"))?;
    probe_pad.add_probe(gst::PadProbeType::BUFFER, move |_, info| {
        if let Some(gst::PadProbeData::Buffer(ref mut buffer)) = info.data {
            let buffer = buffer.make_mut();
            if let Ok(mut map) = buffer.map_writable() {
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                presenter_ndi::test_strip::paint_strip(map.as_mut_slice(), 2560 * 2, now_ms);
            }
        }
        gst::PadProbeReturn::Ok
    });
```

Then delete the diagnostic leftover: `rm -f crates/presenter-ndi/src/bin/ndi_clock_sender.rs`

- [ ] **Step 4: Build the sender to verify it compiles**

Run: `cargo build -p presenter-ndi --features test-helpers --bin ndi_test_sender`
Expected: compiles clean.

- [ ] **Step 5: Commit**

```bash
git add crates/presenter-ndi/src/test_strip.rs crates/presenter-ndi/src/lib.rs crates/presenter-ndi/src/bin/ndi_test_sender.rs
git commit -m "feat(ndi): bake clock strip into synthetic sender for g2g latency e2e"
```

### Task 6: Stage UI — jitter-buffer hints + stats beacon

**Files:**
- Modify: `crates/presenter-ui/src/components/stage/ndi_video.rs`

- [ ] **Step 1: Receiver hints in `attach_ontrack`**

In `attach_ontrack`'s closure, at the TOP of the `if let Ok(s) = streams.get(0)...` body (before the muted/srcObject dance), add:

```rust
            // Pin the receiver's jitter buffer to its minimum and let it
            // shrink back after spikes. On low-end TV WebViews the adaptive
            // buffer otherwise only ratchets UP ("delayed + choppy" stage).
            // jitterBufferTarget (ms) is the standard knob (Chrome/WebView
            // 122+); playoutDelayHint (s) is the legacy fallback. Both set
            // via Reflect (no web_sys bindings); unsupported = silent no-op.
            let receiver = ev.receiver();
            let _ = js_sys::Reflect::set(
                receiver.as_ref(),
                &JsValue::from_str("jitterBufferTarget"),
                &JsValue::from_f64(0.0),
            );
            let _ = js_sys::Reflect::set(
                receiver.as_ref(),
                &JsValue::from_str("playoutDelayHint"),
                &JsValue::from_f64(0.0),
            );
```

`js_sys` is already in scope in this file (used at line ~471 as `js_sys::Reflect`); `ev.receiver()` returns `web_sys::RtcRtpReceiver`.

- [ ] **Step 2: Stats beacon inside the Watchdog tick**

The Watchdog already owns a 1 s interval + `active` flag + teardown. Extend `Watchdog::install` (which receives `&RtcPeerConnection`) — inside the stall-timer closure body (the `{ ... }` block that starts with `let active = Rc::clone(&active);` for the timer), add a tick counter that every 15th tick samples `pc.getStats()` and POSTs a summary. Concretely, inside the timer closure (after the existing stall check, NOT replacing it), add:

```rust
                // Every 15th tick: post a stats beacon so server logs capture
                // this display's real view (fps, jitter buffer, freezes) —
                // "stage is laggy" reports become diagnosable from data.
                tick_count.set(tick_count.get().wrapping_add(1));
                if tick_count.get() % 15 == 0 {
                    let pc = pc_for_stats.clone();
                    let source_id = source_id_for_stats.clone();
                    wasm_bindgen_futures::spawn_local(async move {
                        if let Ok(report) =
                            wasm_bindgen_futures::JsFuture::from(pc.get_stats()).await
                        {
                            post_client_stats(&source_id, &report).await;
                        }
                    });
                }
```

with these captures set up before the closure (alongside the existing `Rc` clones): `let tick_count: Rc<Cell<u32>> = Rc::new(Cell::new(0));` plus `let tick_count = Rc::clone(&tick_count);` inside the block, `let pc_for_stats = pc.clone();`, `let source_id_for_stats = source_id.to_string();`. This requires threading `source_id: &str` into `Watchdog::install` — add it as a parameter and pass it at the call site (`Watchdog::install(&video, &session.pc, &source_id, move || flag.set(true))`).

- [ ] **Step 3: The beacon poster** — append to ndi_video.rs:

```rust
/// Extract inbound-video stats from an RtcStatsReport (a JS Map) and POST a
/// compact summary to /ndi/client-stats. Fire-and-forget; errors are logged.
async fn post_client_stats(source_id: &str, report: &JsValue) {
    let mut frames_decoded = JsValue::NULL;
    let mut fps = JsValue::NULL;
    let mut jb_delay = JsValue::NULL;
    let mut jb_emitted = JsValue::NULL;
    let mut freeze_count = JsValue::NULL;
    let mut frames_dropped = JsValue::NULL;

    let map: &js_sys::Map = report.unchecked_ref();
    let entries = js_sys::try_iter(&map.values()).ok().flatten();
    if let Some(entries) = entries {
        for entry in entries.flatten() {
            let get = |k: &str| js_sys::Reflect::get(&entry, &JsValue::from_str(k))
                .unwrap_or(JsValue::NULL);
            if get("type").as_string().as_deref() == Some("inbound-rtp")
                && get("kind").as_string().as_deref() == Some("video")
            {
                frames_decoded = get("framesDecoded");
                fps = get("framesPerSecond");
                jb_delay = get("jitterBufferDelay");
                jb_emitted = get("jitterBufferEmittedCount");
                freeze_count = get("freezeCount");
                frames_dropped = get("framesDropped");
            }
        }
    }

    let jitter_buffer_ms = match (jb_delay.as_f64(), jb_emitted.as_f64()) {
        (Some(d), Some(n)) if n > 0.0 => Some(d / n * 1000.0),
        _ => None,
    };
    let body = serde_json::json!({
        "sourceId": source_id,
        "framesDecoded": frames_decoded.as_f64(),
        "fps": fps.as_f64(),
        "jitterBufferMs": jitter_buffer_ms,
        "freezeCount": freeze_count.as_f64(),
        "framesDropped": frames_dropped.as_f64(),
    })
    .to_string();

    let init = leptos::web_sys::RequestInit::new();
    init.set_method("POST");
    init.set_body(&JsValue::from_str(&body));
    let Ok(headers) = leptos::web_sys::Headers::new() else { return };
    let _ = headers.set("Content-Type", "application/json");
    init.set_headers(&headers);
    let Ok(request) =
        leptos::web_sys::Request::new_with_str_and_init("/ndi/client-stats", &init)
    else {
        return;
    };
    if let Some(window) = leptos::web_sys::window() {
        let _ = wasm_bindgen_futures::JsFuture::from(window.fetch_with_request(&request)).await;
    }
}
```

(If `serde_json` is not already a presenter-ui dependency, build the JSON body with `format!` on the same keys instead — check `crates/presenter-ui/Cargo.toml` first.)

- [ ] **Step 4: Build the UI crate**

Run: `cargo check -p presenter-ui --target wasm32-unknown-unknown`
Expected: compiles clean. (If the workspace normally builds UI via trunk in CI, `cargo check` with the wasm target is the fast local equivalent.)

- [ ] **Step 5: Commit**

```bash
git add crates/presenter-ui/src/components/stage/ndi_video.rs
git commit -m "feat(stage): jitterBufferTarget=0 hints + client stats beacon

Receiver-level jitterBufferTarget/playoutDelayHint keep low-end TV WebViews
from ratcheting the jitter buffer up. Beacon POSTs fps/jb/freezes every 15s
to /ndi/client-stats for data-backed latency diagnosis."
```

### Task 7: `/ndi/client-stats` endpoint (RED → GREEN)

**Files:**
- Modify: `crates/presenter-server/src/router/integrations/ndi.rs`
- Modify: `crates/presenter-server/src/router.rs` (route registration)
- Modify: `crates/presenter-server/src/router/tests.rs`

- [ ] **Step 1: Write the failing router test** (append to router/tests.rs, mirroring `health_endpoint_returns_ok`):

```rust
#[tokio::test]
async fn ndi_client_stats_beacon_returns_no_content() {
    let app = build_router(AppState::in_memory().await.unwrap());
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/ndi/client-stats")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&json!({
                        "sourceId": "test-src",
                        "framesDecoded": 100,
                        "fps": 30.0,
                        "jitterBufferMs": 12.5,
                        "freezeCount": 0,
                        "framesDropped": 1
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p presenter-server ndi_client_stats_beacon_returns_no_content`
Expected: FAIL — 404 (route does not exist).

- [ ] **Step 3: Implement the handler** — in `crates/presenter-server/src/router/integrations/ndi.rs` (follow the file's existing imports/style):

```rust
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NdiClientStatsBeacon {
    pub source_id: String,
    pub frames_decoded: Option<f64>,
    pub fps: Option<f64>,
    pub jitter_buffer_ms: Option<f64>,
    pub freeze_count: Option<f64>,
    pub frames_dropped: Option<f64>,
}

/// Stage displays POST a compact getStats summary every 15s. Log-only (MVP):
/// journald keeps the history, so "the stage was laggy at 19:40" is
/// answerable from data (fps, jitter buffer, freezes per display).
pub(crate) async fn ndi_client_stats(
    axum::Json(beacon): axum::Json<NdiClientStatsBeacon>,
) -> axum::http::StatusCode {
    tracing::info!(
        source_id = %beacon.source_id,
        frames_decoded = beacon.frames_decoded,
        fps = beacon.fps,
        jitter_buffer_ms = beacon.jitter_buffer_ms,
        freeze_count = beacon.freeze_count,
        frames_dropped = beacon.frames_dropped,
        "NDI stage-display client stats beacon"
    );
    axum::http::StatusCode::NO_CONTENT
}
```

Register in `crates/presenter-server/src/router.rs` next to the other `/ndi/...` routes:

```rust
        .route(
            "/ndi/client-stats",
            post(integrations::ndi::ndi_client_stats),
        )
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p presenter-server ndi_client_stats_beacon_returns_no_content`
Expected: PASS.

- [ ] **Step 5: Commit (test first, then impl)**

```bash
git add crates/presenter-server/src/router/tests.rs
git commit -m "test(server): /ndi/client-stats beacon endpoint [red]"
git add crates/presenter-server/src/router/integrations/ndi.rs crates/presenter-server/src/router.rs
git commit -m "feat(server): /ndi/client-stats beacon endpoint (log-only observability)"
```

### Task 8: Snapshot observability — pushed/dropped + RTCP receiver stats (RED → GREEN)

**Files:**
- Modify: `crates/presenter-ndi/src/pipeline.rs` (SessionSnapshot fields)
- Modify: `crates/presenter-ndi/src/pipeline/consumers.rs` (snapshot gathering)
- Modify: `crates/presenter-ndi/src/pipeline/tests.rs`

- [ ] **Step 1: Write the failing test** (append to tests.rs):

```rust
/// /ndi/snapshot must expose per-consumer fanout counters and (when the
/// browser has sent RTCP RRs) round-trip/jitter/loss — the stage display's
/// own view of the link, readable server-side.
#[tokio::test]
async fn snapshot_includes_fanout_counters_and_rtcp_fields() {
    super::super::init().expect("gst init");
    let mut pipeline = NdiPipeline::stopped_for_test_with_topology("x264enc")
        .expect("test topology");
    pipeline.add_consumer_stub("snap-1").expect("stub consumer");
    let snap = pipeline.snapshot().await;
    assert_eq!(snap.sessions.len(), 1);
    let s = &snap.sessions[0];
    // Stub session pushed nothing — counters exist and are zero.
    assert_eq!(s.buffers_pushed, 0);
    assert_eq!(s.buffers_dropped, 0);
    // No RTCP from a stub webrtcbin — fields present as None (omitted in JSON).
    assert!(s.rtcp_round_trip_ms.is_none());
    let json = serde_json::to_string(&snap).unwrap();
    assert!(json.contains("buffersPushed"), "camelCase serialization: {json}");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p presenter-ndi snapshot_includes_fanout_counters_and_rtcp_fields`
Expected: COMPILE FAIL — `buffers_pushed` not a field of `SessionSnapshot`.

- [ ] **Step 3: Extend `SessionSnapshot`** in `crates/presenter-ndi/src/pipeline.rs`:

```rust
/// Per-consumer snapshot entry.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSnapshot {
    pub id: String,
    pub connection_state: WhepConnectionState,
    /// Buffers forwarded to this consumer by the StreamProducer link.
    pub buffers_pushed: u64,
    /// Buffers dropped because the consumer appsrc queue was full.
    pub buffers_dropped: u64,
    /// RTCP receiver-report round-trip time (ms) — the display's link RTT.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rtcp_round_trip_ms: Option<f64>,
    /// RTCP receiver-report interarrival jitter (ms).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rtcp_jitter_ms: Option<f64>,
    /// RTCP receiver-report cumulative packets lost.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rtcp_packets_lost: Option<i64>,
}
```

- [ ] **Step 4: Gather the data in `snapshot()`** (consumers.rs). Replace the session-snapshot collection with a two-phase gather — cheap fields under the lock, RTCP via `spawn_blocking` (the promise wait must not block the async thread):

```rust
    pub async fn snapshot(&self) -> PipelineSnapshot {
        // Phase 1 (cheap, under the lock): identity + counters + webrtcbin handle.
        let partial: Vec<(String, WhepConnectionState, u64, u64, gst::Element)> = {
            let sessions = self.sessions.lock().await;
            sessions
                .iter()
                .map(|(id, session)| {
                    let connection_state = *session
                        .connection_state
                        .lock()
                        .unwrap_or_else(|p| p.into_inner());
                    (
                        id.clone(),
                        connection_state,
                        session.link.pushed(),
                        session.link.dropped(),
                        session.webrtcbin.clone(),
                    )
                })
                .collect()
        };
        // Phase 2 (blocking): RTCP receiver-report stats per webrtcbin.
        let session_snaps: Vec<SessionSnapshot> = tokio::task::spawn_blocking(move || {
            partial
                .into_iter()
                .map(|(id, connection_state, pushed, dropped, webrtcbin)| {
                    let (rtt, jitter, lost) = rtcp_remote_inbound(&webrtcbin);
                    SessionSnapshot {
                        id,
                        connection_state,
                        buffers_pushed: pushed,
                        buffers_dropped: dropped,
                        rtcp_round_trip_ms: rtt,
                        rtcp_jitter_ms: jitter,
                        rtcp_packets_lost: lost,
                    }
                })
                .collect()
        })
        .await
        .unwrap_or_default();
        // ... (rest of the existing function body unchanged: iterate_encoders etc.)
```

And append the helper to consumers.rs:

```rust
/// Pull RTCP receiver-report stats (the browser's view of the link) from a
/// webrtcbin via its `get-stats` signal. Returns (rtt_ms, jitter_ms,
/// packets_lost); all None when no RTCP has arrived yet (e.g. pre-connect)
/// or the promise times out (500ms bound).
fn rtcp_remote_inbound(webrtcbin: &gst::Element) -> (Option<f64>, Option<f64>, Option<i64>) {
    let (tx, rx) = std::sync::mpsc::channel();
    let promise = gst::Promise::with_change_func(move |reply| {
        if let Ok(Some(stats)) = reply {
            let _ = tx.send(stats.to_owned());
        }
    });
    webrtcbin.emit_by_name::<()>("get-stats", &[&None::<gst::Pad>, &promise]);
    let Ok(stats) = rx.recv_timeout(std::time::Duration::from_millis(500)) else {
        return (None, None, None);
    };
    for (_field, value) in stats.iter() {
        let Ok(s) = value.get::<gst::Structure>() else {
            continue;
        };
        // The remote-inbound (RTCP RR) stats structure is the one carrying
        // round-trip-time; field layout is stable across GStreamer 1.2x.
        if s.has_field("round-trip-time") {
            let rtt = s.get::<f64>("round-trip-time").ok().map(|v| v * 1000.0);
            let jitter = s.get::<f64>("jitter").ok().map(|v| v * 1000.0);
            let lost = s
                .get::<i64>("packets-lost")
                .ok()
                .or_else(|| s.get::<u64>("packets-lost").ok().map(|v| v as i64))
                .or_else(|| s.get::<i32>("packets-lost").ok().map(i64::from));
            return (rtt, jitter, lost);
        }
    }
    (None, None, None)
}
```

NOTE: `add_consumer_stub` (tests.rs) constructs stub sessions — it must populate whatever the new snapshot path reads. It already stores a real `ConsumptionLink` and `webrtcbin`? Check its body; if the stub session's `link` is a `ConsumptionLink::disconnected(...)`, `pushed()/dropped()` return 0 and `rtcp_remote_inbound` on its unnegotiated webrtcbin returns `(None, None, None)` — exactly what the test asserts. If the stub stores something else, adapt the stub (NOT the production code) so the test compiles.

- [ ] **Step 5: Run tests**

Run: `cargo test -p presenter-ndi`
Expected: all PASS including the new snapshot test.

- [ ] **Step 6: Commit (test first, then impl)**

```bash
git add crates/presenter-ndi/src/pipeline/tests.rs
git commit -m "test(ndi): snapshot exposes fanout counters + RTCP fields [red]"
git add crates/presenter-ndi/src/pipeline.rs crates/presenter-ndi/src/pipeline/consumers.rs
git commit -m "feat(ndi): /ndi/snapshot exposes per-consumer pushed/dropped + RTCP RR stats"
```

### Task 9: Glass-to-glass latency e2e guard

**Files:**
- Create: `tests/e2e/ndi-latency.spec.ts`

- [ ] **Step 1: Write the spec** — mirror the harness of `tests/e2e/ndi-webrtc-synthetic.spec.ts` (same `startTestServer` + `discoverSyntheticSource` + `createAndActivateSource` helpers — copy them or import if exported; same `@video-codec @synthetic-ndi` tags so the self-hosted e2e-ndi lane picks it up and the GitHub-hosted lane skips it). Test body:

```typescript
// Glass-to-glass latency guard: the synthetic sender bakes Date.now()%2^24
// into a pixel strip (presenter_ndi::test_strip); this test decodes it from
// the rendered video via canvas per displayed frame. Sender and browser run
// on the SAME machine (self-hosted lane) -> same clock, true g2g latency.
//
// Bounds are deliberately generous for the shared runner (quiet-machine
// reality after the low-latency package is ~120-160ms median): a regression
// back to seconds-level latency or to growing-buffer behavior fails hard.
test("NDI glass-to-glass latency stays low for the synthetic source @video-codec @synthetic-ndi", async ({
  page,
  request,
}) => {
  const synthetic = await discoverSyntheticSource(request);
  expect(
    synthetic,
    "synthetic NDI source '(PRESENTER-TEST)' must be on the network — start ndi_test_sender",
  ).toBeTruthy();
  const src = await createAndActivateSource(request, synthetic!.name, "lat");

  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });
  await page.goto(new URL("/healthz", baseURL).toString());

  const result = await page.evaluate(
    async ({ origin, sourceId, seconds }) => {
      const pc = new RTCPeerConnection();
      pc.addTransceiver("video", { direction: "recvonly" });
      pc.addTransceiver("audio", { direction: "recvonly" });
      const video = document.createElement("video");
      video.muted = true;
      video.autoplay = true;
      video.playsInline = true;
      document.body.appendChild(video);
      const canvas = document.createElement("canvas");
      canvas.width = 1280;
      canvas.height = 720;
      const ctx = canvas.getContext("2d", { willReadFrequently: true })!;

      const samples: number[] = [];
      let badFrames = 0;
      let active = true;

      function decodeStrip(): number | null {
        ctx.drawImage(video, 0, 0, 1280, 720);
        const y = 36; // strip row centre after 2560->1280 downscale
        const luma = (i: number): number => {
          const x = 24 + i * 24 + 12;
          const d = ctx.getImageData(x - 1, y - 1, 3, 3).data;
          let s = 0;
          for (let p = 0; p < d.length; p += 4)
            s += 0.299 * d[p] + 0.587 * d[p + 1] + 0.114 * d[p + 2];
          return s / (d.length / 4);
        };
        const white = luma(0);
        const black = luma(1);
        if (white - black < 60) return null;
        const thr = (white + black) / 2;
        let val = 0;
        for (let bit = 0; bit < 24; bit++)
          val = (val << 1) | (luma(2 + bit) > thr ? 1 : 0);
        return val >>> 0;
      }

      function onFrame() {
        const embedded = decodeStrip();
        if (embedded === null) {
          badFrames++;
        } else {
          const now = Date.now() % (1 << 24);
          let d = now - embedded;
          if (d < -(1 << 23)) d += 1 << 24;
          if (d > 1 << 23) d -= 1 << 24;
          samples.push(d);
        }
        if (active) (video as any).requestVideoFrameCallback(onFrame);
      }
      pc.ontrack = (ev) => {
        if (ev.track.kind === "video") {
          video.srcObject = new MediaStream([ev.track]);
          video.play().catch(() => {});
          (video as any).requestVideoFrameCallback(onFrame);
        }
      };

      const offer = await pc.createOffer();
      await pc.setLocalDescription(offer);
      await new Promise<void>((res) => {
        if (pc.iceGatheringState === "complete") return res();
        const t = setTimeout(() => res(), 4000);
        pc.onicegatheringstatechange = () => {
          if (pc.iceGatheringState === "complete") {
            clearTimeout(t);
            res();
          }
        };
      });
      const resp = await fetch(`${origin}/ndi/whep/${sourceId}`, {
        method: "POST",
        headers: { "Content-Type": "application/sdp" },
        body: pc.localDescription!.sdp,
      });
      if (!resp.ok) return { error: `WHEP ${resp.status}` };
      let loc = resp.headers.get("Location");
      if (loc && !/^https?:/.test(loc)) loc = origin + loc;
      await pc.setRemoteDescription({ type: "answer", sdp: await resp.text() });

      await new Promise((r) => setTimeout(r, seconds * 1000));
      active = false;

      let freezeDurS = 0;
      (await pc.getStats()).forEach((s: any) => {
        if (s.type === "inbound-rtp" && s.kind === "video") {
          freezeDurS = s.totalFreezesDuration ?? 0;
        }
      });
      try {
        if (loc) await fetch(loc, { method: "DELETE" });
      } catch {}
      pc.close();

      samples.sort((a, b) => a - b);
      const pct = (p: number) =>
        samples.length
          ? samples[Math.min(samples.length - 1, Math.floor(samples.length * p))]
          : null;
      return {
        n: samples.length,
        badFrames,
        medianMs: pct(0.5),
        p95Ms: pct(0.95),
        freezeDurS,
      };
    },
    { origin: baseURL, sourceId: src.id, seconds: 20 },
  );

  expect((result as any).error, "WHEP connect must succeed").toBeUndefined();
  const r = result as {
    n: number;
    badFrames: number;
    medianMs: number;
    p95Ms: number;
    freezeDurS: number;
  };
  // 20s at 30fps -> expect hundreds of decoded strip samples.
  expect(r.n, `decoded strip samples (badFrames=${r.badFrames})`).toBeGreaterThan(300);
  expect(r.medianMs, "median glass-to-glass latency").toBeLessThanOrEqual(350);
  expect(r.p95Ms, "p95 glass-to-glass latency").toBeLessThanOrEqual(600);
  expect(r.freezeDurS, "total freeze duration").toBeLessThan(1.0);
  expect(consoleMessages).toEqual([]);
});
```

Use the same `test.beforeAll`/`afterAll` server lifecycle and port-allocation pattern as `ndi-webrtc-synthetic.spec.ts` (each spec file starts its own server). Deactivate/delete the created source in a `finally`/cleanup the same way that spec does.

- [ ] **Step 2: Run it locally**

```bash
cargo build -p presenter-server
cargo build -p presenter-ndi --features test-helpers --bin ndi_test_sender
NDI_RUNTIME_DIR_V6=/usr/lib/ndi PRESENTER_NDI_TEST_NAME="PRESENTER-TEST" \
  nohup ./target/debug/ndi_test_sender > /tmp/ndi_test_sender.log 2>&1 &
echo $! > /tmp/ndi_test_sender.pid
npx playwright test ndi-latency --grep "@synthetic-ndi" --project chrome-video --reporter=line
kill "$(cat /tmp/ndi_test_sender.pid)"
```

Expected: PASS with median well under 350 ms (record the printed numbers for the PR). If the machine is heavily loaded by other sessions, re-run when quiet — the assertion bounds tolerate moderate load only.

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/ndi-latency.spec.ts
git commit -m "test(e2e): glass-to-glass NDI latency guard via clock strip (#latency)"
```

### Task 10: Full local gate + single push + CI

- [ ] **Step 1: Format + lint + tests**

```bash
cargo fmt --all
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
bash scripts/dev/quality-check.sh
```

Expected: all clean. Fix anything that isn't BEFORE pushing.

- [ ] **Step 2: Local release measurement (evidence for the PR)**

```bash
cargo build --release -p presenter-server
# lab server on :8090 with the patched binary; clock sender already provides
# the strip via ndi_test_sender (PRESENTER-TEST), reuse /tmp/g2g_probe.mjs:
NDI_RUNTIME_DIR_V6=/usr/lib/ndi PRESENTER_NDI_TEST_NAME="PRESENTER-TEST" \
  nohup ./target/debug/ndi_test_sender > /tmp/sender.log 2>&1 & echo $! > /tmp/sender.pid
rm -f /tmp/latlab.db
PRESENTER_DB_URL="sqlite:///tmp/latlab.db?mode=rwc" PRESENTER_PORT=8090 \
  PRESENTER_ANDROID_ADB_BIN=true NDI_RUNTIME_DIR_V6=/usr/lib/ndi \
  setsid nohup ./target/release/presenter-server > /tmp/latlab.log 2>&1 &
sleep 6
# create+activate source for "BAKING-AI-5060 (PRESENTER-TEST)" via curl
# (same calls as in tests/e2e: POST /integrations/video-sources + /activate),
# then:
node /tmp/g2g_probe.mjs http://127.0.0.1:8090 <SOURCE_ID> 40
```

Record min/median/p95 — target: median ≤ 160 ms on a quiet machine. Compare against the pre-fix baseline measured 2026-06-11 (median 206 ms quiet deployed / 267-283 ms loaded lab). Then kill the lab server + sender (pid-targeted `kill`, never `pkill -f`).

- [ ] **Step 3: ONE push, monitor CI**

```bash
git push origin dev
gh run list --branch dev --limit 2
```

Then monitor with a single background command (the only allowed pattern):

```
Bash(command: "sleep 420 && gh run view <run-id> --json status,conclusion", run_in_background: true)
```

ALL jobs must go green (including e2e-ndi with the new latency spec and deploy-dev). Any failure: collect ALL errors, fix in ONE commit, push once, repeat.

### Task 11: Post-deploy verification on dev + PR

- [ ] **Step 1: Verify deploy-dev** — `curl http://10.77.8.134:8080/healthz` shows the new version; dev DB was wiped by deploy (BY DESIGN — see memory): re-create the video source mapping for the synthetic sender, activate, open `http://10.77.8.134:8080/stage` (layout `ndi-fullscreen` via `POST /stage/layout {"code":"ndi-fullscreen"}`) in Playwright, screenshot shows the strip pattern, console clean.

- [ ] **Step 2: Quiet-machine measurement on deployed dev** — `node /tmp/g2g_probe.mjs http://10.77.8.134:8080 <source_id> 60`: median ≤ 160 ms expected, trend flat.

- [ ] **Step 3: 30-min soak** — run the probe 3× (start / +15 min / +30 min, 60 s each, machine otherwise idle): medians must NOT grow (±30 ms band) and freezes stay 0. This is the drift-immunity check (receive-time PTS).

- [ ] **Step 4: Beacon + snapshot check** — `curl http://10.77.8.134:8080/ndi/snapshot/<source_id>` shows `buffersPushed` climbing and RTCP fields populated while a consumer is connected; `journalctl -u presenter-dev | grep "client stats beacon"` shows beacons from the stage page.

- [ ] **Step 5: Restore dev state** — deactivate the test source, re-create the user's `dd → RESOLUME-SNV (SP-live)` mapping if it was removed, restore stage layout if changed.

- [ ] **Step 6: PR** — `gh pr create` dev→main, title `feat(ndi): low-latency package — receive-time PTS, GOP 240 + join keyframe, sync=false producer, client jb hints, latency CI guard`, body references the spec, the measured numbers (before/after), and `Closes` nothing (no open issue for this — the work was user-prompted). Verify `mergeable: true` + `mergeable_state: "clean"`, provide URL, WAIT for explicit merge instruction.

- [ ] **Step 7 (after user merges — separate approvals)** — monitor main deploy; then with the user's go: 2-minute clock-source swap on prod (create+activate `clk-test` → sd1 screencap series via prod's adb → decode strip → expect ≤ 250 ms → re-activate original source + delete clk-test). Update the memory file `project_ndi_per_consumer_architecture.md` with the new invariants (sync=false, receive-time, GOP 240 + request_keyframe, jitterBufferTarget hint).

---

## Self-review notes

- Spec coverage: §1 pipeline (Tasks 1-2), §2 join+payloader (Tasks 3-4), §3 client (Task 6), §4 observability (Tasks 7-8), §5 CI guard (Tasks 5+9), acceptance criteria (Tasks 10-11). Strip geometry constants identical in Rust (`test_strip.rs`) and TS decoder (24 px blocks at 1280 = 48 px at 2560).
- The three load-bearing invariants (configure-before-PLAYING, Latency bus watch, await_media_caps) are UNTOUCHED — existing burst/straggler e2e guard them.
- Types: `request_keyframe(&StreamProducer)` used in Task 4 test and consumers.rs; `SessionSnapshot` fields match between Task 8 test and struct; `paint_strip(data, stride, now_ms)` matches sender call.
