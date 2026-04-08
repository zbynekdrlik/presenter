# Arena Connection Reliability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix Arena (Resolume) connections that fail to recover after transient errors, and add diagnostics so operators can see retry status during live services.

**Architecture:** Three targeted fixes: (1) split the aggressive 500ms HTTP timeout into per-operation timeouts (5s for composition fetch, 2s for clip actions), (2) guarantee delivery of config-change commands instead of silently dropping them, (3) add diagnostic fields (consecutive failures, last attempt, error duration) to connection snapshots and display them in the settings UI with a "Test Connection" button.

**Tech Stack:** Rust (reqwest, tokio, axum), JavaScript (settings_script.js), Playwright E2E

**Spec:** `docs/superpowers/specs/2026-04-08-arena-connection-reliability-design.md`

---

## Context

Issue #213: The operator reported that Arena connections fail and never recover, even though Arena is reachable from a browser. Root causes: (1) 500ms HTTP client timeout is too aggressive — Arena under rendering load responds slowly, (2) config update commands (`RefreshConfig`) are sent via `try_send()` and silently dropped when the 16-slot channel is full, (3) no diagnostics to distinguish "retrying" from "stuck."

**Key existing code:**
- `crates/presenter-server/src/resolume/mod.rs` — `ResolumeRegistry`, `ResolumeConnectionSnapshot`, `DEFAULT_TIMEOUT` (500ms), `HOST_COMMAND_CAPACITY` (16)
- `crates/presenter-server/src/resolume/driver.rs` — `HostDriver`, `run_host_worker`, `refresh_mapping()`, `record_error()`, `mark_connected()`
- `crates/presenter-server/src/resolume/tests.rs` — 14 existing tests using wiremock
- `crates/presenter-server/src/router/integrations/resolume.rs` — CRUD endpoints for hosts
- `crates/presenter-server/src/state/integrations.rs` — `sync_resolume_hosts()`, `resolume_status_for()`
- `crates/presenter-server/src/settings_script.js` — JavaScript rendering of host status in settings UI
- `tests/e2e/settings.spec.ts` — E2E test with `startMockResolume()` helper

---

## File Structure

### Modified Files
| File | Change |
|------|--------|
| `crates/presenter-server/src/resolume/mod.rs` | Remove `DEFAULT_TIMEOUT`, add `CONNECT_TIMEOUT`, extend `ResolumeConnectionSnapshot` with 3 new fields, fix `try_send` for critical commands, add warning logs for dropped data commands |
| `crates/presenter-server/src/resolume/driver.rs` | Add `COMPOSITION_TIMEOUT` and `ACTION_TIMEOUT` constants, apply per-request timeouts, update `record_error()` and `mark_connected()` for new diagnostic fields |
| `crates/presenter-server/src/resolume/tests.rs` | Update existing `record_error` test, add new tests for diagnostics |
| `crates/presenter-server/src/router/integrations/resolume.rs` | Add `test_resolume_host()` endpoint |
| `crates/presenter-server/src/state/integrations.rs` | Add `test_resolume_host_connection()` method |
| `crates/presenter-server/src/router.rs` | Register test endpoint route |
| `crates/presenter-server/src/settings_script.js` | Enhanced error display with retry count, "Test Connection" button |
| `crates/presenter-server/src/ui/settings.rs` | Update `SettingsHostRow` for new snapshot fields |
| `tests/e2e/settings.spec.ts` | Add test for diagnostics display and test-connection button |

---

## Task 1: Extend ResolumeConnectionSnapshot with Diagnostic Fields

**Files:**
- Modify: `crates/presenter-server/src/resolume/mod.rs:30-48`
- Modify: `crates/presenter-server/src/resolume/driver.rs:328-357`
- Modify: `crates/presenter-server/src/resolume/tests.rs:922-945`

- [ ] **Step 1: Add new fields to ResolumeConnectionSnapshot**

In `crates/presenter-server/src/resolume/mod.rs`, replace the `ResolumeConnectionSnapshot` struct and its `disabled()` constructor (lines 30-48):

