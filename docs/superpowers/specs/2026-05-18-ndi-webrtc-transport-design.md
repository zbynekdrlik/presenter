# NDI WebRTC Transport — Design

**Issue:** TBD (to be filed after this spec is approved). Successor work to [#250](https://github.com/zbynekdrlik/presenter/issues/250) (MJPEG ladder) and originally planned in [#167](https://github.com/zbynekdrlik/presenter/issues/167) (NDI Stage Display) before being descoped to MJPEG.

**Date:** 2026-05-18
**Status:** Proposed

## Problem

The current NDI transport — single fixed-tier MJPEG at 720p @ 20 fps (#250, shipped in PR #266) — is unwatchable on production stage displays:

- **Latency too high** for live-camera use (operator on stage sees self with visible lag).
- **Quality too low** even at the stabilized 20 fps. Software JPEG decode on cheap Android TVs is CPU-bound and produces dropped frames.
- **No audio path.** Current receiver explicitly passes `null_mut()` for audio; stage displays have no way to hear the source.
- **No layout flexibility.** Single-stream MJPEG works for fullscreen NDI; multi-NDI multiview or PiP needs a different transport model.

Previously, a workaround chain (NDI → OBS → browser source loading VDO.Ninja sender page → OBS re-publishes as NDI → VDO.Ninja viewer in browser) delivered acceptable latency and quality. The chain was operationally complex (OBS+VDO web pages, second box for OBS) and is the thing this design replaces with a native, in-presenter path.

The #250 closing note explicitly committed the next iteration to WebRTC / low-latency HLS. This spec is that next iteration.

## Goals

1. **Sub-300 ms LAN latency** end-to-end (NDI source frame → glass on stage TV).
2. **HW H264 encode on N100** via VAAPI (`vah264enc`). Server CPU < 15 % sustained for 1 × 720p30.
3. **HW H264 decode on Android TV** via Chromium WebView native pipeline.
4. **Audio support** out of the box, AV-synced by the browser.
5. **Browser-side compositing**: each NDI source is its own `<video>` element. Layouts iterate as CSS without touching the server pipeline. Multi-NDI multiview, PiP, lyrics-over-video — all driven by frontend changes alone.
6. **One transport, replace MJPEG entirely.** No dual-stack to maintain.
7. **100 % open source, Rust-native where the ecosystem provides it.** No proprietary SDKs beyond the existing libndi (which already ships free from NDI / Vizrt).

## Non-goals

- Multi-server / multi-site streaming (this is LAN-only).
- Recording the WebRTC output (separate concern, not in scope).
- Transcoding NDI to multiple bitrates per client (single bitrate per source, one encode).
- STUN/TURN / cross-NAT — LAN-only host candidates suffice.

## Stack

The Rust ecosystem (as of 2026-05) does not have a single pure-Rust crate providing both WebRTC protocol and HW-accelerated H264 encode. The cleanest path that meets every goal is the official `gst-plugins-rs` toolkit, which is itself Rust and provides both pieces as GStreamer elements.

| Layer | Choice | Why |
|-------|--------|-----|
| NDI capture | `gst-plugin-ndi` (Rust, `gst-plugins-rs/net/ndi`) | Gives video + audio in one element. Replaces our custom `ndi_sdk.rs` + `receiver.rs`. Uses the same `libndi.so.6` already deployed. |
| H264 encode | `vah264enc` (GStreamer VA plugin) | Auto-selected via GStreamer feature rank. Uses N100's Intel Iris Xe via `/dev/dri/renderD128`. ~5–8 % of one core for 720p30. |
| Opus audio encode | `opusenc` (GStreamer good plugins) | Standard, fast, browser-friendly. |
| WebRTC server | `webrtcsink` (Rust, `gst-plugins-rs/net/webrtc`) | "Batteries-included send-only" element. Handles RTP packetization, DTLS-SRTP, ICE, codec negotiation, and Google Congestion Control. Multiple subscribers natively. Ships a WHIP/WHEP signaller. |
| Rust binding layer | `gstreamer-rs` | Official mature Rust bindings to GStreamer. Compiles into the presenter binary. |
| Pure-Rust alternatives considered | `str0m`, `webrtc-rs` | Both are protocol-only (no encoder). Combining either with `gstreamer-rs` for HW encode = more glue code than `webrtcsink` already provides. Re-evaluate when one of them ships an integrated encoder. |

All chosen pieces are open-source Rust crates we can read, fork, or contribute to. No proprietary frameworks.

## Architecture

### Per-source pipeline

One GStreamer pipeline per active NDI source. Pipelines are independent (a failure in one does not affect others).

```
ndisrc(ndi-name=STREAM-SNV)
  ! ndisrcdemux name=demux
demux.video
  ! videoconvert
  ! vah264enc bitrate=2000 key-int-max=60 rate-control=cbr
  ! video/x-h264,profile=baseline
  ! sink.video_0
demux.audio
  ! audioconvert ! audioresample
  ! opusenc bitrate=64000
  ! sink.audio_0
webrtcsink name=sink signaller::uri=<internal WHEP URL>
```

Built in Rust via `gstreamer-rs` `gst::parse::launch` (or programmatically for finer error handling).

### Pipeline state machine

- **Idle**: source row exists in DB, `is_active = false`. No pipeline.
- **Starting**: operator activated source. Pipeline built, `ndisrc` connecting.
- **Streaming**: first `GST_MESSAGE_ASYNC_DONE` received. WHEP endpoint accepts subscribers.
- **Stopping**: operator deactivated, or NDI source lost > 3 s. Pipeline torn down.

Transitions emit `LiveEvent::NdiConnectionStatus` over WS so the stage display can show "connecting" / "streaming" / "disconnected" overlays.

### Subscriber model

`webrtcsink` tracks each browser subscriber internally. From presenter's perspective the only HTTP surface is:

- `POST /ndi/whep/{source_id}` — body is the browser's SDP offer; presenter forwards to the source's `webrtcsink` default signaller; answer SDP is returned in the HTTP response body.
- `GET /ndi/whep/{source_id}` — returns the source's current SDP (used for reconnect by browsers that previously cached one).
- `DELETE /ndi/whep/{source_id}/{client_id}` — explicit subscriber teardown (optional; ICE timeout handles it otherwise).

Multiple subscribers per source = handled inside `webrtcsink`. Presenter only thinly proxies the signaling HTTP.

### Browser-side layout composition

Each active NDI source = one `<video>` element in the layout, bound to the source's WHEP endpoint. Layout iteration is pure frontend work:

- Fullscreen single source: one `<NdiVideo source_id="..." />`.
- Picture-in-picture: two `<NdiVideo>` components positioned by CSS.
- Quad-view multiview: four `<NdiVideo>` components in a 2×2 grid.
- Lyrics-over-video: one `<NdiVideo>` background plus an overlay div.

No server-side compositor. No fixed list of "supported layouts". The operator can add a new layout component to `presenter-ui/src/components/stage/` and use any combination of `<NdiVideo>` plus other elements.

The WASM component:

```rust
#[component]
pub fn NdiVideo(source_id: String) -> impl IntoView {
    let video_ref = NodeRef::<Video>::new();
    Effect::new(move |_| {
        let video = video_ref.get().expect("video el");
        spawn_local(connect_whep(video, source_id.clone()));
    });
    view! { <video node_ref=video_ref autoplay muted playsinline /> }
}
```

`connect_whep` is ~80 LoC of WASM: create `RtcPeerConnection`, add recvonly video/audio transceivers, POST the SDP offer to `/ndi/whep/{source_id}`, set the answer, hook `ontrack` to set `video.srcObject`. ICE/DTLS/jitter-buffer/AV-sync = browser native.

## Capacity budget

N100 (4 cores, Intel Iris Xe with 24 EU, VAAPI via `/dev/dri/renderD128`).

| Concurrent active NDI sources | VAAPI GPU load | CPU load |
|-------------------------------|----------------|----------|
| 1 × 720p30 | ~6 % | ~3 % |
| 2 × 720p30 | ~12 % | ~5 % |
| 3 × 720p30 | ~18 % | ~8 % |
| 4 × 1080p30 | ~35 % | ~12 % |

Soft ceiling: 4 concurrent active sources. Settings UI warns above this; no hard cap.

Browser side (cheap Hyundai Android TV): empirically verified for 1 × 720p H264 (single fullscreen). 2 × 720p simultaneous expected fine; 4 × 720p uncertain on the cheapest TV. Empirical test = first phase of the implementation plan.

## Module changes

### `presenter-ndi` crate (rewrite)

- **Keep**: `discovery.rs` (NDI source listing for Settings UI — uses libloading FFI to `NDIlib_find_*`).
- **Delete**: `ndi_sdk.rs` (custom FFI receive path), `receiver.rs` (custom frame loop), `encoder.rs` (turbojpeg).
- **New**: `pipeline.rs` (per-source GStreamer pipeline state machine).
- **Rewrite**: `manager.rs` (owns pipelines, exposes WHEP HTTP shim).

Net: ~900 LoC removed, ~400 added.

### `presenter-server`

- `GET /ndi/sources` — unchanged.
- `GET /ndi/mjpeg`, `WS /ndi/mjpeg` — **removed**.
- `POST /ndi/whep/{id}` — **new**.
- `GET /ndi/whep/{id}` — **new**.
- `DELETE /ndi/whep/{id}/{client_id}` — **new** (optional teardown).
- `POST /integrations/video-sources/{id}/activate` — start GStreamer pipeline (was: start MJPEG receiver+encoder).
- `POST /integrations/video-sources/deactivate` — tear down pipeline.

WS events `NdiSourceActivated` / `NdiSourceDeactivated` / `NdiConnectionStatus` — same contract, emitted from pipeline state machine.

### `presenter-ui`

- New: `presenter-ui/src/components/stage/ndi_video.rs` — WHEP-connecting `<video>` component.
- Modified: `ndi_fullscreen.rs`, `api_stage.rs`, `timer_layout.rs` — swap `<img src=/ndi/mjpeg>` for `<NdiVideo source_id=...>`.
- Modified: `presenter-ui/src/api/ndi.rs` — drop MJPEG URL builder, add WHEP URL builder.

## Deploy infra

Add to `deploy.yml` (and matching prod / dev / PP deploy paths):

```
apt-get install -y \
  gstreamer1.0-vaapi \
  gstreamer1.0-plugins-base \
  gstreamer1.0-plugins-good \
  gstreamer1.0-plugins-bad \
  intel-media-va-driver-non-free \
  libva-drm2 libva2
```

`libndi.so.6` at `/usr/lib/ndi/` is unchanged.

`gst-plugin-ndi` and `gst-plugin-webrtc` are Rust crates from `gst-plugins-rs`; they compile into the presenter binary via cargo — no separate apt package needed for those.

`systemd` service file unchanged (still a single `presenter` binary).

## Failure modes

| Failure | Behavior |
|---------|----------|
| `vah264enc` unavailable at startup (driver / kernel issue) | Log ERROR loudly. NDI WebRTC feature disabled. Settings UI hides "Activate" buttons with a reason banner. Operator must fix driver. **No silent software-encode fallback** — N100 software 720p H264 = ~150 % CPU = melts the box. |
| `gstreamer1.0-vaapi` package missing | Detected at startup probe (`gst-inspect-1.0 vah264enc` returns nothing). Same disablement path as above. |
| NDI source drops (no frames > 3 s) | Pipeline stays alive; `ndisrc` auto-reconnects. Browser sees frozen frame, then resumes. `NdiConnectionStatus` WS event emits "disconnected" → "streaming". |
| Browser disconnects | `webrtcsink` GC's the subscriber. Pipeline continues serving other subscribers. ICE timeout = 30 s. |
| Pipeline crash | Manager state machine catches `GST_MESSAGE_ERROR`, tears down pipeline, transitions to Idle, logs root cause. Operator must manually re-activate (no auto-recovery on hard pipeline error — those are real bugs to investigate). |
| Multiple WHEP signaling races | webrtcsink default signaller serializes; concurrent POSTs to the same `/ndi/whep/{id}` get separate subscribers. Idempotency not needed. |

## Migration / cutover

Single PR (dev → main). No feature flag. Steps:

1. Add `gstreamer`, `gstreamer-app`, `gstreamer-base`, `gst-plugin-ndi`, `gst-plugin-webrtc` to `Cargo.toml`.
2. Write new `pipeline.rs` (per-source pipeline state machine).
3. Rewrite `manager.rs` to drive pipelines.
4. Add WHEP HTTP routes; remove MJPEG HTTP/WS routes.
5. Write new `<NdiVideo>` WASM component; swap into existing layouts.
6. Delete `ndi_sdk.rs`, `receiver.rs`, `encoder.rs`, MJPEG router code, MJPEG frontend code.
7. Update Playwright E2E (replace MJPEG header assertions with WebRTC connect + `videoWidth > 0` assertions).
8. Update deploy workflows to apt-install the GStreamer + VA-API packages.
9. CI green → merge dev → main → deploy to prod.
10. Operator verifies on real stage TVs (sd1l–sd4l) with STREAM-SNV.

## Verification surfaces

- **Pipeline start**: Playwright triggers activate, asserts WS event `NdiSourceActivated`, asserts WHEP endpoint returns 200 on probe.
- **Video flows**: Playwright `<video>` element shows `videoWidth > 0` within 3 s of mount, zero browser console errors.
- **HW encode actually used**: presenter logs `vah264enc` at startup; `gst-inspect-1.0 vah264enc` available on the box.
- **Multi-source**: two sources active concurrently, both WHEP endpoints serve, browser plays both.
- **Source loss recovery**: stop the NDI source, wait 5 s, restart, assert browser reconnects.
- **AV sync**: manual operator verification at the church (lips test on prod).
- **CPU budget**: `systemd-cgtop presenter.service` shows < 15 % sustained during 1 × 720p30.

## Named risks

1. **`gst-plugin-ndi` audio path maturity** — some NDI sources mark audio as "skip" frames. Verify with STREAM-SNV (10.77.9.204:5961) early in implementation. Fallback if buggy: pull audio via a parallel custom receiver.
2. **Cheap Hyundai TV decode** — confirmed for 1 × 720p, untested for multi-stream layouts. First implementation milestone: single fullscreen NDI via WebRTC on sd2l (cheapest Hyundai). If solid, proceed to multi-view layouts.
3. **WebRTC ICE on LAN with multiple subscribers** — webrtcsink uses host-only candidates on LAN (no STUN). UDP only. Our env (Fully Kiosk WebView on cheap Android TV) is the unknown; hello-world WHEP stream test first, before any production layout swap.
4. **Codec parameter tuning** — `vah264enc bitrate=2000 key-int-max=60 baseline` is a starting point. May need GOP tweaks or rate-control tuning for cheap TV decoders. Iteration cost is minutes (re-launch pipeline).
5. **Fully Kiosk WebRTC support on the cheapest TVs** — Android System WebView version on the cheapest Hyundai may predate solid WebRTC support. Fallback option (not in this design): swap kiosk to a native Android TV app embedding ExoPlayer with a WHEP client.

## Out of scope (deferred)

- Recording the WebRTC stream (separate concern).
- WAN / cross-NAT streaming (would need STUN/TURN; out of scope for LAN-only church setup).
- Multi-bitrate / simulcast per source (one bitrate per encode is enough for LAN).
- Native Android TV app replacement of Fully Kiosk (only if risk #5 materializes).
- Visual layout iteration design (PiP positions, quad-view geometry, lyrics overlay placement) — that work uses the visual companion in a follow-up brainstorm.
