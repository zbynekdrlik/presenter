# NDI Shared-Encoder Fanout Design

> Closes #336. Successor to #333/#334/#340 (per-source pipeline with whepserversink).

## Problem

The current per-source NDI pipeline ends in `whepserversink` (gst-plugin-rs 0.15), which by upstream design spawns one independent `vah264enc` / `nvh264enc` encoder per WHEP consumer. The N100 production host has limited VA-API concurrent encode slots (~2-3 H264 sessions before contention); 3-4 simultaneous stage-display browsers connecting to the same NDI source saturate the iGPU's VAAPI scheduler. The 2026-05-24 production incident (#333) was triggered by this multiplication. Resource limits (#335) and supervisor cool-off (#337) are defensive backstops; #336 is the actual fix to the multiplication itself.

The acceptance criteria are: ONE encoder element per NDI source regardless of consumer count, CPU under 50% on N100 with 4 concurrent consumers, the existing WHEP HTTP contract preserved end-to-end, and the existing `ndi-webrtc.spec.ts` Playwright suite continues to pass.

## Why `whepserversink` cannot share an encoder

Verified from upstream sources:

- gst-plugin-rs `webrtcsink` ([rswebrtc docs](https://gstreamer.freedesktop.org/documentation/rswebrtc/webrtcsink.html)): "When a consumer is added, its encoder / payloader / webrtcbin elements run in a separately managed pipeline." Encoder sharing is explicitly excluded so that webrtcsink can reserve per-consumer bitrate control for congestion adaptation.
- "Full control over the individual elements used by webrtcsink is not on the roadmap" — Centricular. Shared encoder is also not on the gst-plugin-rs roadmap.
- Pre-encoding upstream of `whepserversink` was attempted in 2026-05 and rejected: webrtcsink's internal h264parse refuses pre-encoded H264 with "broken/invalid nal Type: 1 Slice, Size: 8" (existing comment in `crates/presenter-ndi/src/pipeline.rs:96-103`).
- `webrtcbin2` (Centricular May-2026 devlog) is the long-term answer for scalable fanout but is "emerging" and not yet stable in gst-plugin-rs.

The realistic path is to drop down a level and build the WebRTC plumbing on top of bare `webrtcbin` elements, with our own WHEP HTTP signaller.

## Architecture

One GStreamer pipeline per active NDI source. The pipeline owns the encoder. Per-consumer state is a `WhepSession` that holds one `webrtcbin` element plus the `tee` request-pad it reads from. Sessions attach and detach dynamically as browsers POST and DELETE.

```
NDI source ──► ndisrcdemux ──► videoconvert ──► vah264enc ──► rtph264pay ──► tee
   (ndi-name)        │            (raw YUV)     (or nvh264enc                 │
                     │             one encoder   on dev2)                     │
                     │             per source)                                ├── src_0 ─► queue ─► webrtcbin ─► browser #1
                     │                                                        ├── src_1 ─► queue ─► webrtcbin ─► browser #2
                     │                                                        └── src_N ─► queue ─► webrtcbin ─► browser #N
                     │
                     └─► fakesink (audio — broadcaster may not send any; do NOT block preroll)
```

Per-consumer tee src pad + webrtcbin elements are added on WHEP POST and released on WHEP DELETE (or webrtcbin connection-state `Disconnected`/`Failed`). The encoder is steady-state for the lifetime of the pipeline.

## Modules

| File | Change | Responsibility |
|---|---|---|
| `crates/presenter-ndi/src/pipeline.rs` | Rewrite. Drop `whepserversink`. Build `ndisrc → demux → videoconvert → vah264enc → rtph264pay → tee` core. Add `add_consumer(sdp_offer) -> Result<WhepAnswer>` and `remove_consumer(session_id)`. | Owns the shared-encoder topology. Exposes a session-add/remove API. |
| `crates/presenter-ndi/src/whep_session.rs` | New. ~250 LoC. One `WhepSession` per consumer. Owns: `webrtcbin` element, the tee request-pad it reads from, the ICE candidate queue, and the SDP answer future. | Per-consumer state isolated from the shared pipeline. |
| `crates/presenter-ndi/src/manager.rs` | Trim. `WhepOp::Post/Patch/Delete` and `WhepReply` stay (HTTP-layer interface). Route to `pipeline.add_consumer` / `pipeline.remove_consumer` instead of `whep_signaller_call`'s `emit_by_name`. Remove `whep_signaller_call`. | Manager remains the per-source orchestrator. |
| `crates/presenter-server/src/router/integrations/ndi_whep.rs` | No change. HTTP route signatures already map to `WhepOp::Post/Patch/Delete` and return `WhepReply`. | HTTP contract unchanged. |
| `crates/presenter-ui/src/components/stage/ndi_video.rs` | No change. Browser-side WHEP client unaffected. | Client protocol unchanged. |
| `Cargo.toml` workspace deps | Add `gstreamer-webrtc = "0.25"` (matches the pinned `gstreamer = "0.25"`) for `WebRTCSDPType`, `WebRTCSessionDescription`, the on-negotiation-needed/on-ice-candidate signal helpers. Keep `gst-plugin-webrtc = "0.15"` only as long as a deprecated test references whepserversink; remove in a later cleanup PR if confirmed unused. | `webrtcbin` itself lives in `gstreamer-webrtc`, not in gst-plugin-rs. |

The HTTP shim and the UI component do not change. Everything downstream of `manager.rs` is a transparent rewrite.

## WHEP session lifecycle

The HTTP routes already accept WHEP HTTP verbs. The implementation behind each verb changes from "emit_by_name on whepserversink's signaller" to "call into the new `add_consumer` / `add_ice_candidate` / `remove_consumer` methods".

### POST `/ndi/whep/:source_id`

Browser sends SDP offer in body. Presenter:

1. Resolves `source_id` to the pipeline (404 if not active — preserve existing `SOURCE_NOT_ACTIVE_ERR` literal so the shim's string-match keeps mapping to 404).
2. Enforces the soft consumer cap (8 per source). Rejects with 503 + `Retry-After: 60` if over limit.
3. Calls `pipeline.add_consumer(sdp_offer)`:
   - Requests a new `tee.src_%u` pad.
   - Creates a fresh `webrtcbin` element with a generated session UUID as its name.
   - Adds webrtcbin to the running pipeline (state synced to the pipeline's PLAYING).
   - Adds a `queue` linking `tee.src_N → queue → webrtcbin.sink`.
   - Sets the remote description from the SDP offer.
   - Triggers `create-answer`, waits for the answer.
   - Sets the local description from the answer.
   - Subscribes to `on-ice-candidate` and pushes candidates onto a channel for the next `PATCH` reply (trickle ICE) AND onto the SDP answer's `a=candidate` lines until the gathering state reaches `complete` (or 1.5s timeout for half-trickle).
4. Returns 201 with headers `Location: /ndi/whep/:source_id/:session_id` and `Content-Type: application/sdp`, body = SDP answer.

### PATCH `/ndi/whep/:source_id/:session_id`

Browser sends ICE candidate(s). Presenter calls `pipeline.add_ice_candidate(session_id, candidate)` → `webrtcbin.emit("add-ice-candidate", mlineindex, candidate)`. Returns 204.

### DELETE `/ndi/whep/:source_id/:session_id`

Presenter calls `pipeline.remove_consumer(session_id)`:

1. Set the session's webrtcbin to `Null`.
2. Release the tee request-pad.
3. Remove webrtcbin from the pipeline.
4. Drop the WhepSession (its channels close cleanly).

Returns 204.

## Failure handling

- **Browser disconnect without DELETE:** webrtcbin emits `connection-state-change` to `Disconnected` or `Failed`. The session's state watcher triggers the same teardown path as DELETE — `remove_consumer` is idempotent.
- **NDI source dropout mid-session:** ndisrc fault → pipeline state → `Errored` → existing supervisor (unchanged) rebuilds the pipeline. All active sessions are dropped (their tee parent goes Null when the pipeline tears down). Browsers receive an ICE failure and must re-POST. Matches the current behavior under #337.
- **Encoder failure:** If `vah264enc` itself errors (rare, but possible under VAAPI scheduler contention), the pipeline goes Errored → supervisor rebuilds → encoder slot is freed by the pipeline teardown. The cool-off ceiling from #337 protects against thrash.
- **Soft consumer cap:** Hard limit at 8 concurrent sessions per source. POST #9 returns 503 with `Retry-After: 60`. Picked 8 because realistic church setups have ≤6 stage displays per source (choir, drums, vocals, side-screen-confidence, OBS browser source, plus headroom). Operators see the 503 in browser console — no silent failure.

## Encoder settings (one set per pipeline, applied once at build)

| Encoder | Setting | Value | Reason |
|---|---|---|---|
| `vah264enc` (N100 prod) | `key-int-max` | 30 | 1-second GOP — fast recovery from packet loss |
| `vah264enc` | `bitrate` | 2500 (kbit/s) | Reasonable for 1080p H264 over LAN; matches the current `x264enc` setting |
| `nvh264enc` (dev2) | `gop-size` | 30 | Same 1-second GOP |
| `nvh264enc` | `zerolatency` | true | Disables B-frames / look-ahead |
| `nvh264enc` | `bitrate` | 2500 | Same as vah264enc for parity |
| `x264enc` (test/dev fallback) | `tune` | zerolatency | Disables B-frames and look-ahead pipeline |
| `x264enc` | `speed-preset` | superfast | CPU sanity |
| `x264enc` | `key-int-max` | 30 | 1-second GOP |
| `x264enc` | `bitrate` | 2500 | Match other encoders |

The bitrate is fixed for all consumers — this is the trade for shared encoding. On a gigabit church LAN this is acceptable; per-consumer adaptation is a follow-up only if a real consumer-side bandwidth problem surfaces.

## Testing strategy

1. **Unit test (every CI host, no GPU):** Build a pipeline using `stopped_for_test()`-style harness; call `add_consumer` N times; assert the pipeline iterator yields exactly ONE element with factory name in `{vah264enc, nvh264enc, x264enc}`. Asserts the load-bearing invariant. RED commit first (skeleton `add_consumer` that creates an encoder per call), then GREEN commit with the shared-encoder build.
2. **Unit test (every CI host):** N add_consumer + M remove_consumer calls leave the tee with exactly N-M src pads and the pipeline with exactly N-M webrtcbin elements. Asserts cleanup.
3. **Unit test (every CI host):** soft-cap behavior — 9 add_consumer calls on a fresh pipeline return error on the 9th. Translates to 503 at the HTTP layer.
4. **Playwright `ndi-webrtc.spec.ts`:** unchanged — must pass.
5. **Playwright `ndi-webrtc-recovery.spec.ts`:** unchanged — supervisor behavior preserved.
6. **Playwright (new) `ndi-webrtc-fanout.spec.ts`:** open the same NDI source in 2 tabs simultaneously, assert both `<video>` elements reach `videoWidth > 0`, console errors === 0 on both, and `GET /ndi/snapshot/:source_id` reports exactly 1 encoder element + 2 webrtcbin elements. Runs on dev2 hosting an NDI source (via `ndi-stage-layout.spec.ts`-style fixture).
7. **Manual post-deploy verification (documented procedure, NOT in CI):** open 4 stage-display tabs on prod 10.77.9.205, watch `top -p $(pgrep presenter)` from SSH for 30 seconds, expect <50% CPU sustained. Documented in PR description with the actual numbers.

The RED-before-GREEN commit ordering (#336 is filed as a `bug` label per #333 series, so the regression-test-first hook will check it) is preserved by committing the failing unit test #1 first, then the implementation.

## Diagnostic surface

Add a `GET /ndi/snapshot/:source_id` route returning JSON:

```json
{
  "source_id": "uuid-...",
  "state": "Streaming",
  "encoder_factory": "vah264enc",
  "encoder_count": 1,
  "consumer_count": 2,
  "sessions": [
    { "id": "sess-aaa", "connection_state": "Connected" },
    { "id": "sess-bbb", "connection_state": "Connected" }
  ]
}
```

Used by Playwright fanout test for the assertion. Also useful for operator dashboards and incident debugging. Authenticated by the same path-permission rules as other `/ndi/*` routes (no new boundary).

## Out of scope (filed as follow-ups if real demand emerges)

- Audio passthrough — broadcaster compatibility is fragile; only worth the cost when a real source surfaces with audio we want to carry.
- Per-consumer bitrate adaptation — fixed bitrate is acceptable for LAN; revisit if church goes to WAN/remote consumers.
- Dynamic codec negotiation beyond H264 — every relevant browser supports H264; offering VP8/VP9 would require multiple encoders again.
- Migration to `webrtcbin2` — track upstream; revisit when the gst-plugin-rs release lands stable.

## Acceptance map

| Issue #336 criterion | Spec section |
|---|---|
| ONE encoder element with N consumers | Architecture diagram; testing unit test #1 |
| CPU < 50% on N100 with 4 consumers | Testing manual post-deploy verification (#7) |
| WHEP signalling protocol identical client-side | WHEP session lifecycle (POST/PATCH/DELETE shape unchanged) |
| Existing `ndi-webrtc.spec.ts` passes | Testing #4 |
| New regression test asserts single encoder element | Testing #1 |

## Single feature, single PR

Per `autonomous-batch-issue-development.md`: schema + module + route + UI + tests all ship together. Estimated diff: ~+600 LoC added (pipeline rewrite + whep_session.rs + tests + snapshot route), ~-300 LoC removed (whep_signaller_call + whepserversink-specific code), net ~+300 LoC. Fits comfortably in one PR.

## Risk register

- **WHEP signalling subtleties (mid-trickle ICE, late-PATCH races, browser-vendor-specific SDP quirks):** webrtcsink's internal signaller handles a long tail of these. Hand-rolling means we may surface a bug a real consumer browser triggers. Mitigation: keep the surface area small (POST/PATCH/DELETE only, no PUT, no auth), test with Chromium (Playwright) AND a manual smoke against the actual Android Fully Kiosk stage displays before merge.
- **`tee` request-pad / `webrtcbin` lifecycle in async Rust:** webrtcbin is non-Send. Pattern: do all signal connections + pad linking in a `tokio::task::spawn_blocking` block, then communicate via channels. Same pattern the existing `whep_signaller_call` uses.
- **Encoder failure under sustained load:** unverified on prod until deployed. The cool-off ceiling from #337 is the backstop; if encoder failures become a recurring symptom, file a follow-up to investigate VAAPI driver tuning (e.g. `LIBVA_DRIVER_NAME`, `intel_media` vs `i965`).
