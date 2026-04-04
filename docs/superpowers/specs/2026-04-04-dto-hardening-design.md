# DTO Hardening & Ableton Button Fix — Design Spec

## Problem

The Ableton ON/OFF and Follow ON/OFF buttons on the operator UI have been broken since the WASM migration. CI is green despite this because no E2E test clicks the actual buttons.

### Root Cause

`AbleSetStatusSnapshot` in `crates/presenter-server/src/ableset.rs:66` is the only response DTO (out of 65) missing `#[serde(rename_all = "camelCase")]`. The server serializes field names as snake_case (`follow_enabled`, `http_port`, `osc_port`, `library_name`, `song_prefix_length`, `last_song`, `last_error`), but the client DTO in `crates/presenter-ui/src/api/settings.rs:7` has `#[serde(rename_all = "camelCase")]` and expects `followEnabled`, `httpPort`, etc.

Deserialization fails entirely. The error is swallowed by `if let Ok(status) = ...` in `operator.rs:420`, so `ableset_status` stays `None`. Both buttons default to "OFF" regardless of actual server state. Clicks toggle based on `None` (always reads as `false`), and the response after toggling also fails to deserialize, so the UI never updates.

### Why CI Didn't Catch It

1. `ableton-osc.spec.ts` — tests AbleSet tracking via direct `fetch()` API calls, never clicks operator UI buttons
2. `settings.spec.ts` — tests settings page checkbox (`[data-role="ableset-enabled"]`), not operator header buttons (`[data-role="ableset-enable"]`)
3. No test ever verifies that clicking the operator Ableton/Follow buttons changes their state
4. No contract test for `AbleSetStatusSnapshot` JSON roundtrip — the type isn't in `presenter-core`

## Solution

Three layers of defense to fix the bug and prevent this class of issue from recurring.

### Layer 1: Move Shared Types to presenter-core

Move `AbleSetStatusSnapshot` and `FeatureFlags` from their duplicate definitions into `presenter-core`, making them single-source-of-truth types with both `Serialize` and `Deserialize` + `camelCase`.

**Types to move:**

| Type | From (server) | From (client) | To |
|------|--------------|---------------|-----|
| `AbleSetStatusSnapshot` | `crates/presenter-server/src/ableset.rs:66` | `crates/presenter-ui/src/api/settings.rs:7` | `crates/presenter-core/src/ableset.rs` |
| `FeatureFlags` (client) / `FeatureSettingsResponse` (server) | `crates/presenter-server/src/router/features.rs:10` | `crates/presenter-ui/src/api/settings.rs:49` | `crates/presenter-core/src/feature_flags.rs` (new, named `FeatureFlags`) |

After moving:
- Server imports and serializes the shared type
- Client imports and deserializes the same type
- Serde attributes are defined once — mismatch impossible

**Fix serde attributes on server-internal types:**

| Type | File | Fix |
|------|------|-----|
| `BibleImportSummaryDto` | `crates/presenter-server/src/router/bible.rs:20` | Add `#[serde(rename_all = "camelCase")]` |
| `FavoriteLibraryIdsResponse` | `crates/presenter-server/src/router/libraries.rs:14` | Add `#[serde(rename_all = "camelCase")]` |

### Layer 2: Contract Tests

Add JSON roundtrip contract tests in `crates/presenter-core/src/contract_tests.rs` for every type moved to `presenter-core`:

- `AbleSetStatusSnapshot` — serialize, verify JSON field names are camelCase, deserialize back, assert equality
- `FeatureFlags` — same pattern

These tests run on every `cargo test` and catch serde attribute mismatches before code reaches CI.

### Layer 3: E2E Test — Operator Control Buttons

New file: `tests/e2e/operator-controls.spec.ts`

Tests:
1. **Ableton ON/OFF button toggle**: Enable Ableton via API → navigate to operator → verify button shows "Ableton ON" with `data-state="on"` → click button → verify text changes to "Ableton OFF" with `data-state="off"` → verify API confirms `enabled: false`
2. **Follow ON/OFF button toggle**: Enable Ableton + Follow via API → verify "Follow ON" shown → click → verify "Follow OFF" → verify API confirms `follow_enabled: false`
3. **State after page reload**: Set state via API → reload page → verify buttons reflect correct state
4. **Follow resets when Ableton disables**: Enable both → click Ableton OFF → verify Follow also shows "OFF"
5. **Zero console errors/warnings**: Every test asserts clean browser console

### Layer 4: Serde Convention CI Lint

New script: `scripts/check-serde-conventions.sh`

Checks all Rust files in `crates/presenter-server/src/router/` and `crates/presenter-server/src/` for structs that derive `Serialize` but lack `rename_all`. Allowlist for intentional exceptions (e.g., enums using `snake_case`). Runs in CI pipeline as a quality gate.

## Files Changed

### Modified
- `crates/presenter-core/src/ableset.rs` — add `AbleSetStatusSnapshot` with `Serialize + Deserialize + camelCase`
- `crates/presenter-core/src/lib.rs` — add `feature_flags` module, re-export new types
- `crates/presenter-core/src/contract_tests.rs` — add roundtrip tests for `AbleSetStatusSnapshot`, `FeatureFlags`
- `crates/presenter-server/src/ableset.rs` — remove local `AbleSetStatusSnapshot`, import from `presenter-core`
- `crates/presenter-server/src/router/features.rs` — remove local `FeatureSettingsResponse`, import `FeatureFlags` from `presenter-core`
- `crates/presenter-server/src/router/bible.rs` — add `camelCase` to `BibleImportSummaryDto`
- `crates/presenter-server/src/router/libraries.rs` — add `camelCase` to `FavoriteLibraryIdsResponse`
- `crates/presenter-server/src/router/tests.rs` — update any test assertions affected by field name changes
- `crates/presenter-ui/src/api/settings.rs` — remove local `AbleSetStatusSnapshot` and `FeatureFlags`, import from `presenter-core`
- `.github/workflows/pipeline.yml` — add serde convention lint step

### Created
- `crates/presenter-core/src/feature_flags.rs` — `FeatureFlags` shared type
- `tests/e2e/operator-controls.spec.ts` — operator button E2E tests
- `scripts/check-serde-conventions.sh` — serde attribute lint script

## Success Criteria

1. Ableton ON/OFF and Follow ON/OFF buttons work on operator UI (verified by E2E test clicking actual buttons)
2. All existing tests pass (no regressions)
3. Contract tests prevent future serde mismatches for shared types
4. CI lint prevents future response DTOs from missing `camelCase`
5. Zero console errors in browser when interacting with operator controls
