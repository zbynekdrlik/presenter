# PWA via Cloudflare Tunnel Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Serve each presenter instance over HTTPS via a per-machine Cloudflare Tunnel so the tablet can install the PWA, and ship a LAN/WAN indicator pill + info popover in the tablet UI.

**Architecture:** Three `cloudflared` daemons (one per instance) forward edge HTTPS → `localhost:80`. The server exposes `GET /api/network-mode` that classifies the client as local or remote from `CF-Connecting-IP` vs an operator-provided `PRESENTER_LOCAL_PUBLIC_IP`. The tablet UI adds a pill + info button based on the reaperiem pattern. No cert work in Rust, no TLS in the server, HTTP on LAN stays as-is.

**Tech Stack:** Rust (axum, serde), Leptos WASM, Playwright, cloudflared (Cloudflare-managed), GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-04-16-pwa-via-cloudflare-tunnel-design.md`

---

## Context

The server already binds HTTP on `PRESENTER_PORT` (see `crates/presenter-server/src/config.rs:69-77`). `AppState::from_config` in `crates/presenter-server/src/state/mod.rs:285` wires the full config into the state. The tablet UI's `TabletTimerBar` lives in `crates/presenter-ui/src/pages/tablet.rs:194+` and currently renders `clock / elapsed / state` spans. E2E tests live under `tests/e2e/`.

The reaperiem reference implementation lives at `zbynekdrlik/reaperiem`:
- `iem-mixer/crates/iem-server/src/routes.rs:229-261` — `detect_network_mode` + `is_private_ip`
- `iem-mixer/iem-ui/src/pages/mixer.rs:1346-1359` — network pill
- `iem-mixer/iem-ui/style.css` — `.network-indicator.local/.remote` styles
- `docs/cloudflare-tunnel-setup.md` — tunnel one-off setup steps

**Working directory:** `/home/newlevel/devel/presenter/presenter-dev2`. **Branch:** dev.

The bootstrap step (creating 3 tunnels + registering credentials as GitHub secrets) is a **one-off manual task** done by the operator once (Task 11 below). Until those secrets exist, the deploy workflow's cloudflared installation steps are no-ops — merging this PR before bootstrap is safe.

---

## File Structure

| File | Change |
|------|--------|
| `crates/presenter-server/src/config.rs` | Add `PRESENTER_LOCAL_PUBLIC_IP` parsing in a new `NetworkConfig` section |
| `crates/presenter-server/src/state/mod.rs` | Thread `local_public_ip: Option<String>` into `AppState` |
| `crates/presenter-server/src/router/network_mode.rs` | **New** — `detect_network_mode`, `is_private_ip`, HTTP handler |
| `crates/presenter-server/src/router/mod.rs` | Expose new module (likely `pub mod network_mode;`) |
| `crates/presenter-server/src/router.rs` | Register `GET /api/network-mode` |
| `crates/presenter-ui/src/api/network.rs` | **New** — `fetch_network_mode()` async helper |
| `crates/presenter-ui/src/api/mod.rs` | `pub mod network;` |
| `crates/presenter-ui/src/pages/tablet.rs` | Add network_mode signal, LAN/WAN pill, info button in TabletTimerBar |
| `crates/presenter-ui/src/components/info_popover.rs` | **New** — reusable popover showing version/host/mode |
| `crates/presenter-ui/src/components/mod.rs` | `pub mod info_popover;` |
| `crates/presenter-ui/styles/operator.css` | Pill + popover CSS (tablet uses operator.css) |
| `tests/e2e/tablet-network-indicator.spec.ts` | **New** — Playwright test |
| `.github/workflows/pipeline.yml` | Deploy-dev adds cloudflared install step (idempotent, skip if secret missing) |
| `.github/workflows/deploy.yml` | Deploy-prod same |
| `.github/workflows/release.yml` | Deploy-PP same |
| `deploy/cloudflared.service` | **New** — shared systemd unit template |
| `deploy/cloudflared-config.yml.tmpl` | **New** — shared config template with `__HOSTNAME__` + `__PORT__` placeholders |
| `docs/cloudflare-tunnel-setup.md` | **New** — one-off bootstrap instructions (ported + adapted from reaperiem) |
| `docs/configuration.md` | Document `PRESENTER_LOCAL_PUBLIC_IP` |
| `CLAUDE.md` | One-line note in the env-var table |

---

## Task 1: Add `PRESENTER_LOCAL_PUBLIC_IP` config

**Files:**
- Modify: `crates/presenter-server/src/config.rs`

- [ ] **Step 1: Add `NetworkConfig` struct and loader**

At the bottom of the struct list in `config.rs` (around line 54, after `AndroidConfig`), add:

```rust
#[derive(Debug, Clone, Default)]
pub struct NetworkConfig {
    /// The church's outbound public IP as seen by Cloudflare.
    /// Used by `/api/network-mode` to classify tunnel clients.
    /// Optional — falls back to private-range heuristic when unset.
    pub local_public_ip: Option<String>,
}