```rust
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolumeConnectionSnapshot {
    pub state: ResolumeConnectionState,
    pub last_success: Option<DateTime<Utc>>,
    pub last_latency_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub consecutive_failures: u32,
    pub last_attempt: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_since: Option<DateTime<Utc>>,
}

impl ResolumeConnectionSnapshot {
    pub fn disabled() -> Self {
        Self {
            state: ResolumeConnectionState::Disabled,
            last_success: None,
            last_latency_ms: None,
            last_error: None,
            consecutive_failures: 0,
            last_attempt: None,
            error_since: None,
        }
    }
}
```

Also update the `spawn_host` method's inline `ResolumeConnectionSnapshot` construction (lines 162-167) to include the new fields:

```rust
            ResolumeConnectionSnapshot {
                state: ResolumeConnectionState::Connecting,
                last_success: None,
                last_latency_ms: None,
                last_error: None,
                consecutive_failures: 0,
                last_attempt: None,
                error_since: None,
            }
```

- [ ] **Step 2: Update record_error() to track diagnostics**

In `crates/presenter-server/src/resolume/driver.rs`, replace the `record_error` method (lines 344-357):

```rust
    pub(super) async fn record_error(
        &mut self,
        err: anyhow::Error,
        status: &Arc<RwLock<ResolumeConnectionSnapshot>>,
    ) {
        error!(host = %self.config.host, error = ?err, "resolume host error");
        let mut guard = status.write().await;
        let now = Utc::now();
        if guard.state != ResolumeConnectionState::Error {
            guard.error_since = Some(now);
        }
        guard.state = ResolumeConnectionState::Error;
        guard.last_error = Some(err.to_string());
        guard.consecutive_failures += 1;
        guard.last_attempt = Some(now);
        self.mapping = None;
        self.endpoint = None;
        self.last_mapping_refresh = None;
        self.last_timer_payload = None;
    }
```

- [ ] **Step 3: Update mark_connected() to reset diagnostics**

In `crates/presenter-server/src/resolume/driver.rs`, replace the `mark_connected` method (lines 328-333):

```rust
    pub(super) async fn mark_connected(&self, status: &Arc<RwLock<ResolumeConnectionSnapshot>>) {
        let mut guard = status.write().await;
        guard.state = ResolumeConnectionState::Connected;
        guard.last_success = Some(Utc::now());
        guard.last_error = None;
        guard.consecutive_failures = 0;
        guard.last_attempt = Some(Utc::now());
        guard.error_since = None;
    }
```

- [ ] **Step 4: Update existing record_error test**

In `crates/presenter-server/src/resolume/tests.rs`, replace the `record_error_transitions_to_error_state_and_clears_cache` test (lines 922-945):

```rust
#[tokio::test]
async fn record_error_transitions_to_error_state_and_clears_cache() {
    let (_server, mut driver, status) = setup_bible_driver().await;

    // First establish a mapping
    driver.ensure_mapping().await.expect("mapping");
    assert!(driver.mapping.is_some());

    // Record an error
    driver
        .record_error(anyhow::anyhow!("connection refused"), &status)
        .await;

    let snap = status.read().await;
    assert_eq!(snap.state, ResolumeConnectionState::Error);
    assert_eq!(snap.last_error.as_deref(), Some("connection refused"));
    assert_eq!(snap.consecutive_failures, 1);
    assert!(snap.last_attempt.is_some());
    assert!(snap.error_since.is_some());
    drop(snap);

    // Verify caches are cleared
    assert!(driver.mapping.is_none());
    assert!(driver.endpoint.is_none());
    assert!(driver.last_mapping_refresh.is_none());
    assert!(driver.last_timer_payload.is_none());
}
```

- [ ] **Step 5: Add test for consecutive failures and error_since preservation**

In `crates/presenter-server/src/resolume/tests.rs`, add after the `record_error` test:

