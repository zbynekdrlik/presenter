# Mock Integrations on Dev Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop the dev server from sending live commands to production hardware by embedding in-process mocks for Resolume HTTP and AbleSet HTTP behind a `mock-integrations` Cargo feature, and rewriting dev's integration table rows to point at those mocks.

**Architecture:** Add a Cargo feature on `presenter-server`. When enabled at compile time, `main.rs` spawns two extra axum listeners on `127.0.0.1:8091` (Resolume) and `127.0.0.1:39042` (AbleSet) that speak the same HTTP shapes presenter-server's outbound calls expect. Pipeline.yml's "Replace dev DB" step gets a follow-up sqlite3 step that rewrites `resolume_hosts` + `ableset_settings` to localhost mock ports and DELETEs `android_stage_displays` + `video_sources`. Prod release pipelines add a guard that fails the build if mock symbols are present in the release artifact.

**Tech Stack:** Rust (axum, tokio, serde, anyhow), bash sqlite3 in pipeline, GitHub Actions YAML, Playwright TypeScript.

**Spec:** `docs/superpowers/specs/2026-05-02-mock-integrations-design.md` (commit `1715584`)

**Closes:** Issue #279 — `dev presenter connects to live resolume arena and interferes on production!!!`

---

## Context

### The bug

`pipeline.yml:786-810` "Replace dev database with production snapshot" copies prod's `presenter.db` to dev on every deploy so dev mirrors prod data for realistic testing. The snapshot also carries `resolume_hosts` rows pointing at prod's Resolume Arena. After dev restarts, it reads those rows, sees `is_enabled=true`, and starts pushing commands (clip triggers via POST, text updates via PUT) to PROD's Resolume during live worship services.

`AbleSetSettings` carries the same risk (host column points at prod's AbleSet). `OscSettings` does NOT bleed (no host column — outbound REAPER target is per-environment env var). `AndroidStageDisplays` and `VideoSources` are pull-style (clients connect to the server) — pipeline DELETE-clears them to remove stale prod-side identifiers.

### Verified protocol surfaces

**Resolume (presenter-server outbound calls in `crates/presenter-server/src/resolume/driver.rs`):**

- Base URL pattern: `http://{host}:{port}/api/v1` (constructed at line ~177).
- `GET {base}/composition` — fetches Composition JSON (line 171).
- `PUT {base}/composition/parameter/by-id/{param_id}` — body `{"value": "..."}`, updates clip text (line 213).
- `POST {base}/composition/clips/by-id/{clip_id}/connect` — triggers a clip (line 247).

**AbleSet (presenter-server outbound calls in `crates/presenter-server/src/ableset.rs`):**

- Base URL pattern: `http://{host}:{http_port}` (constructed at line ~14, `SETLIST_ENDPOINT = "/api/setlist"`).
- `GET /api/setlist` — fetches SetlistResponse JSON shape:
  ```rust
  struct SetlistResponse {
      #[serde(rename = "activeSongId")] active_song_id: Option<String>,
      #[serde(default)] songs: Vec<SetlistSong>,
  }
  ```

These are the ONLY endpoints presenter-server calls. Mocks can return minimal valid JSON for the GETs and 200-with-empty-body for the writes.

### Existing infrastructure to leverage

- `axum` is already a workspace dependency (used for the main HTTP server). Mocks reuse axum.
- `serde_json::json!()` macro is already used throughout. Mocks construct response bodies the same way.
- `tokio::net::TcpListener::bind` is already used in `main.rs` for the main server (line 103). Mocks bind on different ports the same way.
- E2E tests use the `support.ts` helper pattern; new mock-roundtrip test follows the same shape.

---

## File Structure

### Created

| File | Responsibility |
|------|---------------|
| `crates/presenter-server/src/mock_integrations/mod.rs` | Feature-gated module entry. `pub async fn start_all()` spawns both mock listeners. Owned by `main.rs`. |
| `crates/presenter-server/src/mock_integrations/request_log.rs` | 1024-entry in-memory ring buffer + `GET /__mock/log` axum handler. Shared between Resolume and AbleSet mocks. |
| `crates/presenter-server/src/mock_integrations/resolume.rs` | axum router serving the 3 Resolume endpoints. |
| `crates/presenter-server/src/mock_integrations/ableset.rs` | axum router serving `/api/setlist`. |
| `tests/e2e/mock-resolume-roundtrip.spec.ts` | Playwright test that triggers a Resolume call from operator UI and asserts `/__mock/log` saw it. Skipped on prod (channel === "release"). |

### Modified

| File | Change |
|------|--------|
| `Cargo.toml` | Workspace version 0.4.52 → 0.4.53 |
| `crates/presenter-ui/Cargo.toml` | presenter-ui version 0.1.21 → 0.1.22 |
| `crates/presenter-server/Cargo.toml` | Add `[features]\nmock-integrations = []` |
| `crates/presenter-server/src/main.rs` | `#[cfg(feature = "mock-integrations")]` import + conditional `mock_integrations::start_all().await?` before the main listener serve loop |
| `.github/workflows/pipeline.yml` | Build step `cargo build --release ... --features mock-integrations`. New "Redirect integrations to mock endpoints" step after line 810. |
| `.github/workflows/deploy.yml` | New "Verify prod artifact has no mock integrations" step after the build. |
| `.github/workflows/release.yml` | Same guard. |

### Lock files

- `Cargo.lock` — auto-updated by `cargo build`/`cargo check`.
- `crates/presenter-ui/Cargo.lock` — separate workspace, auto-updated.

---

## Task 1: Bump Version (Haiku)

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/presenter-ui/Cargo.toml`
- Modify: `Cargo.lock`, `crates/presenter-ui/Cargo.lock` (regenerated)

- [ ] **Step 1: Bump workspace version**

In `/home/newlevel/devel/presenter/presenter-dev2/Cargo.toml`, under `[workspace.package]`, change:

```toml
version = "0.4.52"
```

to:

```toml
version = "0.4.53"
```

- [ ] **Step 2: Bump presenter-ui version**

In `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui/Cargo.toml`, under `[package]`, change:

```toml
version = "0.1.21"
```

to:

```toml
version = "0.1.22"
```

- [ ] **Step 3: Regenerate workspace Cargo.lock**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo check --workspace --all-targets 2>&1 | tail -10
```

Expected: clean check, `Cargo.lock` updated.

- [ ] **Step 4: Regenerate presenter-ui Cargo.lock**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui && cargo check --target wasm32-unknown-unknown 2>&1 | tail -10
```

Expected: clean check.

- [ ] **Step 5: Verify**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && grep -E "^version" Cargo.toml crates/presenter-ui/Cargo.toml | head -3
```

Expected:

```
Cargo.toml:version = "0.4.53"
crates/presenter-ui/Cargo.toml:version = "0.1.22"
```

- [ ] **Step 6: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && git add Cargo.toml Cargo.lock crates/presenter-ui/Cargo.toml crates/presenter-ui/Cargo.lock && git commit -m "chore: bump version to 0.4.53 (#279)"
```

---

## Task 2: Add Cargo Feature + Module Skeleton + Request Log (Sonnet)

**Files:**
- Modify: `crates/presenter-server/Cargo.toml`
- Create: `crates/presenter-server/src/mock_integrations/mod.rs`
- Create: `crates/presenter-server/src/mock_integrations/request_log.rs`

- [ ] **Step 1: Add Cargo feature**

In `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-server/Cargo.toml`, find the section after `[dev-dependencies]` (or add `[features]` if absent — typically after `[dependencies]`). Add:

```toml
[features]
default = []
mock-integrations = []
```

If a `[features]` section already exists, only add the `mock-integrations = []` line, preserving any existing features.

- [ ] **Step 2: Create mock_integrations module entry**

Create `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-server/src/mock_integrations/mod.rs` with:

```rust
//! In-process mock listeners for outbound integrations (Resolume HTTP,
//! AbleSet HTTP). Compiled only when the `mock-integrations` feature is
//! enabled — dev builds use this; prod builds omit it entirely.
//!
//! Closes issue #279: prevents dev from sending live commands to
//! production hardware.

use std::sync::Arc;

pub mod ableset;
pub mod request_log;
pub mod resolume;

/// Start all mock listeners. Called from main.rs under
/// `#[cfg(feature = "mock-integrations")]`.
pub async fn start_all() -> anyhow::Result<()> {
    let log = Arc::new(request_log::RequestLog::new());
    resolume::spawn(log.clone()).await?;
    ableset::spawn(log).await?;
    Ok(())
}
```

- [ ] **Step 3: Create request_log.rs**

Create `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-server/src/mock_integrations/request_log.rs` with:

```rust
//! Shared in-memory ring buffer recording mock requests.
//!
//! Capped at 1024 entries; oldest entries are dropped. Reset on process
//! restart (no persistence). Read via `GET /__mock/log` on either mock.

use std::collections::VecDeque;
use std::sync::Mutex;

use axum::{extract::State, response::Json};
use chrono::{DateTime, Utc};
use serde::Serialize;

const MAX_ENTRIES: usize = 1024;

#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    pub at: DateTime<Utc>,
    pub mock: &'static str,
    pub method: String,
    pub path: String,
    pub body_preview: Option<String>,
}

