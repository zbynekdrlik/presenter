# Stage Debug Frames + Latency Fix

## Goal

Two fixes for the WASM stage display:
1. Add always-on gentle debug frames with box names to all layout boxes
2. Fix latency measurement to use server-measured round-trip (same as old non-WASM stage)

## Architecture

Both changes are CSS + client-side WASM only. No server changes needed.

---

## 1. Always-On Debug Frames

### What

Every layout box gets a subtle border and a tiny name label. Visible when close (laptop, near the TV), invisible from a few meters on a stage TV against the black background.

### Visual spec

- **Border:** `1px solid rgba(255, 255, 255, 0.08)` — barely visible white line
- **Label:** Positioned `top: 1px; left: 4px`, font-size `0.4vw` (scales with viewport), color `rgba(255, 255, 255, 0.12)`, uppercase, letter-spacing `0.08em`
- **Always on** — no toggle, no URL param. Part of the layout permanently.

### Box names (worship-snv layout)

| CSS class | Label text |
|---|---|
| `.stage__current-group` | `current-group` |
| `.stage__current-slide` | `current-slide` |
| `.stage__next-group` | `next-group` |
| `.stage__next-slide` | `next-slide` |
| `.stage__status-bar` | `status-bar` |

### Box names (worship-pp layout)

| CSS class | Label text |
|---|---|
| `.stage-pp__slides-area` | `slides-area` |
| `.stage-pp__playlist-sidebar` | `playlist-sidebar` |
| `.stage__status-bar` | `status-bar` |

### Box names (timer layout)

| CSS class | Label text |
|---|---|
| `.stage-timer__display` | `timer-display` |
| `.stage__status-bar` | `status-bar` |

### Box names (preach layout)

| CSS class | Label text |
|---|---|
| `.stage-preach__display` | `preach-display` |
| `.stage__status-bar` | `status-bar` |

### Implementation

**CSS-only approach** — add a shared `.stage__debug-label` class for labels, and add `border: 1px solid rgba(255, 255, 255, 0.08)` to each box class. Labels are rendered as `<span class="stage__debug-label">box-name</span>` inside each box div.

Each layout component (worship_snv.rs, worship_pp.rs, timer_layout.rs, preach_layout.rs) adds the label span as first child of each box.

### CSS additions (stage.css)

```css
/* Debug frame borders — always on, invisible from distance */
.stage__current-group,
.stage__current-slide,
.stage__next-group,
.stage__next-slide,
.stage__status-bar,
.stage-pp__slides-area,
.stage-pp__playlist-sidebar,
.stage-timer__display,
.stage-preach__display {
    border: 1px solid rgba(255, 255, 255, 0.08);
}

.stage__debug-label {
    position: absolute;
    top: 1px;
    left: 4px;
    font-size: 0.4vw;
    color: rgba(255, 255, 255, 0.12);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    pointer-events: none;
    z-index: 1;
}
```

Note: boxes that don't already have `position: relative` or `position: absolute` need `position: relative` added for the label to position correctly. All current boxes already use `position: absolute`, so no change needed.

---

## 2. Latency Fix

### The bug

`ws/stage.rs` line 114-115 computes latency as:
```rust
let ts_ms = timestamp.timestamp_millis() as f64;
let latency = (now - ts_ms).abs();
```

This is `client_clock - server_clock` = **clock skew**, not network latency. A Google TV with a 2-second clock drift shows 2000ms even with sub-millisecond actual latency.

### The old (correct) behavior

The server measures actual round-trip time:
1. Server sends `Heartbeat { id, timestamp }` at time T1
2. Client receives it, sends `StageHeartbeatAck { heartbeat_id }`
3. Server receives ACK at T2, calculates `round_trip = T2 - T1`
4. Server broadcasts `LiveEvent::StageConnection { snapshot }` where `snapshot.latency_ms = round_trip`
5. Old JS client displayed this server-provided value

### The fix

In `ws/stage.rs`:
1. **Remove** the broken client-side latency calculation from the `Heartbeat` handler (lines 114-116)
2. **Add** a handler for `LiveEvent::StageConnection` that extracts `snapshot.latency_ms` when `snapshot.id` matches our client UUID
3. Update `set_latency_ms` with the server-measured value

The `StageClientSnapshot` already has:
- `id: Uuid` — client identifier
- `latency_ms: Option<u32>` — server-measured round-trip in ms

The client_id in the WASM stage is a string UUID stored in localStorage. Parse it to compare with `snapshot.id`.

### Code change in ws/stage.rs

In the heartbeat handler, remove lines 114-116 (`let ts_ms = ...`, `let latency = ...`, `set_latency_ms.set(...)`).

Add a new match arm in the event handler:
```rust
Ok(LiveEvent::StageConnection { snapshot }) => {
    // Use server-measured round-trip latency for our own client
    if let Ok(our_id) = uuid::Uuid::parse_str(&client_id_for_task) {
        if snapshot.id == our_id {
            set_latency_ms.set(snapshot.latency_ms.map(|ms| ms as f64));
        }
    }
}
```

### Signal type

Currently `latency_ms: ReadSignal<Option<f64>>`. The server sends `u32`. We keep `f64` for the signal to avoid changing all downstream consumers, just cast `ms as f64`.

---

## 3. E2E Latency Regression Test

### What

Add a test to `tests/e2e/stage-status-bar.spec.ts` that asserts the displayed latency is reasonable (under 500ms on localhost/LAN). This catches the clock-skew bug and any future regression.

### Test spec

```typescript
test("stage latency shows server-measured round-trip under 500ms", async ({ context }) => {
  const stagePage = await openStageDisplay(context);

  // Wait for latency to appear in the status bar
  const latencyEl = stagePage.locator(".stage__connection-latency");
  await expect(latencyEl).toBeVisible({ timeout: 10_000 });

  // Extract the latency value
  const text = await latencyEl.textContent();
  const match = text?.match(/(\d+)\s*ms/);
  expect(match).toBeTruthy();

  const latencyValue = parseInt(match![1], 10);
  expect(latencyValue).toBeLessThan(500);
  expect(latencyValue).toBeGreaterThanOrEqual(0);

  await stagePage.close();
});
```

### Why 500ms threshold

- LAN round-trip is typically 1-30ms
- Localhost (E2E test server) is typically <5ms  
- 500ms is generous enough to never flake but catches the 2000ms+ clock-skew bug
- The old implementation showed ~15ms on LAN

---

## Files changed

| File | Change |
|---|---|
| `crates/presenter-ui/styles/stage.css` | Add debug frame borders + `.stage__debug-label` class |
| `crates/presenter-ui/src/components/stage/worship_snv.rs` | Add `<span class="stage__debug-label">` in each box |
| `crates/presenter-ui/src/components/stage/worship_pp.rs` | Add debug labels |
| `crates/presenter-ui/src/components/stage/timer_layout.rs` | Add debug labels |
| `crates/presenter-ui/src/components/stage/preach_layout.rs` | Add debug labels |
| `crates/presenter-ui/src/components/stage/status_bar.rs` | Add debug label to status bar |
| `crates/presenter-ui/src/ws/stage.rs` | Fix latency: remove client calc, use `StageConnection` event |
| `tests/e2e/stage-status-bar.spec.ts` | Add latency threshold test |
