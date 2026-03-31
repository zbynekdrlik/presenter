# WASM Stage Display Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rewrite the stage display (`/stage`) from server-rendered HTML+JS to WASM (Leptos), fixing layout stability issues and deleting the WYSIWYG design editor.

**Architecture:** The stage joins the existing `presenter-ui` WASM crate as a third route (`/stage`). A new `StageContext` + bidirectional WebSocket module handles heartbeat ACK, presence, and reconnection. Layout presets are hardcoded Leptos components with CSS-based positioning. Server changes: `GET /stage` serves the WASM shell instead of rendered HTML; design/appearance endpoints and models are removed.

**Tech Stack:** Leptos 0.7 CSR, gloo-net WebSocket, web-sys DOM measurement (text auto-fit), existing Trunk build pipeline

**Spec:** `docs/superpowers/specs/2026-03-31-wasm-stage-display-design.md`

---

## File Structure

### New files in `crates/presenter-ui/`

```
src/
  pages/
    stage.rs                    — StagePage: orchestration, WS event dispatch, context setup
  components/
    stage/
      mod.rs                    — re-exports all stage components
      worship_snv.rs            — worship-snv preset layout component
      worship_pp.rs             — worship-pp preset layout (slides + playlist sidebar)
      timer_layout.rs           — timer preset layout (large countdown)
      preach_layout.rs          — preach preset layout (stopwatch + overtime)
      status_bar.rs             — shared status bar: clock | live | connection
      bible_overlay.rs          — full-screen Bible overlay
  state/
    stage.rs                    — StageContext: signals for snapshot, WS state, bible, broadcast
  ws/
    stage.rs                    — bidirectional stage WS: presence, heartbeat ACK, reconnect
  utils/
    color.rs                    — FNV-1a group color hash
    autofit.rs                  — text auto-fit binary search via DOM measurement
styles/
  stage.css                     — all stage display CSS
```

### Modified files

```
crates/presenter-ui/src/lib.rs              — add /stage route
crates/presenter-ui/src/pages/mod.rs        — pub mod stage
crates/presenter-ui/src/components/mod.rs   — pub mod stage
crates/presenter-ui/src/state/mod.rs        — pub mod stage
crates/presenter-ui/src/ws/mod.rs           — pub mod stage
crates/presenter-ui/src/utils/mod.rs        — pub mod color, autofit
crates/presenter-ui/index.html              — add stage.css link
crates/presenter-ui/Cargo.toml              — add web-sys features if needed
crates/presenter-server/src/router.rs       — /stage → wasm_ui_shell, remove design/appearance routes
crates/presenter-server/src/router/stage.rs — remove design/appearance handlers
crates/presenter-server/src/main.rs         — remove mod stage_ui
crates/presenter-core/src/live.rs           — remove StageAppearance/StageDesign variants
crates/presenter-core/src/lib.rs            — remove stage_design, stage_appearance re-exports
```

### Deleted files

```
crates/presenter-server/src/stage_ui/mod.rs
crates/presenter-server/src/stage_ui/layouts.rs
crates/presenter-server/src/stage_ui/styles.rs
crates/presenter-server/src/stage_ui/tests.rs
crates/presenter-server/src/ui/stage_design.rs
crates/presenter-core/src/stage_design.rs
crates/presenter-core/src/stage_appearance.rs
crates/presenter-server/src/state/stage_display.rs
tests/e2e/stage-design.spec.ts
tests/e2e/stage-design-reset.spec.ts
tests/e2e/stage-appearance.spec.ts
```

---

### Task 1: FNV-1a Group Color Utility

**Files:**
- Create: `crates/presenter-ui/src/utils/color.rs`
- Modify: `crates/presenter-ui/src/utils/mod.rs`

Pure function — no DOM, no WASM APIs. Unit-testable with `cargo test`.

- [ ] **Step 1: Write tests in `color.rs`**

```rust
// crates/presenter-ui/src/utils/color.rs

/// FNV-1a hash of group name → one of 8 curated colors.
/// Returns (text_color_hex, bg_rgba) for pill styling.
const GROUP_COLORS: [&str; 8] = [
    "#fb7185", // rose
    "#fb923c", // orange
    "#fbbf24", // amber
    "#34d399", // emerald
    "#22d3ee", // cyan
    "#60a5fa", // blue
    "#a78bfa", // violet
    "#f472b6", // pink
];

pub fn group_color(name: &str) -> &'static str {
    let hash = fnv1a(name);
    GROUP_COLORS[(hash as usize) % GROUP_COLORS.len()]
}

fn fnv1a(s: &str) -> u32 {
    let mut hash: u32 = 2166136261;
    for byte in s.bytes() {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(16777619);
    }
    hash
}

/// Convert hex color to rgba with given opacity (0.0-1.0).
pub fn hex_to_rgba(hex: &str, opacity: f32) -> String {
    let hex = hex.trim_start_matches('#');
    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
    format!("rgba({r},{g},{b},{opacity})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_color_deterministic() {
        assert_eq!(group_color("Verse 1"), group_color("Verse 1"));
        assert_eq!(group_color("Chorus"), group_color("Chorus"));
    }

    #[test]
    fn group_color_different_names_can_differ() {
        // Not guaranteed to differ for all pairs, but these specific names should
        let c1 = group_color("Verse 1");
        let c2 = group_color("Bridge");
        // At minimum they're valid colors
        assert!(c1.starts_with('#'));
        assert!(c2.starts_with('#'));
    }

    #[test]
    fn group_color_empty_string() {
        let c = group_color("");
        assert!(GROUP_COLORS.contains(&c));
    }

    #[test]
    fn hex_to_rgba_basic() {
        assert_eq!(hex_to_rgba("#fb7185", 0.25), "rgba(251,113,133,0.25)");
    }

    #[test]
    fn hex_to_rgba_no_hash() {
        assert_eq!(hex_to_rgba("60a5fa", 0.5), "rgba(96,165,250,0.5)");
    }
}
```

- [ ] **Step 2: Run tests to verify they pass**

```bash
cargo test -p presenter-ui --lib color
```

Expected: All 5 tests pass.

- [ ] **Step 3: Add module to `utils/mod.rs`**

Add `pub mod color;` to `crates/presenter-ui/src/utils/mod.rs`.

- [ ] **Step 4: Commit**

```bash
git add crates/presenter-ui/src/utils/color.rs crates/presenter-ui/src/utils/mod.rs
git commit -m "feat(stage): add FNV-1a group color utility"
```

---

### Task 2: Text Auto-Fit Utility

**Files:**
- Create: `crates/presenter-ui/src/utils/autofit.rs`
- Modify: `crates/presenter-ui/src/utils/mod.rs`

Binary search algorithm that measures DOM element scrollHeight vs clientHeight to find the largest font size that fits. Requires web-sys (WASM-only).

- [ ] **Step 1: Create `autofit.rs`**

```rust
// crates/presenter-ui/src/utils/autofit.rs

use web_sys::HtmlElement;

/// Auto-fit text to fill a container by binary-searching font size.
///
/// Sets the element's `font-size` style to the largest value in `[1, max_font_px]`
/// where the content does not overflow the container.
///
/// The element MUST have `overflow: hidden` and a fixed height/width.
pub fn autofit_text(element: &HtmlElement, max_font_px: f64) {
    let style = element.style();

    // Try max first — if it fits, we're done
    let _ = style.set_property("font-size", &format!("{max_font_px}px"));
    if !is_overflowing(element) {
        return;
    }

    // Binary search between 1px and max
    let mut lo: f64 = 1.0;
    let mut hi: f64 = max_font_px;

    while (hi - lo) > 0.5 {
        let mid = (lo + hi) / 2.0;
        let _ = style.set_property("font-size", &format!("{mid}px"));
        if is_overflowing(element) {
            hi = mid;
        } else {
            lo = mid;
        }
    }

    // Use the last known fitting size
    let _ = style.set_property("font-size", &format!("{lo}px"));
}

fn is_overflowing(el: &HtmlElement) -> bool {
    el.scroll_height() > el.client_height() || el.scroll_width() > el.client_width()
}
```

- [ ] **Step 2: Add module to `utils/mod.rs`**

Add `pub mod autofit;` to `crates/presenter-ui/src/utils/mod.rs`.

- [ ] **Step 3: Verify compilation**

```bash
cargo check -p presenter-ui
```

Expected: Compiles (no cargo test for DOM-dependent code — tested via E2E).

- [ ] **Step 4: Commit**

```bash
git add crates/presenter-ui/src/utils/autofit.rs crates/presenter-ui/src/utils/mod.rs
git commit -m "feat(stage): add text auto-fit binary search utility"
```

---

### Task 3: Stage WebSocket Module

**Files:**
- Create: `crates/presenter-ui/src/ws/stage.rs`
- Modify: `crates/presenter-ui/src/ws/mod.rs`

