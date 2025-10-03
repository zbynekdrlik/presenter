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

### Prerequisites

- Docker Engine 24+ with Compose V2 (`docker compose` CLI).
- Node.js 20+ for Playwright and end-to-end tests (already required by `package.json`).
- The Rust toolchain remains useful for local builds and IDE integration, but daily demo servers now run inside Docker.

### Launch the landing page (port 80)

Start the gateway once per machine; it serves `http://localhost/` with links to every running demo.

```bash
./scripts/docker/run-gateway.sh --force
```

Re-running the script refreshes the container with the latest landing page assets. To stop it entirely, execute `docker compose -f docker-compose.gateway.yml down` afterwards.

### Spin up an isolated demo for this repository

```bash
./scripts/docker/run-demo.sh --name "$(git rev-parse --abbrev-ref HEAD)"
```

- Each demo uses its own Compose project (`presenter-<slug>`), so containers, volumes, and networks never collide.
- Host ports default to a deterministic high value (e.g. `http://localhost:18042/`). Override with `--port 19001` if you prefer a specific port.
- Presentation imports default to `Propresenter library/` and persisted data lives under `${XDG_DATA_HOME:-$HOME/.local/share}/presenter-demos/data/<slug>/`.
- A manifest is written to `${XDG_DATA_HOME:-$HOME/.local/share}/presenter-demos/manifests/<slug>.json`; the gateway watches this directory to populate the landing page.

Stop a demo when you are finished:

```bash
./scripts/docker/stop-demo.sh --name "$(git rev-parse --abbrev-ref HEAD)"
```

Add `--delete-data` to wipe the corresponding SQLite snapshot. List all active demos and their ports with `./scripts/docker/list-demos.sh`.

### Full verification + demo refresh

Use the combined harness to execute `cargo test`, run the Playwright suite, rebuild the branch demo, and restart the gateway in one go. Run it with sudo so Docker has access to the daemon while tests still execute as your user:

```bash
sudo -E ./scripts/dev/verify-and-refresh.sh
```

Options:

- `--display-name NAME` overrides the gateway card label (defaults to the current branch).

The helper always rebuilds the demo container and gateway image to avoid stale assets. It exits immediately on the first failing step—fix the reported issue and rerun until it passes.

The script derives the demo slug from the repository folder, so multiple parallel worktrees can run without colliding. It updates `${XDG_DATA_HOME:-$HOME/.local/share}/presenter-demos/manifests/` so the landing page always points at the latest build for that checkout.

### Running tests individually

If you need to run suites separately you still can:

```bash
npm run test:playwright
```

```bash
cargo test
```

### Manual (non-Docker) runs

For quick experiments you can still execute:

```bash
cargo run -p presenter-server
```

or `scripts/dev/run-dev-server.sh` (which seeds defaults for `PRESENTER_DB_URL` and `PRESENTER_PORT`). The Docker pathway is recommended for demos because it isolates data and ports per branch.


## Live Update Surfaces

- The server keeps operator and stage displays in sync through a shared WebSocket endpoint at `ws://{host}:{port}/live/ws`.
### Demo Landing Page

- Visit `/` to open the hub page with quick links to the operator control, tablet UI, Bible UI, and stage displays.

### Operator Control UI

