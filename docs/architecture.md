# Presenter Architecture

> **Single Source of Truth** for system design and architecture decisions.

## Overview

Presenter is a monolithic Rust application for church worship services, providing:

- Lyrics display with rapid search
- Bible passage rendering
- Service timers with stage alerts
- Multi-display stage output
- External integrations (Resolume, Companion, Ableton)

### Design Philosophy

- **Reliability over breadth**: Offline-ready, sub-100ms latency, clear operator feedback
- **Church-specific**: Solve exact requirements for our workflows, not generic features
- **Single binary**: No external runtime dependencies (Node.js, Python, etc.)

## Technology Stack

| Layer     | Technology         | Purpose                         |
| --------- | ------------------ | ------------------------------- |
| Runtime   | Rust 1.83+ / Tokio | Async I/O, performance, safety  |
| HTTP      | Axum 0.8           | Web framework with typed routes |
| UI        | Leptos 0.7 SSR     | Server-rendered reactive UI     |
| Database  | SQLite / SeaORM    | Local-first persistence         |
| Real-time | WebSockets         | Live updates to stage/operators |

See [ADR 0001](adr/0001-architecture-stack.md) for the full decision rationale.

## Workspace Structure

```
crates/
├── presenter-core/        # Domain logic (no server deps)
│   ├── src/
│   │   ├── library.rs     # Libraries, presentations, slides
│   │   ├── bible.rs       # Bible references and translations
│   │   ├── timer.rs       # Timer types and state
│   │   └── stage.rs       # Stage layout definitions
│   └── Cargo.toml
├── presenter-server/      # Axum HTTP/WS server + Leptos UI
│   ├── src/
│   │   ├── main.rs        # Entry point
│   │   ├── router/        # Feature routers (modular)
│   │   ├── state/         # Application state (split into modules)
│   │   ├── ui/            # Leptos pages
│   │   ├── companion/     # Bitfocus Companion protocol
│   │   ├── live.rs        # WebSocket broadcast
│   │   └── resolume.rs    # Resolume Arena integration
│   └── Cargo.toml
├── presenter-persistence/ # SeaORM repository layer
├── presenter-migration/   # Schema evolution
├── presenter-importer/    # ProPresenter import pipeline
└── presenter-bible/       # Bible translation ingestion
```

## Data Flow

### Request Lifecycle

```
HTTP Request → Axum Router → Handler → AppState → Repository → SQLite
                    ↓
              Live WebSocket ← State Changes → Stage Displays
```

### WebSocket Event Flow

```
AppState changes → LiveHub::broadcast() → All subscribers
     ↓
[Operator UI, Tablet UI, Stage Displays, Companion]
```

### Stage Update Pipeline

1. Operator triggers slide change
2. AppState updates current presentation/slide
3. LiveHub broadcasts `StageSnapshot`
4. Stage displays receive via `/live/ws`
5. Resolume Arena receives clip trigger (if configured)

## Domain Model

### Core Entities

```
Library
  └── Presentation
        └── Slide (ordered, content blocks)

BibleTranslation
  └── BibleBook
        └── Chapter
              └── Verse

Timer
  ├── Countdown (with alert thresholds)
  └── Preach (count-up with target)

StageLayout
  └── Regions (lyrics, timers, notes, alerts)
```

### Key Relationships

- Libraries contain presentations (songs)
- Presentations contain ordered slides
- Playlists reference presentations (service order)
- Stage layouts define display regions

## Integration Points

### Internal

- **LiveHub**: Central WebSocket broadcast for all real-time updates
- **AppState**: Shared state with concurrent access (Arc<Mutex>)
- **Repository**: Database abstraction layer

### External

- **Resolume Arena**: HTTP API for clip triggering and text layers
- **Bitfocus Companion**: WebSocket protocol for automation buttons
- **Ableton/AbleSet**: OSC protocol for show automation
- **Android Displays**: ADB commands for Fully Kiosk browser control

See [ADR 0004](adr/0004-resolume-settings-and-integration.md) and [ADR 0005](adr/0005-stage-heartbeat.md) for integration details.

## Key HTTP Endpoints

| Endpoint        | Purpose                      |
| --------------- | ---------------------------- |
| `/healthz`      | Readiness probe              |
| `/ui/operator`  | Desktop control surface      |
| `/ui/tablet`    | Touch-optimized controller   |
| `/ui/bible`     | Bible search/trigger UI      |
| `/ui/settings`  | Configuration interface      |
| `/stage`        | HTML stage display           |
| `/live/ws`      | Live updates (timers, stage) |
| `/companion/ws` | Bitfocus Companion control   |

Full API reference in [README.md](../README.md#http-api-snapshot).

## Configuration

All environment variables and feature flags documented in [configuration.md](configuration.md).

## Versioning and Release Strategy

### Semantic Versioning

All crates use **Semantic Versioning** (SemVer) with workspace-level version management.
The same clean `X.Y.Z` format is used on both `dev` and `main` branches:

```toml
# Cargo.toml (workspace root)
[workspace.package]
version = "0.1.2"
```

### Build Channel

Dev vs production builds are distinguished via a compile-time environment variable:

| Channel         | Env Var                           | Set By           | Healthz                 | UI Footer      |
| --------------- | --------------------------------- | ---------------- | ----------------------- | -------------- |
| `dev` (default) | `PRESENTER_BUILD_CHANNEL=dev`     | `deploy-dev.yml` | `{"channel":"dev"}`     | `v0.1.2 (dev)` |
| `release`       | `PRESENTER_BUILD_CHANNEL=release` | `deploy.yml`     | `{"channel":"release"}` | `v0.1.2`       |

Local builds without the env var default to `dev` channel.

### Branch Strategy

```
main (protected)           ← releases, tags, production-ready
  ↑
  └── PR (requires CI pass)
      ↑
dev (development)          ← daily work, CI validates each push
```

**Rules:**

- `main`: Protected, no direct commits, requires PR with passing CI
- `dev`: Primary development branch, all work happens here
- Feature branches: Optional, merge to `dev` via PR

### Release Flow

1. **Development**: Work on `dev` branch with version `X.Y.Z`
2. **Ready to release**: Create PR from `dev` to `main` (no version change needed)
3. **Merge**: After approval, merge to `main` triggers production deploy
4. **Post-release**: Bump version to next `X.Y.(Z+1)` on `dev`

### Version Display

The application version is available at runtime:

- `/healthz` endpoint includes `version`, `channel`, and `ndi_pipelines: [{source_id, state, last_error?}]` fields. `state` is one of `starting | streaming | stopped | errored`; `last_error` is present only when `state == "errored"`. The `ndi_pipelines` field is always an array (empty when no NDI manager is loaded or no sources are active).
- UI footer displays version with channel suffix for non-release builds
- Logs include version on startup

### CI Validation

The `version-check.yml` workflow enforces:

- Version must be clean semver `X.Y.Z` (no pre-release suffixes)
- Version must be greater than the latest GitHub release
- PRs to main: warns if not from `dev` branch

## Branch Protection

### Main Branch Rules

- Require pull request before merging
- Require status checks to pass (CI, E2E, Quality)
- Require linear history
- Do not allow bypassing the above settings

### Dev Branch Rules

- Require status checks to pass (CI, E2E)
- Allow direct pushes (for autonomous agents)

## Quality Standards

- Files: ≤800 lines (warn), ≤1000 lines (fail)
- Functions: ≤60 lines
- No `unwrap()`, `expect()`, `panic!` in production code
- E2E tests required for user-visible changes

See [Quality Review](issues/41-recurring-quality-architecture-review.md) for full standards.