Bidirectional WebSocket for stage clients. Unlike the existing `use_live_websocket()` (read-only), this module must:
- Send `StagePresence` on connect
- Send `StageHeartbeatAck` in response to `Heartbeat` events
- Track connection state with heartbeat timeout detection
- Exponential backoff reconnection (1s, 2s, 4s... cap 30s)
- Re-fetch snapshot on reconnect

- [ ] **Step 1: Create `ws/stage.rs`**

```rust
// crates/presenter-ui/src/ws/stage.rs

use gloo_net::websocket::{futures::WebSocket, Message};
use gloo_timers::callback::{Interval, Timeout};
use leptos::prelude::*;
use presenter_core::{InboundMessage, LiveEvent};
use std::cell::RefCell;
use std::rc::Rc;

/// Reconnect backoff: 1s, 2s, 4s, 8s, 16s, 30s cap.
const INITIAL_RECONNECT_MS: u32 = 1_000;
const MAX_RECONNECT_MS: u32 = 30_000;

/// How often to check for heartbeat timeout (ms).
const HEARTBEAT_CHECK_INTERVAL_MS: u32 = 500;

/// Default heartbeat config (overridden by server values if available).
const DEFAULT_GRACE_MS: f64 = 4_500.0;
const DEFAULT_DISCONNECT_MS: f64 = 12_000.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageWsState {
    Connecting,
    Connected,
    Reconnecting,
    Disconnected,
}

/// Handle returned by `use_stage_websocket`.
#[derive(Clone)]
pub struct StageWsHandle {
    pub state: ReadSignal<StageWsState>,
    pub last_event: ReadSignal<Option<LiveEvent>>,
    pub latency_ms: ReadSignal<Option<f64>>,
}

/// Create a bidirectional stage WebSocket connection.
///
/// Sends `StagePresence` on connect, responds to heartbeats with `StageHeartbeatAck`,
/// and auto-reconnects with exponential backoff.
pub fn use_stage_websocket(client_id: String, layout_code: RwSignal<String>) -> StageWsHandle {
    let (state, set_state) = signal(StageWsState::Connecting);
    let (last_event, set_last_event) = signal::<Option<LiveEvent>>(None);
    let (latency_ms, set_latency_ms) = signal::<Option<f64>>(None);

    let reconnect_delay = Rc::new(RefCell::new(INITIAL_RECONNECT_MS));
    let last_heartbeat_at = Rc::new(RefCell::new(js_sys::Date::now()));

    // Start heartbeat timeout checker
    let last_hb = last_heartbeat_at.clone();
    let set_state_hb = set_state;
    let _heartbeat_checker = Interval::new(HEARTBEAT_CHECK_INTERVAL_MS, move || {
        let elapsed = js_sys::Date::now() - *last_hb.borrow();
        if elapsed >= DEFAULT_DISCONNECT_MS {
            set_state_hb.set(StageWsState::Disconnected);
        } else if elapsed >= DEFAULT_GRACE_MS {
            set_state_hb.set(StageWsState::Reconnecting);
        }
    });
    // Keep checker alive for the component's lifetime
    _heartbeat_checker.forget();

    // Initial connection
    spawn_stage_ws(
        client_id.clone(),
        layout_code,
        set_state,
        set_last_event,
        set_latency_ms,
        reconnect_delay.clone(),
        last_heartbeat_at.clone(),
    );

    StageWsHandle {
        state,
        last_event,
        latency_ms,
    }
}

fn spawn_stage_ws(
    client_id: String,
    layout_code: RwSignal<String>,
    set_state: WriteSignal<StageWsState>,
    set_last_event: WriteSignal<Option<LiveEvent>>,
    set_latency_ms: WriteSignal<Option<f64>>,
    reconnect_delay: Rc<RefCell<u32>>,
    last_heartbeat_at: Rc<RefCell<f64>>,
) {
    use futures_util::{SinkExt, StreamExt};

    let client_id_clone = client_id.clone();
    let reconnect_delay_clone = reconnect_delay.clone();
    let last_hb = last_heartbeat_at.clone();

    leptos::task::spawn_local(async move {
        let url = ws_url();

        match WebSocket::open(&url) {
            Ok(ws) => {
                let (mut write, mut read) = ws.split();

                // Send StagePresence
                let presence = InboundMessage::StagePresence {
                    client_id: client_id_clone.clone(),
                    layout_code: layout_code.get_untracked(),
                };
                if let Ok(json) = serde_json::to_string(&presence) {
                    let _ = write.send(Message::Text(json)).await;
                }

                set_state.set(StageWsState::Connected);
                *reconnect_delay_clone.borrow_mut() = INITIAL_RECONNECT_MS;
                *last_hb.borrow_mut() = js_sys::Date::now();

                // Wrap write half for sending heartbeat ACKs from the read loop
                let write = Rc::new(RefCell::new(Some(write)));

                while let Some(msg) = read.next().await {
                    match msg {
                        Ok(Message::Text(text)) => {
                            match serde_json::from_str::<LiveEvent>(&text) {
                                Ok(LiveEvent::Heartbeat { id, timestamp }) => {
                                    let now = js_sys::Date::now();
                                    *last_hb.borrow_mut() = now;
                                    set_state.set(StageWsState::Connected);

                                    // Calculate latency from heartbeat timestamp
                                    let ts_ms = timestamp.timestamp_millis() as f64;
                                    let latency = (now - ts_ms).abs();
                                    set_latency_ms.set(Some(latency));

                                    // Send heartbeat ACK
                                    let ack = InboundMessage::StageHeartbeatAck {
                                        client_id: client_id_clone.clone(),
                                        heartbeat_id: Some(id.to_string()),
                                    };
                                    if let Ok(json) = serde_json::to_string(&ack) {
                                        let mut guard = write.borrow_mut();
                                        if let Some(ref mut w) = *guard {
                                            let _ = w.send(Message::Text(json)).await;
                                        }
                                    }
                                }
                                Ok(event) => {
                                    *last_hb.borrow_mut() = js_sys::Date::now();
                                    set_last_event.set(Some(event));
                                }
                                Err(_) => { /* ignore malformed */ }
                            }
                        }
                        Ok(Message::Bytes(_)) => {}
                        Err(_) => break,
                    }
                }

                // Connection closed
                set_state.set(StageWsState::Reconnecting);
            }
            Err(_) => {
                set_state.set(StageWsState::Disconnected);
            }
        }

        // Schedule reconnect with exponential backoff
        let delay = {
            let mut d = reconnect_delay.borrow_mut();
            let current = *d;
            *d = (*d * 2).min(MAX_RECONNECT_MS);
            current
        };

        let client_id = client_id;
        let layout_code = layout_code;
        let reconnect_delay = reconnect_delay;
        let last_heartbeat_at = last_heartbeat_at;

        Timeout::new(delay, move || {
            set_state.set(StageWsState::Connecting);
            spawn_stage_ws(
                client_id,
                layout_code,
                set_state,
                set_last_event,
                set_latency_ms,
                reconnect_delay,
                last_heartbeat_at,
            );
        })
        .forget();
    });
}

fn ws_url() -> String {
    let window = web_sys::window().expect("no global window");
    let location = window.location();
    let protocol = location.protocol().unwrap_or_else(|_| "http:".to_string());
    let host = location.host().unwrap_or_else(|_| "localhost".to_string());
    let ws_protocol = if protocol == "https:" { "wss:" } else { "ws:" };
    format!("{ws_protocol}//{host}/live/ws")
}
```

- [ ] **Step 2: Add module to `ws/mod.rs`**

Add `pub mod stage;` to `crates/presenter-ui/src/ws/mod.rs`.

- [ ] **Step 3: Verify compilation**

```bash
cargo check -p presenter-ui
```

- [ ] **Step 4: Commit**

```bash
git add crates/presenter-ui/src/ws/stage.rs crates/presenter-ui/src/ws/mod.rs
git commit -m "feat(stage): add bidirectional stage WebSocket with heartbeat ACK"
```

---

### Task 4: Stage Context

**Files:**
- Create: `crates/presenter-ui/src/state/stage.rs`
- Modify: `crates/presenter-ui/src/state/mod.rs`

- [ ] **Step 1: Create `state/stage.rs`**