```rust
#[tokio::test]
async fn record_error_increments_consecutive_failures_and_preserves_error_since() {
    let (_server, mut driver, status) = setup_bible_driver().await;

    // First error sets error_since
    driver
        .record_error(anyhow::anyhow!("timeout"), &status)
        .await;
    let first_error_since = status.read().await.error_since;
    assert!(first_error_since.is_some());
    assert_eq!(status.read().await.consecutive_failures, 1);

    // Second error increments counter but preserves error_since
    driver
        .record_error(anyhow::anyhow!("timeout again"), &status)
        .await;
    let snap = status.read().await;
    assert_eq!(snap.consecutive_failures, 2);
    assert_eq!(snap.error_since, first_error_since);
    assert_eq!(snap.last_error.as_deref(), Some("timeout again"));
}

#[tokio::test]
async fn mark_connected_resets_failure_diagnostics() {
    let (_server, mut driver, status) = setup_bible_driver().await;

    // Record some errors
    driver
        .record_error(anyhow::anyhow!("timeout"), &status)
        .await;
    driver
        .record_error(anyhow::anyhow!("timeout"), &status)
        .await;
    assert_eq!(status.read().await.consecutive_failures, 2);

    // Mark connected
    driver.mark_connected(&status).await;
    let snap = status.read().await;
    assert_eq!(snap.state, ResolumeConnectionState::Connected);
    assert_eq!(snap.consecutive_failures, 0);
    assert!(snap.error_since.is_none());
    assert!(snap.last_attempt.is_some());
    assert!(snap.last_success.is_some());
}
```

- [ ] **Step 6: Run tests to verify**

```bash
cargo test -p presenter-server -- resolume --nocapture
```

Expected: All existing tests pass plus 2 new tests.

- [ ] **Step 7: Commit**

```bash
cargo fmt --all
git add crates/presenter-server/src/resolume/mod.rs crates/presenter-server/src/resolume/driver.rs crates/presenter-server/src/resolume/tests.rs
git commit -m "feat(resolume): add connection diagnostic fields to snapshot (#213)

Add consecutive_failures, last_attempt, and error_since to
ResolumeConnectionSnapshot. error_since is preserved across multiple
failures to track how long the connection has been down."
```

---

## Task 2: Split HTTP Timeouts by Operation Type

**Files:**
- Modify: `crates/presenter-server/src/resolume/mod.rs:18-19`
- Modify: `crates/presenter-server/src/resolume/driver.rs:27-29, 161-172, 196-222, 224-267`

- [ ] **Step 1: Replace DEFAULT_TIMEOUT with CONNECT_TIMEOUT**

In `crates/presenter-server/src/resolume/mod.rs`, replace line 18:

```rust
// Old:
const DEFAULT_TIMEOUT: Duration = Duration::from_millis(500);

// New:
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
```

Update `ResolumeRegistry::new()` (lines 108-117) to use connect timeout only, no global response timeout:

```rust
    pub fn new() -> anyhow::Result<Self> {
        let client = Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .map_err(|e| anyhow!("failed to build reqwest client: {e}"))?;
        Ok(Self {
            client,
            hosts: Arc::new(RwLock::new(HashMap::new())),
        })
    }
```

- [ ] **Step 2: Add per-operation timeout constants**

In `crates/presenter-server/src/resolume/driver.rs`, add after line 29 (`RESOLUTION_TTL`):

```rust
const COMPOSITION_TIMEOUT: Duration = Duration::from_secs(5);
const ACTION_TIMEOUT: Duration = Duration::from_secs(2);
```

- [ ] **Step 3: Apply COMPOSITION_TIMEOUT to refresh_mapping**

In `crates/presenter-server/src/resolume/driver.rs`, in `refresh_mapping()` (line 168-172), add `.timeout(COMPOSITION_TIMEOUT)` before `.send()`:

```rust
        let response = self
            .apply_host_header(self.client.get(&url), &endpoint)
            .timeout(COMPOSITION_TIMEOUT)
            .send()
            .await
            .with_context(|| format!("failed to fetch composition from {}", url))?;
```

- [ ] **Step 4: Apply ACTION_TIMEOUT to update_clip_text**

In `crates/presenter-server/src/resolume/driver.rs`, in `update_clip_text()` (line 209-214), add `.timeout(ACTION_TIMEOUT)`:

