# API Stage Layout — NDI Background

> **Date:** 2026-04-24 | **Status:** Approved

## Problem

The `api` stage layout (driven by `PUT /api/stage` from an external app) currently renders on a flat black body. We want a live NDI video source to appear as the background behind the lyric text, so the stage screen shows camera/video content under the slide overlay — similar to a broadcast lower-third, but in our existing worship-snv style.

Scope is **`api` layout only**. The `worship-snv` layout must be unchanged.

## Design

### Routing

In `crates/presenter-ui/src/pages/stage.rs`, replace the implicit catch-all that currently routes `"api"` to `WorshipSnv` with an explicit arm that routes `"api"` to a new `ApiStage` component. The fallback for unknown codes still goes to `WorshipSnv`.

```rust
"api" => view! { <ApiStage ws_state=ws_state latency_ms=latency_ms /> }.into_any(),
_ => view! { <WorshipSnv ws_state=ws_state latency_ms=latency_ms /> }.into_any(),
```

### ApiStage component

New file: `crates/presenter-ui/src/components/stage/api_stage.rs`.

Structure:

```
<div class="stage-api">
  <Show when=ndi_active>
    <img src="/ndi/mjpeg" class="stage-api__ndi" />
  </Show>

  <Show when=ndi_connecting_or_lost>
    <div class="stage-api__overlay">{status message}</div>
  </Show>

  <WorshipSnv ws_state=ws_state latency_ms=latency_ms />
</div>
```

- `ndi_active` / `ndi_status` come from `StageContext` — already broadcast by the server and written into the context in `pages/stage.rs`.
- `WorshipSnv` stays unchanged. Its boxes are absolutely positioned and render above the sibling `<img>` naturally (DOM order + stacking). The status bar inside `WorshipSnv` keeps its existing solid styling.
- When `ndi_active` is `false`, no `<img>` is rendered — the layout looks exactly like the current api layout (black body showing through transparent slide boxes). No broken-image placeholder.
- Connection-state overlay ("Signal Lost — Reconnecting…", "Connecting…") is reused from the pattern in `NdiFullscreen`, styled to sit above the video but below the slide text.

### CSS

Add to `crates/presenter-ui/styles/stage.css`:

```css
.stage-api {
    position: relative;
    width: 100vw;
    height: 100vh;
    background: #000;
    overflow: hidden;
}

.stage-api__ndi {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
}

.stage-api__overlay {
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

.stage-api .stage__slide-text {
    text-shadow:
        0 0 8px rgba(0, 0, 0, 0.9),
        0 2px 4px rgba(0, 0, 0, 0.7);
}
```

- `object-fit: cover` fills the viewport and crops if aspect ratios don't match. Black letterbox bars would look worse on a church stage screen than a slight crop.
- The `.stage-api .stage__slide-text` selector applies a soft dark shadow to the big current/next slide text only when inside the api layout. `worship-snv` slides are untouched.
- Group pills (dynamic colored backgrounds via inline style) and song-name pills (subtle yellow tint) keep their existing styling. They're small and contrasty enough to stay readable without intervention.

### Data Flow

No new server endpoints, no new WebSocket events. The same global "active video source" drives both `ndi-fullscreen` and `api` layouts. The existing WS flow already sets `ctx.ndi_active` / `ctx.ndi_status` in `StageContext` (`pages/stage.rs:80-84`). `ApiStage` reads those signals via `use_context`.

Slide data continues to flow via the existing `api_stage` broadcast path (`PUT /api/stage` → selective broadcast to `layout_code == "api"` WS clients → `ctx.snapshot` → `WorshipSnv` reactive render). Unchanged.

## Testing

### Playwright E2E — `tests/e2e/stage-api-ndi.spec.ts` (new)

Reuse the mock NDI harness from `tests/e2e/ndi-stage-layout.spec.ts`.

1. Navigate to the stage at `api` layout with no active video source. Assert:
   - `img.stage-api__ndi` is **not** in the DOM.
   - Body background is black (computed style check).
   - Slide text renders with the new `text-shadow` CSS rule applied.
2. Activate a mock video source via API. Wait for the WS push. Assert:
   - `img.stage-api__ndi` **is** present with `src="/ndi/mjpeg"`.
   - It has `position: absolute; object-fit: cover` (computed style).
   - Slide text still renders correctly above the image.
3. Deactivate the source. Assert the image is unmounted again.
4. Regression guard: navigate to `worship-snv` layout, assert `img.stage-api__ndi` is never present and no text-shadow is applied to slide text.
5. Browser console must be zero errors/warnings (per airuleset `browser-console-zero-errors`).

### Manual post-deploy verification

Open the deployed dev stage with a real NDI source active. Screenshot. Confirm text-shadow is visible and slide text reads cleanly over the video.

## Out of Scope

- **NDI settings page redesign.** The current settings UI for NDI ("random buttons/widgets") will be redesigned in a separate brainstorm/spec after this ships.
- **Applying the NDI background to other layouts** (`worship-snv`, `bible`, `timer`, `preach`). This change is api-only.
- **Per-layout NDI source selection.** One global active source, same as today.
- **Changing the MJPEG endpoint or NDI server-side plumbing.** Reuses `/ndi/mjpeg` as-is.