```rust
// crates/presenter-ui/src/state/stage.rs

use leptos::prelude::*;
use presenter_core::{BibleSlideOutput, StageDisplaySnapshot};
use uuid::Uuid;

use crate::state::session;

const CLIENT_ID_KEY: &str = "stageClientId";

/// Stage display context — provided at StagePage level.
#[derive(Clone)]
pub struct StageContext {
    /// Unique client ID (persisted in localStorage).
    pub client_id: String,
    /// Current layout code (e.g., "worship-snv").
    pub layout_code: RwSignal<String>,
    /// Latest stage snapshot from server.
    pub snapshot: RwSignal<Option<StageDisplaySnapshot>>,
    /// Whether broadcast is live.
    pub broadcast_live: RwSignal<bool>,
    /// Active Bible overlay (None = no overlay).
    pub bible_overlay: RwSignal<Option<BibleSlideOutput>>,
}

impl StageContext {
    pub fn new(initial_layout: String) -> Self {
        Self {
            client_id: load_or_create_client_id(),
            layout_code: RwSignal::new(initial_layout),
            snapshot: RwSignal::new(None),
            broadcast_live: RwSignal::new(false),
            bible_overlay: RwSignal::new(None),
        }
    }
}

/// Load client ID from persistent storage, or generate a new UUID v4.
fn load_or_create_client_id() -> String {
    if let Some(id) = session::get_persistent(CLIENT_ID_KEY) {
        if !id.is_empty() {
            return id;
        }
    }
    let id = Uuid::new_v4().to_string();
    session::set_persistent(CLIENT_ID_KEY, &id);
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_id_format() {
        // Just test that UUID generation works (no localStorage in test)
        let id = Uuid::new_v4().to_string();
        assert_eq!(id.len(), 36); // UUID v4 string length
        assert!(id.contains('-'));
    }
}
```

- [ ] **Step 2: Add module to `state/mod.rs`**

Add `pub mod stage;` to `crates/presenter-ui/src/state/mod.rs`.

- [ ] **Step 3: Verify compilation**

```bash
cargo check -p presenter-ui
```

- [ ] **Step 4: Commit**

```bash
git add crates/presenter-ui/src/state/stage.rs crates/presenter-ui/src/state/mod.rs
git commit -m "feat(stage): add StageContext with client ID persistence"
```

---

### Task 5: Stage CSS

**Files:**
- Create: `crates/presenter-ui/styles/stage.css`
- Modify: `crates/presenter-ui/index.html`

- [ ] **Step 1: Create `styles/stage.css`**

```css
/* Stage display — full-screen, black background, no scrolling */

body.stage {
    margin: 0;
    padding: 0;
    background: #000;
    color: #fff;
    overflow: hidden;
    font-family: 'Inter', system-ui, -apple-system, sans-serif;
    width: 100vw;
    height: 100vh;
}

.stage-container {
    position: relative;
    width: 100vw;
    height: 100vh;
    overflow: hidden;
}

/* ===== worship-snv layout ===== */

.stage__current-group {
    position: absolute;
    left: 25%;
    top: 1%;
    width: 50%;
    height: 5%;
    display: flex;
    align-items: center;
    justify-content: center;
    overflow: hidden;
}

.stage__current-slide {
    position: absolute;
    left: 2%;
    top: 7%;
    width: 96%;
    height: 48%;
    display: flex;
    align-items: flex-start;
    justify-content: center;
    overflow: hidden;
}

.stage__next-group {
    position: absolute;
    left: 25%;
    top: 56%;
    width: 50%;
    height: 4%;
    display: flex;
    align-items: center;
    justify-content: center;
    overflow: hidden;
}

.stage__next-slide {
    position: absolute;
    left: 2%;
    top: 61%;
    width: 96%;
    height: 30%;
    display: flex;
    align-items: flex-start;
    justify-content: center;
    overflow: hidden;
}

/* ===== Status bar ===== */

.stage__status-bar {
    position: absolute;
    left: 0;
    bottom: 0;
    width: 100%;
    height: 7%;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 2%;
    box-sizing: border-box;
    overflow: hidden;
}

.stage__clock {
    color: #38bdf8;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    font-size: 2vw;
}

.stage__live-pill {
    padding: 0.3rem 1.2rem;
    border-radius: 999px;
    font-weight: 700;
    font-size: 0.9vw;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    white-space: nowrap;
    flex-shrink: 0;
}

.stage__live-pill--off {
    background: rgba(34, 197, 94, 0.9);
    color: #fff;
    box-shadow: 0 0 20px rgba(34, 197, 94, 0.5);
}

.stage__live-pill--on {
    background: rgba(239, 68, 68, 0.95);
    color: #fff;
    box-shadow: 0 0 30px rgba(239, 68, 68, 0.7);
    animation: stage-pulse 2s ease-in-out infinite;
}

@keyframes stage-pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.7; }
}

.stage__connection {
    color: #38bdf8;
    font-size: 0.9vw;
    font-weight: 600;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    white-space: nowrap;
}

.stage__connection-latency {
    opacity: 0.7;
    font-variant-numeric: tabular-nums;
}

/* ===== Group pills ===== */

.stage__group-pill {
    border-radius: 999px;
    padding: 0.15rem 0.6rem;
    letter-spacing: 0.18em;
    text-transform: uppercase;
    font-weight: 700;
}

.stage__current-group .stage__group-pill {
    font-size: 0.9vw;
}

.stage__next-group .stage__group-pill {
    font-size: 0.7vw;
}

/* ===== Slide text ===== */

.stage__slide-text {
    text-align: center;
    line-height: 1.1;
    margin: 0;
    white-space: pre-wrap;
    font-weight: 700;
    width: 100%;
}

.stage__current-slide .stage__slide-text {
    color: #f8fafc;
}

.stage__next-slide .stage__slide-text {
    color: #cbd5f5;
}

/* ===== Bible overlay ===== */

.stage__bible-overlay {
    position: absolute;
    top: 0;
    left: 0;
    width: 100%;
    height: 100%;
    background: rgba(0, 0, 0, 0.92);
    z-index: 100;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 4%;
    box-sizing: border-box;
}

.stage__bible-overlay[data-visible="false"] {
    display: none;
}

.stage__bible-text {
    color: #f8fafc;
    font-size: 3vw;
    font-weight: 700;
    text-align: center;
    line-height: 1.3;
    margin-bottom: 1rem;
}

.stage__bible-reference {
    color: #94a3b8;
    font-size: 1.5vw;
    font-weight: 600;
    text-align: center;
    letter-spacing: 0.05em;
}

.stage__bible-secondary {
    margin-top: 2rem;
    padding-top: 2rem;
    border-top: 1px solid rgba(255, 255, 255, 0.15);
    text-align: center;
}

.stage__bible-secondary[data-visible="false"] {
    display: none;
}

.stage__bible-secondary-text {
    color: #cbd5e1;
    font-size: 2.2vw;
    font-weight: 600;
    line-height: 1.3;
    margin-bottom: 0.5rem;
}

.stage__bible-secondary-ref {
    color: #64748b;
    font-size: 1.2vw;
    font-weight: 600;
    letter-spacing: 0.05em;
}

/* ===== worship-pp layout (slides + playlist sidebar) ===== */

.stage-pp__slides-area {
    position: absolute;
    left: 0;
    top: 0;
    width: 70%;
    height: 92%;
    overflow: hidden;
}

.stage-pp__playlist-sidebar {
    position: absolute;
    right: 0;
    top: 0;
    width: 30%;
    height: 92%;
    overflow-y: auto;
    border-left: 1px solid rgba(255, 255, 255, 0.1);
    padding: 1% 1.5%;
    box-sizing: border-box;
}

.stage-pp__playlist-entry {
    padding: 0.4rem 0.6rem;
    color: #94a3b8;
    font-size: 0.9vw;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    border-radius: 4px;
    margin-bottom: 2px;
}

.stage-pp__playlist-entry--active {
    background: rgba(56, 189, 248, 0.15);
    color: #f8fafc;
    font-weight: 700;
}

/* ===== timer layout ===== */

.stage-timer__display {
    position: absolute;
    left: 5%;
    top: 10%;
    width: 90%;
    height: 75%;
    display: flex;
    align-items: center;
    justify-content: center;
    overflow: hidden;
}

.stage-timer__text {
    color: #f8fafc;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    text-align: center;
}

/* ===== preach layout ===== */

.stage-preach__display {
    position: absolute;
    left: 5%;
    top: 10%;
    width: 90%;
    height: 75%;
    display: flex;
    align-items: center;
    justify-content: center;
    overflow: hidden;
}

.stage-preach__text {
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    text-align: center;
}

.stage-preach__text--normal {
    color: #f8fafc;
}

.stage-preach__text--overtime {
    color: #ef4444;
}

/* ===== Connection state indicators ===== */

.stage__connection--connecting {
    color: #fbbf24;
}

.stage__connection--connected {
    color: #38bdf8;
}

.stage__connection--reconnecting {
    color: #fb923c;
}

.stage__connection--disconnected {
    color: #ef4444;
}
```

- [ ] **Step 2: Add CSS link to `index.html`**

In `crates/presenter-ui/index.html`, add after the existing CSS links:

```html
<link data-trunk rel="css" href="styles/stage.css" />
```

- [ ] **Step 3: Commit**

```bash
git add crates/presenter-ui/styles/stage.css crates/presenter-ui/index.html
git commit -m "feat(stage): add stage display CSS with layout presets"
```

---

### Task 6: Status Bar Component

