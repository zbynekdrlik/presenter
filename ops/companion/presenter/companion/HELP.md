# Presenter Companion WS

Connects Companion to the Presenter `/companion/ws` socket for automation.

## Setup

1. Add the module in Companion and configure:
   - **Host**: Presenter host or IP (e.g. `10.77.9.21`)
   - **Port**: Branch demo `18175`, testing `8081`, production `8080`.
   - **TLS**: Enable only if Presenter is fronted by HTTPS.
   - **Token**: Optional shared secret when `PRESENTER_COMPANION_TOKEN` is set on the server.
2. The module sends the hello handshake automatically and starts streaming variables once connected.
3. Use the exposed actions to trigger timer, stage, and Bible commands.

## Variables

All Presenter state variables are exposed with the prefix `presenter:` (e.g. `presenter:stage_current_main`, `presenter:timer_countdown_state`).

## Actions

- Timer start/pause/reset
- Set countdown target (ISO timestamp)
- Stage slide updates / blank
- Bible trigger & clear

## Feedback

Use the built-in `Countdown running` feedback or compare any variable against expected text to drive button colours.
