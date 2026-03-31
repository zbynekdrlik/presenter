# WASM Stage Display Redesign

**Date:** 2026-03-31
**Status:** Approved
**Approach:** C — WASM stage with predefined layout presets, delete WYSIWYG editor

## Problem

The current stage display (`/stage`) is server-rendered HTML with ~500 lines of embedded JavaScript. The layout breaks when content varies — long lyrics overflow their boxes and push other elements around. The WYSIWYG design editor was a workaround for not being able to get the layout right in code. The desired behavior is ProPresenter-like: rock-solid layout regardless of content.

Specific production issues:
- Clock too small to read from distance
- Live indicator pill ("VYSIELANIE JE VYPNUTE") line-breaks when it should fit in one row
- Current lyrics vertically centered in box instead of top-aligned — wastes space, smaller than needed
- Next lyrics pushed toward bottom — singers need text visible above crowd heads
- Text overflow causes layout shifts between boxes

## Solution

Rewrite stage display as WASM (Leptos) with hardcoded layout presets. Delete the WYSIWYG editor, StageDesign/StageBox models, and StageAppearance settings entirely.

### WASM Compatibility

- **iPad Safari 12:** Already solved — MVP WASM build with nightly + build-std (PR #196)
- **Google TV + Fully Kiosk Browser:** Verified working — Android 12+ WebView supports WASM. Tested by loading `/ui/tablet` (WASM) on actual Google TV hardware.

## Architecture

### Single WASM Binary, Three UIs

The stage display joins the existing `presenter-ui` WASM crate. Route-based rendering:
- `/ui/operator` → operator components
- `/ui/tablet` → tablet components
- `/stage` → stage components (new)

### Server Changes

- `/stage` route returns the WASM app shell (like `/ui/tablet`) instead of server-rendered HTML
- All existing API endpoints stay: `/stage/snapshot`, `/stage/state`, `/stage/clear`, `/stage/layout`, `/stage/connections`, `/stage/broadcast-live`, `/live/ws`
- Removed endpoints: `/stage/design/*`, `/stage/appearance/*`

### New Files in presenter-ui

```
components/
  stage/
    mod.rs          — route entry, layout switcher
    worship_snv.rs  — worship-snv preset layout
    worship_pp.rs   — worship-pp preset layout
    timer.rs        — timer preset layout
    preach.rs       — preach preset layout
    text_fit.rs     — auto-fit algorithm (shared by all presets)
    status_bar.rs   — clock, live indicator, connection status (shared)
    websocket.rs    — WebSocket + reconnect + heartbeat
```

## Layout Design Rules

### Global Rules (all presets)

1. **Text alignment: top** — all lyric/group boxes use `align-items: flex-start`, never center
2. **Text size: maximize** — auto-fit grows text as large as the box allows, shrinks when content is long
3. **No minimum font size** — text shrinks until it fits, no floor. Content length is the user's responsibility.
4. **Boxes never move** — all positions are absolute percentages, independent of each other
5. **overflow: hidden** on every box as safety net
6. **Status bar: single flex row** — clock, live indicator, connection share one bar with `justify-content: space-between`. No individual absolute boxes.
7. **No wrapping on status elements** — `white-space: nowrap` on clock, live pill, connection text. Auto-shrink font if needed, never line-break.

### Text Auto-Fit Algorithm

1. Set text to maximum font size for the box
2. Measure: compare `scrollHeight` vs `clientHeight` (and width)
3. If overflow: binary search downward until it fits
4. No minimum — shrink as much as needed
5. Runs inside Leptos `create_effect` tied to content signal — guaranteed to run after DOM update, before paint
6. Each box is independent — fitting one box has zero effect on others

### worship-snv Layout

| Box | Position | Alignment | Styling |
|-----|----------|-----------|---------|
| current-group | top 1%, center, w:50% h:5% | center | FNV-1a color hash pill, 25% opacity bg, uppercase, letter-spacing 0.18em |
| current-slide | top 7%, full width (96%), h:48% | **top-aligned**, maximize font | #f8fafc, bold, `pre-wrap` |
| next-group | top 56%, center, w:50% h:4% | center | FNV-1a color hash pill (same system as current) |
| next-slide | top 61%, full width (96%), h:30% | **top-aligned**, maximize font | #cbd5f5 (dimmed), bold, `pre-wrap` |
| status-bar | bottom 0%, full width, h:7% | flex row, space-between | Single row: clock \| live \| connected |

**Status bar elements:**
- Clock: #38bdf8, large bold font, tabular-nums, left
- Live indicator: green pill (off) / red pulsing pill (on), `nowrap`, center
- Connection: #38bdf8, uppercase, right, with latency

**Group color system:** FNV-1a hash of group name → 8 curated colors (rose #fb7185, orange #fb923c, amber #fbbf24, emerald #34d399, cyan #22d3ee, blue #60a5fa, violet #a78bfa, pink #f472b6). Color applied as text color + 25% opacity background on pill.

### worship-pp Layout

Same as worship-snv but with playlist sidebar:
- Slides area takes ~70% width
- Playlist sidebar takes ~30% width (right side)
- Playlist shows numbered entries, active entry highlighted and auto-scrolled into view
- Same text rules (top-aligned, maximize, auto-shrink)

### timer Layout

- Large countdown timer centered (maximize font)
- Status bar at bottom (same as worship-snv)

### preach Layout

- Large stopwatch centered (maximize font)
- Overtime indicator (color shift when over target)
- Status bar at bottom (same as worship-snv)

## Data Flow

1. **Initial load** — WASM app fetches `/stage/snapshot` via HTTP
2. **Live updates** — connects to `/live/ws`, listens for `LiveEvent::Stage`
3. **Heartbeat** — sends `StageHeartbeatAck` back to server (same protocol)
4. **Layout switch** — listens for `LiveEvent::StageLayout`, swaps preset component
5. **Reconnect** — on disconnect, auto-reconnect with exponential backoff (1s, 2s, 4s... cap 30s). On reconnect: re-send `StagePresence`, fetch fresh snapshot to sync missed events.
6. **Fully Kiosk safety net** — Fully Kiosk's "reload on connection restore" setting provides additional recovery

## What Gets Removed

### Models (presenter-core)
- `stage_design.rs` — `StageDesign`, `StageBox`, `StageBoxType`, defaults
- `stage_appearance.rs` — `StageAppearance`, per-layout settings

### Server (presenter-server)
- `stage_ui/` module — `mod.rs`, `layouts.rs`, `styles.rs`, `tests.rs` (server-rendered HTML + embedded JS)
- `ui/stage_design.rs` — WYSIWYG editor UI
- `state/stage_display.rs` — appearance/design get/set/reset methods
- API endpoints: `GET/PUT /stage/appearance/{layout}`, `GET/PUT /stage/design/{layout}`, `POST /stage/design/{layout}/reset`

### Database
- Settings keys: `stage-appearance:*`, `stage-design:*`

### E2E Tests
- Delete: `stage-design.spec.ts`, `stage-design-reset.spec.ts`, `stage-appearance.spec.ts`
- Rewrite: `stage-snapshot.spec.ts`, `stage-status-bar.spec.ts`, `wasm-stage.spec.ts`

## What Stays Unchanged

- `stage_client.rs` — connection tracking
- `stage_connections.rs` — heartbeat management
- `router/stage.rs` — trimmed (remove design/appearance endpoints, keep layout/snapshot/state/clear/connections)
- `state/stage.rs` — snapshot building logic
- `broadcasting.rs` — WebSocket broadcast
- `live.rs` — WebSocket handler

## Bible Overlay

The current stage has a Bible overlay (`stage__bible-overlay`) that covers the full screen with semi-transparent background. This stays as-is in the WASM version — a full-screen overlay component that renders when Bible broadcast is active.

## Runtime Configuration

Minimal — only layout switching from operator header dropdown. No appearance sliders, no box dragging. If a new layout variant is needed, it's a new Rust component.
