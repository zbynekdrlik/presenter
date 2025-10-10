# 0005: Persistence Repository Modularization

Date: 2025-10-10

Status: Accepted

## Context

The persistence layer was implemented as a single monolithic `repository.rs` (~2.4k LOC). This violated our 2025 code-organization baseline (files < ~800 LOC) and made changes risky. As we entered the initial design round, we prioritized clarity, reliability, and testability over backward compatibility with earlier drafts.

## Decision

Split the monolithic repository into feature-scoped modules under `crates/presenter-persistence/src/repository/`:

- `app_settings.rs` – App settings K/V helpers
- `libraries.rs` – Libraries, presentations (list/summary paths)
- `presentations.rs` – Presentation CRUD and slide editing
- `playlists.rs` – Playlist CRUD, favorites, entries
- `search.rs` – Presenter-wide search across libraries/presentations/slides
- `bible.rs` – Bible translations, ingestion, passage search/find
- `osc.rs` – OSC settings CRUD
- `ableset.rs` – AbleSet settings CRUD and table bootstrap
- `resolume.rs` – Resolume host CRUD
- `android_stage.rs` – Android Stage display CRUD
- `stage.rs` – Stage state singleton
- `timer_state.rs` – Timers singleton
- `util.rs` – Shared mappers, conversions, and error types

`mod.rs` now only orchestrates, re-exports types, and hosts tests; it stays < 1000 lines.

## Rationale

- Aligns with the Presenter Agent Handbook (2025 baseline) regarding file/module size and separation by feature.
- Improves test isolation and speeds up iteration during high‑stakes services (safer changes, clearer ownership).
- Removes legacy anchors; favors clean rebuilds as required by the Design Iteration Policy.

## Consequences

- No public API changes for callers; module paths are internal to the persistence crate.
- E2E and unit tests continue to validate the same workflows; new modules include targeted unit tests for pure logic and rely on E2E for cross-surface behavior.

## Verification

- `cargo test` passes across the workspace.
- Playwright E2E passes without focused/skipped tests; demo + gateway validations succeed via `sudo -E ./scripts/dev/verify-and-refresh.sh`.
- Quality gate: `scripts/dev/quality-check.sh --strict --against origin/main` passes.

## Migration

- No runtime migration—schema remains unchanged. Pre-release policy stays: rebuild SQLite dbs when the schema evolves.

