# NDI → Stage WebRTC: ICE never connects (black/white stage screen)

**Date:** 2026-06-08
**Status:** Approved (design), implementation via TDD
**Severity:** 🔴 production regression — NDI video on stage displays does not work at all
**Related:** regression from #336 (shared-encoder fanout rewrite, `615b4d7`)

---

## ⚠️ Implementation correction — ACTUAL root cause (found via TDD on live dev, real Google Chrome)

The ICE/DTLS/payload-type hypotheses below were **not** the real break. Verified
against a real Google Chrome (the bundled Playwright Chromium has no H.264, so it
never exercised the H.264 path — a major source of confusion):

- ICE **connected**, DTLS **completed** (`connectionState: connected`), and the
  SDP answer was correct **H.264 sendonly**. The connection was fine.
- The break was **media flow**: `add_consumer` linked the per-consumer branch
  into the shared `tee` **after** the branch was already PLAYING (the "defer the
  tee link" approach). The new tee src pad then never received the tee's sticky
  events, so it forwarded **zero RTP** → connected but black.
- The break manifested **only when an audio m-line was also negotiated**
  (video + audio, what the real client sends). A **video-only** offer happened to
  decode frames even on the broken build — which is why the `e2e-ndi` guard was
  GREEN while every real browser was black. **The test used video-only.**

**Fix:** link the tee → branch while the branch is still NULL and bring the whole
branch to PLAYING *afterwards*, so sticky events propagate during the transition
(`consumers.rs`). Plus: per-consumer `leaky=downstream` queue (a dead consumer
can't back-pressure the shared tee); the watchdog no longer reconnects before the
first frame renders (`ndi_video.rs`); and the regression test now offers
**video + audio** as the real client does (`ndi-webrtc-synthetic.spec.ts`).

**Verified:** single stage display renders real video; two **different-IP**
consumers (TV + laptop) both render simultaneously (the shared-encoder fanout
works). Regression test RED on the broken build, GREEN on the fix.

## Problem

After the #336 shared-encoder fanout work merged (CI green), the NDI-video-to-stage
feature is completely broken in production:

- Stage display TV: white background instead of video.
- Laptop Chrome: black background instead of video.

The encoder side is healthy (`/healthz` reports the pipeline `streaming`), so the
break is in browser delivery.

## Root cause (proven on live dev with a real browser)

The WebRTC media path never establishes because **ICE never connects** — neither
peer ever receives the other's ICE candidates:

1. **Server → browser candidates are lost.** `NdiPipeline::add_consumer`
   (`crates/presenter-ndi/src/pipeline/consumers.rs`) returns the SDP answer
   immediately after `create-answer`, draining ICE candidates from `webrtcbin`
   for only 50 ms. In practice it drains **zero**, and the few it could collect
   are then **dropped**: `NdiManager::whep_signaller_call`
   (`crates/presenter-ndi/src/manager/whep.rs`, `WhepOp::Post`) puts only
   `answer.sdp_answer` in the reply body and never sends `initial_candidates`.
   Measured: WHEP answer SDP contains `0` `a=candidate` lines.

2. **Browser → server candidates are never sent.** The WASM client
   `connect_whep` (`crates/presenter-ui/src/components/stage/ndi_video.rs`)
   registers no `onicecandidate` handler and never PATCHes trickle candidates to
   the WHEP resource. Measured: browser gathers candidates but issues **zero**
   PATCH requests (network log shows POST/DELETE only).

Result, measured against the live streaming dev pipeline:
`iceConnectionState` stays `"new"` indefinitely → no DTLS → no SRTP → no frames →
`<video>` stuck at `readyState=0`, `videoWidth=0`, `currentTime=0` → black/white
screen. The browser watchdog then loops POST → 3 s stall → DELETE → POST forever.

Codec and signaling are fine (`H264/90000`, `201 Created`) — which is why the bug
looked healthy from the outside.

The pre-#336 path used `whepserversink`, which handled the full ICE exchange
internally. The #336 rewrite to hand-rolled per-consumer `webrtcbin` (to enable
one-encoder-N-consumers fanout) dropped ICE wiring in both directions.

## Why CI stayed green (the gap to close)

The Playwright E2E job (`pipeline.yml` → `e2e`) runs on `ubuntu-latest`, which has
no NDI SDK and no NDI sources on its network. Every real-frame test in
`tests/e2e/ndi-webrtc.spec.ts` is gated with
`test.skip(!available / sources.length === 0, …)` and therefore **never runs in
CI**. The single NDI test that does run accepts `503` and never asserts a decoded
frame. Net: **zero real-video coverage executes on any PR**. This is a
skip-in-disguise that `test-strictness.md` forbids.

## Fix — full ICE gather on both sides (LAN host candidates, no STUN/TURN)

The deployment is LAN-only (stage TVs + operator laptops on the church network),
so host ICE candidates are sufficient and gathering completes in well under a
second. Use non-trickle (gather-then-send) on both peers — no PATCH plumbing
needed:

1. **Server** — `NdiPipeline::add_consumer`
   (`crates/presenter-ndi/src/pipeline/consumers.rs`):
   after `create-answer` + `set-local-description`, wait for `webrtcbin`'s
   `ice-gathering-state` to reach `COMPLETE`, then read the final
   `local-description` SDP (now carrying `a=candidate` lines) and return **that**
   as the WHEP answer. Bound the wait with a timeout (e.g. 5 s) and a structured
   warning on timeout. The `initial_candidates`/50 ms-drain mechanism is removed.

2. **Browser** — `connect_whep`
   (`crates/presenter-ui/src/components/stage/ndi_video.rs`):
   after `createOffer` + `setLocalDescription`, await
   `iceGatheringState === 'complete'` (via `icegatheringstatechange` /
   end-of-candidates) **before** POSTing the offer, so the offer SDP carries the
   browser's candidates.

Outcome: offer has browser candidates, answer has server candidates → ICE
connects → frames decode. The existing server PATCH handler may remain for WHEP
spec-compliance but is not required for connectivity.

Out of scope: WAN/remote stage viewers over the Cloudflare tunnel would need a
TURN server (host candidates do not route off-LAN). Tracked separately if needed.

## TDD plan

1. **RED (server, runs anywhere with libnice):** a Rust test asserting the SDP
   answer returned by `add_consumer` for a streaming pipeline contains at least
   one `a=candidate` line. Fails on current code.
2. **RED (E2E, real frames):** a Playwright test that, against a live streaming
   NDI source on a GPU+NDI host, asserts `videoWidth > 0 && readyState >= 2 &&
   currentTime` advances. Fails on current code; must **not** skip when NDI is
   available.
3. **GREEN:** apply the two-sided ICE fix; both tests pass.

## CI improvement — synthetic NDI lane on the self-hosted (dev2) runner

`ndisink`/`ndisinkcombiner` are compiled into the project's statically-registered
gst NDI plugin (`gst-plugin-ndi` default feature `sink`). Add a small Rust
test-helper that registers the plugin and publishes a deterministic synthetic
source (`videotestsrc → ndisinkcombiner → ndisink ndi-name="PRESENTER-TEST"`).

Add a new `e2e-ndi` job on `runs-on: self-hosted` (the dev2 runner with NDI SDK +
NVENC) that:

1. Starts the synthetic NDI sender helper (publishes `PRESENTER-TEST`).
2. Runs the presenter, creates+activates a `PRESENTER-TEST` source, sets
   `ndi-fullscreen`.
3. Runs the real-frame Playwright test in branded Chrome (H.264) and asserts
   frames decode.

This test is **required**, not skipped — when NDI is expected (self-hosted lane)
and frames don't flow, the job fails. Deterministic: no dependency on Resolume
being broadcast.

## Verification

After implementation, prove end-to-end on dev with a real browser:
WHEP answer SDP has `a=candidate` ≥ 1, `iceConnectionState` reaches `connected`,
`videoWidth > 0`, `readyState >= 2`, `currentTime` advancing, 0 console errors.
Then deploy and re-verify on production.