```rust
        let response = self
            .apply_host_header(self.client.put(&url), endpoint)
            .json(&serde_json::json!({ "value": payload.as_ref() }))
            .timeout(ACTION_TIMEOUT)
            .send()
            .await
            .with_context(|| format!("failed to update text parameter {}", param_id))?;
```

- [ ] **Step 5: Apply ACTION_TIMEOUT to trigger_clips**

In `crates/presenter-server/src/resolume/driver.rs`, in `trigger_clips()` (line 247-250), add `.timeout(ACTION_TIMEOUT)`:

```rust
                let response = request
                    .timeout(ACTION_TIMEOUT)
                    .send()
                    .await
                    .with_context(|| format!("failed to trigger clip {}", clip_id))?;
```

- [ ] **Step 6: Update test client construction**

In `crates/presenter-server/src/resolume/tests.rs`, find all places where `Client::builder().timeout(DEFAULT_TIMEOUT).build()` is used and replace with `Client::builder().connect_timeout(CONNECT_TIMEOUT).build()`. Also update the import from `use super::DEFAULT_TIMEOUT;` to `use super::CONNECT_TIMEOUT;`.

Search for `DEFAULT_TIMEOUT` in tests.rs and replace each occurrence. The import line (around line 11) should change from:

```rust
use super::{..., DEFAULT_TIMEOUT, ...};
```

to:

```rust
use super::{..., CONNECT_TIMEOUT, ...};
```

And each `Client::builder().timeout(DEFAULT_TIMEOUT).build()` becomes `Client::builder().connect_timeout(CONNECT_TIMEOUT).build()`.

- [ ] **Step 7: Run tests**

```bash
cargo test -p presenter-server -- resolume --nocapture
```

Expected: All tests pass. The wiremock server responds instantly, so timeouts don't affect test behavior.

- [ ] **Step 8: Commit**

```bash
cargo fmt --all
git add crates/presenter-server/src/resolume/mod.rs crates/presenter-server/src/resolume/driver.rs crates/presenter-server/src/resolume/tests.rs
git commit -m "fix(resolume): split 500ms timeout into per-operation timeouts (#213)

Replace single 500ms client timeout with: 3s connect, 5s composition
fetch, 2s clip trigger/text update. The aggressive 500ms timeout
caused permanent connection failures when Arena responded slowly
under rendering load."
```

---

## Task 3: Fix Silent Command Drops

**Files:**
- Modify: `crates/presenter-server/src/resolume/mod.rs:119-208`

- [ ] **Step 1: Make RefreshConfig and Shutdown use blocking send**

In `crates/presenter-server/src/resolume/mod.rs`, replace the `set_hosts` method (lines 119-157). Change `try_send` to `send().await` for `Shutdown` and `RefreshConfig`:

```rust
    pub async fn set_hosts(&self, hosts: Vec<ResolumeHost>) {
        let mut guard = self.hosts.write().await;
        let mut desired: HashMap<ResolumeHostId, ResolumeHost> =
            hosts.into_iter().map(|host| (host.id, host)).collect();

        // Stop hosts that no longer exist
        let existing_ids: Vec<_> = guard.keys().copied().collect();
        for id in existing_ids {
            if !desired.contains_key(&id) {
                if let Some(entry) = guard.remove(&id) {
                    let _ = entry.command_tx.send(HostCommand::Shutdown).await;
                    entry.handle.abort();
                }
            }
        }

        // Add or update entries
        for (id, host) in desired.drain() {
            match guard.get_mut(&id) {
                Some(entry) => {
                    if entry.config.host != host.host
                        || entry.config.port != host.port
                        || entry.config.is_enabled != host.is_enabled
                    {
                        let _ = entry
                            .command_tx
                            .send(HostCommand::RefreshConfig(host.clone()))
                            .await;
                        entry.config = host;
                    } else if entry.config.label != host.label {
                        entry.config = host;
                    }
                }
                None => {
                    let entry = self.spawn_host(host);
                    guard.insert(id, entry);
                }
            }
        }
    }
```

- [ ] **Step 2: Add warning logs for dropped data commands**

