# Arena Connection Reliability Design

**Issue:** #213 — Connections to Arena not reliable
**Date:** 2026-04-08

## Problem

The Resolume Arena integration fails to reconnect after a transient network error, even though Arena is reachable from a browser. The operator reported: "failed to fetch composition from http://10.77.8.201:8090/api/v1/composition and there was no way to make it work again. I tried changing the port, removing and adding it again, nothing helped."

## Root Causes

Three bugs/weaknesses in the current implementation:

### 1. HTTP timeout too aggressive (500ms)

The reqwest `Client` in `ResolumeRegistry::new()` uses a single 500ms timeout for all operations. This is the total timeout including DNS resolution, TCP connect, TLS (if any), request send, and response read. Composition responses (full Arena state JSON) can be large and Arena may respond slowly under rendering load. A browser uses ~30 seconds; presenter's 500ms causes consistent failures when Arena is slightly slow.

**Location:** `crates/presenter-server/src/resolume/mod.rs:18` — `DEFAULT_TIMEOUT = 500ms`

### 2. Config updates silently dropped

When the operator changes a host's port/IP in the settings UI, `set_hosts()` sends `RefreshConfig` via `try_send()` on a 16-slot bounded channel. If the channel is full (e.g., queued stage/timer updates), the config change is silently discarded (`let _ = ...`). The worker continues using the old config. The operator sees "saved" but the connection doesn't actually update.

**Location:** `crates/presenter-server/src/resolume/mod.rs:143-144` — `try_send(RefreshConfig)` with `let _`

### 3. No connection diagnostics

The UI shows only the error message and connection state (disabled/connecting/connected/error). The operator cannot see:
- Whether the system is actively retrying
- How many consecutive failures have occurred
- When the last attempt was made
- How long the error state has persisted

This makes it impossible to distinguish "retrying and will recover" from "permanently stuck."

## Design

### Fix 1: Split HTTP timeouts by operation type

Remove the single 500ms client-level timeout. Instead:

- **Client-level connect timeout:** 3 seconds (`Client::builder().connect_timeout(3s)`)
- **Client-level default timeout:** None (controlled per-request)
- **Composition fetch:** 5-second per-request timeout (infrequent, large response)
- **Clip trigger / text update:** 2-second per-request timeout (frequent during live service)

Apply per-request timeouts using reqwest's `RequestBuilder::timeout()` method.

**Constants to add in `driver.rs`:**
```rust
const COMPOSITION_TIMEOUT: Duration = Duration::from_secs(5);
const ACTION_TIMEOUT: Duration = Duration::from_secs(2);
```

**Modify `mod.rs`:**
```rust
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

// In ResolumeRegistry::new():
let client = Client::builder()
    .connect_timeout(CONNECT_TIMEOUT)
    .build()?;
```

**Modify `driver.rs` request sites:**
- `refresh_mapping()`: add `.timeout(COMPOSITION_TIMEOUT)` before `.send()`
- `update_clip_text()`: add `.timeout(ACTION_TIMEOUT)` before `.send()`
- `trigger_clips()`: add `.timeout(ACTION_TIMEOUT)` before `.send()`

### Fix 2: Guaranteed delivery for config commands

Change `RefreshConfig` and `Shutdown` to use `send().await` (async blocking) instead of `try_send()`. These are rare, critical commands that must not be dropped.

For `Stage`, `Bible`, and `Timer` commands, keep `try_send` but log a warning when the channel is full:

```rust
if entry.command_tx.try_send(HostCommand::Stage(update.clone())).is_err() {
    tracing::warn!(host_id = %id, "resolume command channel full, dropping stage update");
}
```

**Changes in `mod.rs`:**
- `set_hosts()` line 143-144: change `let _ = entry.command_tx.try_send(RefreshConfig(...))` to `let _ = entry.command_tx.send(RefreshConfig(...)).await`
- `set_hosts()` line 129: change `let _ = entry.command_tx.try_send(Shutdown)` to `let _ = entry.command_tx.send(Shutdown).await`
- `stage_update()`, `bible_update()`, `timer_update()`: add warning log on `try_send` failure

### Fix 3: Connection diagnostics

**Extend `ResolumeConnectionSnapshot` with new fields:**

```rust
pub struct ResolumeConnectionSnapshot {
    pub state: ResolumeConnectionState,
    pub last_success: Option<DateTime<Utc>>,
    pub last_latency_ms: Option<f64>,
    pub last_error: Option<String>,
    // New fields:
    pub consecutive_failures: u32,
    pub last_attempt: Option<DateTime<Utc>>,
    pub error_since: Option<DateTime<Utc>>,
}
```

**Update `record_error()` in `driver.rs`:**
- Increment `consecutive_failures`
- Set `last_attempt` to now
- Set `error_since` to now only if currently not in Error state (preserve original error time)

**Update `mark_connected()` in `driver.rs`:**
- Reset `consecutive_failures` to 0
- Set `last_attempt` to now
- Clear `error_since`

**Add "Test Connection" API endpoint:**

`POST /integrations/resolume/hosts/{id}/test` — performs a one-shot composition fetch against the host and returns the result (success with latency, or error message). This does NOT go through the background worker; it creates a temporary HTTP request directly to avoid channel issues.

**UI changes in settings page:**

In the existing Resolume host status area (`ui/settings.rs` and `settings_script.js`):

- When in Error state, show: `Retrying... (N failures since HH:MM:SS)` instead of just the error message
- Show last attempt timestamp: `Last attempt: HH:MM:SS`
- Add "Test Connection" button that calls the test endpoint and shows result inline
- When Connected, show: `Connected (latency: Xms)`

## Testing

### Unit tests (Rust)

- Test `record_error` increments `consecutive_failures` and sets `error_since`
- Test `mark_connected` resets `consecutive_failures` and clears `error_since`
- Test `error_since` is preserved across multiple `record_error` calls (not reset each time)

### E2E test (Playwright)

- Navigate to settings page
- Verify Resolume host status displays correctly for configured hosts
- Click "Test Connection" button, verify result appears
- Verify error state shows retry count and error duration

### Integration considerations

- The test endpoint uses a fresh HTTP request, not the worker channel, so it works even if the channel is full
- Existing tests for `refresh_mapping`, `trigger_clips`, etc. must be updated to account for per-request timeouts (reqwest test mocks should still work since timeouts are just request decorators)

## Out of scope

- Connection health dashboard with latency graphs (issue #213 is about reliability, not monitoring)
- Auto-reconnect notifications (sound/popup when connection recovers)
- Retry backoff (the 10-second interval is already reasonable; exponential backoff would delay recovery)
