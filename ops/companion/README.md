# Companion Integration Assets

These assets ship a ready-to-import Companion workspace profile, connection notes, and validation helpers for Presenter’s `/companion/ws` socket (issue #29).

## Files

| Path | Purpose |
|------|---------|
| `presenter-companion-profile.json` | Blueprint describing the button layout, macros, and connection settings. Generate an importable Companion export with `node ops/companion/generate-profile.mjs`. |
| `button-reference.md` | Human-readable summary of the shipped buttons, assigned variables, and expected behaviour. |
| `presenter/` | Companion user-module source (package, manifest, help, index). Copy to `/companion/modules/user/presenter/` and run `npm install` before restarting Companion to use the built-in module. |
| `tests/companion-session.spec.ts` | Playwright spec that simulates a Companion session against `startTestServer`, asserting the handshake, ack flow, and variable updates. |

## Quick Start

1. Render the blueprint into an importable export:
   ```bash
   node ops/companion/generate-profile.mjs
   ```
   The script writes `ops/companion/generated/presenter-companion-profile.export.json`.
2. Open Companion (v3.2+) → *Connections* → *Import connection* and select the generated export.
3. Edit the newly created **Presenter Companion WS** module instance:
   - Set *Host or IP* to your controller hostname (default export keeps the dev demo `10.77.9.21`).
   - Adjust the *Port* to match the value configured in Presenter (see **Settings → Companion**). Each demo should choose a unique port; default is `18175`.
   - Paste the shared secret under *Token* if the server defines `PRESENTER_COMPANION_TOKEN`.
   - In Presenter’s **Settings → Companion** card, toggle “Companion WebSocket” on and pick a unique port. Saving hot-reloads the listener; disable it for parallel demos that should not expose the service.
4. Navigate to *Surfaces* → map the supplied button pages (Page 1: Timers, Page 2: Stage/Bible) to your stream deck or emulator.
5. Press the **Connect** button; Companion should report “connected” and the Timer button will light green when `timer_countdown_state === running`.

6. New live variables `song_name` and `band_name` mirror the active song title (prefix trimmed) and its library. Bind them to buttons or feedback for quick context.

### Module distribution (container builds)

- Package the module tarball before promoting to infrastructure:
  ```bash
  scripts/companion/package-module.sh
  ```
  This produces `ops/companion/releases/presenter-companion-ws-0.5.0.tgz` and an accompanying `latest.json` manifest (version + SHA-256). Automations in `nl-infrastructure` can curl those files directly from the repo to seed Companion images.
- Regenerate the tarball after any module change so Docker rebuilds always use the matching version. The packaging script rewrites files deterministically to keep diffs reviewable.

## Button Layout Overview

A detailed reference lives in `button-reference.md`; highlights:

- **Countdown Control (Page 1, buttons 1–6)**
  - Start / Pause / Reset, ±5 minutes, and a direct HH:MM setter using dynamic text fields.
  - Feedback based on `timer_countdown_state` (green = running, amber = paused, red = idle), a `timer_countdown_remaining_hhmm` text tile, and a progress bar bound to `timer_countdown_remaining_seconds`.
  - Enter times like `18:30` to jump to the next occurrence of that time-of-day or use plain minutes (e.g. `15`) for relative durations.
- **Stage Output (Page 2, buttons 1–4)**
  - Quick layout presets (SNV lyrics, PP overview, Timer, Preach) using the shared `stage.layout` action with a per-button code payload, plus a panic-button macro.
  - Each button reflects `stage_layout_name` alongside lyric snippets (`stage_current_main`, `stage_next_main`).
- **Bible (Page 2, buttons 5–6)**
  - Quick trigger for configurable passages plus a clear button.
- **Alert Macro**
  - `macro_presenter_alert` flashes the panic button red and fires a Companion notification whenever `live_ws_connected` is false or a command returns an error.

## Token & Security Notes

- Configure `PRESENTER_COMPANION_TOKEN` on the server to require authentication. The profile ships with the token field blank; operators must fill it manually.
- Connection failures return specific socket codes:
  - `4000` – malformed handshake (Companion bug or misconfigured macro).
  - `4001` – token missing/invalid.
  - `4002` – concurrent connection limit (disconnect another Companion client first).
  Companion macros surface these via the alert button; keep them enabled.

## Validation

`tests/e2e/companion-session.spec.ts` runs automatically under `npm run test:playwright`. It:

1. Spawns a Presenter test server.
2. Opens a WebSocket, performs the hello handshake, and waits for `welcome` + `variables`.
3. Sends a `timer.reset_preach` command and asserts an `ack` response.
4. Triggers `stage.layout` to confirm layout switching and checks the refreshed variable payload.

Run the spec directly during development with:

```bash
npm run test:playwright -- tests/e2e/companion-session.spec.ts
```

## Customising

- Duplicate the profile per environment, adjusting host/port + Companion page labels accordingly.
- Extend timers or Bible buttons by copying the provided actions—ensure the outgoing command JSON matches Presenter’s payloads exactly.
- If you add new Presenter commands, create matching buttons and update `button-reference.md` so the runbook stays accurate.

Please keep this documentation and profile exports in sync with any protocol changes. After editing, re-run `sudo -E ./scripts/dev/verify-and-refresh.sh` to confirm the automated check passes before requesting review.