**Files:**
- Create: `crates/presenter-ui/src/components/stage/status_bar.rs`
- Create: `crates/presenter-ui/src/components/stage/mod.rs`
- Modify: `crates/presenter-ui/src/components/mod.rs`

- [ ] **Step 1: Create `components/stage/mod.rs`**

```rust
// crates/presenter-ui/src/components/stage/mod.rs

pub mod status_bar;
```

- [ ] **Step 2: Create `status_bar.rs`**

```rust
// crates/presenter-ui/src/components/stage/status_bar.rs

use gloo_timers::callback::Interval;
use leptos::prelude::*;

use crate::state::stage::StageContext;
use crate::ws::stage::StageWsState;

/// Shared status bar: clock | live indicator | connection status.
#[component]
pub fn StatusBar(
    ws_state: ReadSignal<StageWsState>,
    latency_ms: ReadSignal<Option<f64>>,
) -> impl IntoView {
    let ctx = use_context::<StageContext>().expect("StageContext not provided");

    // Real-time clock signal, updated every second
    let (clock_text, set_clock_text) = signal(current_time_string());
    let _clock_interval = Interval::new(1_000, move || {
        set_clock_text.set(current_time_string());
    });
    _clock_interval.forget();

    // Connection label
    let connection_label = move || match ws_state.get() {
        StageWsState::Connecting => "CONNECTING\u{2026}",
        StageWsState::Connected => "CONNECTED",
        StageWsState::Reconnecting => "RECONNECTING\u{2026}",
        StageWsState::Disconnected => "DISCONNECTED",
    };

    let connection_class = move || {
        let base = "stage__connection";
        match ws_state.get() {
            StageWsState::Connecting => format!("{base} {base}--connecting"),
            StageWsState::Connected => format!("{base} {base}--connected"),
            StageWsState::Reconnecting => format!("{base} {base}--reconnecting"),
            StageWsState::Disconnected => format!("{base} {base}--disconnected"),
        }
    };

    let latency_text = move || {
        latency_ms.get().map(|ms| format!("\u{00b7} {:03} ms", ms as u32))
    };

    let broadcast_live = ctx.broadcast_live;

    view! {
        <div class="stage__status-bar">
            // Clock
            <span class="stage__clock">{clock_text}</span>

            // Live indicator
            {move || {
                let is_live = broadcast_live.get();
                let (class, text) = if is_live {
                    ("stage__live-pill stage__live-pill--on", "LIVE")
                } else {
                    ("stage__live-pill stage__live-pill--off", "VYSIELANIE JE VYPNUTE")
                };
                view! { <span class=class>{text}</span> }
            }}

            // Connection status
            <span class=connection_class>
                {connection_label}
                {move || latency_text().map(|t| view! {
                    <span class="stage__connection-latency">{" "}{t}</span>
                })}
            </span>
        </div>
    }
}

fn current_time_string() -> String {
    let now = js_sys::Date::new_0();
    format!(
        "{:02}:{:02}:{:02}",
        now.get_hours(),
        now.get_minutes(),
        now.get_seconds()
    )
}
```

- [ ] **Step 3: Add `pub mod stage;` to `components/mod.rs`**

Add `pub mod stage;` to `crates/presenter-ui/src/components/mod.rs`.

- [ ] **Step 4: Verify compilation**

```bash
cargo check -p presenter-ui
```

- [ ] **Step 5: Commit**

```bash
git add crates/presenter-ui/src/components/stage/ crates/presenter-ui/src/components/mod.rs
git commit -m "feat(stage): add status bar component with clock, live indicator, connection"
```

---

### Task 7: Bible Overlay Component

**Files:**
- Create: `crates/presenter-ui/src/components/stage/bible_overlay.rs`
- Modify: `crates/presenter-ui/src/components/stage/mod.rs`

- [ ] **Step 1: Create `bible_overlay.rs`**

```rust
// crates/presenter-ui/src/components/stage/bible_overlay.rs

use leptos::prelude::*;
use presenter_core::BibleSlideOutput;

/// Full-screen Bible overlay — visible when Bible broadcast is active.
#[component]
pub fn BibleOverlay(overlay: RwSignal<Option<BibleSlideOutput>>) -> impl IntoView {
    let visible = move || overlay.get().is_some();
    let data_visible = move || if visible() { "true" } else { "false" };

    view! {
        <div class="stage__bible-overlay" data-visible=data_visible>
            {move || overlay.get().map(|output| {
                let has_secondary = !output.secondary_text.is_empty();
                let secondary_visible = if has_secondary { "true" } else { "false" };

                view! {
                    <div class="stage__bible-text">{output.main_text.clone()}</div>
                    <div class="stage__bible-reference">{output.main_reference.clone()}</div>

                    <div class="stage__bible-secondary" data-visible=secondary_visible>
                        <div class="stage__bible-secondary-text">{output.secondary_text.clone()}</div>
                        <div class="stage__bible-secondary-ref">{output.secondary_reference.clone()}</div>
                    </div>
                }
            })}
        </div>
    }
}
```

- [ ] **Step 2: Add to `stage/mod.rs`**

```rust
pub mod bible_overlay;
```

- [ ] **Step 3: Verify compilation**

```bash
cargo check -p presenter-ui
```

- [ ] **Step 4: Commit**

```bash
git add crates/presenter-ui/src/components/stage/bible_overlay.rs crates/presenter-ui/src/components/stage/mod.rs
git commit -m "feat(stage): add Bible overlay component"
```

---

### Task 8: Worship-SNV Layout Component

**Files:**
- Create: `crates/presenter-ui/src/components/stage/worship_snv.rs`
- Modify: `crates/presenter-ui/src/components/stage/mod.rs`

Primary layout. Uses auto-fit for text sizing, group color pills, top-aligned text.

- [ ] **Step 1: Create `worship_snv.rs`**

```rust
// crates/presenter-ui/src/components/stage/worship_snv.rs

use leptos::prelude::*;
use web_sys::HtmlElement;

use crate::state::stage::StageContext;
use crate::utils::autofit::autofit_text;
use crate::utils::color::{group_color, hex_to_rgba};
use crate::ws::stage::StageWsState;

/// Maximum font sizes for auto-fit (px).
const CURRENT_MAX_FONT: f64 = 120.0;
const NEXT_MAX_FONT: f64 = 80.0;

/// worship-snv layout: current/next slides with group labels and status bar.
#[component]
pub fn WorshipSnv(
    ws_state: ReadSignal<StageWsState>,
    latency_ms: ReadSignal<Option<f64>>,
) -> impl IntoView {
    let ctx = use_context::<StageContext>().expect("StageContext not provided");

    // Node refs for auto-fit measurement
    let current_text_ref = NodeRef::<leptos::html::Div>::new();
    let next_text_ref = NodeRef::<leptos::html::Div>::new();

    // Current slide text
    let current_text = move || {
        ctx.snapshot.get().and_then(|s| {
            s.current.map(|slide| {
                if !slide.stage.is_empty() { slide.stage } else { slide.main }
            })
        }).unwrap_or_default()
    };

    // Next slide text
    let next_text = move || {
        ctx.snapshot.get().and_then(|s| {
            s.next.map(|slide| {
                if !slide.stage.is_empty() { slide.stage } else { slide.main }
            })
        }).unwrap_or_default()
    };

    // Current group
    let current_group = move || {
        ctx.snapshot.get().and_then(|s| s.current.and_then(|slide| slide.group))
    };

    // Next group
    let next_group = move || {
        ctx.snapshot.get().and_then(|s| s.next.and_then(|slide| slide.group))
    };

    // Auto-fit effect for current text
    {
        let current_text_ref = current_text_ref.clone();
        Effect::new(move |_| {
            let _text = current_text(); // track dependency
            if let Some(el) = current_text_ref.get() {
                let html_el: &HtmlElement = &el;
                // Use requestAnimationFrame to ensure DOM is updated
                let el_clone = html_el.clone();
                let cb = wasm_bindgen::closure::Closure::once_into_js(move || {
                    autofit_text(&el_clone, CURRENT_MAX_FONT);
                });
                let _ = web_sys::window()
                    .expect("window")
                    .request_animation_frame(cb.as_ref().unchecked_ref());
            }
        });
    }

    // Auto-fit effect for next text
    {
        let next_text_ref = next_text_ref.clone();
        Effect::new(move |_| {
            let _text = next_text(); // track dependency
            if let Some(el) = next_text_ref.get() {
                let html_el: &HtmlElement = &el;
                let el_clone = html_el.clone();
                let cb = wasm_bindgen::closure::Closure::once_into_js(move || {
                    autofit_text(&el_clone, NEXT_MAX_FONT);
                });
                let _ = web_sys::window()
                    .expect("window")
                    .request_animation_frame(cb.as_ref().unchecked_ref());
            }
        });
    }

    view! {
        <div class="stage-container" data-layout="worship-snv">
            // Current group pill
            <div class="stage__current-group">
                {move || current_group().map(|name| {
                    let color = group_color(&name);
                    let bg = hex_to_rgba(color, 0.25);
                    view! {
                        <span
                            class="stage__group-pill"
                            style=format!("color:{color};background:{bg};")
                        >
                            {name}
                        </span>
                    }
                })}
            </div>

            // Current slide text (top-aligned, auto-fit)
            <div class="stage__current-slide">
                <div
                    node_ref=current_text_ref
                    class="stage__slide-text"
                >
                    {current_text}
                </div>
            </div>

            // Next group pill
            <div class="stage__next-group">
                {move || next_group().map(|name| {
                    let color = group_color(&name);
                    let bg = hex_to_rgba(color, 0.25);
                    view! {
                        <span
                            class="stage__group-pill"
                            style=format!("color:{color};background:{bg};")
                        >
                            {name}
                        </span>
                    }
                })}
            </div>

            // Next slide text (top-aligned, auto-fit)
            <div class="stage__next-slide">
                <div
                    node_ref=next_text_ref
                    class="stage__slide-text"
                >
                    {next_text}
                </div>
            </div>

            // Status bar
            <super::status_bar::StatusBar ws_state=ws_state latency_ms=latency_ms />

            // Bible overlay
            <super::bible_overlay::BibleOverlay overlay=ctx.bible_overlay />
        </div>
    }
}
```

