# Presenter

Presenter is a purpose-built presentation and worship display tool tailored for our community's needs. It focuses on delivering the essentials—lyrics, Bible passages, timers, and stage displays—without the complexity of larger presentation suites.

## Vision
- **Lyrics:** Fast entry, arrangement management, and flexible display options.
- **Bible:** On-demand scripture lookups with multiple translations and quick formatting.
- **Timers:** Service, segment, and speaker timers surfaced on control and stage views.
- **Stage Displays:** Configurable layouts for confidence monitors and stage prompts.

## Guiding Principles
- Built for operators: minimal clicks, clear status indicators, keyboard-first workflow.
- Built for flexibility: modular scene/layout system that adapts to different service flows.
- Built for reliability: offline-ready data, transparent caching, and automatic backups.

## Planned Milestones
1. Define core data models for songs, scripture references, timers, and layouts.
2. Build a prototype controller UI with live preview and multi-display output.
3. Integrate lyrics and scripture import/export pipelines.
4. Deliver stage display customizer with drag-and-drop regions.

## Project Status
Early implementation. The repository now includes the first-pass Rust workspace:

- `presenter-core` – domain crate covering libraries, presentations, slides, playlists, Bible references, and timers.
- `presenter-server` – Axum/Tokio web server exposing JSON endpoints and housing the future operator & tablet interfaces.
- `presenter-persistence` – SeaORM-backed repository and migration runner targeting SQLite (dev/test) with a forward path to Postgres.
- `presenter-migration` – schema evolution crate orchestrated by the persistence layer.
- ADRs under `docs/adr/` capture stack decisions as they are finalized.

Refer to issue #2 for the end-to-end delivery plan.

```
presenter/
├── Cargo.toml             # Workspace definition
├── crates/
│   ├── presenter-core     # Domain logic + unit tests
│   └── presenter-server   # HTTP/WebSocket entrypoint (axum)
├── docs/                  # Functional specs and ADRs
└── Propresenter library/  # Source assets slated for import pipeline
```

## Contributing
Internal project. Please coordinate changes via issues and planning sessions before opening pull requests.

## Development Environment

1. Install the latest stable Rust toolchain (1.90.0 or newer) via `rustup`.
2. From the repo root, run the server locally:

   ```bash
   cargo run -p presenter-server
   ```

   The server binds to `0.0.0.0:8877` by default; override with `PRESENTER_PORT`.
   Use `PRESENTER_DB_URL` to point at a SQLite/SQL database (defaults to `sqlite://presenter_dev.db`).

3. Execute the full test suite (core + server) with:

   ```bash
   cargo test
   ```

4. Keep development, testing, and production deployments in sync with AGENTS.md runtime instructions as branches progress.
