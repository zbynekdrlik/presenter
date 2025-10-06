# Presenter Companion WS

Connects Companion to the Presenter `/companion/ws` socket for automation.

## Setup

1. In Presenter, enable “Companion WebSocket” under **Settings → Companion** and assign a unique port (default `18175`).
2. Add the module in Companion and configure:
   - **Host or IP**: Presenter instance (default demo is `10.77.9.21`).
   - **Port**: Match the port configured in Presenter.
   - **TLS**: Enable only if Presenter is fronted by HTTPS.
   - **Token**: Optional shared secret when `PRESENTER_COMPANION_TOKEN` is set on the server.
3. The module sends the hello handshake automatically and starts streaming variables once connected.
4. Use the exposed actions to trigger timer, stage layout, and Bible commands.

## Variables

All Presenter state variables are exposed with the prefix `presenter:` (e.g. `presenter:stage_current_main`).

Timers now expose derived formats ready for button text:

- `timer_countdown_remaining_hhmm` / `timer_preach_elapsed_hhmm`
- `timer_countdown_remaining_hms` / `timer_preach_elapsed_hms`
- `timer_countdown_remaining_mmss` / `timer_preach_elapsed_mmss`
- `timer_countdown_remaining_readable` / `timer_preach_elapsed_readable`

The currently selected stage layout is surfaced as `stage_layout_code`, `stage_layout_name`, and `stage_layout_description`.
The active song metadata is available as `song_name` (prefix-trimmed presentation name) and `band_name` (source library).

## Actions

- Timer start/pause/reset
- Set countdown duration (plain minutes or HH:MM time-of-day for the next occurrence)
- Stage layout switcher (single `stage.layout` action with dropdown for SNV, PP, Timer, Preach)
- Bible trigger & clear

## Feedback

Use the built-in `Countdown running` feedback or compare any variable against expected text to drive button colours.