pub struct RequestLog {
    entries: Mutex<VecDeque<LogEntry>>,
}

impl RequestLog {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(VecDeque::with_capacity(MAX_ENTRIES)),
        }
    }

    pub fn record(
        &self,
        mock: &'static str,
        method: &str,
        path: &str,
        body_preview: Option<String>,
    ) {
        let entry = LogEntry {
            at: Utc::now(),
            mock,
            method: method.to_string(),
            path: path.to_string(),
            body_preview,
        };
        let mut entries = self.entries.lock().expect("request_log mutex poisoned");
        if entries.len() == MAX_ENTRIES {
            entries.pop_front();
        }
        entries.push_back(entry);
    }

    pub fn snapshot(&self) -> Vec<LogEntry> {
        self.entries
            .lock()
            .expect("request_log mutex poisoned")
            .iter()
            .cloned()
            .collect()
    }
}

/// Axum handler: `GET /__mock/log` returns the current ring buffer as
/// JSON. Mounted by both mocks so either can serve it.
pub async fn log_handler(State(log): State<std::sync::Arc<RequestLog>>) -> Json<Vec<LogEntry>> {
    Json(log.snapshot())
}
```

- [ ] **Step 4: Verify compilation with feature off**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo check --workspace --all-targets 2>&1 | tail -10
```

Expected: clean. The new files do not compile without the feature, so they don't affect default builds. Even though `mod.rs` declares `pub mod ableset;` and `pub mod resolume;` (which don't exist yet), the `mock_integrations` module itself is not yet referenced from `main.rs`, so the unresolved submods are not reachable. **However** to keep the workspace `cargo check` clean even with future expansion, gate the `mod.rs` contents behind a comment explaining the dep order: tasks 3 and 4 add the missing modules. If `cargo check` complains about `mod ableset; mod resolume;` not existing, temporarily comment those lines and uncomment in Task 5.

If you encounter the unresolved submod error, replace `mod.rs` Step 2 contents with:

```rust
//! In-process mock listeners. Submodules added in Tasks 3 (resolume) and 4 (ableset).
pub mod request_log;
```

and add the resolume/ableset module declarations in Tasks 3/4 respectively (each task adds its own `pub mod NAME;` line).

- [ ] **Step 5: Verify compilation with feature on**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo check --workspace --features mock-integrations --all-targets 2>&1 | tail -10
```

Expected: same as Step 4 (the mock_integrations module is still not referenced from main.rs yet, but submod files don't exist so cargo would fail if mod.rs references them). Per Step 4, ensure the mod.rs only declares `request_log` for now.

- [ ] **Step 6: Run clippy**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo clippy --workspace --all-targets -- -D warnings -W clippy::all 2>&1 | tail -10
```

Expected: zero warnings.

- [ ] **Step 7: Run cargo fmt**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo fmt --all
```

- [ ] **Step 8: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && git add crates/presenter-server/Cargo.toml crates/presenter-server/src/mock_integrations/ && git commit -m "feat(server): add mock-integrations Cargo feature + request log skeleton (#279)"
```

---

## Task 3: Resolume Mock (Sonnet)

**Files:**
- Create: `crates/presenter-server/src/mock_integrations/resolume.rs`
- Modify: `crates/presenter-server/src/mock_integrations/mod.rs` (add `pub mod resolume;`)