- Visit `/ui/operator` from a desktop browser to access the control surface.
- A stage status card above the slide grid mirrors the current/next text (including group labels) so you can confirm what every output is showing before triggering the next cue.
- The stage status now mirrors only the **Main** lyric line (with alternating A/B lanes) so Resolume and stage displays stay synchronized with the confidence monitors.
- The page renders imported libraries and their presentations; choose any slide to publish it instantly via `POST /stage/state`.
- Use the **New** buttons at the top of the Libraries and Playlists panels to create empty containers. Drag presentations from the library list into the active playlist to assemble a run sheet, and drag playlist entries to reorder them inline.
- Favorite frequently used libraries via the star action; favorites stay pinned in the compact view while the “All libraries” dialog surfaces everything else. Playlists now use the same star control so the dashboard only shows the sets we actually want to keep on screen.
- Global search is accent/diacritic insensitive **and** token based—queries like `jezis, ja tymy` resolve to the “Ježiš, ja” slides inside the TYMY library. Click any result to jump directly to the source or drag presentation hits into the active playlist for immediate scheduling.
- Slide cards distinguish Main, Translation, and Stage text with dedicated styling. Group badges now appear only on slides that explicitly anchor a new group, keeping inherited slides clean while still propagating the effective group through the presentation.
- The On Air status panel sits beside the Slides heading, showing the current and next main lyric lines at a glance. It collapses automatically in Edit mode so the editor has maximum vertical space.
- Live mode exposes a dedicated **Clear Slide** control that blanks every output while leaving the current playlist selection untouched. Edit mode enables inline duplication, deletion, and keyboard navigation, with line-limit warnings scoped to the global limit configured in the toolbar.
- Countdown timers now use a time-only selector (HH:MM). Type a new value, hit Enter (or the **Start** button) and Presenter schedules the next occurrence while keeping the running/paused state intact. Quick `±5 min` controls make last-second offsets easy.
- A dedicated OBS-ready overlay lives at `/overlays/timer`; it renders the countdown on a transparent background and updates live via the same WebSocket feed.
- A single stage output selector sits beside the global search bar. Choosing a new mode issues `POST /stage/layout`, reloads the shared `/stage` endpoint automatically, and keeps on-stage monitors, Resolume, and OBS sources aligned without touching URLs.
- If you arrive before a show is running, the page fetches an initial stage snapshot (`GET /stage/snapshot`) for the currently selected layout so active/next slides are highlighted even before the first WebSocket event.
- Troubleshooting: if highlights or timers stop updating, verify the `/live/ws` connection in your browser console (look for reconnect attempts) and confirm the server log shows “presenter server listening” on the expected port.

### Tablet Control UI

- Use `/ui/tablet` on iPad/tablet browsers for a touch-first controller.
- Libraries and presentations are selected from large buttons; slides render as responsive tiles optimised for tapping.
- Live mode mirrors the operator behaviour—tap slides to trigger `/stage/state` updates instantly.
- Switch to Edit mode to update slide text in place (main, translation, stage, group); changes persist through the new slide content endpoint and rebroadcast to live displays.
- Connection status, mode selection, and toast confirmations follow the same WebSocket feed so operators see immediate feedback on every action.
 Events are tagged with a `type`:
  - `timers` – includes `overview` with countdown/preach snapshots.
  - `stage` – includes a `snapshot` payload for the currently active stage layout.
- Static HTML previews under `GET /ui/operator` and `GET /stage` now hydrate in the browser: as soon as the WebSocket connects they update countdowns, preach timers, and lyric groups without requiring a refresh.
- To exercise the flow locally:
  1. Start the dev server with `scripts/dev/run-dev-server.sh`.
  2. Visit `http://localhost:${PRESENTER_PORT:-8877}/ui/operator` in one tab and `http://localhost:${PRESENTER_PORT:-8877}/stage` in another.
  3. Issue timer commands (`POST /timers/command`) or stage updates (`POST /stage/state`) and observe both tabs update instantly.
- Sample message (pretty-printed) emitted on `/live/ws`:

  ```json
  {
    "type": "timers",
    "overview": {
      "countdown_to_start": { "state": "running", "seconds_remaining": 840, "target": "2025-09-27T18:00:00Z" },
      "preach_timer": { "state": "paused", "seconds_elapsed": 0 }
    }
  }
  ```


### Bible Control UI

- Visit `/ui/bible` to search translations, trigger passages, and clear the active verse. The interface issues `/bible/search`, `/bible/trigger`, and `/bible/clear` requests and mirrors updates across all connected clients through the `/live/ws` feed (`bible` / `bible_cleared` events).
- The active passage panel auto-updates when new verses are triggered via integrations or other operator consoles, so you always see what Resolume/stage endpoints should display.
- Use the search field for quick lookups (`"John 3:16"`, `"Psalm 23"`, keywords). Up to 25 matches are returned per query; refine terms for more specific results.


### Companion Control WS

