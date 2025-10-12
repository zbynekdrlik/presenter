# Project Context

## Purpose
Presenter is a monolithic, production-ready lyrics, Bible, timer, and stage display system built solely for our church services. It exists to give operators a fast, reliable control surface, keep every output (front-of-house, stage, broadcast) synchronized, and make service prep/import pipelines repeatable with minimal manual effort.

## Tech Stack
- Rust workspace with `presenter-core` (domain logic) and `presenter-server` (Axum/Tokio HTTP + WebSocket entrypoint).
- SeaORM for persistence backed by SQLite in development/testing, with a forward path to Postgres in production.
- Leptos-based UI surfaces (operator, tablet, Bible, settings, timer overlay) rendered from the server crate.
- TypeScript + Playwright for end-to-end testing and frontend tooling (runs via Node.js 22.x).
- Docker Engine 24+ with Compose V2 for isolated demos and the always-on gateway landing page.
- Shared WebSocket channel (`/live/ws`) for live stage/operator synchronization and external device feeds.
- OpenSpec CLI for proposal/spec workflow and structured change management.

## Project Conventions

### Code Style
- Use rustfmt/clippy defaults; keep Rust source files under ~800 lines (hard cap 1,000) and functions under 60 lines by splitting features into focused modules.
- Organize `presenter-server` feature-first: routers under `router/`, state handlers under `state/<feature>.rs`, and keep each Leptos page in `ui/{page}.rs`.
- Store reusable styles in `ui/styles.rs` or static assets; keep per-surface inline styles below ~200 lines before extracting.
- Reserve `mod.rs` files for orchestration/re-exports only; modules stay snake_case while types and components use PascalCase.
- Add succinct comments only where behavior is non-obvious; rely on descriptive naming and tests for most documentation.

### Architecture Patterns
- Single monolithic Rust application optimized for in-house workflows; refactors prefer clean rebuilds over incremental compatibility layers.
- `presenter-core` encapsulates domain models (songs, scripture, timers, stage layouts) with unit tests; `presenter-server` exposes HTTP/WebSocket APIs plus UI shells.
- Feature-specific routers/state modules coordinate downstream clients (AbleSet, OSC, Resolume, Android stage app, Companion) while maintaining deterministic state snapshots for outputs.
- Offline-ready caches, automatic backups, and sanitized ProPresenter import pipelines keep demos and operators consistent across devices.
- Docker-managed demos (`run-demo.sh`) and the port-80 gateway provide branch-specific stacks; every change is validated and deployed through `verify-and-refresh.sh`.
- Maintain three runtime tiers (development, testing, production); keep dev refreshed after each approved PR and gate production promotion on explicit sign-off with zero-downtime handoffs.
- Capture architectural decisions as numbered ADRs and keep documentation in sync with behaviour after every change.

### Testing Strategy
- Follow TDD: add unit tests for pure Rust logic and Playwright E2E coverage for every user-visible workflow and cross-process side effect.
- Primary verification command: `sudo -E ./scripts/dev/verify-and-refresh.sh` (runs `cargo test`, Playwright, rebuilds demo containers, and refreshes the gateway; must pass before reporting status).
- Run suites individually with `cargo test` and `npm run test:playwright` for focused debugging—no `.only`/`.skip` committed.
- Keep tests deterministic (fixed seeds, poll helpers instead of sleeps, animations disabled) and assert both UI interactions and downstream effects (stage layout updates, gateway card freshness, WebSocket broadcasts).
- Recreate SQLite databases after schema edits and rerun import pipelines so dev/test/prod stay aligned.
- Health checks within the suites must confirm the always-on gateway card updates (`data-updated-at`) and that stage/live WebSocket feeds respond within expected latency budgets.
- Run `scripts/dev/quality-check.sh --against origin/main` frequently and `--strict` before merge; open questions or gaps become TODOs in `tasks.md` before implementation proceeds.

### Git Workflow
- Work exclusively on feature branches (`feature/<task>`); create a PR immediately after branching and update it continuously.
- Never commit directly to `main`; agents do not merge PRs and must wait for user approval.
- Keep commits small, descriptive, and single-purpose; push frequently so reviewers can follow progress.
- Run `scripts/dev/quality-check.sh --against origin/main` regularly and `--strict` before merge; ensure working trees are clean before handing off.
- Use OpenSpec proposals/tasks for any new capability, breaking change, or architecture shift; validate with `openspec validate <change-id> --strict` before coding.

## Domain Context
Presenter powers worship services with quick lyric selection, scripture lookup, timer management, and synchronized stage displays. Operators drive services from `/ui/operator`, assembling playlists/run sheets, triggering slides, adjusting timers, and monitoring what every output shows. Stage displays, Resolume layers, OBS overlays, and the tablet/stage apps subscribe to the same WebSocket feed, so layout changes (`POST /stage/layout`) and slide cues propagate instantly. Sanitized ProPresenter libraries live outside the repo (`../presenter-libraries`) and feed the import pipeline that populates every branch demo for consistent content. Stage profiles cover main stage, choir, and broadcast monitors, including automatic Android kiosk relaunch and latency/heartbeat indicators. AbleSet/OSC automation can pre-trigger playlists while operators retain manual override, and Resolume clip metadata is validated so A/B lane naming stays service-ready.

## Important Constraints
- Reliability, predictable behavior, and minimal operator friction take precedence over feature breadth; design every change for high-stakes live services.
- Treat redesigns as greenfield efforts—remove or replace legacy artifacts rather than preserving compatibility with earlier drafts.
- Maintain separate development, testing, and production installations; refresh the Docker demo and gateway after each change and never stop services you did not start.
- Stick to Docker-managed demos (one per repo checkout) and ensure `run-gateway.sh` remains active on port 80.
- Schema is still mutable pre-release: modify the initial migration directly, then rebuild databases and rerun import scripts instead of adding incremental migrations.
- Avoid destructive Git commands (`reset --hard`, etc.) and respect existing local changes unless instructed otherwise.
- Every code or asset change must finish with `sudo -E ./scripts/dev/verify-and-refresh.sh`, confirm the live demo via browser/curl, and document results before reporting progress.
- Keep proposals/designs in OpenSpec up to date; archive completed changes only after deployment using `openspec archive`.

## External Dependencies
- Sanitized ProPresenter library exports that seed songs, scriptures, and playlists.
- Media and control integrations: Resolume, OBS timer overlay, AbleSet/Ableton via OSC, Companion, and Android stage display clients.
- Docker Engine + Compose, Node.js 22.x toolchain, and Playwright browsers (installed via `./scripts/ops/bootstrap-host.sh`).
- SQLite for local storage with a planned move to Postgres via SeaORM in production deployments.
- Android platform tools (ADB) for managing stage tablets via automation scripts.
- Zerotier/LAN connectivity for remote gateway access during services and rehearsals.