The Resolume mock implements 3 endpoints exercised by `crates/presenter-server/src/resolume/driver.rs`:
1. `GET /api/v1/composition` — returns minimal valid Composition JSON.
2. `PUT /api/v1/composition/parameter/by-id/:id` — accepts `{"value": "..."}`, returns 200.
3. `POST /api/v1/composition/clips/by-id/:id/connect` — triggers a clip, returns 200.

It also serves `GET /__mock/log` so the operator can inspect what dev sent.

- [ ] **Step 1: Create resolume.rs**

Create `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-server/src/mock_integrations/resolume.rs` with:

```rust
//! Mock Resolume Arena HTTP listener for dev. Listens on
//! `127.0.0.1:8091`, accepts the 3 endpoints presenter-server's outbound
//! resolume driver calls, returns minimal-valid responses, and records
//! every request to the shared `RequestLog`.

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Context;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
    routing::{get, post, put},
    Router,
};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tracing::{info, warn};

use super::request_log::{log_handler, RequestLog};

const MOCK_RESOLUME_ADDR: &str = "127.0.0.1:8091";
const MOCK_NAME: &str = "resolume";

pub async fn spawn(log: Arc<RequestLog>) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/api/v1/composition", get(get_composition))
        .route(
            "/api/v1/composition/parameter/by-id/:id",
            put(put_parameter),
        )
        .route(
            "/api/v1/composition/clips/by-id/:id/connect",
            post(post_clip_connect),
        )
        .route("/__mock/log", get(log_handler))
        .with_state(log);

    let addr: SocketAddr = MOCK_RESOLUME_ADDR
        .parse()
        .context("invalid MOCK_RESOLUME_ADDR")?;
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("mock-resolume failed to bind {addr}"))?;
    info!(%addr, "mock-resolume listener started");

    tokio::spawn(async move {
        if let Err(err) = axum::serve(listener, app).await {
            warn!(?err, "mock-resolume listener exited");
        }
    });
    Ok(())
}

async fn get_composition(State(log): State<Arc<RequestLog>>) -> Json<Value> {
    log.record(MOCK_NAME, "GET", "/api/v1/composition", None);
    Json(json!({
        "name": "Mock Composition",
        "layers": [],
        "columns": [],
    }))
}

async fn put_parameter(
    State(log): State<Arc<RequestLog>>,
    Path(id): Path<String>,
    body: String,
) -> StatusCode {
    let preview = if body.len() > 256 { Some(format!("{}...", &body[..256])) } else { Some(body) };
    log.record(
        MOCK_NAME,
        "PUT",
        &format!("/api/v1/composition/parameter/by-id/{id}"),
        preview,
    );
    StatusCode::OK
}

async fn post_clip_connect(
    State(log): State<Arc<RequestLog>>,
    Path(id): Path<String>,
) -> StatusCode {
    log.record(
        MOCK_NAME,
        "POST",
        &format!("/api/v1/composition/clips/by-id/{id}/connect"),
        None,
    );
    StatusCode::OK
}
```

- [ ] **Step 2: Add `pub mod resolume;` to mod.rs**

In `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-server/src/mock_integrations/mod.rs`, add `pub mod resolume;` and uncomment the `resolume::spawn(log.clone()).await?;` line if Task 2 deferred it.

If Task 2's mod.rs only contains `pub mod request_log;`, replace the contents with the full version from Task 2 Step 2 (the one with `pub mod resolume;`, `pub mod ableset;`, and `start_all()`). Note that `pub mod ableset;` will compile-fail until Task 4 — keep this commented temporarily:

```rust
//! In-process mock listeners for outbound integrations (Resolume HTTP,
//! AbleSet HTTP). Compiled only when the `mock-integrations` feature is
//! enabled — dev builds use this; prod builds omit it entirely.

use std::sync::Arc;

// pub mod ableset; // added in Task 4
pub mod request_log;
pub mod resolume;

pub async fn start_all() -> anyhow::Result<()> {
    let log = Arc::new(request_log::RequestLog::new());
    resolume::spawn(log.clone()).await?;
    // ableset::spawn(log).await?; // added in Task 4
    let _ = log; // placeholder to silence unused-arc warning until Task 4
    Ok(())
}
```

- [ ] **Step 3: Build with feature**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo build --workspace --features mock-integrations 2>&1 | tail -10
```

Expected: clean build.

- [ ] **Step 4: Run clippy with feature**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo clippy --workspace --features mock-integrations --all-targets -- -D warnings -W clippy::all 2>&1 | tail -10
```

Expected: zero warnings.

- [ ] **Step 5: Run clippy WITHOUT feature**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo clippy --workspace --all-targets -- -D warnings -W clippy::all 2>&1 | tail -10
```

Expected: zero warnings (mocks are entirely cfg-gated).

- [ ] **Step 6: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo fmt --all && git add crates/presenter-server/src/mock_integrations/ && git commit -m "feat(server): mock Resolume HTTP listener on 127.0.0.1:8091 (#279)"
```

---

## Task 4: AbleSet Mock (Sonnet)

**Files:**
- Create: `crates/presenter-server/src/mock_integrations/ableset.rs`
- Modify: `crates/presenter-server/src/mock_integrations/mod.rs` (uncomment ableset)

The AbleSet mock implements 1 endpoint exercised by `crates/presenter-server/src/ableset.rs`:
- `GET /api/setlist` — returns SetlistResponse JSON shape.

- [ ] **Step 1: Create ableset.rs**

Create `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-server/src/mock_integrations/ableset.rs` with:

```rust
//! Mock AbleSet HTTP listener for dev. Listens on `127.0.0.1:39042`,
//! serves a static-but-valid `/api/setlist` response, and records every
//! request to the shared `RequestLog`.

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Context;
use axum::{extract::State, response::Json, routing::get, Router};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tracing::{info, warn};

use super::request_log::{log_handler, RequestLog};

const MOCK_ABLESET_ADDR: &str = "127.0.0.1:39042";
const MOCK_NAME: &str = "ableset";

pub async fn spawn(log: Arc<RequestLog>) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/api/setlist", get(get_setlist))
        .route("/__mock/log", get(log_handler))
        .with_state(log);

    let addr: SocketAddr = MOCK_ABLESET_ADDR
        .parse()
        .context("invalid MOCK_ABLESET_ADDR")?;
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("mock-ableset failed to bind {addr}"))?;
    info!(%addr, "mock-ableset listener started");

    tokio::spawn(async move {
        if let Err(err) = axum::serve(listener, app).await {
            warn!(?err, "mock-ableset listener exited");
        }
    });
    Ok(())
}

async fn get_setlist(State(log): State<Arc<RequestLog>>) -> Json<Value> {
    log.record(MOCK_NAME, "GET", "/api/setlist", None);
    Json(json!({
        "activeSongId": null,
        "songs": [],
    }))
}
```

