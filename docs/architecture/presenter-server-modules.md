# Presenter Server Module Layout (2025-10-11)

The server crate now follows the feature-first structure mandated in the Presenter Agent Handbook.
This document is the high-level map so future feature work lands in the right place without
rediscovering the split.

## Top-Level Crate Layout

| Path | Responsibility |
| --- | --- |
| `src/main.rs` | wiring: load `ServerConfig`, create `AppState`, start Axum router |
| `src/config.rs` | typed env/config parsing; no networking or state beyond pure data |
| `src/router/` | HTTP entry points grouped by capability (`bible`, `presentations`, `integrations/*`, `settings`, etc.) |
| `src/state/` | domain coordinators per feature; `app.rs` orchestrates and re-exports light handles |
| `src/ui/` | Leptos pages with subcomponents under `ui/components/` |
| `src/ableset.rs`, `src/osc.rs`, etc. | external client implementations with mocked variants for tests |

`mod.rs` files exist only to re-export submodules—feature logic lives in leaf files.

## State Layer Highlights

- `state/app.rs` is the single constructor and owns shared services (repository, live hub, clients).
- Each feature (`state/ableset.rs`, `state/presentations.rs`, `state/stage.rs`, etc.) exposes
  minimal async helpers used by both router handlers and UI generators.
- `state/settings.rs` centralises feature flags (Companion enable/port) and exposes an atomic
  view of the toggles so UI/services stay in sync.

## Persistence Integration

The SeaORM repository crate is now split by feature under
`crates/presenter-persistence/src/repository/*`. The server only calls the narrow
feature-specific APIs surfaced via `AppState`.

## UI Surfaces

- `ui/operator.rs`, `ui/settings.rs`, `ui/timer_overlay.rs`, and the tablet/bible counterparts
  produce HTML documents. Shared cards/widgets live in `ui/components/`.
- Scripts/styles remain in `ui/scripts.rs` and `ui/styles.rs` so we avoid inline bloating.
- New Leptos components should live in `ui/components/<name>.rs` and be re-exported from
  `ui/components/mod.rs`.

## Router Modules

- REST+JSON handlers sit under `router/` by feature, mirroring the state modules they call.
- `router/settings.rs` wires the `/settings/features` endpoints to `AppState::set_companion_settings`.
- `router/ui_routes.rs` is the thin adapter that renders the Leptos pages via the helpers above.

## External Integrations

- `ableset.rs`, `osc.rs`, `resolume.rs`, `android_stage.rs`, and `companion/` each encapsulate
  their network clients. Tests rely on the mock structs living alongside the real implementations.
- The state modules call these clients through trait objects, keeping the router/UI ignorant of
  transport details.

## Testing Expectations

- Unit tests live next to the code and exercise pure logic (`state/tests.rs`, repository tests, etc.).
- E2E/Playwright coverage lives under `tests/e2e/` and is invoked automatically by
  `sudo -E ./scripts/dev/verify-and-refresh.sh`.

Use this map when adding new features: find or create the relevant `state/<feature>.rs`, add
router endpoints under `router/<feature>.rs`, and extend the UI under `ui/` with shared components
split into `ui/components/` when reusable.
