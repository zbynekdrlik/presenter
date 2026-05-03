# API Stage Layout Gate — Design

**Date:** 2026-05-03
**Status:** Proposed
**Scope:** Backend (`presenter-server`) — server-side gate on the api-stage update path
**Issue:** [#281](https://github.com/zbynekdrlik/presenter/issues/281) — `PUT /api/stage` switches operator preview even when stage layout ≠ `api`

## Goal

Stop `PUT /api/stage` from leaking into the operator preview when the operator has selected a non-`api` stage layout (e.g. `worship-snv`). The api-stage state continues to be **stored** so it's ready when the operator switches to `api` layout, but it must not **publish** a `LiveEvent::Stage` while another layout is active. Additionally, when the operator switches TO `api` layout, the preview should immediately reflect the most recent api-stage state instead of waiting for the next PUT.

## Why

The codebase already has the inverse direction handled: `publish_stage_context` (in `state/broadcasting.rs:82-88`) skips publishing regular stage updates when `stage_layout == "api"` — the comment says *"The 'api' layout is driven by PUT /api/stage, not by internal state. Skip normal broadcasting to avoid overwriting API-pushed data."* The mirror gate is missing on the api-update path: `update_api_stage` (in `state/mod.rs:735-740`) publishes unconditionally.

The bug surface in `update_api_stage`:

```rust
pub(crate) async fn update_api_stage(&self, state: ApiStageState) -> anyhow::Result<()> {
    let snapshot = self.build_api_stage_snapshot(&state).await;
    *self.api_stage.write().await = state;
    self.live_hub.publish(LiveEvent::Stage { snapshot });   // ← always publishes
    Ok(())
}
```

When external integrations (AbleSet, custom REST clients) push to `/api/stage` continuously, the operator's preview gets switched away from whatever layout (e.g. `worship-snv`) they intentionally selected.

## Approach

### Component 1 — gate the publish

In `crates/presenter-server/src/state/mod.rs`, modify `update_api_stage` to read the current layout and only publish when it's `"api"`:

```rust
pub(crate) async fn update_api_stage(&self, state: ApiStageState) -> anyhow::Result<()> {
    let snapshot = self.build_api_stage_snapshot(&state).await;
    *self.api_stage.write().await = state;
    if self.stage_layout_code().await == "api" {
        self.live_hub.publish(LiveEvent::Stage { snapshot });
    }
    Ok(())
}
```

The state is still stored (the `*self.api_stage.write().await = state` line). External integrations PUT successfully (200 OK). No Stage event fires while non-api layouts are active.

### Component 2 — refresh on switch-to-api

In `crates/presenter-server/src/state/stage_display.rs`, extend `set_stage_layout_code` so that when transitioning to `"api"`, the most recent api state is published:

```rust
pub async fn set_stage_layout_code(&self, code: &str) -> anyhow::Result<StageDisplayLayout> {
    let layout = StageDisplayLayout::built_in()
        .into_iter()
        .find(|layout| layout.code == code)
        .ok_or_else(|| anyhow::anyhow!("unknown stage layout: {code}"))?;
    let previous_code = {
        let mut guard = self.stage_layout.write().await;
        if *guard == layout.code {
            return Ok(layout);
        }
        let prev = guard.clone();
        *guard = layout.code.clone();
        prev
    };
    self.live_hub.publish(LiveEvent::StageLayout {
        code: layout.code.clone(),
    });
    if layout.code == "api" {
        // Just switched TO api — publish stored api_stage so the preview
        // reflects the most recent PUT instead of waiting for the next one.
        let snapshot = self.api_stage_snapshot().await;
        self.live_hub.publish(LiveEvent::Stage { snapshot });
    } else {
        self.broadcast_stage_snapshots().await?;
    }
    let _ = previous_code; // reserved for future use; not needed today.
    Ok(layout)
}
```

The `broadcast_stage_snapshots` call moves into the `else` branch — it's the regular flow's source of Stage events for non-api layouts and already short-circuits when layout is api (per the existing gate at `broadcasting.rs:82-88`). Adding the `api_stage_snapshot` publish in the if branch makes the switch-to-api flow consistent.

## Testing

### Unit tests (Rust, in `crates/presenter-server/src/state/tests.rs`)

Three new tests:

1. **`api_input_does_not_leak_when_layout_is_worship`**
   - Setup: `AppState::in_memory()`, set layout to `"worship-snv"`.
   - Subscribe to live_hub.
   - Call `update_api_stage` with non-empty content.
   - Assert no `LiveEvent::Stage` is received within a small timeout.
   - Assert the api_stage state IS stored (read it back via `api_stage.read().await`).

2. **`api_input_publishes_when_layout_is_api`**
   - Setup: layout = `"api"`.
   - Subscribe.
   - `update_api_stage(content)`.
   - Assert a `LiveEvent::Stage` arrives with the api layout in its snapshot.

3. **`switching_to_api_publishes_stored_api_state`**
   - Setup: layout = `"worship-snv"`.
   - Pre-store api state via `update_api_stage` (which won't publish — gate in component 1).
   - Subscribe.
   - `set_stage_layout_code("api")`.
   - Assert events arrive in order: `LiveEvent::StageLayout { code: "api" }`, then `LiveEvent::Stage` with the stored api content.

### Playwright E2E

Add to `tests/e2e/api-stage.spec.ts` (existing file from PR #255):
- Flip layout to `worship-snv`, PUT `/api/stage`, assert operator preview does NOT change to api content.
- Flip layout to `api`, assert preview now reflects the previously-stored api state.

### Manual verification on dev

Per spec template after deploy:
1. Open `/ui/operator/worship` on dev (10.77.8.134:8080).
2. Curl `PUT /api/stage` with sample content.
3. Verify operator preview stays on the current worship layout (no flash to api content).
4. Switch operator's stage to `api` via the layout selector.
5. Verify preview NOW shows the api content from step 2.

## File structure

### Modified files
| File | Change |
|------|--------|
| `Cargo.toml` | Workspace version 0.4.54 → 0.4.55 |
| `crates/presenter-ui/Cargo.toml` | presenter-ui version 0.1.23 → 0.1.24 |
| `crates/presenter-server/src/state/mod.rs` | Gate `LiveEvent::Stage` publish in `update_api_stage` on `stage_layout_code == "api"` |
| `crates/presenter-server/src/state/stage_display.rs` | In `set_stage_layout_code`, publish stored api state when switching TO `api`; move existing `broadcast_stage_snapshots` call into the non-api branch |
| `crates/presenter-server/src/state/tests.rs` | 3 new unit tests covering both gate paths + switch-to-api refresh |
| `tests/e2e/api-stage.spec.ts` | New layout-isolation test |

### Lock files
- `Cargo.lock` and `crates/presenter-ui/Cargo.lock` — auto-updated.

## Out of scope

- **Companion variables** (`crates/presenter-server/src/companion/variables.rs`) consume `LiveEvent::Stage` independently. After this fix it no longer receives api snapshots while non-api layouts are active — that matches the operator-preview behavior and is the desired outcome. No companion code change.
- **Resolume / AbleSet outbound integrations** push stage updates through the regular `publish_stage_context` path, which already gates on the api layout. Unaffected.
- **Stage WebSocket renderers** at `/stage` reflect whatever LiveEvent::Stage they receive. They become consistent with the operator preview once the gate is in place. No client change needed.
- **Returning a 409 Conflict from `PUT /api/stage`** when layout != api — rejected (Approach 3 from brainstorm). Brittle for integrations that push continuously regardless of operator state.

## Closes

- Issue #281 — api worship input switching preview when layout = worship-snv.

## Risks / unknowns

- **Companion behavior depends on stage events.** When layout flips to `worship-snv`, companion no longer sees api snapshots. Verify by reading `companion/variables.rs:32` (`apply_stage_snapshot`) — if companion variables track api content separately, the change is observable but correct: companion now sees the layout-relevant snapshot only.
- **Test setup for layout switching.** `AppState::in_memory()` initializes the layout from `DEFAULT_STAGE_LAYOUT_CODE` (currently `worship-snv`-equivalent — verify before writing tests). If the default isn't `worship-snv`, test 1 must explicitly set it.
- **The `previous_code` capture in `set_stage_layout_code`** is included for symmetry but unused. If clippy flags it as dead code, replace with `let _ = guard.clone();` or remove the local binding entirely.