- [ ] **Step 2: Add to `stage/mod.rs`**

```rust
pub mod worship_snv;
```

- [ ] **Step 3: Add `use wasm_bindgen::JsCast;` to worship_snv.rs imports if needed for `unchecked_ref`**

Check if `JsCast` is needed for the `request_animation_frame` call. If so, add:
```rust
use wasm_bindgen::JsCast;
```

- [ ] **Step 4: Verify compilation**

```bash
cargo check -p presenter-ui
```

- [ ] **Step 5: Commit**

```bash
git add crates/presenter-ui/src/components/stage/worship_snv.rs crates/presenter-ui/src/components/stage/mod.rs
git commit -m "feat(stage): add worship-snv layout component with auto-fit text"
```

---

### Task 9: Remaining Layout Components

**Files:**
- Create: `crates/presenter-ui/src/components/stage/worship_pp.rs`
- Create: `crates/presenter-ui/src/components/stage/timer_layout.rs`
- Create: `crates/presenter-ui/src/components/stage/preach_layout.rs`
- Modify: `crates/presenter-ui/src/components/stage/mod.rs`

- [ ] **Step 1: Create `worship_pp.rs`**

```rust
// crates/presenter-ui/src/components/stage/worship_pp.rs

use leptos::prelude::*;
use web_sys::HtmlElement;
use wasm_bindgen::JsCast;

use crate::state::stage::StageContext;
use crate::utils::autofit::autofit_text;
use crate::utils::color::{group_color, hex_to_rgba};
use crate::ws::stage::StageWsState;

const CURRENT_MAX_FONT: f64 = 100.0;
const NEXT_MAX_FONT: f64 = 60.0;

/// worship-pp layout: slides area (70%) + playlist sidebar (30%).
#[component]
pub fn WorshipPp(
    ws_state: ReadSignal<StageWsState>,
    latency_ms: ReadSignal<Option<f64>>,
) -> impl IntoView {
    let ctx = use_context::<StageContext>().expect("StageContext not provided");

    let current_text_ref = NodeRef::<leptos::html::Div>::new();
    let next_text_ref = NodeRef::<leptos::html::Div>::new();

    let current_text = move || {
        ctx.snapshot.get().and_then(|s| {
            s.current.map(|slide| if !slide.stage.is_empty() { slide.stage } else { slide.main })
        }).unwrap_or_default()
    };

    let next_text = move || {
        ctx.snapshot.get().and_then(|s| {
            s.next.map(|slide| if !slide.stage.is_empty() { slide.stage } else { slide.main })
        }).unwrap_or_default()
    };

    let current_group = move || {
        ctx.snapshot.get().and_then(|s| s.current.and_then(|slide| slide.group))
    };

    let next_group = move || {
        ctx.snapshot.get().and_then(|s| s.next.and_then(|slide| slide.group))
    };

    let playlist_entries = move || {
        ctx.snapshot.get().and_then(|s| s.playlist_entries).unwrap_or_default()
    };

    // Auto-fit effects (same pattern as worship-snv)
    {
        let r = current_text_ref.clone();
        Effect::new(move |_| {
            let _t = current_text();
            if let Some(el) = r.get() {
                let html_el: &HtmlElement = &el;
                let el_clone = html_el.clone();
                let cb = wasm_bindgen::closure::Closure::once_into_js(move || {
                    autofit_text(&el_clone, CURRENT_MAX_FONT);
                });
                let _ = web_sys::window().expect("window")
                    .request_animation_frame(cb.as_ref().unchecked_ref());
            }
        });
    }
    {
        let r = next_text_ref.clone();
        Effect::new(move |_| {
            let _t = next_text();
            if let Some(el) = r.get() {
                let html_el: &HtmlElement = &el;
                let el_clone = html_el.clone();
                let cb = wasm_bindgen::closure::Closure::once_into_js(move || {
                    autofit_text(&el_clone, NEXT_MAX_FONT);
                });
                let _ = web_sys::window().expect("window")
                    .request_animation_frame(cb.as_ref().unchecked_ref());
            }
        });
    }

    view! {
        <div class="stage-container" data-layout="worship-pp">
            <div class="stage-pp__slides-area">
                <div class="stage__current-group" style="left:14%;width:72%;">
                    {move || current_group().map(|name| {
                        let color = group_color(&name);
                        let bg = hex_to_rgba(color, 0.25);
                        view! {
                            <span class="stage__group-pill" style=format!("color:{color};background:{bg};")>
                                {name}
                            </span>
                        }
                    })}
                </div>
                <div class="stage__current-slide" style="width:66%;left:2%;">
                    <div node_ref=current_text_ref class="stage__slide-text">{current_text}</div>
                </div>
                <div class="stage__next-group" style="left:14%;width:72%;">
                    {move || next_group().map(|name| {
                        let color = group_color(&name);
                        let bg = hex_to_rgba(color, 0.25);
                        view! {
                            <span class="stage__group-pill" style=format!("color:{color};background:{bg};")>
                                {name}
                            </span>
                        }
                    })}
                </div>
                <div class="stage__next-slide" style="width:66%;left:2%;">
                    <div node_ref=next_text_ref class="stage__slide-text">{next_text}</div>
                </div>
            </div>

            <div class="stage-pp__playlist-sidebar">
                <For
                    each=playlist_entries
                    key=|entry| entry.name.clone()
                    children=move |entry| {
                        let class = if entry.is_active {
                            "stage-pp__playlist-entry stage-pp__playlist-entry--active"
                        } else {
                            "stage-pp__playlist-entry"
                        };
                        view! { <div class=class>{entry.name.clone()}</div> }
                    }
                />
            </div>

            <super::status_bar::StatusBar ws_state=ws_state latency_ms=latency_ms />
            <super::bible_overlay::BibleOverlay overlay=ctx.bible_overlay />
        </div>
    }
}
```

- [ ] **Step 2: Create `timer_layout.rs`**

```rust
// crates/presenter-ui/src/components/stage/timer_layout.rs

use leptos::prelude::*;
use web_sys::HtmlElement;
use wasm_bindgen::JsCast;

use crate::state::stage::StageContext;
use crate::utils::autofit::autofit_text;
use crate::ws::stage::StageWsState;

const TIMER_MAX_FONT: f64 = 300.0;

/// timer layout: large countdown centered + status bar.
#[component]
pub fn TimerLayout(
    ws_state: ReadSignal<StageWsState>,
    latency_ms: ReadSignal<Option<f64>>,
) -> impl IntoView {
    let ctx = use_context::<StageContext>().expect("StageContext not provided");

    let timer_ref = NodeRef::<leptos::html::Div>::new();

    let timer_text = move || {
        ctx.snapshot.get().map(|s| {
            let overview = &s.timers;
            if let Some(countdown) = &overview.countdown_to_start {
                format_seconds(countdown.seconds_remaining)
            } else {
                "00:00".to_string()
            }
        }).unwrap_or_else(|| "00:00".to_string())
    };

    {
        let r = timer_ref.clone();
        Effect::new(move |_| {
            let _t = timer_text();
            if let Some(el) = r.get() {
                let html_el: &HtmlElement = &el;
                let el_clone = html_el.clone();
                let cb = wasm_bindgen::closure::Closure::once_into_js(move || {
                    autofit_text(&el_clone, TIMER_MAX_FONT);
                });
                let _ = web_sys::window().expect("window")
                    .request_animation_frame(cb.as_ref().unchecked_ref());
            }
        });
    }

    view! {
        <div class="stage-container" data-layout="timer">
            <div class="stage-timer__display">
                <div node_ref=timer_ref class="stage-timer__text">{timer_text}</div>
            </div>
            <super::status_bar::StatusBar ws_state=ws_state latency_ms=latency_ms />
            <super::bible_overlay::BibleOverlay overlay=ctx.bible_overlay />
        </div>
    }
}

fn format_seconds(seconds: i64) -> String {
    let secs = seconds.max(0);
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h:02}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}
```