- [ ] **Step 2: Update mod.rs to enable ableset**

In `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-server/src/mock_integrations/mod.rs`, replace the file contents with:

```rust
//! In-process mock listeners for outbound integrations (Resolume HTTP,
//! AbleSet HTTP). Compiled only when the `mock-integrations` feature is
//! enabled — dev builds use this; prod builds omit it entirely.

use std::sync::Arc;

pub mod ableset;
pub mod request_log;
pub mod resolume;

pub async fn start_all() -> anyhow::Result<()> {
    let log = Arc::new(request_log::RequestLog::new());
    resolume::spawn(log.clone()).await?;
    ableset::spawn(log).await?;
    Ok(())
}
```

- [ ] **Step 3: Build with feature**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo build --workspace --features mock-integrations 2>&1 | tail -10
```

Expected: clean build.

- [ ] **Step 4: Run clippy with feature**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo clippy --workspace --features mock-integrations --all-targets -- -D warnings -W clippy::all 2>&1 | tail -10
```

Expected: zero warnings.

- [ ] **Step 5: Run clippy WITHOUT feature**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo clippy --workspace --all-targets -- -D warnings -W clippy::all 2>&1 | tail -10
```

Expected: zero warnings.

- [ ] **Step 6: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo fmt --all && git add crates/presenter-server/src/mock_integrations/ && git commit -m "feat(server): mock AbleSet HTTP listener on 127.0.0.1:39042 (#279)"
```

---

## Task 5: Wire Mocks into main.rs (Sonnet)

**Files:**
- Modify: `crates/presenter-server/src/main.rs`

- [ ] **Step 1: Read existing main.rs structure**

Run:

```bash
sed -n '90,120p' /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-server/src/main.rs
```

Identify:
- Where `async fn main()` begins (~line 96)
- Where the main `TcpListener::bind` happens (~line 103)
- Where `axum::serve(listener, app).await` is called (~line 107)

The mock listeners must be spawned BEFORE `axum::serve(listener, app).await` (because `axum::serve` blocks until shutdown), but AFTER any state initialization that integration code might depend on.

- [ ] **Step 2: Add module declaration**

Near the top of `crates/presenter-server/src/main.rs`, after the existing `mod` declarations (use `grep -n "^mod " crates/presenter-server/src/main.rs | head -10` to find them), add:

```rust
#[cfg(feature = "mock-integrations")]
mod mock_integrations;
```

If the file uses `pub mod` for some modules, follow the existing convention (use `mod` if private modules dominate; `pub mod` only if main.rs exports symbols, which is unusual for binary-crate main.rs — `mod` is correct here).

- [ ] **Step 3: Spawn mocks before serve**

In `crates/presenter-server/src/main.rs`, find the line `axum::serve(listener, app).await.context("server failure")` (around line 107). Immediately BEFORE that line, add:

```rust
    #[cfg(feature = "mock-integrations")]
    mock_integrations::start_all().await?;
```

The full surrounding context BEFORE looks like (verify by reading the file — adapt if the actual lines differ):

```rust
    let listener = TcpListener::bind(addr)
        .await
        .context("failed to bind")?;
    axum::serve(listener, app).await.context("server failure")
}
```

AFTER:

```rust
    let listener = TcpListener::bind(addr)
        .await
        .context("failed to bind")?;
    #[cfg(feature = "mock-integrations")]
    mock_integrations::start_all().await?;
    axum::serve(listener, app).await.context("server failure")
}
```

- [ ] **Step 4: Build with feature**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo build --workspace --features mock-integrations 2>&1 | tail -10
```

Expected: clean build.

- [ ] **Step 5: Build WITHOUT feature**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo build --workspace 2>&1 | tail -10
```