impl NetworkConfig {
    fn load() -> Self {
        let local_public_ip = env::var("PRESENTER_LOCAL_PUBLIC_IP")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        Self { local_public_ip }
    }
}
```

Add `pub network: NetworkConfig,` to `ServerConfig` and wire `network: NetworkConfig::load(),` in `ServerConfig::load()`:

```rust
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub http: HttpConfig,
    pub database: DatabaseConfig,
    pub companion: CompanionConfig,
    pub osc: OscConfig,
    pub stage: StageConfig,
    #[allow(dead_code)]
    pub android: AndroidConfig,
    pub network: NetworkConfig,
}

impl ServerConfig {
    pub fn load() -> Result<Self> {
        Ok(Self {
            http: HttpConfig::load()?,
            database: DatabaseConfig::load(),
            companion: CompanionConfig::load(),
            osc: OscConfig::load(),
            stage: StageConfig::load(),
            android: AndroidConfig::load(),
            network: NetworkConfig::load(),
        })
    }
}
```

- [ ] **Step 2: Verify build**

Run: `cd /home/newlevel/devel/presenter/presenter-dev2 && cargo check -p presenter-server`
Expected: clean. `state/mod.rs::from_config` destructures `config.database.url` etc. — we don't touch the existing fields so it still compiles.

- [ ] **Step 3: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git add crates/presenter-server/src/config.rs
git commit -m "feat(config): add PRESENTER_LOCAL_PUBLIC_IP for tunnel client classification (#218)"
```

---

## Task 2: Thread `local_public_ip` into `AppState`

**Files:**
- Modify: `crates/presenter-server/src/state/mod.rs`

- [ ] **Step 1: Add field to `AppState`**

Find the `AppState` struct definition (near the top of `state/mod.rs`). Add a field:

```rust
pub struct AppState {
    // ...existing fields...
    pub local_public_ip: Arc<Option<String>>,
}
```

`Arc` is used so the state can be cheaply cloned into per-request extractors. `Arc` is already imported at the top of `state/mod.rs` via `use std::sync::Arc` (verify; if not, add it).

- [ ] **Step 2: Populate it in `from_config`**

In `state/mod.rs::from_config` (around line 285), after the existing body, add `local_public_ip: Arc::new(config.network.local_public_ip),` to the `Self { ... }` construction. If the state is built via a helper like `new_with_heartbeat`, add the param there too and forward it.

If the construction goes through multiple layers, add a temp TODO with the exact location first (`grep -n "fn new" crates/presenter-server/src/state/mod.rs`) and keep the rename minimal.

- [ ] **Step 3: Verify build**

Run: `cargo check -p presenter-server`
Expected: clean. Existing handlers that take `State<AppState>` keep working.

- [ ] **Step 4: Commit**

```bash
git add crates/presenter-server/src/state/mod.rs
git commit -m "feat(state): expose local_public_ip on AppState (#218)"
```

---

## Task 3: `detect_network_mode` helper + unit tests (TDD)

**Files:**
- Create: `crates/presenter-server/src/router/network_mode.rs`
- Modify: `crates/presenter-server/src/router/mod.rs`

- [ ] **Step 1: Write the failing tests**

Create `crates/presenter-server/src/router/network_mode.rs`:

```rust
//! Network-mode classifier: determines whether a client is on the church LAN
//! (direct or via tunnel-from-same-egress) or truly remote. Used by
//! `GET /api/network-mode` and by the tablet UI's LAN/WAN pill.

use axum::{extract::State, http::HeaderMap, Json};
use serde::Serialize;

use crate::state::AppState;

/// Classifies a client from request headers. Returns `"local"` or `"remote"`.
///
/// Rules:
/// - No `CF-Connecting-IP` header → direct connection, not via tunnel → `local`.
/// - `CF-Connecting-IP` matches `local_public_ip` → same egress IP as the server,
///   so client is on the church LAN just using the tunnel URL → `local`.
/// - Otherwise, if `local_public_ip` is set → `remote`.
/// - Otherwise (no configured IP) → fall back to `is_private_ip` on the client IP.
pub fn detect_network_mode(headers: &HeaderMap, local_public_ip: Option<&str>) -> &'static str {
    let client_ip = headers
        .get("cf-connecting-ip")
        .or_else(|| headers.get("x-forwarded-for"))
        .or_else(|| headers.get("x-real-ip"))
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(',').next().unwrap_or(s).trim().to_string());

    match (&client_ip, local_public_ip) {
        (Some(client), Some(local)) if client == local => "local",
        (Some(_), Some(_)) => "remote",
        (None, _) => "local",
        (Some(ip), None) if is_private_ip(ip) => "local",
        (Some(_), None) => "remote",
    }
}

/// Return true for IPs in private/loopback/link-local ranges.
pub fn is_private_ip(ip: &str) -> bool {
    ip.parse::<std::net::IpAddr>().is_ok_and(|addr| match addr {
        std::net::IpAddr::V4(v4) => v4.is_private() || v4.is_loopback() || v4.is_link_local(),
        std::net::IpAddr::V6(v6) => v6.is_loopback(),
    })
}

#[derive(Debug, Serialize)]
pub struct NetworkModeResponse {
    pub mode: String,
}

pub async fn get_network_mode(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Json<NetworkModeResponse> {
    let mode = detect_network_mode(&headers, state.local_public_ip.as_deref());
    Json(NetworkModeResponse {
        mode: mode.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers_with(name: &str, value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(name, HeaderValue::from_str(value).unwrap());
        h
    }

    #[test]
    fn no_proxy_headers_is_local() {
        let h = HeaderMap::new();
        assert_eq!(detect_network_mode(&h, Some("203.0.113.50")), "local");
        assert_eq!(detect_network_mode(&h, None), "local");
    }

    #[test]
    fn cf_connecting_ip_matching_configured_is_local() {
        let h = headers_with("cf-connecting-ip", "203.0.113.50");
        assert_eq!(detect_network_mode(&h, Some("203.0.113.50")), "local");
    }

    #[test]
    fn cf_connecting_ip_different_from_configured_is_remote() {
        let h = headers_with("cf-connecting-ip", "198.51.100.10");
        assert_eq!(detect_network_mode(&h, Some("203.0.113.50")), "remote");
    }

    #[test]
    fn x_forwarded_for_falls_through_to_classification() {
        let h = headers_with("x-forwarded-for", "203.0.113.50, 10.0.0.1");
        assert_eq!(detect_network_mode(&h, Some("203.0.113.50")), "local");
    }

    #[test]
    fn no_configured_ip_falls_back_to_private_range() {
        let h = headers_with("cf-connecting-ip", "10.77.9.50");
        assert_eq!(detect_network_mode(&h, None), "local");

        let h_public = headers_with("cf-connecting-ip", "198.51.100.10");
        assert_eq!(detect_network_mode(&h_public, None), "remote");
    }

    #[test]
    fn is_private_ip_handles_ranges() {
        assert!(is_private_ip("10.1.2.3"));
        assert!(is_private_ip("192.168.0.1"));
        assert!(is_private_ip("172.20.30.40"));
        assert!(is_private_ip("127.0.0.1"));
        assert!(is_private_ip("::1"));
        assert!(!is_private_ip("8.8.8.8"));
        assert!(!is_private_ip("203.0.113.5"));
        assert!(!is_private_ip("not-an-ip"));
    }
}
```

- [ ] **Step 2: Register the module**

Edit `crates/presenter-server/src/router/mod.rs`. Add `pub mod network_mode;` alongside the other `pub mod` lines.

- [ ] **Step 3: Run the tests — expect PASS**

