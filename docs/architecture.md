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

## Stream Graphics Subsystem

The stream-graphics subsystem (epic #718, [ADR 0009](adr/0009-stream-graphics.md))
is a self-hosted **replacement for the church's Resolume stream compositing**. It
renders a nameable, transparent WASM page (`/stream/{slug}`) that is added to OBS
as a browser source and driven from Bitfocus Companion. See
[configuration.md](configuration.md#stream-graphics-obs-browser-source-717) for
the OBS setup.

### Model: outputs → scenes → elements

- **Output** — one transparent page, addressed by a URL **slug** (`/stream/{slug}`).
  The migration seeds one output, slug `stream`. Each output has a
  `default_transition_ms` (the crossfade duration) and a persisted **active
  show-state** (`active_scene_id` + the set of active overlay scene ids).
- **Scene** — a named layer belonging to an output, of kind **`base`** or
  **`overlay`**. Exactly one base scene is active at a time (exclusive); any
  number of overlay scenes can be active simultaneously and independently. Base
  renders first; overlays stack on top in activation order.
- **Element** — a typed graphic inside a scene, positioned by a percentage
  `Frame` and ordered by `z_order`. Four kinds:
  `image` (an uploaded asset), `countdown` (bound to a Presenter timer),
  `lyrics` (main + optional translation, from the worship-stage pipeline), and
  `verse` (main + optional secondary/translation, from the Bible pipeline).
  Lyrics and verse REUSE the existing `Stage` / `BibleSlide` live events — the
  stream subsystem adds no new content events, only new rendering.

### Show-state vs. config: `config_revision`

Two distinct change channels keep the OBS page lightweight:

- **Activation** (switch base scene, toggle an overlay, clear) changes only the
  show-state. It is broadcast by the `StreamState` live event and applied
  DIRECTLY by clients. It does **not** bump `config_revision` — a scene switch
  must never force a full config refetch.
- **Config writes** (create/rename/delete/reorder a scene, create/patch/delete
  an element) bump the output's `config_revision` and broadcast
  `StreamConfigChanged`. A client refetches the full output def only when the
  revision advances (or on WS reconnect, since the live hub does not replay).

This split is mirrored on the server (`bump_config_revision` fires on config
writes only) and on the WASM editor + output-page clients.

### Companion command surface

Companion drives the show over `/companion/ws`. Scenes are addressed by **name,
case-insensitively**; `output` defaults to `"stream"`:

| Command                 | Payload            | Effect                              |
| ----------------------- | ------------------ | ----------------------------------- |
| `stream_scene_set`      | `{scene, output?}` | Activate a base scene (exclusive)   |
| `stream_scene_clear`    | `{output?}`        | Clear the base only                 |
| `stream_overlay_on`     | `{scene, output?}` | Activate an overlay                 |
| `stream_overlay_off`    | `{scene, output?}` | Deactivate an overlay               |
| `stream_overlay_toggle` | `{scene, output?}` | Toggle an overlay                   |
| `stream_clear`          | `{output?}`        | Clear the base and all overlays     |

Each command executes through the `AppState` activation methods (so `StreamState`
fires) and surfaces every refusal (unknown output/scene, wrong kind, bad payload)
as a non-fatal error reply the plugin logs — never a panic. Two companion
variables track the live state: `stream_scene` (active base name, or `-`) and
`stream_overlays` (comma-joined active overlay names, or `-`).

### Wire-casing split (deliberate, documented)

The stream JSON wire format uses **two casings on purpose**, and this split is a
settled decision (ADR 0009) — do not "unify" it:

- **`snake_case` with an internal tag** — the two serde-tagged enums:
  `StreamElementProps` (`#[serde(tag = "kind", rename_all = "snake_case")]`, so
  `kind`, `asset_id`, `show_main`, `show_translation`, `main_style`,
  `content_transition`, …) and `ContentTransition`
  (`#[serde(tag = "mode", rename_all = "snake_case")]`, so `mode`, `duration_ms`).
  The `kind` tag value MUST equal the `stream_elements.kind` column
  (`image|countdown|lyrics|verse`).
- **`camelCase` everywhere else** — every DTO and nested value struct: `Frame`
  (`xPct`, `yPct`, `wPct`, `hPct`), `TextStyle` (`fontFamily`, `sizePct`,
  `lineHeight`, …), `Shadow` (`xPx`, `yPx`, `blurPx`), `StreamShowState`
  (`activeSceneId`, `activeOverlayIds`, `configRevision`), and the output/scene/
  element def DTOs (`defaultTransitionMs`, `zOrder`, `isActive`, `transitionMs`).

So an element create/patch body is bare `StreamElementProps` JSON — snake_case
tag/fields with camelCase-keyed `frame`/`style` VALUES nested inside. Both halves
are internally consistent; every REST/WS fixture and client DTO is reconciled to
this split.

## Key HTTP Endpoints

| Endpoint        | Purpose                      |
| --------------- | ---------------------------- |
| `/healthz`      | Readiness probe              |
| `/ui/operator`  | Desktop control surface      |
| `/ui/tablet`    | Touch-optimized controller   |
| `/ui/bible`     | Bible search/trigger UI      |
| `/ui/settings`  | Configuration interface      |
| `/stage`        | HTML stage display           |
| `/ui/stream`    | Stream-graphics editor       |
| `/stream/{slug}`| Transparent OBS output page  |
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

### Stage video latency (server→display)

The stage's `server→displej · N ms` readout is the TRUE server→display video
latency: **network one-way (RTT/2 from the `/ndi/time` pipeline-clock handshake) +
render residual (`expectedDisplayTime − receiveTime` = jitter-buffer + decode +
present)**. It shows `n/a` — never a misleading number — when there is no fresh
`/ndi/time` offset, and is labelled server→display (not glass-to-glass, not
lip-sync). The old residual-only figure ignored network transit (why Tailscale read
lowest) and is no longer a headline number. `estimatedPlayoutTimestamp` is absent on
the real Android-TV WebViews (and desktop Chrome) with our monotonic-clock RTCP SRs,
so the metric deliberately avoids it. See `docs/adr/0008-ndi-true-latency.md`.

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
