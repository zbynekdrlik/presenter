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

   or use the helper script (`scripts/dev/run-dev-server.sh`) which seeds defaults for `PRESENTER_DB_URL` and `PRESENTER_PORT`.

  For a continuously running demo environment that rebuilds on every file change, enable the bundled systemd unit:

  ```bash
  sudo ./scripts/ops/setup-systemd.sh dev test
  ```

  This installs the templated systemd unit (`presenter@.service`) and boots two managed environments:

  - `presenter@dev.service` → auto-refreshing developer instance on port **80**. It runs `cargo watch` via `scripts/dev/watch-demo.sh`, re-importing the ProPresenter & Bible data on every rebuild so the demo always mirrors the latest code.
  - `presenter@test.service` → stable test instance on port **8081** backed by its own SQLite database. Use it for pre-production validation without disrupting the live demo.

  When production promotion is required, run `sudo ./scripts/ops/setup-systemd.sh prod` to install and start `presenter@prod.service` (port **8080**, release build) together with the nightly backup timer `presenter-backup@prod.timer`.

  Useful management commands:

  ```bash
  systemctl status presenter@dev.service            # health + active processes (dev)
  journalctl -u presenter@dev.service -f            # follow live logs
  sudo systemctl restart presenter@dev.service      # force refresh + redeploy dev
  sudo systemctl stop presenter@dev.service         # temporarily halt dev

  sudo systemctl restart presenter@test.service     # restart test instance
  sudo systemctl restart presenter@prod.service     # restart production instance
  sudo systemctl status presenter-backup@prod.timer # confirm nightly backups
  ```

  The manual helper script (`scripts/dev/start-demo-server.sh`) still exists for ad-hoc runs without systemd and now delegates to the same `run-env.sh dev` entrypoint used by the unit.
  All environments bind to `0.0.0.0`; override with `PRESENTER_PORT` if you need a different port (binding to 80 typically requires `cap_net_bind_service`, granted via the unit).
   Use `PRESENTER_DB_URL` to point at a SQLite/SQL database (defaults to `sqlite://presenter_dev.db`).
   If you want Companion connections to require authentication, set `PRESENTER_COMPANION_TOKEN` before starting the server—clients must echo this token in their WebSocket handshake.

3. Rebuild the development database with live content (presentations + Bibles):

   ```bash
   ./scripts/dev/refresh-dev-data.sh
   ```

  The script deletes the existing SQLite file, imports every ProPresenter library under `Propresenter library/`, and pulls the default Bible bundle (King James Version + Slovenský ekumenický + Roháčkov + Slovenský evanjelický) straight from their upstream archives. Pass an alternative source directory as the first argument if your libraries live elsewhere. When you only need to re-import Bibles without touching presentation data, run `cargo run -p presenter-importer --bin ingest_bibles` directly—`scripts/dev/ingest-default-bibles.sh` also wipes the SQLite database before ingesting. For finer control (purging behaviour, alternate library roots), inspect the importer CLI via `cargo run -p presenter-importer --bin import_propresenter -- --help`.

   Managed services write their SQLite databases to `var/data/<env>/presenter_<env>.db` so backups and environment swaps never pollute the working tree.

4. Back up or restore an environment database:

   ```bash
   ./scripts/ops/db-maintenance.sh backup dev
   ./scripts/ops/db-maintenance.sh list prod
   sudo ./scripts/ops/db-maintenance.sh restore prod /path/to/backup.db
   ```

   The production systemd timer `presenter-backup@prod.timer` (installed with `./scripts/ops/setup-systemd.sh prod`) runs nightly at 02:30 UTC and writes timestamped archives to `var/backups/prod/` by default. Override `PRESENTER_BACKUP_ROOT` when you want backups under `/var/lib/presenter/backups` or network storage instead of the repository.

5. Execute the full test suite (core + server) with:

  ```bash
  cargo test
  ```

6. Keep development, testing, and production deployments in sync with AGENTS.md runtime instructions as branches progress.

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
- Countdown timers can be set/reset by selecting a target datetime and pressing the provided buttons. All timer commands reuse the `/timers/command` API, so every interaction is backed by server validation.
- If you arrive before a show is running, the page fetches an initial stage snapshot (`GET /stage/snapshots/worship-snv`) so active/next slides are highlighted even before the first WebSocket event.
- Troubleshooting: if highlights or timers stop updating, verify the `/live/ws` connection in your browser console (look for reconnect attempts) and confirm the server log shows “presenter server listening” on the expected port.

### Tablet Control UI

- Use `/ui/tablet` on iPad/tablet browsers for a touch-first controller.
- Libraries and presentations are selected from large buttons; slides render as responsive tiles optimised for tapping.
- Live mode mirrors the operator behaviour—tap slides to trigger `/stage/state` updates instantly.
- Switch to Edit mode to update slide text in place (main, translation, stage, group); changes persist through the new slide content endpoint and rebroadcast to live displays.
- Connection status, mode selection, and toast confirmations follow the same WebSocket feed so operators see immediate feedback on every action.
 Events are tagged with a `type`:
  - `timers` – includes `overview` with countdown/preach snapshots.
  - `stage` – includes a `snapshot` payload for each built-in stage layout.
- Static HTML previews under `GET /ui/operator` and `GET /stage/{code}` now hydrate in the browser: as soon as the WebSocket connects they update countdowns, preach timers, and lyric groups without requiring a refresh.
- To exercise the flow locally:
  1. Start the dev server with `scripts/dev/run-dev-server.sh`.
  2. Visit `http://localhost:${PRESENTER_PORT:-8877}/ui/operator` in one tab and `http://localhost:${PRESENTER_PORT:-8877}/stage/worship-snv` in another.
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
  - `timer.start_preach`, `timer.pause_preach`, `timer.reset_preach`
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
- `GET /stage/{code}` – HTML stage display for the chosen layout (SSR preview of lyrics/timers).
- `GET /stage/snapshots/{code}` – JSON snapshot for a layout (current/next slide IDs, timers).
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
