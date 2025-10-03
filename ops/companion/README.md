# Companion Integration Assets

These assets ship a ready-to-import Companion workspace profile, connection notes, and validation helpers for Presenter’s `/companion/ws` socket (issue #29).

## Files

| Path | Purpose |
|------|---------|
| `presenter-companion-profile.json` | Blueprint describing the button layout, macros, and connection settings. Generate an importable Companion export with `node ops/companion/generate-profile.mjs`. |
| `button-reference.md` | Human-readable summary of the shipped buttons, assigned variables, and expected behaviour. |
| `tests/companion-session.spec.ts` | Playwright spec that simulates a Companion session against `startTestServer`, asserting the handshake, ack flow, and variable updates. |

## Quick Start

1. Render the blueprint into an importable export:
   ```bash
   node ops/companion/generate-profile.mjs
   ```
   The script writes `ops/companion/generated/presenter-companion-profile.export.json`.
2. Open Companion (v3.2+) → *Connections* → *Import connection* and select the generated export.
3. Edit the newly created **Presenter** Generic Websocket instance:
   - Set `Host` to your controller hostname (default export is `presenter.dev.local`).
   - Adjust the port if you are targeting testing/production (80/8081/8080 per runbook).
   - Paste the shared secret under *Password* if the server defines `PRESENTER_COMPANION_TOKEN`.
4. Navigate to *Surfaces* → map the supplied button pages (Page 1: Timers, Page 2: Stage/Bible) to your stream deck or emulator.
5. Press the **Connect** button; Companion should report “connected” and the Timer button will light green when `timer_countdown_state === running`.

## Button Layout Overview

A detailed reference lives in `button-reference.md`; highlights:

- **Countdown Control (Page 1, buttons 1–6)**
  - Start / Pause / Reset, ±5 minutes, and a direct HH:MM setter using dynamic text fields.
  - Feedback based on `timer_countdown_state` (green = running, amber = paused, red = idle) and a progress bar bound to `stage_countdown_remaining_seconds`.
- **Stage Output (Page 2, buttons 1–4)**
  - Clear stage, jump to next slide, emergency blank, and “panic” (all outputs blank) actions.
  - Each button shows the current/next lyric snippet via `stage_current_main` / `stage_next_main`.
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
4. Triggers `stage.set` with known IDs and verifies a follow-up `variables` payload contains the IDs.

Run the spec directly during development with:

```bash
npm run test:playwright -- tests/e2e/companion-session.spec.ts
```

## Customising

- Duplicate the profile per environment, adjusting host/port + Companion page labels accordingly.
- Extend timers or Bible buttons by copying the provided actions—ensure the outgoing command JSON matches Presenter’s payloads exactly.
- If you add new Presenter commands, create matching buttons and update `button-reference.md` so the runbook stays accurate.

Please keep this documentation and profile exports in sync with any protocol changes. After editing, re-run `sudo -E ./scripts/dev/verify-and-refresh.sh` to confirm the automated check passes before requesting review.