In `crates/presenter-server/src/resolume/mod.rs`, replace the `stage_update`, `bible_update`, and `timer_update` methods (lines 188-208):

```rust
    pub async fn stage_update(&self, update: StageUpdate) {
        for (id, entry) in self.hosts.read().await.iter() {
            if entry
                .command_tx
                .try_send(HostCommand::Stage(update.clone()))
                .is_err()
            {
                tracing::warn!(host_id = %id, "resolume command channel full, dropping stage update");
            }
        }
    }

    pub async fn bible_update(&self, update: BibleUpdate) {
        for (id, entry) in self.hosts.read().await.iter() {
            if entry
                .command_tx
                .try_send(HostCommand::Bible(update.clone()))
                .is_err()
            {
                tracing::warn!(host_id = %id, "resolume command channel full, dropping bible update");
            }
        }
    }

    pub async fn timer_update(&self, frame: TimerFrame) {
        for (id, entry) in self.hosts.read().await.iter() {
            if entry
                .command_tx
                .try_send(HostCommand::Timer(frame.clone()))
                .is_err()
            {
                tracing::warn!(host_id = %id, "resolume command channel full, dropping timer update");
            }
        }
    }
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p presenter-server -- resolume --nocapture
```

- [ ] **Step 4: Commit**

```bash
cargo fmt --all
git add crates/presenter-server/src/resolume/mod.rs
git commit -m "fix(resolume): guarantee delivery of config commands (#213)

Use blocking send().await for RefreshConfig and Shutdown commands
so they cannot be silently dropped when the channel is full. Add
warning logs when data commands (stage/bible/timer) are dropped."
```

---

## Task 4: Add Test Connection Endpoint

**Files:**
- Modify: `crates/presenter-server/src/router/integrations/resolume.rs`
- Modify: `crates/presenter-server/src/router.rs:157-161`
- Modify: `crates/presenter-server/src/state/integrations.rs`

- [ ] **Step 1: Add test_resolume_host_connection to AppState**

In `crates/presenter-server/src/state/integrations.rs`, add after the `resolume_status_for` method (after line 27):

```rust
    pub async fn test_resolume_host_connection(
        &self,
        id: ResolumeHostId,
    ) -> anyhow::Result<crate::resolume::TestConnectionResult> {
        let host = self
            .repository
            .list_resolume_hosts()
            .await?
            .into_iter()
            .find(|h| h.id == id)
            .ok_or_else(|| anyhow::anyhow!("Resolume host not found"))?;
        crate::resolume::test_connection(&host).await
    }
```

- [ ] **Step 2: Add test_connection function and TestConnectionResult**

In `crates/presenter-server/src/resolume/mod.rs`, add before the `#[cfg(test)]` line (before line 227):

```rust
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestConnectionResult {
    pub success: bool,
    pub latency_ms: Option<f64>,
    pub error: Option<String>,
}

pub async fn test_connection(host: &ResolumeHost) -> anyhow::Result<TestConnectionResult> {
    use std::time::Instant;

    let client = Client::builder()
        .connect_timeout(Duration::from_secs(3))
        .build()
        .map_err(|e| anyhow!("failed to build test client: {e}"))?;

    let url = format!("http://{}:{}/api/v1/composition", host.host, host.port);
    let start = Instant::now();
    match client
        .get(&url)
        .timeout(Duration::from_secs(5))
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => Ok(TestConnectionResult {
            success: true,
            latency_ms: Some(start.elapsed().as_secs_f64() * 1000.0),
            error: None,
        }),
        Ok(response) => Ok(TestConnectionResult {
            success: false,
            latency_ms: Some(start.elapsed().as_secs_f64() * 1000.0),
            error: Some(format!("HTTP {}", response.status())),
        }),
        Err(err) => Ok(TestConnectionResult {
            success: false,
            latency_ms: None,
            error: Some(err.to_string()),
        }),
    }
}
```

- [ ] **Step 3: Add the router endpoint**

In `crates/presenter-server/src/router/integrations/resolume.rs`, add at the end of the file (before the closing):

