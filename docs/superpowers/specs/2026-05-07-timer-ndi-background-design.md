# Timer Stage Layout: NDI Background (Design)

> **Status:** Approved
> **Issue:** #306
> **Created:** 2026-05-07

## Problem

The `api` and `ndi-fullscreen` stage layouts render the live NDI MJPEG video as a backdrop. The `timer` layout currently shows the countdown digits on a black background. The user wants the same NDI backdrop on `timer` so the audience sees a single visual scene whether the operator is showing the timer or other content.

## Goal

Match the existing `api_stage` pattern in `timer_layout.rs`. When an NDI source is active, render `<img src="/ndi/mjpeg">` covering the viewport, with the timer digits sitting on top.

## Approach

Mirror `api_stage.rs` exactly. No refactor, no shared component — the existing two-layout duplication of the MJPEG `<img>` + status overlay is small and survives one more copy.

## Components

### `crates/presenter-ui/src/components/stage/timer_layout.rs`

Add inside the existing `<div class="stage-container" data-layout="timer">`:

1. `<Show when=ndi_active>` → `<img src="/ndi/mjpeg" class="stage-timer__ndi" />`
2. `<Show when=status==connecting||disconnected>` → status overlay (same copy as `ApiStage`: `"Connecting..."` / `"Signal Lost — Reconnecting..."`)
3. The existing `.stage-timer__display` block, unchanged in markup but elevated above the NDI img via `z-index`.

Bind `let ndi_active = ctx.ndi_active;` and `let ndi_status = ctx.ndi_status;` from the already-fetched `StageContext`. No new context fields, no new API calls — `pages/stage.rs` is already wiring `LiveEvent::NdiSourceActivated/Deactivated/ConnectionStatus` into these signals.

### `crates/presenter-ui/styles/stage.css`

Add to the timer section (around line 382):

```css
.stage-timer__ndi {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
}

.stage-timer__overlay {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(0, 0, 0, 0.7);
    color: #ef4444;
    font-size: 1.2rem;
    z-index: 1;
}
```

Modify the existing `.stage-timer__display` and `.stage-timer__text` rules to ensure the timer text sits above the video and stays legible:

```css
.stage-timer__display {
    /* existing rules + */
    position: relative;
    z-index: 2;
}

.stage-timer__text {
    /* existing rules + */
    text-shadow:
        0 0 8px rgba(0, 0, 0, 0.9),
        0 2px 4px rgba(0, 0, 0, 0.7);
}
```

`.stage-container` already provides `position: relative; width: 100vw; height: 100vh; overflow: hidden;` — the absolute-positioned NDI img inherits the right containment.

### No server changes

`StageContext.ndi_active` and `ndi_status` are already populated by `pages/stage.rs` from live WebSocket events and the initial `api::ndi::list_video_sources()` fetch. No router or state changes needed.

## Tests

### Playwright (E2E)

New test file: `tests/e2e/stage-timer-ndi.spec.ts`. No dedicated stage-timer test exists today; mirror the structure of `tests/e2e/stage-api-ndi.spec.ts` (which is the closest analogue: it covers `api` layout with active/inactive NDI source).

Scenarios:

1. **Active NDI source → backdrop visible.** Activate a video source via the API. Open `/stage` with `layout_code=timer`. Assert `[class="stage-timer__ndi"]` exists, is visible, and has a non-zero `naturalWidth` after 2 seconds (proves the MJPEG stream is actually loading frames).
2. **No NDI source → no backdrop.** Open `/stage` with `layout_code=timer` and no active NDI source. Assert `[class="stage-timer__ndi"]` does NOT exist (the `<Show when=ndi_active>` block is gated).
3. **Timer text visible over backdrop.** With active NDI source, assert `[data-role="timer-display"]` (or whichever selector the existing test uses for the timer) is visible and readable.
4. **Browser-console-zero-errors.** Per global rule, every E2E test ends with `expect(consoleMessages).toEqual([])`.

### Manual verification

After deploy to dev, browse to `http://10.77.8.134:8080/stage?layout=timer` (or set the layout via the operator), trigger an NDI source on the dev presenter, and confirm by eye that the countdown sits over the video.

## Out of scope

- `preach` layout NDI background (not requested in #306). If desired, separate issue.
- Refactoring `api_stage.rs` and `ndi_fullscreen.rs` to share an `NdiBackdrop` component. The existing duplication survives one more copy; refactor when there's a fourth or when behavior diverges.
- Any change to how the operator selects the NDI source.

## Risks

| Risk | Mitigation |
|---|---|
| Timer text becomes unreadable over bright/busy video | Text-shadow added (same approach as `ApiStage` slide text). |
| `<img src="/ndi/mjpeg">` mounts and disconnects rapidly when `ndi_active` toggles | Same `<Show>` gating as `ApiStage` and `NdiFullscreen` — no observed issues there. |
| Existing timer E2E tests break because of the new wrapper markup | The existing `.stage-timer__display` and `.stage-timer__text` selectors are preserved. Test selectors that target those classes continue to work. |

## Verification checklist

| Check | Method |
|---|---|
| `<img class="stage-timer__ndi">` renders when NDI active | Playwright E2E |
| `<img class="stage-timer__ndi">` absent when NDI inactive | Playwright E2E |
| Timer digits visible and unobscured | Playwright + manual on dev |
| Browser console clean | Playwright `expect(consoleMessages).toEqual([])` |
| Version bump 0.4.72 → 0.4.73 | `Cargo.toml` workspace version |
| WASM clippy clean | `cargo clippy --target wasm32-unknown-unknown -p presenter-ui` |