Expected: clean build (mock module isn't compiled).

- [ ] **Step 6: Smoke test the binary with feature**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && PRESENTER_PORT=18080 PRESENTER_DB_URL=sqlite::memory: timeout 5 ./target/debug/presenter-server 2>&1 | tail -30 &
sleep 2 && curl -s http://127.0.0.1:8091/api/v1/composition && echo
sleep 1 && curl -s http://127.0.0.1:39042/api/setlist && echo
sleep 1 && curl -s http://127.0.0.1:8091/__mock/log
wait
```

Expected:
- Composition response: `{"columns":[],"layers":[],"name":"Mock Composition"}`
- Setlist response: `{"activeSongId":null,"songs":[]}`
- Log: 2 entries (the GET composition and the GET setlist).

If port 18080 conflicts, pick another. The actual main listener port doesn't matter for this smoke test; we only care that the mock ports respond.

- [ ] **Step 7: Run clippy both modes**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo clippy --workspace --all-targets -- -D warnings -W clippy::all 2>&1 | tail -5
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo clippy --workspace --features mock-integrations --all-targets -- -D warnings -W clippy::all 2>&1 | tail -5
```

Expected: zero warnings in both modes.

- [ ] **Step 8: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo fmt --all && git add crates/presenter-server/src/main.rs && git commit -m "feat(server): start mock integrations under feature flag (#279)"
```

---

## Task 6: Unit Tests for Mocks (Sonnet)

**Files:**
- Modify: `crates/presenter-server/src/mock_integrations/resolume.rs` (add `#[cfg(test)] mod tests`)
- Modify: `crates/presenter-server/src/mock_integrations/ableset.rs` (add `#[cfg(test)] mod tests`)

The tests use `axum::Router::oneshot` (via `tower::ServiceExt::oneshot`) to drive the routers WITHOUT binding a port. `tower` is already in `dev-dependencies` (used by router tests).

- [ ] **Step 1: Add tests to resolume.rs**

At the END of `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-server/src/mock_integrations/resolume.rs`, append:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    use axum::{
        body::Body,
        http::{Method, Request, StatusCode},
    };
    use tower::ServiceExt;

    fn router(log: Arc<RequestLog>) -> Router {
        Router::new()
            .route("/api/v1/composition", get(get_composition))
            .route(
                "/api/v1/composition/parameter/by-id/:id",
                put(put_parameter),
            )
            .route(
                "/api/v1/composition/clips/by-id/:id/connect",
                post(post_clip_connect),
            )
            .route("/__mock/log", get(log_handler))
            .with_state(log)
    }

    #[tokio::test]
    async fn accepts_composition_get() {
        let log = Arc::new(RequestLog::new());
        let app = router(log.clone());

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/v1/composition")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("oneshot");

        assert_eq!(response.status(), StatusCode::OK);
        let body_bytes = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .expect("body bytes");
        let value: serde_json::Value = serde_json::from_slice(&body_bytes).expect("json");
        assert_eq!(value["name"], "Mock Composition");
        assert!(value["layers"].is_array());
        assert!(value["columns"].is_array());

        // Log should record the request.
        let entries = log.snapshot();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].method, "GET");
    }

    #[tokio::test]
    async fn accepts_clip_trigger_and_logs_path() {
        let log = Arc::new(RequestLog::new());
        let app = router(log.clone());

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/composition/clips/by-id/abc-123/connect")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("oneshot");

        assert_eq!(response.status(), StatusCode::OK);
        let entries = log.snapshot();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].method, "POST");
        assert_eq!(
            entries[0].path,
            "/api/v1/composition/clips/by-id/abc-123/connect"
        );
    }
}
```

- [ ] **Step 2: Add test to ableset.rs**

At the END of `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-server/src/mock_integrations/ableset.rs`, append:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    use axum::{
        body::Body,
        http::{Method, Request, StatusCode},
    };
    use tower::ServiceExt;

    #[tokio::test]
    async fn serves_setlist_shape() {
        let log = Arc::new(RequestLog::new());
        let app = Router::new()
            .route("/api/setlist", get(get_setlist))
            .with_state(log.clone());

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/setlist")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("oneshot");

        assert_eq!(response.status(), StatusCode::OK);
        let body_bytes = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .expect("body bytes");
        let value: serde_json::Value = serde_json::from_slice(&body_bytes).expect("json");
        // Must have the keys presenter-server's SetlistResponse expects.
        assert!(value.get("activeSongId").is_some(), "missing activeSongId");
        assert!(value["songs"].is_array());

        let entries = log.snapshot();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].method, "GET");
        assert_eq!(entries[0].path, "/api/setlist");
    }
}
```

- [ ] **Step 3: Run tests with feature**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo test --workspace --features mock-integrations -- mock_integrations 2>&1 | tail -15
```

Expected: 3 tests pass.

- [ ] **Step 4: Run clippy both modes**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo clippy --workspace --all-targets -- -D warnings -W clippy::all 2>&1 | tail -5
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo clippy --workspace --features mock-integrations --all-targets -- -D warnings -W clippy::all 2>&1 | tail -5
```

Expected: zero warnings in both modes.

- [ ] **Step 5: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo fmt --all && git add crates/presenter-server/src/mock_integrations/ && git commit -m "test(server): unit tests for resolume + ableset mocks (#279)"
```

---

## Task 7: Pipeline.yml — Feature Flag + DB Redirect (Sonnet)

**Files:**
- Modify: `.github/workflows/pipeline.yml`

- [ ] **Step 1: Update build command to use feature flag**

In `/home/newlevel/devel/presenter/presenter-dev2/.github/workflows/pipeline.yml`, find the "Build release binaries" step (use `grep -n "Build release binaries" .github/workflows/pipeline.yml`). The current command is:

```yaml
- name: Build release binaries
  run: cargo build --release -p presenter-server -p presenter-importer
  env:
    PRESENTER_BUILD_CHANNEL: dev
```

Change to:

```yaml
- name: Build release binaries
  run: cargo build --release -p presenter-server -p presenter-importer --features presenter-server/mock-integrations
  env:
    PRESENTER_BUILD_CHANNEL: dev
```

The `--features presenter-server/mock-integrations` syntax addresses the feature on the named member crate (necessary because `cargo build` at workspace root with `-p` doesn't inherit features by default).

- [ ] **Step 2: Add DB redirect step after "Replace dev database with production snapshot"**

In `.github/workflows/pipeline.yml`, find the step "Replace dev database with production snapshot" (around line 786-810). It ends with a `REMOTE_SCRIPT` heredoc block. AFTER that step, BEFORE the "Validate database schema" step, insert:

```yaml
      - name: Redirect integrations to mock endpoints
        run: |
          ssh deploy-target << 'REMOTE_SCRIPT'
          set -e
          DB_PATH="/opt/presenter-dev/presenter.db"
          if [ ! -f "$DB_PATH" ]; then
            echo "::warning::No dev database found; skipping integration redirect"
            exit 0
          fi
          # Issue #279: prod-snapshot DB carries production-side hosts that would
          # cause dev to send commands to live hardware. Rewrite to mock endpoints
          # baked into the dev binary (--features mock-integrations).
          sqlite3 "$DB_PATH" <<'SQL'
          UPDATE resolume_hosts SET host='127.0.0.1', port=8091, label=label || ' (mock)';
          UPDATE ableset_settings SET host='127.0.0.1', port=39042;
          DELETE FROM android_stage_displays;
          DELETE FROM video_sources;
          SQL
          echo "Integration tables redirected to localhost mock endpoints"
          REMOTE_SCRIPT
```

Indent must match the surrounding steps (typically 6 spaces for steps under `jobs.<job>.steps`). Verify with `grep -B1 'name: Replace dev' .github/workflows/pipeline.yml`.

- [ ] **Step 3: Validate the YAML syntax**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && python3 -c "import yaml; yaml.safe_load(open('.github/workflows/pipeline.yml'))" && echo "YAML OK"
```

Expected: `YAML OK`. If parsing fails, fix indentation/quoting.