- Connect Bitfocus Companion (or any WebSocket automation client) to `ws://{host}:{port}/companion/ws`.
- The first message must be a `hello` envelope:

  ```json
  {"type":"hello","client":"Companion","instanceName":"FrontDesk","token":"optional-secret"}
  ```

  If `PRESENTER_COMPANION_TOKEN` is set on the server, the `token` field must match; otherwise it may be omitted.
- On success the server replies with `welcome` and an initial `variables` payload. Variables are emitted as name/value pairs so Companion buttons and feedbacks can bind directly (examples: `stage_current_main`, `stage_next_main`, `timer_countdown_remaining_seconds`, `bible_reference`). Every subsequent stage/timer/Bible change pushes a fresh `variables` message.
- Send commands by posting JSON envelopes of the form `{"type":"command","command":"<name>","payload":{...}}`. Supported commands:
  - `stage.set` – payload `{ "presentationId": UUID, "currentSlideId": UUID, "nextSlideId": UUID? }`
  - `timer.set_countdown_target` – payload `{ "target": "2025-09-27T18:00:00Z" }`
  - `timer.start_countdown`, `timer.pause_countdown`, `timer.reset_countdown`
  - `timer.start_preach`, `timer.reset_preach`
  - `bible.trigger` – payload `{ "translation": "KJV", "book": "John", "chapter": 3, "verseStart": 16, "verseEnd": 17? }`
  - `bible.clear`
- Each successful command responds with `{ "type":"ack","command":"<name>" }`. Failures return `{ "type":"error","message":"..." }`. State-changing commands immediately refresh the variable feed so Companion feedbacks update without waiting for additional events.


## HTTP API Snapshot – September 2025
- `GET /healthz` – readiness probe used by systemd/supervisor.
- `GET /libraries` – returns imported libraries with presentations and slides.
- `GET /libraries/summary[?q=term]` – lightweight summaries (ids, names, presentation counts, presentation listings).
- `GET /bible/translations` – lists Bible translations currently stored in the repository.
- `GET /bible/passage?translation=CODE&book=John&chapter=3&verse_start=16[&verse_end=17]` – fetches a single passage if it exists.
- `GET /bible/active` – returns the currently triggered passage (if any) consumed by Resolume/stage/Bible UIs.
- `POST /bible/translations/refresh` – downloads and ingests the default translation set (currently King James Version).
- `POST /bible/trigger` – broadcasts a passage based on translation/reference, updating the live WebSocket feed.
- `POST /bible/clear` – clears the active passage and notifies clients to blank downstream outputs.
- `GET /ui/operator` – server-rendered preview of libraries and available Bible translations (temporary developer surface).
- `GET /ui/tablet` – touch-optimised controller for triggering and editing slides in real time.
- `GET /ui/bible` – Bible control surface for search/trigger workflows.
- `GET /stage-displays` – list of built-in stage layouts (WORSHIP SNV, WORSHIP PP, TIMER, PREACH).
- `GET /stage` – HTML stage display for the active layout (SSR preview of lyrics/timers).
- `GET /stage/snapshot` – JSON snapshot for the active layout (current/next slide IDs, timers).
- `GET /stage/layout` – Returns the active layout code and metadata for all available layouts.
- `POST /stage/layout` – Switch the active stage layout (e.g., worship lyrics → countdown timer).
- `POST /stage/state` – set the active slide for stage displays (presentation + current/next slide IDs).
- `GET /timers/overview` – snapshot of countdown/preach timers stored in the database.
- `POST /timers/command` – mutate timers state (set countdown target, start/pause/reset countdown/preach timers).
- `GET /presentations/{id}` – presentation detail with slides for operator/stage tooling.
- `PATCH /presentations/{presentation_id}/slides/{slide_id}` – update slide text (main/translation/stage/group) with validation and live refresh.
- `GET /live/ws` (WebSocket) – streams JSON events for timers and stage-state snapshots (see “Live Update Surfaces”).
- `GET /companion/ws` (WebSocket) – bidirectional Companion control channel (see “Companion Control WS”).

## Upcoming Work
- Expanded interface specifications and additional Bible translations are being planned under issue #5.
- Advanced control protocols (OSC, MIDI/Bome Network Pro, deeper Companion automation) are tracked in issue #6.
- Resolume Arena auto-discovery, clip mapping, and latency hardening live under issue #7.