- [ ] **Step 3: Create `preach_layout.rs`**

```rust
// crates/presenter-ui/src/components/stage/preach_layout.rs

use leptos::prelude::*;
use web_sys::HtmlElement;
use wasm_bindgen::JsCast;

use crate::state::stage::StageContext;
use crate::utils::autofit::autofit_text;
use crate::ws::stage::StageWsState;

const PREACH_MAX_FONT: f64 = 300.0;

/// preach layout: stopwatch centered + overtime indicator + status bar.
#[component]
pub fn PreachLayout(
    ws_state: ReadSignal<StageWsState>,
    latency_ms: ReadSignal<Option<f64>>,
) -> impl IntoView {
    let ctx = use_context::<StageContext>().expect("StageContext not provided");

    let preach_ref = NodeRef::<leptos::html::Div>::new();

    let preach_data = move || {
        ctx.snapshot.get().map(|s| {
            let overview = &s.timers;
            if let Some(preach) = &overview.preach_timer {
                let text = format_seconds(preach.seconds_elapsed);
                let overtime = preach.seconds_elapsed > preach.target_seconds.unwrap_or(i64::MAX);
                (text, overtime)
            } else {
                ("00:00".to_string(), false)
            }
        }).unwrap_or_else(|| ("00:00".to_string(), false))
    };

    {
        let r = preach_ref.clone();
        Effect::new(move |_| {
            let _d = preach_data();
            if let Some(el) = r.get() {
                let html_el: &HtmlElement = &el;
                let el_clone = html_el.clone();
                let cb = wasm_bindgen::closure::Closure::once_into_js(move || {
                    autofit_text(&el_clone, PREACH_MAX_FONT);
                });
                let _ = web_sys::window().expect("window")
                    .request_animation_frame(cb.as_ref().unchecked_ref());
            }
        });
    }

    view! {
        <div class="stage-container" data-layout="preach">
            <div class="stage-preach__display">
                <div
                    node_ref=preach_ref
                    class=move || {
                        let (_, overtime) = preach_data();
                        if overtime {
                            "stage-preach__text stage-preach__text--overtime"
                        } else {
                            "stage-preach__text stage-preach__text--normal"
                        }
                    }
                >
                    {move || preach_data().0}
                </div>
            </div>
            <super::status_bar::StatusBar ws_state=ws_state latency_ms=latency_ms />
            <super::bible_overlay::BibleOverlay overlay=ctx.bible_overlay />
        </div>
    }
}

fn format_seconds(seconds: i64) -> String {
    let secs = seconds.max(0);
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h:02}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}
```

- [ ] **Step 4: Update `stage/mod.rs` with all modules**

```rust
pub mod bible_overlay;
pub mod preach_layout;
pub mod status_bar;
pub mod timer_layout;
pub mod worship_pp;
pub mod worship_snv;
```

- [ ] **Step 5: Verify compilation**

```bash
cargo check -p presenter-ui
```

- [ ] **Step 6: Commit**

```bash
git add crates/presenter-ui/src/components/stage/
git commit -m "feat(stage): add worship-pp, timer, preach layout components"
```

---

### Task 10: Stage Page + Route Registration

**Files:**
- Create: `crates/presenter-ui/src/pages/stage.rs`
- Modify: `crates/presenter-ui/src/pages/mod.rs`
- Modify: `crates/presenter-ui/src/lib.rs`

This is the orchestration component that wires everything together.

- [ ] **Step 1: Create `pages/stage.rs`**

```rust
// crates/presenter-ui/src/pages/stage.rs

use leptos::prelude::*;
use presenter_core::LiveEvent;
use wasm_bindgen::prelude::*;

use crate::api;
use crate::components::stage::{
    preach_layout::PreachLayout,
    timer_layout::TimerLayout,
    worship_pp::WorshipPp,
    worship_snv::WorshipSnv,
};
use crate::state::stage::StageContext;
use crate::ws::stage::{self, StageWsState};

/// Stage display page — full-screen WASM stage.
#[component]
pub fn StagePage() -> impl IntoView {
    // Set body class for stage CSS
    if let Some(body) = crate::utils::window::document_body() {
        let _ = body.set_attribute("class", "stage");
    }

    // Create context with default layout
    let ctx = StageContext::new("worship-snv".to_string());
    provide_context(ctx.clone());

    // Expose test helpers
    expose_test_globals(&ctx);

    // Connect stage WebSocket
    let ws_handle = stage::use_stage_websocket(
        ctx.client_id.clone(),
        ctx.layout_code,
    );

    // Expose connection state for E2E tests
    {
        let ws_state = ws_handle.state;
        Effect::new(move |_| {
            let state_str = match ws_state.get() {
                StageWsState::Connecting => "connecting",
                StageWsState::Connected => "connected",
                StageWsState::Reconnecting => "reconnecting",
                StageWsState::Disconnected => "disconnected",
            };
            set_global_string("__presenterStageConnectionState", state_str);
        });
    }

    // Handle WebSocket events → update context signals
    {
        let ctx = ctx.clone();
        let last_event = ws_handle.last_event;
        Effect::new(move |_| {
            let Some(event) = last_event.get() else { return };
            match event {
                LiveEvent::Stage { snapshot } => {
                    ctx.snapshot.set(Some(snapshot));
                }
                LiveEvent::StageLayout { code } => {
                    ctx.layout_code.set(code);
                }
                LiveEvent::BibleSlide { output } => {
                    ctx.bible_overlay.set(Some(output));
                }
                LiveEvent::BibleCleared => {
                    ctx.bible_overlay.set(None);
                }
                LiveEvent::BroadcastLive { enabled } => {
                    ctx.broadcast_live.set(enabled);
                }
                LiveEvent::Timers { overview } => {
                    // Update timers in snapshot (for timer/preach layouts)
                    ctx.snapshot.update(|snap| {
                        if let Some(s) = snap {
                            s.timers = overview;
                        }
                    });
                }
                _ => {} // Ignore events not relevant to stage
            }
        });
    }

    // Fetch initial data
    {
        let ctx = ctx.clone();
        leptos::task::spawn_local(async move {
            // Fetch current layout
            if let Ok(layout_resp) = api::stage::get_layout().await {
                ctx.layout_code.set(layout_resp.code);
            }
            // Fetch initial snapshot
            if let Ok(snapshot) = api::stage::get_snapshot().await {
                ctx.snapshot.set(Some(snapshot));
            }
            // Fetch broadcast live state
            if let Ok(broadcast) = api::stage::get_broadcast_live().await {
                ctx.broadcast_live.set(broadcast.enabled);
            }
            // Fetch active bible slide (if any)
            if let Ok(Some(output)) = api::bible::get_active_slide_output().await {
                ctx.bible_overlay.set(Some(output));
            }
        });
    }

    // Render layout based on layout_code signal
    let ws_state = ws_handle.state;
    let latency_ms = ws_handle.latency_ms;
    let layout_code = ctx.layout_code;

    view! {
        {move || {
            let code = layout_code.get();
            match code.as_str() {
                "worship-pp" => view! { <WorshipPp ws_state=ws_state latency_ms=latency_ms /> }.into_any(),
                "timer" => view! { <TimerLayout ws_state=ws_state latency_ms=latency_ms /> }.into_any(),
                "preach" => view! { <PreachLayout ws_state=ws_state latency_ms=latency_ms /> }.into_any(),
                _ => view! { <WorshipSnv ws_state=ws_state latency_ms=latency_ms /> }.into_any(),
            }
        }}
    }
}

fn expose_test_globals(ctx: &StageContext) {
    set_global_string("__presenterStageClientId", &ctx.client_id);
    set_global_string("__presenterStageLayout", &ctx.layout_code.get_untracked());
}

fn set_global_string(name: &str, value: &str) {
    if let Ok(js_val) = js_sys::Reflect::set(
        &js_sys::global(),
        &JsValue::from_str(name),
        &JsValue::from_str(value),
    ) {
        let _ = js_val;
    }
}
```

- [ ] **Step 2: Check if `api::bible::get_active_slide_output` exists**

Verify whether this function exists in `crates/presenter-ui/src/api/bible.rs`. If not, add it:

```rust
pub async fn get_active_slide_output() -> Result<Option<BibleSlideOutput>, ApiError> {
    let response: Option<BibleSlideOutput> = super::get_json("/bible/active-slide").await?;
    Ok(response)
}
```

The server endpoint `GET /bible/active-slide` already exists (`get_active_bible_slide_output` in `router/bible.rs`).