- [ ] **Step 4: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && git add .github/workflows/pipeline.yml && git commit -m "ci(pipeline): build dev with mock-integrations + redirect dev DB to mocks (#279)"
```

---

## Task 8: Prod-No-Mocks Guard (Haiku)

**Files:**
- Modify: `.github/workflows/deploy.yml`
- Modify: `.github/workflows/release.yml`

- [ ] **Step 1: Add guard step to deploy.yml**

In `/home/newlevel/devel/presenter/presenter-dev2/.github/workflows/deploy.yml`, find the "Build release binaries" step (around line 50-60 based on the deploy job). AFTER that step, BEFORE any deploy/upload step, insert:

```yaml
      - name: Verify prod artifact has no mock integrations
        run: |
          if strings target/release/presenter-server | grep -E "mock_integrations::|MOCK_RESOLUME_ADDR|MOCK_ABLESET_ADDR" >/dev/null; then
            echo "::error::Prod build contains mock-integrations symbols. Rebuild without --features mock-integrations."
            exit 1
          fi
          echo "Prod artifact verified: no mock integration symbols"
```

Indent 6 spaces (or match the surrounding `      - name:` lines).

- [ ] **Step 2: Add the same guard step to release.yml**

In `/home/newlevel/devel/presenter/presenter-dev2/.github/workflows/release.yml`, find "Build release binaries" and insert the SAME step (identical YAML block, same indent) AFTER it.

- [ ] **Step 3: Validate YAML for both files**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && python3 -c "import yaml; yaml.safe_load(open('.github/workflows/deploy.yml')); yaml.safe_load(open('.github/workflows/release.yml'))" && echo "YAML OK"
```

Expected: `YAML OK`.

- [ ] **Step 4: Verify the regex catches the mock symbols**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo build --release -p presenter-server --features mock-integrations 2>&1 | tail -3
strings target/release/presenter-server | grep -E "mock_integrations::|MOCK_RESOLUME_ADDR|MOCK_ABLESET_ADDR" | head -5
```

Expected: at least one match (proves the regex would catch a mock-feature build). If NO match, the symbols may have been inlined; expand the regex (e.g. add `mock-resolume` or `mock-ableset` log message strings).

Then verify the inverse:

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo build --release -p presenter-server 2>&1 | tail -3
strings target/release/presenter-server | grep -E "mock_integrations::|MOCK_RESOLUME_ADDR|MOCK_ABLESET_ADDR" | head -5
```

Expected: NO match (default build has no mock symbols).

If either verification fails, adjust the grep pattern in BOTH workflow files until it correctly differentiates.

- [ ] **Step 5: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && git add .github/workflows/deploy.yml .github/workflows/release.yml && git commit -m "ci(deploy,release): guard against accidental mock-integrations in prod (#279)"
```

---

## Task 9: Playwright E2E — Mock Roundtrip (Sonnet)

**Files:**
- Create: `tests/e2e/mock-resolume-roundtrip.spec.ts`

- [ ] **Step 1: Find an operator action that triggers a Resolume call**

Read existing `tests/e2e/operator-controls.spec.ts` and `tests/e2e/settings.spec.ts` to find tests that already exercise Resolume. The simplest action: configure a Resolume host and use the "Test Connection" endpoint added in PR #213 (`POST /integrations/resolume/hosts/{id}/test`), or perform a clip mapping change.

The test must:
1. Skip on prod builds (channel === "release") via reading `/healthz`.
2. Hit the operator UI to perform an action that causes presenter-server to call its outbound Resolume driver.
3. Wait briefly for the driver to make the request.
4. Hit `http://10.77.8.134:8080/__mock/log` and assert at least one `resolume` entry appears.

Note: in the real dev pipeline, `/__mock/log` is exposed on `127.0.0.1:8091` (the mock's own port), NOT on the main presenter port. The test needs to reach the mock directly. From the LAN, port 8091 may not be exposed unless the systemd unit binds to `0.0.0.0` (it doesn't — the spec says `127.0.0.1:8091`).

This is a real constraint: the Playwright test runs from CI's runner, which on the dev machine IS `127.0.0.1` from the dev server's perspective. But Playwright runs against `baseURL = http://10.77.8.134:8080`, which is the LAN URL. The runner machine and the dev server are the same machine, so `127.0.0.1:8091` works from the runner.

If they were different machines, this would fail; verify by running locally.

- [ ] **Step 2: Create the test file**

Create `/home/newlevel/devel/presenter/presenter-dev2/tests/e2e/mock-resolume-roundtrip.spec.ts` with:

```typescript
import { test, expect } from "@playwright/test";

/**
 * E2E test for issue #279: dev's outbound Resolume calls must hit the
 * embedded mock listener on 127.0.0.1:8091, NOT a real Resolume Arena.
 *
 * Skipped on prod builds (channel === "release") because the mock
 * doesn't exist there.
 *
 * The mock's request log is on `127.0.0.1:8091/__mock/log`. This test
 * runs from the CI runner which lives on the same machine as the dev
 * server, so `127.0.0.1` resolves to the dev server.
 */
test("dev resolume calls hit the mock, not real arena", async ({
  page,
  baseURL,
  request,
}) => {
  if (!baseURL) test.skip(true, "baseURL required");

  const consoleMessages: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error" || msg.type() === "warning") {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  const healthRes = await request.get(new URL("/healthz", baseURL!).toString());
  const health = (await healthRes.json()) as { channel: string };
  if (health.channel === "release" || health.channel === "") {
    test.skip(true, "mock-integrations not built into release binaries");
  }

  // Snapshot mock log size before action.
  const mockLogUrl = "http://127.0.0.1:8091/__mock/log";
  const beforeRes = await request.get(mockLogUrl);
  if (!beforeRes.ok()) {
    test.skip(true, "mock-resolume not reachable from CI runner");
  }
  const beforeEntries = (await beforeRes.json()) as Array<{ method: string }>;
  const beforeCount = beforeEntries.length;

  // Trigger a Resolume call by hitting the test-connection endpoint of
  // the first configured host.
  await page.goto(new URL("/ui/operator/settings", baseURL!).toString());
  await page.waitForLoadState("networkidle");

  const hostsRes = await request.get(
    new URL("/integrations/resolume/hosts", baseURL!).toString(),
  );
  const hosts = (await hostsRes.json()) as Array<{ id: string }>;
  expect(hosts.length).toBeGreaterThan(0);
  const hostId = hosts[0].id;

  await request.post(
    new URL(
      `/integrations/resolume/hosts/${hostId}/test`,
      baseURL!,
    ).toString(),
  );

  // Allow the driver to make the call (small latency).
  await page.waitForTimeout(500);

  // Mock log should now have at least one new entry.
  const afterRes = await request.get(mockLogUrl);
  expect(afterRes.ok()).toBeTruthy();
  const afterEntries = (await afterRes.json()) as Array<{
    method: string;
    mock: string;
  }>;
  expect(afterEntries.length).toBeGreaterThan(beforeCount);

  const newEntries = afterEntries.slice(beforeCount);
  const resolumeEntries = newEntries.filter((e) => e.mock === "resolume");
  expect(resolumeEntries.length).toBeGreaterThan(0);

  // Console should be clean.
  expect(consoleMessages).toEqual([]);
});
```

