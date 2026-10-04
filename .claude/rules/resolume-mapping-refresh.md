---
paths:
  - "crates/presenter-server/src/resolume/driver.rs"
  - "crates/presenter-server/src/resolume/mod.rs"
  - "crates/presenter-server/src/resolume/mapping_refresh*.rs"
  - "crates/presenter-server/src/resolume/handlers.rs"
  - "crates/presenter-server/src/resolume/bible_clear.rs"
  - "crates/presenter-server/src/resolume/port_drift.rs"
  - "crates/presenter-server/src/mock_integrations/resolume.rs"
  - "crates/presenter-ui/src/pages/settings/resolume.rs"
---

# Resolume composition fetches — never on a timer (#808)

## Why

`GET /api/v1/composition` returns Arena's whole composition: 16.3 MB on SNV.
Arena needs about 0.5 s to build it. On win-resolume each response held CPU 0
in an NDIS DPC for 4–11 ms, and the LED wall, SongPlayer's VBAN/NDI output and
cg OBS all stalled with it. The driver used to fetch it every 10 s.

## The rule

The 10 s worker tick (`HostDriver::tick`, `mapping_refresh.rs`) is a liveness
probe, `GET /api/v1/product`. That returns Resolume's ~64 B `ProductInfo`
(`{"name": "Arena" | "Avenue", ...}`), validated by
`port_drift::is_resolume_product_body`. A 404 counts as alive only while
`/product` has never identified the host, because an Arena older than the
endpoint has no such route. Once it has answered (`product_verified`), a 404
means the server changed and is a failure. The settings "Test" button
(`resolume::test_connection`) probes `/product` too, on `dial_port()`. Only on
such an older Arena (404) does it fall back to a `/composition` request.

Never add a timer, a staleness check or a "periodic resync" that reads
`/composition`. The driver fetches it for exactly four reasons (`FetchReason`):

- `missing`: no mapping yet (cold start, config change).
- `error-invalidated`: the #563b threshold (3 consecutive failures) or a port
  drift dropped the mapping. The next tick or push refetches it once; this is
  the "host recovered" refetch. A 1–2 failure blip keeps the mapping and
  refetches nothing.
- `stale-id`: a push got a 404 for a mapped id. `HostDriver::dispatch_push`
  invalidates the mapping without calling `record_error` and retries the push
  once. The retry's `ensure_mapping` refetches inline. A 404 on a FRESH mapping
  pauses stale refetches for 60 s (`STALE_REFETCH_COOLDOWN`). Fresh means the
  push fetched the mapping itself (`last_mapping_refresh` changed during the
  attempt), or the retry still got a 404. While the pause is active those 404s
  are ordinary failures (Error, #484 backoff). `record_error` does NOT let the
  #563b threshold drop the mapping for them, because the mapping is known to
  be fresh. A permanently broken id therefore costs at most one stale refetch
  per 60 s pause, never one per push. The host may flip between probe-green
  and push-red meanwhile, which is honest: those pushes really fail.
- `manual`: the operator. The settings card's "Refresh mapping" button posts
  `POST /integrations/resolume/hosts/{id}/refresh-mapping`, which goes through
  `ResolumeRegistry::refresh_mapping` and `HostCommand::RefreshMapping` to
  `HostDriver::manual_refresh`. It runs inside a backoff window too. The UI
  allows one refresh at a time. This is how a clip edit in Arena reaches
  Presenter now; the operator chip's missing-clips tooltip says so.

## When you add or change a push path

- **Map a 404 to `StaleIdError`.** Every new PUT `/parameter/by-id/{id}` or POST
  `/composition/clips/by-id/{id}/connect` must return
  `StaleIdError::text_parameter(id)` / `StaleIdError::clip(id)` on a 404, as
  `put_text_param_future` (handlers.rs) and `trigger_clips` (driver.rs) do.
  A plain `anyhow!` makes a composition change look like a host failure:
  Error, backoff, and no refetch.
- **Keep the push idempotent up to the point it succeeds.** The retry runs
  the whole push again. That is safe only because lane flips
  (`lane_state.flip`) and dedup payloads (`last_*_payload`) change only after
  their PUT/connect succeeded. Never flip a lane or record a dedup payload
  before the request that justifies it has returned OK.
- **Dedup across a refetch:** `reset_dedup_for_changed_ids` re-sends the timer,
  song and band text when their param ids changed, or when there was no
  mapping to compare with. The stale-id, recovery and cold paths drop the
  mapping first, so they always re-send the text once (identical text, no
  flicker). A manual refresh compares the ids, so unchanged ones keep their
  dedup. A new deduped metadata slot belongs in that function.
- **Telemetry:** a stale retry runs `handle_stage` twice. So one push writes
  two audit rows: `error: Resolume has no … (404 Not Found) …`, then `ok`
  with `refetched=true`. That is intended: the first attempt really failed.

## Worker loop

`run_host_worker`'s `select!` is `biased` toward commands. When a command and
a tick are both ready, the command runs first. A push that arrives while a
probe is in flight still waits for it, up to `LIVENESS_TIMEOUT` (5 s). The
interval uses `MissedTickBehavior::Delay`, so a long push burst does not leave
a queue of missed ticks that probe back to back.

## Testing

- `resolume/mapping_refresh_tests.rs` drives `tick()` and `dispatch_push()`
  directly against `MockArena`, a wiremock Arena with a swappable composition
  and an online flag. It counts the requests it received. Skip a backoff
  window with `driver.next_retry_at = None`, never with a sleep.
- An UNMOUNTED wiremock route answers 404. That is how the tests model a stale
  id: never mount the old id's route.
- Every mock Arena must serve `/api/v1/product`: the embedded
  `mock_integrations/resolume.rs` does, and so does the Playwright
  `startMockResolume` (tests/e2e/support.ts), which also has
  `requestCount(method, path)`.
- The E2E `settings.spec.ts` "refresh mapping re-reads the composition only on
  demand" test proves it end to end: one composition read at the cold start,
  `/product` on the ticks, exactly one more read per button click.