```rust
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TestConnectionResponse {
    success: bool,
    latency_ms: Option<f64>,
    error: Option<String>,
}

#[instrument(skip_all)]
pub(crate) async fn test_resolume_host(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<TestConnectionResponse>, AppError> {
    let result = state
        .test_resolume_host_connection(ResolumeHostId::from_uuid(id))
        .await?;
    Ok(Json(TestConnectionResponse {
        success: result.success,
        latency_ms: result.latency_ms,
        error: result.error,
    }))
}
```

- [ ] **Step 4: Register the route**

In `crates/presenter-server/src/router.rs`, add a new route after the existing Resolume host routes (after line 161):

```rust
        .route(
            "/integrations/resolume/hosts/{id}/test",
            post(integrations::resolume::test_resolume_host),
        )
```

- [ ] **Step 5: Run tests and clippy**

```bash
cargo test -p presenter-server -- resolume --nocapture
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
```

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/presenter-server/src/resolume/mod.rs crates/presenter-server/src/router/integrations/resolume.rs crates/presenter-server/src/router.rs crates/presenter-server/src/state/integrations.rs
git commit -m "feat(resolume): add test connection endpoint (#213)

POST /integrations/resolume/hosts/{id}/test performs a one-shot
composition fetch bypassing the background worker channel. Returns
success status, latency, and error message."
```

---

## Task 5: Update Settings UI with Diagnostics

**Files:**
- Modify: `crates/presenter-server/src/settings_script.js:614-658`
- Modify: `crates/presenter-server/src/ui/settings.rs:296-320`

- [ ] **Step 1: Update JavaScript host rendering with diagnostics**

In `crates/presenter-server/src/settings_script.js`, replace the `renderHosts()` function's list item template (the `const items = state.hosts.map(...)` block, approximately lines 620-658). The key changes are:
1. Show retry count and error duration when in error state
2. Add "Test Connection" button

Replace the `const items = state.hosts.map((host) => {` block with:

```javascript
      const items = state.hosts
        .map((host) => {
          const statusObj = host.status || {};
          const stateLabel = (statusObj.state || host.statusState || (host.isEnabled ? 'connecting' : 'disabled')).toLowerCase();
          const statusLabel = stateLabel.charAt(0).toUpperCase() + stateLabel.slice(1);
          const normalizedState = (stateLabel || 'disabled').toLowerCase();
          const statusClass = `settings__status settings__status--${normalizedState}`;
          const updated = formatDate(host.updatedAtDisplay || host.updatedAt);
          const created = formatDate(host.createdAtDisplay || host.createdAt);
          const latencySource = statusObj.lastLatencyMs ?? host.lastLatencyMs;
          const latency = typeof latencySource === 'number'
            ? `${latencySource.toFixed(1)} ms`
            : '—';

          // Diagnostics: error detail with retry info
          let statusDetail = '';
          const errorMessage = statusObj.lastError || host.statusMessage;
          if (errorMessage && normalizedState === 'error') {
            const failures = statusObj.consecutiveFailures || 0;
            const errorSince = statusObj.errorSince;
            let sinceText = '';
            if (errorSince) {
              try { sinceText = ` since ${formatDate(errorSince)}`; } catch (_) {}
            }
            statusDetail = `<p class="settings__list-meta settings__list-meta--warning" data-role="host-error-detail">⚠ Retrying\u2026 (${failures} failure${failures !== 1 ? 's' : ''}${sinceText})</p>`;
          } else if (errorMessage) {
            statusDetail = `<p class="settings__list-meta settings__list-meta--warning">⚠ ${errorMessage}</p>`;
          }

          return `
<li class="settings__list-item" data-id="${host.id}" data-enabled="${host.isEnabled}">
  <div class="settings__list-primary">
    <div class="settings__list-title">
      <span class="settings__host-label">${host.label}</span>
      <span class="${statusClass}">${statusLabel}</span>
    </div>
    <p class="settings__list-line"><code>${host.host}</code><span class="settings__host-port">:${host.port}</span></p>
    <p class="settings__list-meta">Updated ${updated}</p>
    <p class="settings__list-meta">Created ${created}</p>
    <p class="settings__list-meta">Latency ${latency}</p>
    ${statusDetail}
  </div>
  <div class="settings__list-actions">
    <button type="button" class="settings__button settings__button--ghost" data-role="host-test" data-id="${host.id}">Test</button>
    <button type="button" class="settings__button settings__button--ghost" data-role="host-edit" data-id="${host.id}">Edit</button>
    <button type="button" class="settings__button settings__button--danger" data-role="host-delete" data-id="${host.id}">Delete</button>
  </div>
</li>`;
        })
        .join('');
```

- [ ] **Step 2: Add Test Connection click handler**

In `crates/presenter-server/src/settings_script.js`, find the event delegation section where `host-edit` and `host-delete` clicks are handled (search for `data-role="host-edit"` or `host-delete`). Add a handler for the `host-test` button in the same delegation block:

```javascript
    if (role === 'host-test') {
      const id = target.dataset.id;
      if (!id) return;
      target.disabled = true;
      target.textContent = 'Testing…';
      try {
        const resp = await fetch(`/integrations/resolume/hosts/${id}/test`, { method: 'POST' });
        const result = await resp.json();
        if (result.success) {
          showToast(`Connection OK (${result.latencyMs.toFixed(1)} ms)`);
        } else {
          showToast(`Connection failed: ${result.error || 'unknown error'}`);
        }
      } catch (err) {
        showToast(`Test failed: ${err.message}`);
      } finally {
        target.disabled = false;
        target.textContent = 'Test';
        await refreshHostList();
      }
    }
```

- [ ] **Step 3: Update Leptos SSR rendering for new snapshot fields**

In `crates/presenter-server/src/ui/settings.rs`, update the `SettingsHostRow` struct to include new diagnostic display fields. Find where `status_message` is set from the snapshot (around line 764-780) and ensure the new fields from `ResolumeConnectionSnapshot` flow through to the JavaScript. The `status` field already carries the full `ResolumeConnectionSnapshot`, so the JavaScript `statusObj.consecutiveFailures` and `statusObj.errorSince` will work automatically from the serialized snapshot.

Verify that the `status: Option<ResolumeConnectionSnapshot>` field in `SettingsHostRow` is populated. It is (based on exploration). No Leptos SSR changes are needed — the JavaScript reads from `host.status` which already contains the full snapshot with the new fields.

- [ ] **Step 4: Commit**

```bash
cargo fmt --all
git add crates/presenter-server/src/settings_script.js crates/presenter-server/src/ui/settings.rs
git commit -m "feat(resolume): add connection diagnostics and test button to UI (#213)

Settings page now shows retry count and error duration when a
Resolume host is in error state. Added Test Connection button that
performs a one-shot composition fetch bypassing the worker channel."
```

---

## Task 6: E2E Test

**Files:**
- Modify: `tests/e2e/settings.spec.ts`

- [ ] **Step 1: Add E2E test for diagnostics and test connection**

In `tests/e2e/settings.spec.ts`, add a new test after the existing `'resolume settings CRUD with status feedback'` test (after line 192):

```typescript
test('resolume connection diagnostics and test button', async ({ page }) => {
  const consoleMessages: string[] = [];
  page.on('console', (msg) => {
    if (msg.type() === 'error' || msg.type() === 'warning') {
      consoleMessages.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  if (!mockResolume) {
    throw new Error('Mock Resolume server not started');
  }

  const mockHost = '127.0.0.1';
  const mockPort = String(mockResolume.port);

  await page.goto(new URL('/ui/settings', baseURL).toString());
  await page.waitForLoadState('networkidle');

  // Create a connection pointing at mock Resolume
  const testLabel = `Diag Test ${Date.now()}`;
  await page.fill(selectors.labelInput, testLabel);
  await page.fill(selectors.hostInput, mockHost);
  await page.fill(selectors.portInput, mockPort);
  await page.check(selectors.enabledCheckbox);
  await page.click(selectors.submitButton);
  await waitForToast(page, 'Added Resolume connection.');

  // Wait for connected state
  await expect.poll(async () => {
    const hosts = await getHostsViaApi(page);
    return hosts[0]?.status.state;
  }, { timeout: 30_000 }).toEqual('connected');

  // Test Connection button should work
  const hostsAfter = await getHostsViaApi(page);
  const hostId = hostsAfter[0].id;

  // Click test button
  const testBtn = page.locator(`[data-role="host-test"][data-id="${hostId}"]`);
  await expect(testBtn).toBeVisible({ timeout: 10_000 });
  await testBtn.click();
  await waitForToast(page, /Connection OK/);

  // Take connection offline — verify diagnostics appear
  mockResolume.setOnline(false);

  // Wait for error state with consecutive failures
  await expect.poll(async () => {
    const hosts = await getHostsViaApi(page);
    const status = hosts[0]?.status;
    return status?.state === 'error' && (status?.consecutiveFailures ?? 0) > 0;
  }, { timeout: 30_000 }).toBeTruthy();

  // Verify the error detail is displayed in UI
  await page.reload();
  await page.waitForLoadState('networkidle');
  const errorDetail = page.locator('[data-role="host-error-detail"]');
  await expect(errorDetail).toContainText('Retrying', { timeout: 20_000 });
  await expect(errorDetail).toContainText('failure', { timeout: 5_000 });

  // Test connection while offline should fail
  const testBtnAfterReload = page.locator(`[data-role="host-test"][data-id="${hostId}"]`);
  await testBtnAfterReload.click();
  await waitForToast(page, /Connection failed/);

  // Bring back online — should recover
  mockResolume.setOnline(true);
  await expect.poll(async () => {
    const hosts = await getHostsViaApi(page);
    return hosts[0]?.status.state;
  }, { timeout: 30_000 }).toEqual('connected');

  // Verify diagnostics reset
  await expect.poll(async () => {
    const hosts = await getHostsViaApi(page);
    return hosts[0]?.status.consecutiveFailures;
  }, { timeout: 10_000 }).toEqual(0);

  // Clean up — delete the host
  page.once('dialog', (dialog) => dialog.accept());
  await page.locator(`[data-role="host-delete"][data-id="${hostId}"]`).click();
  await waitForToast(page, 'Deleted Resolume connection.');

  // Clean console check
  expect(consoleMessages).toEqual([]);
});
```

- [ ] **Step 2: Run E2E test locally**

```bash
npm run test:playwright -- settings
```

Expected: Both settings tests pass.

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/settings.spec.ts
git commit -m "test(e2e): add resolume connection diagnostics E2E test (#213)

Verifies: test connection button (online and offline), error
diagnostics display with retry count, recovery after reconnect,
and diagnostic reset on recovery."
```

---

## Task 7: Version Bump, Format Check, Push, Monitor CI

- [ ] **Step 1: Bump version**

```bash
# Check current versions
git fetch origin
grep '^version' Cargo.toml | head -1
```

Bump the patch version in `Cargo.toml` workspace `[workspace.package].version`.

- [ ] **Step 2: Commit version bump**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore: bump version to X.Y.Z"
```

- [ ] **Step 3: Run local checks**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
cargo test -p presenter-server -- resolume --nocapture
```

Fix any issues in ONE commit if needed.

- [ ] **Step 4: Push and monitor CI**

```bash
git push origin dev
gh run list --branch dev --limit 3
```

Monitor until ALL jobs complete. If any fail, `gh run view <run-id> --log-failed`, fix ALL issues in ONE commit, push again.

---

## Verification Summary

| Check | How to verify |
|-------|---------------|
| Timeout split works | Composition fetch uses 5s timeout, clip trigger uses 2s |
| Config updates delivered | Change port in settings → verify worker uses new port (E2E test covers this via existing CRUD test) |
| Diagnostics display | Settings page shows "Retrying... (N failures since HH:MM)" when connection is down |
| Test Connection button | Click "Test" → shows latency on success, error message on failure |
| Recovery works | Take mock offline → error state → bring online → connected state with reset diagnostics |
| No regressions | All existing resolume tests and settings E2E test still pass |
| Clean console | No browser console errors or warnings |
