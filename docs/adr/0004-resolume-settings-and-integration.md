# ADR 0004: Resolume Settings Surface and Clip Integration

## Status
Proposed – 30 September 2025

## Context
Issue #7 introduces the first control-surface integration with Resolume Arena. We must let operators
configure one or more Resolume controllers (DNS hostnames like `resolume.lan` on port 8090) from a
central Settings page, persist those endpoints, and drive Presenter outputs via Resolume's Web
Server + WebSocket APIs. The current code base has no generic settings surface or persistence for
external connections, so neither the UI nor storage layer can describe these hosts.

Operators require:

- A predictable navigation path to settings from the main menu.
- CRUD for Resolume host entries, defaulting to port 8090 while allowing overrides and DNS names.
- Clear health status (connected, reconnecting, error) per host.
- Automatic discovery of decks and clips whenever a host connects or the active deck changes.
- Deterministic mapping between Presenter slide intents and Resolume clips using token suffixes
  (e.g., `#main-a`, `#main-b`).
- Sub-100 ms propagation from trigger → clip update with low jitter.

## Decision

### Settings Architecture
- Introduce a dedicated settings route (`GET /ui/settings`) rendered via a new Leptos component in
  `ui/settings.rs`. The Operator header gains a "Settings" entry that opens this document in a
  separate tab to avoid disrupting live control.
- The page renders cards for each configuration group. The Resolume card lists existing hosts, exposes
  add/edit/delete dialogs, and shows live status.
- Fetch settings via JSON endpoints (`GET /integrations/resolume/hosts`,
  `POST/PUT/DELETE /integrations/resolume/hosts/{id}`) returning minimal DTOs (`id`, `label`, `host`,
  `port`, `status`).
- UI interactions occur over `fetch` with optimistic updates and error toasts; no client-side routing is
  introduced yet.

### Persistence Model
- Extend the base migration with a `resolume_hosts` table containing:
  - `id` `TEXT` PK (UUID v4 string).
  - `label` operator-friendly name (unique per install).
  - `host` hostname or IP.
  - `port` integer (default 8090).
  - `is_enabled` boolean (default true) so operators can temporarily suppress reconnects.
  - `created_at`/`updated_at` timestamps.
- Add `Repository` CRUD helpers and a new domain struct `ResolumeHost` within `presenter-core` to
  expose typed models to the application layer.
- Because the project is pre-release, we mutate the initial migration and regenerate SQLite snapshots
  after the change.

### Runtime Integration
- Introduce a `ResolumeRegistry` inside `AppState` that tracks active host connections. Each entry owns:
  - An async worker that fetches `/api/v1/composition` on start and at fixed intervals (10 s) or upon
    pipeline commands to refresh clip metadata.
  - A bounded channel (capacity 16) receiving Presenter text intents; the worker serializes payloads to
    Resolume REST endpoints (`PUT /api/v1/parameter/by-id/{id}` for text and `POST
    /api/v1/composition/clips/by-id/{clipId}/connect` to trigger the clip).
  - Composition metadata is refreshed proactively whenever cached mappings are older than one
    second or a command fails, so deck changes in Resolume remap the clip targets before the next
    slide fires.
  - A DNS resolver that caches resolved socket addresses for five minutes while preserving the original
    hostname via the HTTP `Host` header so Dockerized deployments avoid repeated lookups yet still reach
    `resolume.lan`.
- The registry surfaces status snapshots (`connecting`, `connected`, `error`, `disabled`) cached in
  memory and served via the settings API.
- AppState exposes `create_resolume_host`, `update_resolume_host`, `delete_resolume_host`, and
  `resolume_status_snapshot` helpers so HTTP handlers and future Playwright fixtures can observe
  connection health.

### Clip Discovery & Mapping
- When a connection starts the worker fetches the active composition, indexes clips by `name`, and
  buckets them into token groups. Tokens follow `#token[-lane]` conventions (e.g., `#main-a`). Matching
  is case-insensitive, ignores surrounding text, and supports alias tokens (`main-a`, `Main A`).
- Maintain two clip references per output channel (Main, Translation, Bible, Bible Translation),
  alternating between A/B lanes. `#bible-clear` remains a dedicated clip.
- Clip names may include suffix modifiers: `-u` forces uppercase output, `-re` collapses
  multi-line payloads into a single space-delimited line. Modifiers can be combined (e.g.
  `#translate-b-u-re`).
- If a token is missing we surface the error to the settings UI and log with `warn` so operators can
  correct naming before service.

### Latency Safeguards
- All outbound updates use a pre-allocated JSON buffer and reuse the `reqwest` client with HTTP/1.1
  keep-alive to minimize connection setup costs.
- The worker tracks round-trip times; if latency exceeds 80 ms for two consecutive commands we mark the
  host `Degraded` and trigger an exponential reconnect to resynchronize state.
- Repeated failures when updating parameters reset the cached mapping and flag the host `error` so the
  next command forces a full refresh.
- Add integration tests with a mocked Resolume server to assert the 100 ms SLA using Tokio time control.

## Consequences
- New settings UI and API endpoints expand the server surface; Playwright tests must cover CRUD and status
  rendering to avoid regressions.
- Application bootstrap now requires loading Resolume hosts and spawning registry tasks; we will ensure
  graceful shutdown (join handles) to avoid leaking tasks in tests.
- Docker demos bundle the same settings UI; branch manifests should note the exposed port for manual
  verification with the real Resolume instance.
- Until we implement other configuration groups, the settings page will show placeholder cards for
  future modules (e.g., Network Bridges) to communicate structure without functionality.

## Alternatives Considered
- Embedding settings in the Operator UI drawer – rejected to keep live controls uncluttered and reduce the
  risk of accidental edits mid-service.
- Storing hosts in a JSON blob – rejected in favor of normalized tables so future features (per-host
  credentials, deck preferences) can extend the schema.
- Poll-only discovery – rejected because Resolume's WebSocket API supplies deck-change events with lower
  latency and network overhead.

## Follow-up
1. Expose a `/integrations/resolume/hosts/{id}/status` streaming endpoint once we surface in-UI toast
   updates.
2. Extend `ResolumeRegistry` to translate Presenter stage/bible events once the text pipeline is ready.
3. Document firewall and DNS configuration in `docs/runbook/resolume.md` after implementation.