- [ ] **Step 3: Add `pub mod stage;` to `pages/mod.rs`**

- [ ] **Step 4: Update routing in `lib.rs`**

In the `App` component's `page_view` match, add before the `else` fallback:

```rust
} else if p == "/stage" {
    view! { <pages::stage::StagePage /> }.into_any()
}
```

- [ ] **Step 5: Verify compilation**

```bash
cargo check -p presenter-ui
```

- [ ] **Step 6: Commit**

```bash
git add crates/presenter-ui/src/pages/stage.rs crates/presenter-ui/src/pages/mod.rs crates/presenter-ui/src/lib.rs
git commit -m "feat(stage): add StagePage with route registration and WS event dispatch"
```

---

### Task 11: Server — Serve WASM Shell for /stage

**Files:**
- Modify: `crates/presenter-server/src/router.rs`

This is the critical switchover. After this, `/stage` serves the WASM app instead of server-rendered HTML.

- [ ] **Step 1: Change `/stage` route handler**

In `crates/presenter-server/src/router.rs`, change line 146:

```rust
// Before:
.route("/stage", get(stage::stage_display_selected_html))

// After:
.route("/stage", get(wasm_ui::wasm_ui_shell))
```

- [ ] **Step 2: Verify compilation**

```bash
cargo check -p presenter-server
```

The old `stage_display_selected_html` handler still exists but is now unreferenced. That's fine — it will be removed in the cleanup task.

- [ ] **Step 3: Commit**

```bash
git add crates/presenter-server/src/router.rs
git commit -m "feat(stage): serve WASM shell for /stage instead of server-rendered HTML"
```

---

### Task 12: Server — Remove Design/Appearance Endpoints

**Files:**
- Modify: `crates/presenter-server/src/router.rs`
- Modify: `crates/presenter-server/src/router/stage.rs`
- Modify: `crates/presenter-server/src/router/ui_routes.rs`

- [ ] **Step 1: Remove routes from `router.rs`**

Remove these route registrations:

```rust
// Remove these lines:
.route("/ui/stage-settings", get(ui_routes::stage_settings_ui))
.route("/ui/stage-design", get(ui_routes::stage_design_ui))
.route(
    "/stage/appearance/{layout}",
    get(stage::get_stage_appearance).put(stage::update_stage_appearance),
)
.route(
    "/stage/design/{layout}",
    get(stage::get_stage_design).put(stage::update_stage_design),
)
.route(
    "/stage/design/{layout}/reset",
    post(stage::reset_stage_design),
)
```

- [ ] **Step 2: Remove handler functions from `router/stage.rs`**

Remove these functions: `get_stage_appearance`, `update_stage_appearance`, `get_stage_design`, `update_stage_design`, `reset_stage_design`, `stage_display_selected_html`.

Also remove the `use crate::stage_ui;` import if it becomes unused.

- [ ] **Step 3: Remove handler functions from `router/ui_routes.rs`**

Remove `stage_settings_ui` and `stage_design_ui` functions.

- [ ] **Step 4: Verify compilation**

```bash
cargo check -p presenter-server
```

Fix any remaining references.

- [ ] **Step 5: Commit**

```bash
git add crates/presenter-server/src/router.rs crates/presenter-server/src/router/stage.rs crates/presenter-server/src/router/ui_routes.rs
git commit -m "refactor(stage): remove design/appearance endpoints and WYSIWYG UI routes"
```

---

### Task 13: Server — Remove Stage UI Module

**Files:**
- Delete: `crates/presenter-server/src/stage_ui/mod.rs`
- Delete: `crates/presenter-server/src/stage_ui/layouts.rs`
- Delete: `crates/presenter-server/src/stage_ui/styles.rs`
- Delete: `crates/presenter-server/src/stage_ui/tests.rs` (if exists)
- Modify: `crates/presenter-server/src/main.rs` — remove `mod stage_ui;`

- [ ] **Step 1: Remove `mod stage_ui;` from `main.rs`**

- [ ] **Step 2: Delete the `stage_ui/` directory**

```bash
rm -rf crates/presenter-server/src/stage_ui/
```

- [ ] **Step 3: Delete `ui/stage_design.rs`**

```bash
rm crates/presenter-server/src/ui/stage_design.rs
```

Remove `pub mod stage_design;` from `crates/presenter-server/src/ui/mod.rs` and remove the `render_stage_design_ui` reference.

- [ ] **Step 4: Remove `state/stage_display.rs` methods**

Delete `crates/presenter-server/src/state/stage_display.rs` and remove `mod stage_display;` from `crates/presenter-server/src/state/mod.rs`.

If there are methods in `stage_display.rs` used by other parts, move those to `state/stage.rs` first.

- [ ] **Step 5: Verify compilation**

```bash
cargo check -p presenter-server
```

Fix any remaining references.

- [ ] **Step 6: Commit**

```bash
git add -A crates/presenter-server/
git commit -m "refactor(stage): delete server-rendered stage UI, design editor, appearance state"
```

---

### Task 14: Core — Remove StageDesign/StageAppearance Types

**Files:**
- Delete: `crates/presenter-core/src/stage_design.rs`
- Delete: `crates/presenter-core/src/stage_appearance.rs`
- Modify: `crates/presenter-core/src/lib.rs` — remove re-exports
- Modify: `crates/presenter-core/src/live.rs` — remove LiveEvent variants

- [ ] **Step 1: Remove `LiveEvent::StageAppearance` and `LiveEvent::StageDesign` from `live.rs`**

Also remove the imports of `StageAppearance` and `StageDesign` at the top of `live.rs`.

- [ ] **Step 2: Delete type files**

```bash
rm crates/presenter-core/src/stage_design.rs
rm crates/presenter-core/src/stage_appearance.rs
```

- [ ] **Step 3: Remove from `lib.rs`**

Remove `mod stage_design;`, `mod stage_appearance;` and their `pub use` re-exports.

- [ ] **Step 4: Fix compilation**

```bash
cargo check --workspace
```

Fix any remaining references to `StageDesign`, `StageBox`, `StageBoxType`, `StageAppearance`, `TextAlign` across the workspace.

- [ ] **Step 5: Commit**

```bash
git add -A crates/presenter-core/
git commit -m "refactor(core): remove StageDesign, StageAppearance, and related LiveEvent variants"
```

---

### Task 15: Delete Obsolete E2E Tests

**Files:**
- Delete: `tests/e2e/stage-design.spec.ts`
- Delete: `tests/e2e/stage-design-reset.spec.ts`
- Delete: `tests/e2e/stage-appearance.spec.ts`

- [ ] **Step 1: Delete the files**

```bash
rm tests/e2e/stage-design.spec.ts
rm tests/e2e/stage-design-reset.spec.ts
rm tests/e2e/stage-appearance.spec.ts
```

- [ ] **Step 2: Commit**

```bash
git add -A tests/e2e/
git commit -m "test(stage): delete obsolete design/appearance E2E tests"
```

---

### Task 16: Update Existing E2E Tests for WASM Stage

**Files:**
- Modify: `tests/e2e/stage-status-bar.spec.ts`
- Modify: `tests/e2e/stage-snapshot.spec.ts`
- Modify: `tests/e2e/stage-clear.spec.ts`

The WASM stage exposes the same `window.__presenterStageConnectionState` global as the old JS stage, so the existing `openStageDisplay()` helper pattern should work. But we need to also wait for `body[data-wasm-ready="true"]` since the WASM app takes a moment to load.

- [ ] **Step 1: Update `openStageDisplay()` in each test file**

In each file that has an `openStageDisplay` helper, add a WASM readiness wait:

```typescript
async function openStageDisplay(context: BrowserContext) {
    await context.request.post(new URL("/stage/layout", baseURL).toString(), {
        data: { code: "worship-snv" },
    });
    const stagePage = await context.newPage();
    await stagePage.goto(new URL("/stage", baseURL).toString(), {
        waitUntil: "domcontentloaded",
    });
    // Wait for WASM to load
    await stagePage.waitForSelector('body[data-wasm-ready="true"]', {
        timeout: 30_000,
    });
    // Wait for WebSocket connection
    await stagePage.waitForFunction(
        () => window.__presenterStageConnectionState === "connected",
        { timeout: 30_000 },
    );
    return stagePage;
}
```

- [ ] **Step 2: Review test assertions for DOM structure changes**

The WASM version uses different CSS class names:
- Old: `#current-text`, `#next-text`, `#stage-clock`, `#stage-live`
- New: `.stage__current-slide .stage__slide-text`, `.stage__next-slide .stage__slide-text`, `.stage__clock`, `.stage__live-pill`

Update selectors in each test file to match the new WASM DOM structure.

- [ ] **Step 3: Run existing tests to check**

```bash
npm run test:playwright -- --grep "stage"
```

- [ ] **Step 4: Fix any failing assertions**

- [ ] **Step 5: Commit**

