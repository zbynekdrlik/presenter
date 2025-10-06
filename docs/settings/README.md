# Settings Overview

Presenter features that bind network ports are controlled via environment flags so that parallel
development stacks do not collide. Enable features explicitly per instance; the defaults keep
sandbox servers quiet.

## Companion WebSocket

- **Flags:**
  - `PRESENTER_COMPANION_ENABLED` (default `0`)
  - `PRESENTER_COMPANION_PORT` (default `18175`)
- **Effect:** When enabled, Presenter starts a dedicated websocket listener on the configured port
  for Bitfocus Companion modules. Disabling the flag tears the listener down so parallel demos can
  reclaim the port.
- **Typical usage:**
  - Local demo: `scripts/docker/run-demo.sh --enable-companion --port 18042`
  - Manual run: `PRESENTER_COMPANION_ENABLED=1 PRESENTER_COMPANION_PORT=18175 cargo run -p presenter-server`
- **Settings UI:** The **Companion** card in `/ui/settings` exposes a single row with a toggle and port
  field for “Companion WebSocket”. Saving persists the settings and hot-reloads the listener without
  restarting the server. Each demo should pick a unique port to avoid collisions on shared hosts.

Disable the flag for stack clones that should not host the Companion socket so multiple demos can run
in parallel without port conflicts.

![Companion Settings Card](./companion-dashboard.png)
