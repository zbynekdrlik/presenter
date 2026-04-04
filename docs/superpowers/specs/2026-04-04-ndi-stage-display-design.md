# NDI Stage Display — Design Spec

**Issue:** #167  
**Date:** 2026-04-04  
**Status:** Approved

## Summary

Add built-in NDI video support to the stage display. Presenter receives NDI streams from the network, encodes them, and serves them to stage display browsers via WebRTC. A new `ndi-fullscreen` layout preset shows the video stream full-viewport with status bar overlay.

## Architecture

### New Crate: `presenter-ndi`

A new workspace crate encapsulating all NDI and WebRTC functionality:

```
crates/presenter-ndi/
  src/
  ├── lib.rs           — public API, NdiManager
  ├── ndi_sdk.rs       — libloading FFI to NDI SDK (libndi.so)
  ├── discovery.rs     — NDI source discovery (find/list on network)
  ├── receiver.rs      — NDI frame receiver (video + audio)
  ├── encoder.rs       — H.264 (openh264) + Opus encoding
  ├── webrtc.rs        — WebRTC session management (webrtc-rs)
  └── whep.rs          — WHEP HTTP endpoint for signaling
```

### Data Flow

```
NDI Source (camera/OBS/vMix)
  → NDI SDK via libloading (receive raw frames + audio)
  → openh264 encode (video) + opus encode (audio)
  → webrtc-rs (RTP packetization, DTLS-SRTP)
  → WebRTC over UDP on LAN
  → Browser <video> element on stage display
```

### Dependencies (Rust crates)

- `libloading` — dynamic loading of NDI SDK at runtime (pattern from camera-box)
- `openh264` — H.264 video encoding (Cisco BSD-licensed codec)
- `opus` — Opus audio encoding
- `webrtc` (webrtc-rs) — pure Rust WebRTC stack (ICE, DTLS, SRTP, RTP)

### Runtime Dependencies

- **On Presenter server:** `libndi.so` (NDI SDK runtime, free download from ndi.video). Loaded at runtime. If absent, NDI features are gracefully disabled.
- **On stage display browser:** Nothing extra. Standard WebRTC API.

## Database

### Table: `video_sources`

```sql
CREATE TABLE video_sources (
  id         TEXT PRIMARY KEY,  -- UUID
  label      TEXT NOT NULL,     -- User-friendly name ("Main Camera")
  ndi_name   TEXT NOT NULL,     -- NDI source name ("CAM1 (usb)")
  is_active  BOOLEAN DEFAULT 0, -- Currently selected for display
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
```

Only one source can have `is_active = true` at a time. Activating one deactivates all others.

## HTTP API

| Method | Path | Purpose |
|--------|------|---------|
| GET | `/ndi/sources` | Discover NDI sources on network (live scan) |
| POST | `/ndi/whep` | WebRTC signaling (SDP offer → answer) |
| GET | `/integrations/video-sources` | List configured video sources from DB |
| POST | `/integrations/video-sources` | Create video source {label, ndi_name} |
| PUT | `/integrations/video-sources/:id` | Update video source |
| DELETE | `/integrations/video-sources/:id` | Delete video source |
| POST | `/integrations/video-sources/:id/activate` | Set as active (starts NDI receive + WebRTC) |
| POST | `/integrations/video-sources/deactivate` | Stop NDI receive (no active source) |

## WebSocket Events

New `LiveEvent` variants broadcast to all connected stage displays:

- `NdiSourceActivated { ndi_name, label }` — stage displays with NDI region connect to `/ndi/whep`
- `NdiSourceDeactivated` — stage displays stop video, show placeholder
- `NdiConnectionStatus { status }` — "connecting" | "streaming" | "disconnected"

## Source Lifecycle

```
No active source ──activate──→ Connecting to NDI ──frames──→ Streaming
Streaming ──deactivate──→ No active source
Streaming ──activate other──→ Connecting to new NDI (old stops, new starts)
Connecting ──NDI lost──→ Disconnected ──auto-retry 5s──→ Connecting
```

## Layout

### ndi-fullscreen (v1 — only layout)

Full viewport NDI video with status bar overlay at bottom. The `<video>` element fills the entire stage display area. Status bar (clock, live indicator, connection status) overlays at the bottom, same as existing layouts.

### Browser-side WebRTC flow

1. Stage page loads with `ndi-fullscreen` layout selected
2. WASM component creates `RTCPeerConnection`
3. Sends SDP offer via `POST /ndi/whep`
4. Receives SDP answer
5. WebRTC connection established (UDP on LAN)
6. `<video>` element plays the stream (autoplay, muted initially per browser autoplay policy; user tap unmutes audio)
7. On `NdiSourceActivated` WS event → reconnect to new stream
8. On `NdiSourceDeactivated` → show placeholder

## Settings UI

New "Video Sources" section in Settings page:

- List of configured sources with label, NDI name, active status
- **Add Source** button — dialog with label (free text) + NDI name (dropdown from network scan or free text)
- **Scan Network** button — triggers `/ndi/sources` to discover available NDI sources
- **Activate** button per source — sets it as active, starts streaming
- **Edit / Delete** per source
- Section hidden entirely when NDI SDK is not available

## Graceful Degradation

### NDI SDK not installed
- `libloading` fails to find `libndi.so` at startup
- NDI features disabled (log warning)
- `/ndi/*` endpoints return 503 with message
- Settings UI hides Video Sources section
- `ndi-fullscreen` layout hidden from operator dropdown
- Presenter works normally for all other features

### NDI source goes offline
- Receiver detects no frames for 3 seconds
- Status → "Disconnected"
- WebSocket broadcasts `NdiConnectionStatus { status: "disconnected" }`
- Stage display shows "Signal Lost" overlay
- Auto-retry connection every 5 seconds
- When source returns: auto-reconnect, resume stream

### No active source selected
- Stage shows "No video source configured" placeholder
- No WebRTC connection attempted

### Browser WebRTC failure
- Retry with exponential backoff
- "Connecting..." overlay on video region

## Integration Points

### presenter-server
- `AppState` holds `NdiManager` (optional, `None` if SDK unavailable)
- New route module for `/ndi/*` and `/integrations/video-sources/*` endpoints
- `NdiManager` manages: source discovery, receiver lifecycle, encoder, WebRTC sessions

### presenter-ui (WASM)
- New layout component: `NdiFullscreen` in `components/stage/`
- WHEP client using browser WebRTC API (`web_sys::RtcPeerConnection`)
- Settings page: new Video Sources section
- Operator header: `ndi-fullscreen` in layout dropdown (conditionally, only when NDI available)

### presenter-core
- New `StageDisplayLayout` entry: `ndi-fullscreen` / "NDI FULLSCREEN"
- New `VideoSource` model struct

## Latency Target

< 500ms end-to-end on LAN, comparable to VDO.Ninja. Achieved via:
- WebRTC UDP transport (no buffering)
- Hardware H.264 decode on Android TV browser
- Minimal jitter buffer on stable LAN
