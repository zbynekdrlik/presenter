# 0004 - Stage Display Heartbeat And Link Status

## Status
Accepted - October 1, 2025

## Context
Stage operators previously saw the stage status pill report `Connected` whenever the WebSocket opened, even if no slide traffic flowed and the link had silently stalled. Operators requested a persistent heartbeat so they can trust the connection without poking the system. We also want the presenter server to surface which stage displays are actively receiving updates so future operator dashboards can monitor live endpoints. Finally, the UI should dim stale lyrics when a display freezes to avoid misleading the worship team.

## Decision
- Add a `LiveEvent::Heartbeat` broadcast emitted on a configurable interval (`PRESENTER_HEARTBEAT_INTERVAL_MS`, default 1500 ms). Every stage client acknowledges each heartbeat so the server can observe round-trip liveness.
- Introduce `StageConnections`, a runtime registry that records the last heartbeat per `client_id`, tracks state transitions (`connecting`, `connected`, `reconnecting`, `disconnected`), and exposes `/stage/connections` for operator tooling.
- Stage displays now generate a stable `clientId`, advertise their layout via a `stage_presence` message, and acknowledge server heartbeats. The UI consumes server-measured round-trip latency (`Connected · <latency>`), shows `Reconnecting…` during loss, and dims when outputs are stale.
- The operator header now surfaces a compact `connected/issues` counter tied to the stage-preview card. Counts auto-expand as new displays come online, while a one-click reset lets technical leads re-baseline mid-service.
- Stage heartbeat thresholds are configurable through environment variables and are embedded into the rendered HTML so clients and tests share the same timing parameters.
- Playwright and unit tests cover heartbeat transitions, stale-output dimming, the stage monitor badge, and the new server registry semantics.

## Consequences
- Stage status now reflects real transport health even when slides are idle, reducing operator guesswork.
- The presenter server maintains awareness of every connected stage client, enabling future operator dashboards or alerting without additional instrumentation.
- Stage displays introduce small, lightweight heartbeat traffic; the interval defaults (~1.5 s) keep load minimal while staying responsive.
- Debug hooks allow automated tests to simulate heartbeat loss deterministically.
- Operators (and automated health checks) can query `/stage/connections` to audit active stage monitors.
