# Android Stage Launcher Resilience (#245)

**Status:** Proposed
**Date:** 2026-04-14
**Issue:** #245

## Problem

Operators report that Android TV stage displays (`sd1l.lan` through `sd4l.lan`) boot to the Google TV home screen instead of the Fully Kiosk stage display app. The user "had it added on production" previously, but no backup from the last 3 days (prod + dev, 10 files total) contains a single row in `android_stage_displays`. The table exists from the initial migration, but it's empty.

Diagnostics on 2026-04-14:
- All 4 TVs are reachable on TCP port 5555.
- Manual `adb connect sd1l.lan:5555 && adb shell am start -n com.fullykiosk.videokiosk/de.ozerov.fully.MainActivity` launches the kiosk and the activity foregrounds on the TV.
- Adding a display via the Settings UI on dev persists correctly: the row is created, the launcher worker fires within 130ms, `adb connect` + `am start` both succeed, and `dumpsys activity activities` confirms `FullyActivity` is resumed.
- End-to-end the existing code works. The observable "bug" is that the table is empty.

The feature is silent about its emptiness: the UI shows `0 Displays` with a fallback message, but no alert, log, or onboarding prompt nudges the operator to add them. If a migration or deploy event ever wipes the table, the feature appears broken with no signal.

## Goals

- The 4 known stage displays on the production LAN get re-added and stay added.
- The launcher is more resilient to ADB's well-known stale-device behavior after TV power cycles.
- Operators can verify a display works without waiting for the 20-second retry tick.

Non-goals:
- mDNS or Bonjour auto-discovery of Android TVs.
- Dynamic registration (e.g., "any TV that connects once gets auto-added"). Too much magic for too little value.
- Rewriting the retry loop. The 20s cadence is fine.

## Architecture

Three surgical changes:

### 1. One-time seed migration

New incremental migration `m20260414_000002_seed_android_stage_displays`. In `up()`:

```sql
SELECT COUNT(*) FROM android_stage_displays
```

If the count is `0`, insert 4 rows:

| label      | host       | port | launch_component                                              | is_enabled |
|------------|------------|------|---------------------------------------------------------------|------------|
| Stage SD1  | sd1l.lan   | 5555 | com.fullykiosk.videokiosk/de.ozerov.fully.MainActivity        | 1          |
| Stage SD2  | sd2l.lan   | 5555 | com.fullykiosk.videokiosk/de.ozerov.fully.MainActivity        | 1          |
| Stage SD3  | sd3l.lan   | 5555 | com.fullykiosk.videokiosk/de.ozerov.fully.MainActivity        | 1          |
| Stage SD4  | sd4l.lan   | 5555 | com.fullykiosk.videokiosk/de.ozerov.fully.MainActivity        | 1          |

`created_at` / `updated_at` = current epoch millis. `id` = a UUID generated per row.

**Crucially:** the seed only runs when the table is empty. If an operator later deletes all 4 displays, a subsequent deploy does NOT re-seed — the idempotency guard (`COUNT(*) = 0`) is what makes this safe. The migration still runs exactly once per SeaORM's migration tracking, but the INSERT path inside is guarded so it never overwrites operator intent.

`down()` is a no-op. We don't want to delete operator data on rollback.

### 2. `adb disconnect` before `adb connect` in the launcher worker

In `crates/presenter-server/src/android_stage.rs::connect_and_launch`, before the existing `adb connect` call:

```rust
// Clear any stale offline device entry from a previous attempt.
// ADB leaves stale entries after TV power cycles which then cause
// subsequent `-s serial` commands to fail until the daemon is restarted.
let _ = timeout(
    ADB_COMMAND_TIMEOUT,
    Command::new(adb_bin).arg("disconnect").arg(&serial).output(),
)
.await;
```

Errors from `disconnect` are intentionally ignored (the expected case is "not connected", which returns non-zero).

### 3. "Test launch" button in Settings UI

