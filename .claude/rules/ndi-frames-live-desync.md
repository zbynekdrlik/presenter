---
paths:
  - "crates/presenter-ui/src/pages/stage.rs"
  - "crates/presenter-ui/src/components/stage/ndi_frame_stats.rs"
  - "crates/presenter-ui/src/components/stage/ndi_health_ticker.rs"
  - "crates/presenter-ui/src/components/stage/ndi_video.rs"
---

# `ndi_frames_live` — the per-session Cell vs shared-signal desync (#757, #732, #500)

The stage "are NDI frames presenting?" state lives in TWO places that MUST stay in sync:

1. **The shared reactive signal** `StageContext::ndi_frames_live: RwSignal<bool>` — what the UI
   binds to. Since #732 the `<video>`'s `stage-ndi-video--dormant` (opacity:0) class is
   `move || !ndi_frames_live.get()` (`ndi_video.rs`), so a stuck-`false` signal HIDES live video.
2. **The per-session `FrameStats.frames_live: Cell<bool>`** (`ndi_frame_stats.rs`) — the
   last-EMITTED state. `mark_frames_live` writes the shared signal `true` ONLY on the Cell's
   `false→true` transition (so the rVFC observer doesn't churn the signal ~30×/s), and
   `refresh_frames_live_staleness` writes `false` ONLY on the `true→false` staleness transition.

**The trap:** any code that writes the SHARED signal `false` directly — WITHOUT also resetting the
per-session Cell — creates a permanent desync. The Cell is still `true` (frames never stopped), so
`mark_frames_live` sees no transition and NEVER re-emits `true`. The shared signal stays `false`
forever → the video is hidden while frames flow. #757 was exactly this: the `NdiSourceActivated`
handler in `stage.rs` unconditionally did `ndi_frames_live.set(false)` on EVERY activation, so
re-activating the already-active source (operator "zapnúť NDI") stuck the stage TV black.

**Rules when touching this state:**

- **A same-value / same-source re-activation must NOT reset the gate.** The `NdiSourceActivated`
  handler guards with `ndi_activation_resets_gate(incoming, current)` (== `sync_ndi_source_state`'s
  `ndi_active_source_id != id` guard) — reset the neutral cover / frames gate ONLY when the source
  genuinely CHANGED (or was `None`). A fresh/changed source legitimately resets (no frames yet);
  a same-source re-activation leaves the live gate alone.
- **Never write the shared `ndi_frames_live` signal directly `false` while frames may be flowing**
  unless you ALSO reset `FrameStats.frames_live` to `false`. If you can't reach the Cell (it's
  per-`NdiVideo`-session), don't touch the shared signal — let the frame path / staleness ticker
  own it.
- **Defense-in-depth already exists:** the 1s health ticker calls
  `reassert_frames_live_if_desynced` (decided by pure `should_reassert_frames_live`), which
  re-emits `true` when frames are fresh (`!frames_are_stale`) but the shared signal reads `false`.
  It reads the shared signal via the `FramesLiveReader` (`StageSignalSetters.frames_live_read`,
  an untracked `get_untracked()` poll — a poll, NOT a subscription). So a stray desync self-heals
  within ~1s — but that is a SAFETY NET, not a licence to write the shared signal false carelessly.

**Testing pattern (Tier-0, presenter-ui is host-tested via `cargo test --lib` in the crate dir):**
keep the DECISION pure and clock-free (`ndi_activation_resets_gate`, `should_reassert_frames_live`)
so it is host-unit-testable; the clock-reading wiring (`reassert_frames_live_if_desynced`, which
calls `now_ms()`) is exercised only in WASM/E2E — the same pure-core / clock-wired split as
`frames_are_stale` (tested) vs `refresh_frames_live_staleness` (not). `now_ms()` calls
`js_sys::Date::now()` on host and panics, so a host test must never call it. The end-to-end guard
is `ndi-webrtc-synthetic.spec.ts` "keeps NDI video revealed after re-activating the ALREADY-active
source" — real frames flowing, POST activate the SAME source, assert the `<video>` never carries
`stage-ndi-video--dormant`, using continued frame advance (not a sleep) as the settle signal.
