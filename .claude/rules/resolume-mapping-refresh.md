---
paths:
  - "crates/presenter-server/src/resolume/driver.rs"
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
`port_drift::is_resolume_product_body`. A 404 counts as alive, because an
Arena older than `/product` has no such endpoint. Never add a timer, a
staleness check or a "periodic resync" that reads `/composition`. The
composition is fetched for exactly four reasons (`FetchReason`):

- `missing`: no mapping yet (cold start, config change).
- `error-invalidated`: the #563b threshold (3 consecutive failures) or a port
  drift dropped the mapping. The next tick or push refetches it once; this is
  the "host recovered" refetch. A 1–2 failure blip keeps the mapping and
  refetches nothing.
- `stale-id`: a push got a 404 for a mapped id. `HostDriver::dispatch_push`
  invalidates the mapping without calling `record_error` and retries the push
  once. The retry's `ensure_mapping` refetches inline. If the retry on a FRESH
  mapping still gets a 404, stale refetches pause for 60 s
  (`STALE_REFETCH_COOLDOWN`). Those 404s then ride the normal threshold and
  backoff, so a broken id cannot turn every push into a 16 MB fetch.
- `manual`: the operator. The settings card's "Refresh mapping" button posts
  `POST /integrations/resolume/hosts/{id}/refresh-mapping`, which goes through
  `ResolumeRegistry::refresh_mapping` and `HostCommand::RefreshMapping` to
  `HostDriver::manual_refresh`. It runs inside a backoff window too. This is
  how a clip edit in Arena reaches Presenter now.

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
  mapping to compare with. Unchanged ids keep their dedup, so nothing
  flickers. A new deduped metadata slot belongs in that function.

## Worker loop

`run_host_worker`'s `select!` is `biased` toward commands: a queued lyric line
never waits behind a probe, and pushes mark the host connected themselves. The
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