```bash
git add tests/e2e/
git commit -m "test(stage): update E2E tests for WASM stage DOM structure"
```

---

### Task 17: New WASM Stage E2E Test

**Files:**
- Create: `tests/e2e/wasm-stage-display.spec.ts`

Comprehensive E2E test for the WASM stage display covering all key behaviors.

- [ ] **Step 1: Write the test file**

```typescript
import { test, expect, BrowserContext } from "@playwright/test";
import {
    deriveTestConfig,
    refreshDevData,
    startTestServer,
    stopServer,
    type ServerHandle,
} from "./support";

let serverHandle: ServerHandle | undefined;
let baseURL: string;

test.describe.configure({ timeout: 180_000 });

test.beforeAll(async ({}, testInfo) => {
    const config = deriveTestConfig(testInfo);
    baseURL = config.baseURL;
    await refreshDevData(config.dbUrl);
    serverHandle = await startTestServer(config.port, config.dbUrl);
});

test.afterAll(async () => {
    await stopServer(serverHandle);
});

async function openStageDisplay(context: BrowserContext, layout = "worship-snv") {
    await context.request.post(new URL("/stage/layout", baseURL).toString(), {
        data: { code: layout },
    });
    const stagePage = await context.newPage();

    const consoleMessages: string[] = [];
    stagePage.on("console", (msg) => {
        if (msg.type() === "error" || msg.type() === "warning") {
            consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
        }
    });

    await stagePage.goto(new URL("/stage", baseURL).toString(), {
        waitUntil: "domcontentloaded",
    });
    await stagePage.waitForSelector('body[data-wasm-ready="true"]', {
        timeout: 30_000,
    });
    await stagePage.waitForFunction(
        () => (window as any).__presenterStageConnectionState === "connected",
        { timeout: 30_000 },
    );
    return { stagePage, consoleMessages };
}

// Helper: trigger a slide on the server
async function triggerSlide(
    context: BrowserContext,
    presentationId: string,
    currentSlideId: string,
    nextSlideId?: string,
) {
    await context.request.post(new URL("/stage/state", baseURL).toString(), {
        data: {
            presentation_id: presentationId,
            current_slide_id: currentSlideId,
            next_slide_id: nextSlideId || null,
            playlist_id: null,
        },
    });
}

test.describe("WASM Stage Display", () => {
    test("loads and connects via WebSocket", async ({ context }) => {
        const { stagePage, consoleMessages } = await openStageDisplay(context);

        // Status bar shows "CONNECTED"
        const connection = stagePage.locator(".stage__connection");
        await expect(connection).toContainText("CONNECTED");

        // Clock is visible and updating
        const clock = stagePage.locator(".stage__clock");
        await expect(clock).toBeVisible();
        const clockText = await clock.textContent();
        expect(clockText).toMatch(/\d{2}:\d{2}:\d{2}/);

        // Live indicator visible
        const livePill = stagePage.locator(".stage__live-pill");
        await expect(livePill).toBeVisible();

        expect(consoleMessages).toEqual([]);
        await stagePage.close();
    });

    test("displays current and next slide text", async ({ context }) => {
        const { stagePage, consoleMessages } = await openStageDisplay(context);

        // Get a presentation and slide IDs from the API
        const libs = await (
            await context.request.get(new URL("/libraries", baseURL).toString())
        ).json();
        const firstLib = libs[0];
        const presentations = await (
            await context.request.get(
                new URL(`/libraries/${firstLib.id}/presentations`, baseURL).toString(),
            )
        ).json();

        if (presentations.length > 0) {
            const pres = presentations[0];
            const presDetail = await (
                await context.request.get(
                    new URL(`/presentations/${pres.id}`, baseURL).toString(),
                )
            ).json();

            if (presDetail.slides && presDetail.slides.length >= 2) {
                const slide1 = presDetail.slides[0];
                const slide2 = presDetail.slides[1];

                await triggerSlide(context, pres.id, slide1.id, slide2.id);

                // Wait for current slide text to appear
                const currentSlide = stagePage.locator(".stage__current-slide .stage__slide-text");
                await expect(currentSlide).not.toBeEmpty({ timeout: 5_000 });

                // Next slide should also have text
                const nextSlide = stagePage.locator(".stage__next-slide .stage__slide-text");
                await expect(nextSlide).not.toBeEmpty({ timeout: 5_000 });
            }
        }

        expect(consoleMessages).toEqual([]);
        await stagePage.close();
    });

    test("layout switching works", async ({ context }) => {
        const { stagePage, consoleMessages } = await openStageDisplay(context, "worship-snv");

        // Verify initial layout
        const container = stagePage.locator(".stage-container");
        await expect(container).toHaveAttribute("data-layout", "worship-snv");

        // Switch layout via API
        await context.request.post(new URL("/stage/layout", baseURL).toString(), {
            data: { code: "timer" },
        });

        // WASM should reactively switch layout (no page reload)
        await expect(container).toHaveAttribute("data-layout", "timer", { timeout: 5_000 });

        // Switch back
        await context.request.post(new URL("/stage/layout", baseURL).toString(), {
            data: { code: "worship-snv" },
        });
        await expect(container).toHaveAttribute("data-layout", "worship-snv", { timeout: 5_000 });

        expect(consoleMessages).toEqual([]);
        await stagePage.close();
    });

    test("clean console - no errors or warnings", async ({ context }) => {
        const { stagePage, consoleMessages } = await openStageDisplay(context);

        // Wait a few seconds for any async errors
        await stagePage.waitForTimeout(3_000);

        expect(consoleMessages).toEqual([]);
        await stagePage.close();
    });
});
```

- [ ] **Step 2: Run the new test**

```bash
npm run test:playwright -- wasm-stage-display
```

- [ ] **Step 3: Fix any failures**

- [ ] **Step 4: Commit**

```bash
git add tests/e2e/wasm-stage-display.spec.ts
git commit -m "test(stage): add comprehensive WASM stage display E2E tests"
```

---

### Task 18: Cargo.toml — Add Missing web-sys Features

**Files:**
- Modify: `crates/presenter-ui/Cargo.toml`

Check if any additional web-sys features are needed for the stage components. The auto-fit utility needs `scroll_height`, `scroll_width`, `client_height`, `client_width` on `HtmlElement` — these should be available with the existing `HtmlElement` feature. But `request_animation_frame` needs the `Window` feature (already present).

- [ ] **Step 1: Check if compilation works without changes**

```bash
cargo check -p presenter-ui
```

If it compiles, no changes needed. If not, add any missing web-sys features.

- [ ] **Step 2: If needed, add features**

Potential additions:
```toml
"CssStyleDeclaration",  # for element.style()
```

The `HtmlElement` feature already includes `style()` access, so this may not be needed.

- [ ] **Step 3: Commit if changes were needed**

```bash
git add crates/presenter-ui/Cargo.toml
git commit -m "build(ui): add web-sys features for stage display"
```

---

### Task 19: Local Lint + Full Compilation Check

**Files:** None (verification only)

- [ ] **Step 1: Run fmt**

```bash
cargo fmt --all --check
```

Fix if needed: `cargo fmt --all`

- [ ] **Step 2: Run clippy**

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

Fix all warnings.

- [ ] **Step 3: Run unit tests**

```bash
cargo test --workspace
```

All tests must pass.

- [ ] **Step 4: Fix any issues and commit**

```bash
git add -A
git commit -m "fix: address clippy warnings and formatting"
```

---

### Task 20: Version Bump, Push, CI

- [ ] **Step 1: Bump version**

```bash
git fetch origin
```

Check current version on dev vs main in `Cargo.toml` workspace `[workspace.package].version`. Bump minor version (this is a feature, not a patch).

- [ ] **Step 2: Commit version bump**

```bash
git add Cargo.toml
git commit -m "chore: bump version to X.Y.Z"
```

- [ ] **Step 3: Push and monitor CI**

```bash
git push origin dev
```

Monitor all pipeline jobs until terminal state. All must pass including E2E and deploy-dev.

- [ ] **Step 4: Fix any CI failures**

If CI fails, investigate with `gh run view <id> --log-failed`, fix, and push again.

---

## Verification

1. **Local**: `cargo check --workspace` compiles, `cargo test --workspace` passes, `cargo clippy` clean
2. **Dev deploy**: `http://10.77.8.134:8080/stage` loads WASM app, shows connected status, displays lyrics when triggered
3. **Layout switching**: Change layout via operator dropdown → stage display switches layout without page reload
4. **Text auto-fit**: Long lyrics shrink to fit, short lyrics are large
5. **Bible overlay**: Trigger Bible broadcast → overlay appears on stage
6. **E2E**: All Playwright tests pass including new `wasm-stage-display.spec.ts`
7. **CI**: All pipeline jobs green
8. **Google TV**: User confirms `http://10.77.9.205/stage` loads on Google TV with Fully Kiosk Browser after production deploy