Run:
```bash
cd /home/newlevel/devel/presenter/presenter-dev2
cargo test -p presenter-server -- network_mode --nocapture
```
Expected: 6 tests pass. The implementation lives in the same file so tests pass immediately (TDD "red" is implicit — without the code the file wouldn't compile).

- [ ] **Step 4: Commit**

```bash
git add crates/presenter-server/src/router/network_mode.rs crates/presenter-server/src/router/mod.rs
git commit -m "feat(server): add detect_network_mode classifier with tests (#218)"
```

---

## Task 4: Register `GET /api/network-mode` route

**Files:**
- Modify: `crates/presenter-server/src/router.rs`

- [ ] **Step 1: Add the route**

Find the router builder (`build_router` or similar). Near the Android stage routes we registered earlier (`router.rs:170-180`), add:

```rust
        .route(
            "/api/network-mode",
            get(router::network_mode::get_network_mode),
        )
```

Adjust the module path to match the crate's conventions — if handlers are already imported via `integrations::...` elsewhere, use `network_mode::get_network_mode` with a `use crate::router::network_mode;` at the top.

- [ ] **Step 2: Add a router-level integration test**

Append to `crates/presenter-server/src/router/tests.rs`:

```rust
    #[tokio::test]
    async fn network_mode_endpoint_returns_local_for_direct_request() {
        let state = AppState::in_memory().await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/network-mode")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["mode"], "local");
    }

    #[tokio::test]
    async fn network_mode_endpoint_returns_remote_with_foreign_cf_ip() {
        // State without a configured local_public_ip → falls back to private-range check.
        let state = AppState::in_memory().await.unwrap();
        let app = build_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/network-mode")
                    .header("CF-Connecting-IP", "198.51.100.10")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["mode"], "remote");
    }
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p presenter-server -- network_mode --nocapture`
Expected: all pass including the 2 new integration tests.

- [ ] **Step 4: Commit**

```bash
git add crates/presenter-server/src/router.rs crates/presenter-server/src/router/tests.rs
git commit -m "feat(router): expose GET /api/network-mode (#218)"
```

---

## Task 5: UI API client `fetch_network_mode`

**Files:**
- Create: `crates/presenter-ui/src/api/network.rs`
- Modify: `crates/presenter-ui/src/api/mod.rs`

- [ ] **Step 1: Create the client helper**

Create `crates/presenter-ui/src/api/network.rs`:

```rust
use serde::Deserialize;

use super::{get_json, ApiError};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkModeDto {
    pub mode: String,
}

/// Fetch `/api/network-mode`. Returns `"local"` or `"remote"` on success.
pub async fn fetch_network_mode() -> Result<String, ApiError> {
    let dto: NetworkModeDto = get_json("/api/network-mode").await?;
    Ok(dto.mode)
}
```

If `get_json` or `ApiError` have different names/paths in the presenter-ui crate, adjust — grep for `pub async fn get_json` and `pub enum ApiError` (or `pub struct ApiError`) to find the actual API. The pattern should mirror `crates/presenter-ui/src/api/ndi.rs` which calls `get_json("/ndi/sources")`.

- [ ] **Step 2: Register the module**

Edit `crates/presenter-ui/src/api/mod.rs`. Add `pub mod network;` alongside the other `pub mod` declarations.

- [ ] **Step 3: Verify build**

Run: `cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui && cargo check --lib`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git add crates/presenter-ui/src/api/network.rs crates/presenter-ui/src/api/mod.rs
git commit -m "feat(ui-api): add fetch_network_mode client helper (#218)"
```

---

## Task 6: LAN/WAN pill in TabletTimerBar

**Files:**
- Modify: `crates/presenter-ui/src/pages/tablet.rs`

- [ ] **Step 1: Add the signal and fetch-on-mount**

Open `tablet.rs`. Inside `TabletTimerBar`'s function body (around line 194), after the existing `clock` signal and interval, add:

```rust
    let (network_mode, set_network_mode) = leptos::prelude::signal(String::new());
    leptos::task::spawn_local(async move {
        if let Ok(mode) = crate::api::network::fetch_network_mode().await {
            let _ = set_network_mode.try_set(mode);
        }
    });
```

`spawn_local` is the existing pattern used elsewhere (e.g., `slide_list.rs:240`).

- [ ] **Step 2: Render the pill**

Replace the `view! { <div class="tablet-timer-bar" ... > ... </div> }` block's contents so the pill sits between the elapsed span and the state span. Use a `Show` component so the pill is hidden until the fetch returns:

```rust
    view! {
        <div class="tablet-timer-bar" data-zone=zone data-role="timer-bar">
            <span class="tablet-timer-bar__clock" data-role="timer-clock">{move || clock.get()}</span>
            <span class="tablet-timer-bar__elapsed" data-role="timer-elapsed">{elapsed_text}</span>
            <Show when=move || !network_mode.get().is_empty() fallback=|| ()>
                {move || {
                    let mode = network_mode.get();
                    let (class, label) = if mode == "local" {
                        ("network-indicator network-indicator--local", "LAN")
                    } else {
                        ("network-indicator network-indicator--remote", "WAN")
                    };
                    view! {
                        <span class=class data-role="network-indicator">{label}</span>
                    }
                }}
            </Show>
            <span class="tablet-timer-bar__state" data-role="timer-state">{state_label}</span>
        </div>
    }
```

`Show` is already imported via `leptos::prelude::*` at the top of the file. If not, add `use leptos::prelude::Show;`.

- [ ] **Step 3: Verify build**

Run: `cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui && cargo check --lib`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git add crates/presenter-ui/src/pages/tablet.rs
git commit -m "feat(tablet): add LAN/WAN pill to timer bar (#218)"
```

---

## Task 7: Info popover component + info button

**Files:**
- Create: `crates/presenter-ui/src/components/info_popover.rs`
- Modify: `crates/presenter-ui/src/components/mod.rs`
- Modify: `crates/presenter-ui/src/pages/tablet.rs`

- [ ] **Step 1: Create the popover component**

Create `crates/presenter-ui/src/components/info_popover.rs`:

```rust
use leptos::prelude::*;

#[component]
pub fn InfoPopover(
    /// "local" / "remote" / empty if not yet fetched.
    network_mode: Signal<String>,
) -> impl IntoView {
    let (open, set_open) = signal(false);

    // Captured once at mount; these don't change during the session.
    let version = env!("CARGO_PKG_VERSION");
    let channel = option_env!("PRESENTER_BUILD_CHANNEL").unwrap_or("dev");
    let hostname = web_sys::window()
        .and_then(|w| w.location().host().ok())
        .unwrap_or_default();

    let reload = move |_| {
        if let Some(w) = web_sys::window() {
            let _ = w.location().reload();
        }
    };

    view! {
        <div class="info-popover-wrap">
            <button
                type="button"
                class="info-button"
                aria-label="Info"
                data-role="info-button"
                on:click=move |_| { let _ = set_open.try_update(|v| *v = !*v); }
            >"\u{24D8}"</button>  // ⓘ
            <Show when=move || open.get() fallback=|| ()>
                <div class="info-popover" data-role="info-popover">
                    <dl>
                        <dt>"Version"</dt>
                        <dd>{format!("{version} ({channel})")}</dd>
                        <dt>"Host"</dt>
                        <dd>{hostname.clone()}</dd>
                        <dt>"Network"</dt>
                        <dd>{move || {
                            let m = network_mode.get();
                            if m == "local" { "LAN".to_string() }
                            else if m == "remote" { "WAN".to_string() }
                            else { "unknown".to_string() }
                        }}</dd>
                    </dl>
                    <button type="button" class="info-popover__reload" on:click=reload>"Reload"</button>
                </div>
            </Show>
        </div>
    }
}
```

- [ ] **Step 2: Register the module**

Edit `crates/presenter-ui/src/components/mod.rs`. Add `pub mod info_popover;` alongside the others.

- [ ] **Step 3: Wire the component into TabletTimerBar**

In `crates/presenter-ui/src/pages/tablet.rs`, import the popover and place it inside the timer bar, AFTER the state span:

```rust
use crate::components::info_popover::InfoPopover;
```

Then inside the view, just before the closing `</div>` of `.tablet-timer-bar`:

```rust
            <InfoPopover network_mode=network_mode.into() />
```

`network_mode` is already in scope from Task 6. `.into()` converts the `ReadSignal<String>` to `Signal<String>`.

- [ ] **Step 4: Verify build**

Run: `cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui && cargo check --lib`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git add crates/presenter-ui/src/components/info_popover.rs crates/presenter-ui/src/components/mod.rs crates/presenter-ui/src/pages/tablet.rs
git commit -m "feat(tablet): add info popover with version/host/mode + Reload (#218)"
```

---

## Task 8: CSS for pill and popover

**Files:**
- Modify: `crates/presenter-ui/styles/operator.css`

- [ ] **Step 1: Append styles**

Add to the END of `crates/presenter-ui/styles/operator.css`:

```css
/* Network mode indicator (LAN/WAN pill) */
.network-indicator {
  font-size: 0.75rem;
  font-weight: 700;
  padding: 0.15rem 0.45rem;
  border-radius: 999px;
  letter-spacing: 0.04em;
  white-space: nowrap;
  margin: 0 0.35rem;
}
.network-indicator--local {
  background: rgba(74, 222, 128, 0.18);
  color: #22c55e;
  border: 1px solid rgba(74, 222, 128, 0.4);
}
.network-indicator--remote {
  background: rgba(124, 111, 255, 0.2);
  color: #a094ff;
  border: 1px solid rgba(124, 111, 255, 0.4);
}

/* Info button + popover */
.info-popover-wrap {
  position: relative;
  display: inline-block;
}
.info-button {
  appearance: none;
  background: transparent;
  color: inherit;
  border: 1px solid rgba(148, 163, 184, 0.3);
  border-radius: 50%;
  width: 1.6rem;
  height: 1.6rem;
  font-size: 1rem;
  line-height: 1;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  padding: 0;
  margin-left: 0.35rem;
}
.info-button:hover { background: rgba(148, 163, 184, 0.15); }
.info-popover {
  position: absolute;
  right: 0;
  top: calc(100% + 0.4rem);
  min-width: 220px;
  background: #1f2937;
  color: #e5e7eb;
  border: 1px solid rgba(148, 163, 184, 0.25);
  border-radius: 0.5rem;
  padding: 0.6rem 0.75rem;
  box-shadow: 0 6px 24px rgba(0, 0, 0, 0.35);
  z-index: 1000;
  font-size: 0.85rem;
}
.info-popover dl {
  display: grid;
  grid-template-columns: max-content 1fr;
  gap: 0.2rem 0.75rem;
  margin: 0 0 0.5rem 0;
}
.info-popover dt {
  font-weight: 600;
  color: rgba(229, 231, 235, 0.7);
}
.info-popover dd { margin: 0; }
.info-popover__reload {
  appearance: none;
  background: rgba(148, 163, 184, 0.15);
  color: inherit;
  border: 1px solid rgba(148, 163, 184, 0.3);
  border-radius: 0.35rem;
  padding: 0.3rem 0.6rem;
  cursor: pointer;
  font-size: 0.8rem;
}
.info-popover__reload:hover { background: rgba(148, 163, 184, 0.25); }
```

- [ ] **Step 2: Commit**

```bash
git add crates/presenter-ui/styles/operator.css
git commit -m "style(tablet): LAN/WAN pill + info popover styles (#218)"
```

---

## Task 9: Playwright E2E test

**Files:**
- Create: `tests/e2e/tablet-network-indicator.spec.ts`

- [ ] **Step 1: Write the test**

Create `tests/e2e/tablet-network-indicator.spec.ts`:

```typescript
import { test, expect } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

test.describe.configure({ timeout: 60_000 });

let server: ServerHandle | undefined;
let baseURL = "";

test.beforeAll(async ({}, testInfo) => {
  const cfg = deriveTestConfig(testInfo);
  baseURL = cfg.baseURL;
  await refreshDevData(cfg.dbUrl);
  server = await startTestServer(cfg.port, cfg.dbUrl, cfg.oscPort);
});

test.afterAll(async () => {
  await stopServer(server);
  server = undefined;
});

test("tablet network indicator renders LAN for direct fetch", async ({ page }) => {
  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  await page.goto(`${baseURL}/ui/tablet`);
  await page.waitForSelector('body[data-wasm-ready="true"]', { timeout: 30_000 });

  const indicator = page.locator('[data-role="network-indicator"]');
  await expect(indicator).toBeVisible({ timeout: 10_000 });
  // Direct test client = no CF headers = local.
  await expect(indicator).toHaveText("LAN");

  // Info button opens popover with version and host.
  const infoBtn = page.locator('[data-role="info-button"]');
  await expect(infoBtn).toBeVisible();
  await infoBtn.click();

  const popover = page.locator('[data-role="info-popover"]');
  await expect(popover).toBeVisible();
  await expect(popover).toContainText("Version");
  await expect(popover).toContainText("Host");
  await expect(popover).toContainText("Network");
  await expect(popover).toContainText("LAN");

  expect(consoleMessages).toEqual([]);
});
```

- [ ] **Step 2: Run locally (optional, if Playwright is installed)**

Run: `npm run test:playwright -- tablet-network-indicator`
Expected: test passes. If Playwright isn't set up locally, rely on CI — the spec will run via the existing `Playwright E2E` jobs.

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/tablet-network-indicator.spec.ts
git commit -m "test(e2e): tablet LAN/WAN indicator + info popover (#218)"
```

---

## Task 10: Deploy artifacts — `cloudflared.service` + config template

**Files:**
- Create: `deploy/cloudflared.service`
- Create: `deploy/cloudflared-config.yml.tmpl`

- [ ] **Step 1: Create the systemd unit template**

Create `deploy/cloudflared.service`:

```ini
[Unit]
Description=Cloudflare Tunnel for presenter
Wants=network-online.target
After=network-online.target

[Service]
Type=simple
User=root
ExecStart=/usr/bin/cloudflared tunnel --config /etc/cloudflared/config.yml run
Restart=on-failure
RestartSec=5
NoNewPrivileges=true

[Install]
WantedBy=multi-user.target
```

- [ ] **Step 2: Create the config template**

Create `deploy/cloudflared-config.yml.tmpl`:

```yaml
tunnel: __TUNNEL_ID__
credentials-file: /etc/cloudflared/__TUNNEL_ID__.json
no-autoupdate: true

ingress:
  - hostname: __HOSTNAME__
    service: http://localhost:__BACKEND_PORT__
  - service: http_status:404
```

Placeholders replaced by the deploy workflow: `__TUNNEL_ID__`, `__HOSTNAME__`, `__BACKEND_PORT__`.

- [ ] **Step 3: Commit**

```bash
git add deploy/cloudflared.service deploy/cloudflared-config.yml.tmpl
git commit -m "feat(deploy): cloudflared systemd unit + config template (#218)"
```

---

## Task 11: Deploy workflow — install cloudflared (all three targets)

**Files:**
- Modify: `.github/workflows/pipeline.yml` (deploy-dev)
- Modify: `.github/workflows/deploy.yml` (deploy to prod)
- Modify: `.github/workflows/release.yml` (deploy to PP)

For each workflow, add a step that runs AFTER the existing deploy (service restart) and is conditional on the relevant secret being present.

- [ ] **Step 1: Add the step to pipeline.yml (deploy-dev)**

Locate the `deploy-dev` job near the end of `pipeline.yml`. After the final `systemctl start` step, add:

```yaml
      - name: Install & configure cloudflared (skipped if secret unset)
        if: ${{ secrets.CLOUDFLARED_CREDS_DEV != '' && vars.CLOUDFLARED_TUNNEL_ID_DEV != '' && vars.CLOUDFLARED_HOSTNAME_DEV != '' }}
        env:
          TUNNEL_ID: ${{ vars.CLOUDFLARED_TUNNEL_ID_DEV }}
          HOSTNAME: ${{ vars.CLOUDFLARED_HOSTNAME_DEV }}
          BACKEND_PORT: ${{ vars.CLOUDFLARED_BACKEND_PORT_DEV || '8080' }}
        run: |
          ssh deploy-target "set -euo pipefail
            if ! command -v cloudflared >/dev/null 2>&1; then
              wget -q https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-amd64.deb -O /tmp/cf.deb
              sudo dpkg -i /tmp/cf.deb
              rm /tmp/cf.deb
            fi
            sudo mkdir -p /etc/cloudflared
            sudo tee /etc/cloudflared/config.yml >/dev/null <<'CFG'
$(sed -e 's|__TUNNEL_ID__|'"$TUNNEL_ID"'|g' \
      -e 's|__HOSTNAME__|'"$HOSTNAME"'|g' \
      -e 's|__BACKEND_PORT__|'"$BACKEND_PORT"'|g' \
      deploy/cloudflared-config.yml.tmpl)
CFG
          "
          scp deploy/cloudflared.service deploy-target:/tmp/cloudflared.service
          ssh deploy-target "sudo mv /tmp/cloudflared.service /etc/systemd/system/cloudflared.service && sudo systemctl daemon-reload"

          # Write credentials (short-lived in /tmp then move with sudo)
          printf '%s' '${{ secrets.CLOUDFLARED_CREDS_DEV }}' > /tmp/cf-creds.json
          chmod 600 /tmp/cf-creds.json
          scp /tmp/cf-creds.json deploy-target:/tmp/cf-creds.json
          rm /tmp/cf-creds.json
          ssh deploy-target "sudo mv /tmp/cf-creds.json /etc/cloudflared/${TUNNEL_ID}.json && sudo chmod 600 /etc/cloudflared/${TUNNEL_ID}.json && sudo chown root:root /etc/cloudflared/${TUNNEL_ID}.json"

          ssh deploy-target "sudo systemctl enable --now cloudflared && sudo systemctl restart cloudflared"
```

**Note:** The heredoc with `sed` is tricky across SSH. If the one-liner proves fragile, generate the config locally first, then `scp` it to the target. The functional intent is: render `cloudflared-config.yml.tmpl` with the three env-substituted placeholders and place it at `/etc/cloudflared/config.yml` on the remote.

Cleaner alternative that the implementer should use if the heredoc feels wrong:

```yaml
      - name: Install & configure cloudflared (skipped if secret unset)
        if: ${{ secrets.CLOUDFLARED_CREDS_DEV != '' && vars.CLOUDFLARED_TUNNEL_ID_DEV != '' && vars.CLOUDFLARED_HOSTNAME_DEV != '' }}
        env:
          TUNNEL_ID: ${{ vars.CLOUDFLARED_TUNNEL_ID_DEV }}
          HOSTNAME: ${{ vars.CLOUDFLARED_HOSTNAME_DEV }}
          BACKEND_PORT: ${{ vars.CLOUDFLARED_BACKEND_PORT_DEV || '8080' }}
          CREDS: ${{ secrets.CLOUDFLARED_CREDS_DEV }}
        run: |
          set -euo pipefail
          # 1. Render config.yml locally
          sed -e "s|__TUNNEL_ID__|$TUNNEL_ID|g" \
              -e "s|__HOSTNAME__|$HOSTNAME|g" \
              -e "s|__BACKEND_PORT__|$BACKEND_PORT|g" \
              deploy/cloudflared-config.yml.tmpl > /tmp/cf-config.yml

          # 2. Write credentials
          printf '%s' "$CREDS" > /tmp/cf-creds.json
          chmod 600 /tmp/cf-creds.json

          # 3. Ship files
          scp /tmp/cf-config.yml deploy-target:/tmp/cf-config.yml
          scp /tmp/cf-creds.json deploy-target:/tmp/cf-creds.json
          scp deploy/cloudflared.service deploy-target:/tmp/cloudflared.service
          rm /tmp/cf-config.yml /tmp/cf-creds.json

          # 4. Install + activate
          ssh deploy-target "set -euo pipefail
            if ! command -v cloudflared >/dev/null 2>&1; then
              wget -q https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-amd64.deb -O /tmp/cf.deb
              sudo dpkg -i /tmp/cf.deb
              rm /tmp/cf.deb
            fi
            sudo mkdir -p /etc/cloudflared
            sudo mv /tmp/cf-config.yml /etc/cloudflared/config.yml
            sudo mv /tmp/cf-creds.json /etc/cloudflared/${TUNNEL_ID}.json
            sudo chown root:root /etc/cloudflared/config.yml /etc/cloudflared/${TUNNEL_ID}.json
            sudo chmod 600 /etc/cloudflared/${TUNNEL_ID}.json
            sudo mv /tmp/cloudflared.service /etc/systemd/system/cloudflared.service
            sudo systemctl daemon-reload
            sudo systemctl enable --now cloudflared
            sudo systemctl restart cloudflared
          "
```

Use the cleaner version. The `if:` guard means the step is a no-op until secrets are set.

- [ ] **Step 2: Add the same step to deploy.yml (prod)**

Same step, but with `CLOUDFLARED_CREDS_PROD`, `CLOUDFLARED_TUNNEL_ID_PROD`, `CLOUDFLARED_HOSTNAME_PROD`, `CLOUDFLARED_BACKEND_PORT_PROD` (default `80`). Append after the prod deploy's service restart.

- [ ] **Step 3: Add the same step to release.yml (PP)**

Same step, but with `CLOUDFLARED_CREDS_PP`, `CLOUDFLARED_TUNNEL_ID_PP`, `CLOUDFLARED_HOSTNAME_PP`, `CLOUDFLARED_BACKEND_PORT_PP` (default `80`). Append after the release's service restart.

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/pipeline.yml .github/workflows/deploy.yml .github/workflows/release.yml
git commit -m "ci: install + configure cloudflared on each deploy target (#218)"
```

---

## Task 12: Bootstrap documentation

**Files:**
- Create: `docs/cloudflare-tunnel-setup.md`
- Modify: `docs/configuration.md`
- Modify: `CLAUDE.md`

- [ ] **Step 1: Write the bootstrap doc**

Create `docs/cloudflare-tunnel-setup.md`:

```markdown
# Cloudflare Tunnel Setup

One-off bootstrap for #218. Run once per instance (dev, prod, PP).

## Prerequisites

- Cloudflare account with `newlevel.media` zone.
- Access to the target machine (SSH) to run `cloudflared tunnel login` **OR** use the Cloudflare dashboard.

## Create the tunnel

From any machine with cloudflared installed (or using the dashboard):

```bash
cloudflared tunnel login                          # opens browser, downloads cert.pem to ~/.cloudflared/
cloudflared tunnel create presenter-dev           # prints a Tunnel ID (UUID) and writes <id>.json to ~/.cloudflared/
cloudflared tunnel route dns presenter-dev presenter-dev.newlevel.media
```

Repeat for `presenter-prod` → `presenter.newlevel.media` and `presenter-pp` → `presenter-pp.newlevel.media`.

## Register GitHub secrets and variables

Per environment (Dev / Prod / PP) in **Settings → Environments → \<env\> → Add secret/variable**:

**Secrets (encrypted):**
- `CLOUDFLARED_CREDS_DEV` / `_PROD` / `_PP` — full contents of the `<tunnel-id>.json` file from `~/.cloudflared/`.

**Variables (plaintext):**
- `CLOUDFLARED_TUNNEL_ID_DEV` / `_PROD` / `_PP` — the Tunnel ID UUID.
- `CLOUDFLARED_HOSTNAME_DEV` / `_PROD` / `_PP` — e.g., `presenter-dev.newlevel.media`.
- `CLOUDFLARED_BACKEND_PORT_DEV` / `_PROD` / `_PP` — `8080` for dev, `80` for prod/PP.

## Configure the church public IP

Get it from any machine on the church LAN:

```bash
curl -s ifconfig.me
```

Set on each presenter-server install via the service env file (e.g., `/etc/default/presenter-dev`):

```
PRESENTER_LOCAL_PUBLIC_IP=203.0.113.50
```

Restart the service. Verify via `curl http://localhost:80/api/network-mode` (expect `{"mode":"local"}` from the same machine).

## Verify

1. Trigger a deploy. The new `Install & configure cloudflared` step activates the tunnel.
2. On a phone over cellular, open `https://presenter-dev.newlevel.media` → page loads → "Add to home screen" installs as PWA.
3. Tablet shows `WAN` pill (different public IP). On LAN WiFi, tablet shows `LAN` pill.
```

- [ ] **Step 2: Update docs/configuration.md**

Add an entry for `PRESENTER_LOCAL_PUBLIC_IP` in the existing env var table. Find the table (it likely exists; grep for `PRESENTER_PORT` or `PRESENTER_DB_URL` to locate it):

```markdown
| `PRESENTER_LOCAL_PUBLIC_IP` | unset | Optional public IP of the church LAN's outbound gateway. Enables `LAN` vs `WAN` classification when traffic arrives via Cloudflare Tunnel. |
```

- [ ] **Step 3: Update CLAUDE.md**

In the env var table in `CLAUDE.md`, add:

```markdown
| `PRESENTER_LOCAL_PUBLIC_IP`   | unset                   | Church's public egress IP for LAN/WAN detection via Cloudflare Tunnel |
```

- [ ] **Step 4: Commit**

```bash
git add docs/cloudflare-tunnel-setup.md docs/configuration.md CLAUDE.md
git commit -m "docs: cloudflare tunnel bootstrap + PRESENTER_LOCAL_PUBLIC_IP env (#218)"
```

---

## Task 13: Version bump, fmt/clippy, push, monitor CI

- [ ] **Step 1: Bump workspace version**

Edit `Cargo.toml`:
```
[workspace.package]
version = "0.4.25"
```
(or the next patch beyond whatever is on dev).

Run `cargo check -p presenter-server` to refresh `Cargo.lock`.

- [ ] **Step 2: Local checks**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
cargo fmt --all
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
cargo test --workspace
```

Fix anything that surfaces.

- [ ] **Step 3: Commit + push**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore: bump version for PWA via Cloudflare Tunnel (#218)"
git push origin dev
```

- [ ] **Step 4: Monitor CI**

```bash
gh run list --branch dev --limit 3
gh run view <pipeline-id>
```

Expected flow:
- Format / Clippy / Test / Playwright E2E all pass
- Deploy to Dev runs as usual
- New cloudflared step is a no-op because secrets aren't set yet (by design)

On a fully-green pipeline, open PR to `main`.

---

## Verification Summary

| Check | How to verify |
|-------|---------------|
| `detect_network_mode` handles all 5 cases | `cargo test -p presenter-server -- network_mode` — 6 tests pass |
| `GET /api/network-mode` returns JSON | router integration test (Task 4) |
| Tablet pill renders | Playwright test (Task 9) |
| Info popover shows version/host/mode | Playwright test (Task 9) |
| CI green | all jobs succeed on dev |
| Deploy step is no-op without secrets | Deploy logs show "Install & configure cloudflared" as skipped |
| PWA installs after tunnel bootstrap | Manual on tablet — `https://presenter-pp.newlevel.media/ui/tablet` → Add to home screen → opens standalone |


---

## Self-Review Notes

1. **Spec coverage:** All 10 spec bullet points have tasks — detect_network_mode (T3), endpoint (T4), env var (T1+T2), tablet pill (T6), info popover (T7), CSS (T8), E2E (T9), deploy (T10+T11), bootstrap docs (T12), version+CI (T13). ✓
2. **Placeholder scan:** No "TBD"/"implement later". One intentional hedge in Task 11 ("If the heredoc proves fragile...") — kept because the cleaner alternative is also fully specified right there. ✓
3. **Type consistency:** `detect_network_mode(&HeaderMap, Option<&str>) -> &'static str` used everywhere. `fetch_network_mode() -> Result<String, ApiError>` consistent between Task 5 and Task 6. `NetworkConfig::local_public_ip: Option<String>` consistent across Tasks 1-3. ✓