- [ ] **Step 3: Run the test locally (against the running dev server)**

Verify the dev server is running with the feature enabled. The test machine is the same as the dev server, so:

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && npx playwright test tests/e2e/mock-resolume-roundtrip.spec.ts --reporter=list 2>&1 | tail -20
```

Expected: 1 passed (or 1 skipped if dev hasn't been redeployed with the feature yet — which is fine; CI will run it after deploy).

If the test fails:
- Test-connection endpoint missing or different shape → adjust the action in Step 2's test (find another action that triggers a Resolume call).
- `127.0.0.1:8091/__mock/log` returns 404 → mock not started; rerun Task 5's Step 6 to verify.
- Hosts list empty on dev → the dev DB needs at least one resolume_hosts row; the pipeline's prod-snapshot copy should provide that. If running locally without that snapshot, manually insert a host via the settings UI before running the test.

- [ ] **Step 4: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && git add tests/e2e/mock-resolume-roundtrip.spec.ts && git commit -m "test(e2e): mock-resolume roundtrip — verify dev hits mock not real Arena (#279)"
```

---

## Task 10: Local Checks, Push, CI Monitor, Dev Verification, Open PR (Controller)

This task is handled by the controller. Local Rust + WASM builds are allowed.

### Local pre-push checks

- [ ] **Step 1: Workspace fmt**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo fmt --all --check
```

Expected: zero output.

- [ ] **Step 2: Workspace clippy WITHOUT feature**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo clippy --workspace --all-targets -- -D warnings -W clippy::all 2>&1 | tail -10
```

Expected: zero warnings.

- [ ] **Step 3: Workspace clippy WITH feature**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo clippy --workspace --features presenter-server/mock-integrations --all-targets -- -D warnings -W clippy::all 2>&1 | tail -10
```

Expected: zero warnings.

- [ ] **Step 4: presenter-ui WASM clippy**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui && cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings -W clippy::all 2>&1 | tail -5
```

Expected: zero warnings.

- [ ] **Step 5: Workspace tests with feature**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && cargo test --workspace --features presenter-server/mock-integrations 2>&1 | tail -10
```

Expected: all tests pass, including the 3 new mock tests.

- [ ] **Step 6: Push**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && git push origin dev
```

- [ ] **Step 7: Monitor CI to terminal state**

Per `core/ci-monitoring.md`: ONE background `sleep + gh run view` per cycle. Never `gh run watch`. Never custom monitor scripts.

```bash
RUN_ID=$(gh run list --branch dev --limit 1 --json databaseId --jq '.[0].databaseId')
sleep 1500 && gh run view $RUN_ID --json status,conclusion,jobs --jq '{status, conclusion, jobs: [.jobs[] | {name, status, conclusion}]}'
```

Wait for ALL jobs `completed`. If any fails, `gh run view <run-id> --log-failed`, fix root cause in ONE commit, push, monitor again.

### Dev verification

- [ ] **Step 8: Verify dev shows v0.4.53 and mocks respond**

```bash
echo "=== /healthz ===" && curl -s http://10.77.8.134:8080/healthz
echo "=== Mock Resolume ===" && curl -s http://10.77.8.134:8091/api/v1/composition || echo "(mock port not LAN-exposed; SSH check below)"
```

If port 8091 is not LAN-exposed (the spec says 127.0.0.1 binding), SSH locally:

```bash
sshpass -p 'newlevel' ssh -o StrictHostKeyChecking=no newlevel@10.77.8.134 "curl -s http://127.0.0.1:8091/api/v1/composition; curl -s http://127.0.0.1:39042/api/setlist; curl -s http://127.0.0.1:8091/__mock/log"
```

Or, since this dev2 machine IS the dev server (10.77.8.134 = local), just:

```bash
curl -s http://127.0.0.1:8091/api/v1/composition && echo
curl -s http://127.0.0.1:39042/api/setlist && echo
curl -s http://127.0.0.1:8091/__mock/log
```

Expected: composition + setlist responses, log entries showing the curls.

- [ ] **Step 9: Verify dev's resolume_hosts now point at 127.0.0.1**

```bash
sudo -n sqlite3 /opt/presenter-dev/presenter.db "SELECT host, port, label FROM resolume_hosts;"
```

Expected: every row has `host=127.0.0.1`, `port=8091`, label suffixed with ` (mock)`.

- [ ] **Step 10: Verify prod's resolume is UNTOUCHED**

```bash
echo "=== Prod /healthz (must still be release v0.4.52 — this PR not merged yet) ===" && curl -s http://10.77.9.205/healthz
sshpass -p 'newlevel' ssh -o StrictHostKeyChecking=no newlevel@presenter.lan "sqlite3 /opt/presenter/presenter.db 'SELECT host, port FROM resolume_hosts;'"
```

Expected: prod still on the previous version, prod's resolume_hosts still point at the real Resolume IPs (NOT 127.0.0.1).

- [ ] **Step 11: Functional roundtrip via Playwright MCP**

Open dev's operator UI, find a Resolume host, click "Test Connection", check `/__mock/log`:

```
mcp__plugin_playwright_playwright__browser_navigate(url: "http://10.77.8.134:8080/ui/operator/settings")
mcp__plugin_playwright_playwright__browser_evaluate(function: "async () => { const r = await fetch('/integrations/resolume/hosts'); const hosts = await r.json(); if (!hosts.length) return 'no hosts'; const tr = await fetch(`/integrations/resolume/hosts/${hosts[0].id}/test`, { method: 'POST' }); return { test: tr.status, hosts_count: hosts.length }; }")
```

Then locally:

```bash
curl -s http://127.0.0.1:8091/__mock/log | python3 -m json.tool | tail -15
```

Expected: at least one `{"method":"GET","path":"/api/v1/composition",...}` entry from the test-connection.

### Open PR

- [ ] **Step 12: Open PR**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2 && gh pr create --base main --head dev --title "fix(server): mock dev integrations to stop prod resolume bleed (#279)" --body "$(cat <<'EOF'
## Summary

Stops the dev server from sending live commands to production hardware. Dev binaries now embed in-process mocks for Resolume HTTP and AbleSet HTTP behind a `mock-integrations` Cargo feature; the dev pipeline rewrites integration table rows to point at those mocks. Prod is unaffected (same source tree, but builds without the feature; CI guard in deploy.yml + release.yml prevents accidental drift).

Closes #279.

## Mechanism of the bug

`pipeline.yml:786` copies prod's `presenter.db` to dev on every deploy. The snapshot carries prod's `resolume_hosts` rows. Dev restarts, reads them, and starts pushing clip triggers + text updates to PROD's Resolume Arena during live worship services.

## What changed

- New module `crates/presenter-server/src/mock_integrations/` (feature-gated): Resolume mock on `127.0.0.1:8091`, AbleSet mock on `127.0.0.1:39042`, shared in-memory request log at `/__mock/log`.
- `Cargo.toml` of presenter-server: new `mock-integrations` feature.
- `main.rs`: spawns mocks at startup under `#[cfg(feature = "mock-integrations")]`.
- `pipeline.yml`: dev build uses `--features presenter-server/mock-integrations`; new "Redirect integrations to mock endpoints" sqlite3 step rewrites dev's resolume_hosts + ableset_settings, DELETEs android_stage_displays + video_sources.
- `deploy.yml` + `release.yml`: new "Verify prod artifact has no mock integrations" guard step that fails the build if mock symbols leak into a prod artifact.
- 3 new unit tests for the mocks (oneshot router invocation, no port binding).
- New Playwright E2E `mock-resolume-roundtrip.spec.ts` (skipped on release builds): triggers a Resolume call via the operator UI, asserts the mock's request log saw it.
- Bumped version 0.4.52 → 0.4.53.

## Production data treatment

**Untouched.** Prod's database, binary, and runtime are unchanged. The CI guard ensures prod can never receive a mocked binary.

## Test plan

- [x] `cargo clippy --workspace --all-targets -- -D warnings -W clippy::all` (no feature) — zero warnings
- [x] `cargo clippy --workspace --features presenter-server/mock-integrations --all-targets -- -D warnings -W clippy::all` — zero warnings
- [x] `cargo test --workspace --features presenter-server/mock-integrations` — all tests pass (including 3 new mock tests)
- [x] `cargo fmt --all --check` — clean
- [x] CI green on dev
- [x] **Manual dev verification:**
  - `/healthz` shows v0.4.53 (dev)
  - `127.0.0.1:8091/api/v1/composition` returns mock composition JSON
  - `127.0.0.1:39042/api/setlist` returns mock setlist JSON
  - `/__mock/log` shows recent requests
  - `resolume_hosts` rows in dev DB all have `host='127.0.0.1', port=8091`
  - Prod `/healthz` shows the OLD version, prod's `resolume_hosts` still point at the real Arena IPs (no leak)
  - Test connection from dev's operator UI → confirmed only the mock's log changed; prod's Arena was NOT touched

Closes #279
EOF
)"
```

- [ ] **Step 13: Confirm PR is mergeable + clean**

```bash
PR_NUM=$(gh pr list --head dev --base main --json number --jq '.[0].number')
gh api repos/zbynekdrlik/presenter/pulls/$PR_NUM --jq '{mergeable: .mergeable, mergeable_state: .mergeable_state}'
```

Expected: `{"mergeable": true, "mergeable_state": "clean"}`.

If `unstable`, wait for in-progress jobs (Mutation Testing). If `behind`, sync dev with main and push. Never bypass branch protection.

### Pre-completion gate

- [ ] **Step 14: Run `/plan-check`**

Audit every plan + spec requirement. Every item must be `[x]`.

- [ ] **Step 15: Run `/review`** on the PR diff

Address every 🔴, 🟡, AND 🔵 finding inside the diff. Re-review until both audits return `0 🔴 0 🟡 0 🔵`.

- [ ] **Step 16: Send completion report**

Per `core/completion-report.md`. Include:

- `✅ CI: green` (with run id)
- `✅ /plan-check: N/N fulfilled`
- `✅ /review: clean — 0 🔴 0 🟡 0 🔵`
- `✅ Deploy: dev shows v0.4.53; mocks responding on 127.0.0.1:8091 and 127.0.0.1:39042; dev DB redirected; prod untouched`
- `🌐 Dev:  http://10.77.8.134:8080/ui/operator`
- `🌐 Prod: http://10.77.9.205/ui/operator`
- `[presenter] PR #N: <full title>` + URL
- Wait for explicit "merge it" before merging.

---

## Verification Summary

| Check | How to verify |
|-------|---------------|
| Mocks live in feature-gated module | `ls crates/presenter-server/src/mock_integrations/` shows mod.rs, request_log.rs, resolume.rs, ableset.rs |
| Default build has no mock symbols | `strings target/release/presenter-server \| grep -E "MOCK_RESOLUME_ADDR\|MOCK_ABLESET_ADDR"` returns nothing |
| Feature build has mock symbols | Same grep with `--features presenter-server/mock-integrations` returns matches |
| Mocks respond | `curl 127.0.0.1:8091/api/v1/composition` and `curl 127.0.0.1:39042/api/setlist` return JSON |
| Request log works | `curl 127.0.0.1:8091/__mock/log` shows entries from the curls above |
| Dev DB redirected | `sqlite3 /opt/presenter-dev/presenter.db 'SELECT host FROM resolume_hosts;'` returns all `127.0.0.1` |
| Prod untouched | `curl prod/healthz` shows old version; prod `resolume_hosts` still has real Arena IPs |
| CI guard works | A test commit with `--features mock-integrations` in deploy.yml fails the verify step |
| All tests green | `cargo test --workspace --features presenter-server/mock-integrations` 187+ tests pass |
| Playwright E2E green | `mock-resolume-roundtrip.spec.ts` passes in CI |