New endpoint `POST /integrations/android-stage/displays/{id}/launch-now`. Sends `DeviceCommand::LaunchNow` to the worker's command channel (already wired up). Returns `204 No Content` on enqueue. The response does NOT wait for the launch to complete — the UI polls the existing status snapshot for state transitions.

New "Test" button in `crates/presenter-server/src/ui/components/settings.rs` on each display row. Click → POST → refresh status after 2 seconds → show updated state.

## File Structure

| File | Change |
|------|--------|
| `crates/presenter-migration/src/m20260414_000002_seed_android_stage_displays.rs` | **New** — one-time seed |
| `crates/presenter-migration/src/lib.rs` | Register new migration |
| `crates/presenter-server/src/android_stage.rs` | Add `adb disconnect` before `adb connect` in `connect_and_launch` |
| `crates/presenter-server/src/router/integrations/android_stage.rs` | Add `launch_now_android_stage_display` handler |
| `crates/presenter-server/src/router.rs` | Register POST route |
| `crates/presenter-server/src/state/integrations.rs` | Add `launch_now_android_stage_display` method that sends `DeviceCommand::LaunchNow` to the registry |
| `crates/presenter-server/src/android_stage.rs` | Add public `launch_now` method on `AndroidStageRegistry` |
| `crates/presenter-server/src/ui/components/settings.rs` | Add "Test" button + click handler per display row |

## Data Flow

```
Deploy → migration runs → if android_stage_displays empty → insert 4 rows
Server boot → state::sync_android_stage_displays → registry.set_displays
Registry → spawn worker per display → ticker every 20s
Worker tick → adb disconnect → adb connect → am start → state = Running
Operator clicks Test → POST /launch-now → state.launch_now → registry.launch_now →
  tx.send(DeviceCommand::LaunchNow) → worker picks up → runs connect_and_launch immediately
```

## Error Handling

- Seed migration failure: rolls back the transaction, deploy fails loudly. Expected 0% failure rate on a fresh or unchanged table.
- `adb disconnect` failure: silently ignored. Expected case (device was never connected).
- `adb connect` failure: existing behavior — record_error, worker retries next tick.
- `launch-now` with unknown id: 404.
- `launch-now` against a disabled display: returns 409 Conflict with a clear message, because the worker won't process `LaunchNow` when `!is_enabled`.

## Testing

### Unit tests (presenter-persistence / presenter-server)

- Existing android stage display CRUD tests already cover the schema layer — not touched.
- New test: `seed_inserts_four_displays_on_empty_table` — spin up an in-memory DB, run migrations, assert row count = 4 and all hosts match `sd1l..sd4l`.
- New test: `seed_does_not_overwrite_existing_rows` — run migrations on a fresh in-memory DB (which triggers the seed and lands 4 rows). Manually insert a 5th display with a distinct label via the repository API. Invoke the seed migration's `up()` a second time directly (`Migration.up(&schema_manager).await`). Assert: row count stays at 5 (not 9), the custom row survives, no duplicate seed rows are inserted. This validates the `COUNT(*) = 0` guard.
- New test: `launch_now_enqueues_command` — create a display, call `state.launch_now_android_stage_display(id)`, assert the mock worker received `DeviceCommand::LaunchNow`.

### Manual verification on dev (after deploy)

1. `curl http://10.77.8.134:8080/integrations/android-stage/displays` → expect 4 rows with `sd1l..sd4l`.
2. Click Test on sd1l → watch `lastAttempt` bump and state transition to `running`.
3. SSH to sd1l and run `dumpsys activity activities | grep mResumedActivity` → expect `FullyActivity`.
4. Power-cycle sd1l via `adb reboot`, wait for it to come back, observe state recovery (should re-run within ~20s without operator intervention).

### Manual verification on prod (after dev is green)

Same 4 checks against `http://10.77.9.205`.

## Open Questions

None.

## Future Work (out of scope)

- Last-success age indicator in the Settings UI (e.g., "launched 3s ago" vs "launched 2h ago") — nice to have, not required for this fix.
- Per-display custom launch component — current default works for all 4 TVs.
- Automatic recovery if the TV is in a stuck app state (e.g., `am kill` + `am start` combo).
