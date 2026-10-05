---
paths:
  - "crates/presenter-server/src/resolume/driver.rs"
  - "crates/presenter-server/src/resolume/mod.rs"
  - "crates/presenter-server/src/resolume/mapping_refresh*.rs"
  - "crates/presenter-server/src/resolume/provisional_*.rs"
  - "crates/presenter-server/src/resolume/keepalive_tests.rs"
  - "crates/presenter-server/src/resolume/handlers.rs"
  - "crates/presenter-server/src/resolume/bible_clear.rs"
  - "crates/presenter-server/src/resolume/port_drift.rs"
  - "crates/presenter-server/src/mock_integrations/resolume.rs"
  - "crates/presenter-ui/src/pages/settings/resolume.rs"
  - "tests/e2e/support.ts"
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
`/composition`. The driver fetches it only for these reasons (`FetchReason`).
The last three are the 2026-10-05 regression fix, see "A fetched mapping can
be wrong with no 404" below:

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
  Presenter now; the operator chip's missing-clips tooltip says so. A manual
  result with at least one recognized clip becomes that deck's last-good
  reference, so an intentional clip removal stops being "suspect".
- `follow-up`: a step of the follow-up schedule of a suspect mapping.
- `lane-missing`: a stage/Bible push needed a lane the cached mapping lacks
  but should have (`LaneRefetch` limits it).
- `deck-changed`: the deck check found the cached deck no longer selected.

## A fetched mapping can be wrong with no 404 (#808 regression, 2026-10-05)

The on-demand model assumes every fetched mapping is right until a push gets
a 404. Two cases break that assumption, and in both a push is skipped
silently. `update_lane_text` logs `Resolume has no clips configured for lane`
and returns `Ok`, so no request is sent, no 404 comes back, and nothing
refetches:

- **A cold or recovery fetch can hit Arena while it is still loading the
  composition.** On PP, right after an Arena restart, the `error-invalidated`
  fetch got 27 tag-less clips (110 KB) instead of 874. Every lyric push was
  skipped until someone pressed "Refresh mapping".
- **A Resolume deck switch changes every clip id.** `/composition` lists only
  the SELECTED deck's clips (SNV: 25 decks, 1221 clips). The cached ids then
  point at a deck that is no longer on the wall. Writes to them succeed (the
  clips still exist), so the lines simply go nowhere.

The fix lives in `provisional_mapping.rs`. It never reads the composition on
a timer:

- **Every fetch records `decks[].selected.value`** (`selected_deck_id`) and
  compares its destination kinds (`ClipMapping::destination_kinds`, the names
  in the missing-clip list) with the last good kinds of that deck.
  - The last good kinds are kept per deck, because decks legitimately differ.
  - A body without `decks` (Arena mid-load) is judged against the deck
    selected before (`reference_deck`). It has no deck to check
    (`selected_deck` is `None`). After bodies with decks, such a body is
    suspect even when it has every kind (`deckless_after_decks`): the
    follow-ups bring the deck list, and so the deck check, back.
  - The mapping is *suspect* if it lacks a kind the deck had. A deck without
    history is suspect only if it has no recognized destination at all.
  - While a mapping is suspect, follow-up fetches run `FOLLOW_UP_DELAYS`
    apart: 2, 5, 15, 30 and 60 s, at most 5 per episode. The deadline is the
    worker's `select!` branch `follow_up_deadline`, not a timer. The schedule
    stops at the first complete mapping.
  - A step that falls due inside a #484 backoff window waits until
    `next_retry_at`. A failed step is used up.
  - Only `missing`, `error-invalidated`, `stale-id`, `deck-changed` and an
    empty manual result start the schedule. `follow-up` advances it.
    `lane-missing` never restarts it.
  - **Settling.** After the last step, a composition becomes the deck's
    reference (`Settled`, one WARN) when the last two FOLLOW-UP fetches, about
    a minute apart, saw the same kinds and the same clip count
    (`last_follow_up`). That record resets when a schedule starts, on a
    complete or adopted fetch, and on a deck change. A lane refetch in
    between does not count. Settling also requires recognized clips, or a
    listed deck without history (a video deck). This stops a deck whose clips were removed on purpose,
    or a deck with no presenter clips, from costing 5 follow-ups on every
    visit.
  - A deck that HAD clips and still shows none stays suspect: that is Arena
    still loading.
  - Residual risk: Presenter cold-starts during an Arena load that takes
    longer than about 2 min, with a loading body that lists a deck id the
    loaded composition keeps. That settles as "no clips" until a deck change
    or Refresh mapping.
  - Residual risk: a cold-start body taken mid-load that already carries SOME
    recognized tags is accepted as complete. The deck has no history to
    compare with, so no follow-ups run and `lane_expected` is false for the
    missing lanes. Lines for those lanes are skipped until a deck change, a
    stale id, a recovery or Refresh mapping. Without per-deck history there
    is nothing to tell "partly loaded" from "this deck has only these
    clips".
- **Deck check.** `GET /composition/decks/by-id/{selected}` (~360 B,
  `ACTION_TIMEOUT`) runs before every stage/Bible push. It also runs on the
  tick, after a successful `/product` probe (`follow_deck_switch`).
  - On the push path it lives in `HostDriver::ensure_mapping_for_push`, which
    `handle_stage` and `handle_bible` call instead of `ensure_mapping`. Its
    time therefore counts as `t_ensure_mapping_ms`, and every fetch it does
    sets the audit row's `refetched`.
  - `selected:false` or a 404 calls `invalidate_mapping(DeckChanged)`.
    `ensure_mapping` then refetches inline (never rate-limited) and the push
    lands on the new deck.
  - If that refetch fails, the push fails like any composition fetch. It
    never writes to the old deck's ids.
  - If the check itself fails, the push goes out on the cached mapping. The
    WARN comes from the #563h limiter, keyed `deck-check`, so at most once
    per 300 s.
  - A 404 for a deck that the fresh composition still selects is a broken
    answer. It costs one refetch, and then that deck's 404s are ignored
    (`contradicted_deck`, one WARN). Otherwise every push and every tick
    would fetch the composition.
  - Timer frames (`handle_timer`, plain `ensure_mapping`) never check the
    deck. A composition without `decks` gets no check at all.
  - Residual risk, deliberately not guarded: Arena answering `selected:false`
    for the deck its own fresh composition selects would refetch on every
    push and tick. Treating such an answer as "contradicted" (as the 404
    case does) would also swallow a real A→B→A switch race, and then lines
    would go to a deck that is off the wall. A 404 is unambiguous (the id
    does not exist), a `selected:false` is not.
- **Lane refetch** (`LaneRefetch`). When the cached mapping was not just
  fetched, a push that needs a destination the mapping lacks refetches
  before it is applied. It does so only if the deck's last good mapping had
  that destination, or the deck has no history and the mapping has no
  destination at all.
  - The refetch runs before the push, not as apply-then-retry. With
    apply-then-retry, a partly mapped push would trigger its main clips
    twice and flip the lane twice.
  - **Armed** means the next such push refetches. The refetch is re-armed in
    `advance_follow_up` by every follow-up step, even a FAILED one, so a
    failed last step can never leave it used up for good. A deck change, a
    complete fetch, a manual refresh and a config change re-arm it too.
  - **RetryAt** comes after a refetch that left the lane empty, or after a
    failed refetch. It allows the next refetch `LANE_REFETCH_RETRY` (**3 s**)
    later, or earlier if a follow-up step comes first.
  - **Owner rule (2026-10-05): no text may be skipped.** The gap was 30 s,
    and a line 5 s after an empty refetch was skipped even though Arena had
    finished loading by then. A provisional mapping (Arena loading, or a deck
    that never produced a complete mapping in this process) now refetches 3 s
    after the previous lane refetch, so a line is skipped for at most 3 s.
    Pinned by `a_push_5_s_after_an_empty_lane_refetch_refetches_and_lands`
    and its cold-start twin.
    - Only a deck ACCEPTED as lacking a kind stops refetching for it: its
      follow-ups ran out on an unchanged composition (`Settled`), or its
      last good mapping simply never had the kind. Its last-good reference
      has no such kind, so `lane_expected` is false and it never refetches.
    - Never raise the gap back "to save fetches". A refetch only happens on a
      line the mapping cannot place, so it is bounded by operator clicks,
      and only while the mapping is provisional.
    - **The one exception is a TIMED-OUT lane refetch** (a hanging
      `/composition`, i.e. host trouble): the next one waits
      `LANE_REFETCH_AFTER_TIMEOUT` (30 s, `lane_refetch_retry_after`).
      - The host worker is serial, and the spend runs after the 15 s
        `COMPOSITION_TIMEOUT`. A 3 s gap would block it 15 s out of every
        18 s.
      - Every queued line would wait behind it: placeable lanes, Bible,
        timer frames. Once the 16-slot command channel is full, `try_send`
        drops updates, which skips exactly the text this rule protects.
      - Fast failures (refused, reset, a 5xx) keep the 3 s gap.
      - A failed refetch never blocks the push; it goes out on the cached
        mapping.
    - **Logs stay bounded at a 3 s refetch rate.**
      - The lane-refetch WARNs ("still lacks these clips",
        "lane refetch failed") go through the #563h limiter, keys
        `lane-refetch` / `lane-refetch-failed`: at most one WARN per 300 s
        per host, DEBUG otherwise.
      - The fetch-time "mapping missing expected clips" WARN fires only when
        the missing list changed since the previous fetch
        (`log_missing_clips`).
  - A failed refetch only logs a WARN, and the push goes out on the cached
    mapping.
- **The per-push lane WARN is rate-limited.** `update_lane_text`'s "Resolume
  has no clips configured for lane" / "lane missing clips" goes through the
  #563h limiter (keys `lane-unconfigured` / `lane-half-missing`): at most
  once per 300 s per host, DEBUG otherwise. A deck without a lane is normal,
  and the fetch-time "mapping missing expected clips" WARN names the kinds.
- **A host or deck that never had a kind never refetches for it.** Bridge PP
  has no `#main` and SNV has no `#translate`. A switch to a deck that lacks
  lyric clips but has other presenter clips (Bible, metadata) is not suspect
  and gets no follow-ups. A deck with no presenter clip at all runs the
  follow-ups once, then settles.

When you touch this area:

- **Never treat a fetched mapping as final.** Every new fetch path must go
  through `refresh_mapping_with_reason`, so `note_fetched_mapping` sees it.
- **A new stage/Bible-style push path calls `ensure_mapping_for_push`** with
  the kinds it writes (`stage_required_kinds` / `bible_required_kinds`),
  never the plain `ensure_mapping`.
- **`fetched_before` (`dispatch_push`) is taken before the attempt.** Any
  fetch inside it (cold start, recovery, deck switch, lane refetch) makes
  the mapping fresh. A 404 on a fresh mapping pauses stale refetches instead
  of fetching again.
- **A new destination kind goes into `clip_map.rs`'s `destinations()` list
  only.** The missing-clip list and the kind comparison both come from it.
- **Mocks: any mock Arena that lists `decks` must serve the deck-by-id
  route.** Otherwise every push sees a 404, that is, a "deck switch", and
  refetches. A mock whose composition has no tagged clip looks exactly like
  Arena mid-load and is re-read on the follow-up schedule. The embedded dev
  mock and `startMockResolume` therefore serve one selected deck plus tagged
  clips.

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

## The HTTP client never reuses a connection

Every Resolume request goes through `resolume_http_client()` (`resolume/mod.rs`):
the host workers through `ResolumeRegistry::new`, and the settings Test button.
It sets `pool_max_idle_per_host(0)`, so hyper-util builds no idle pool and every
request dials a fresh TCP connection. Arena closes idle keep-alive connections
at irregular times (on SNV after 2 min, then after 30 s). With reqwest's default
pool, the next probe, PUT or clip connect went out on a socket Arena was already
closing and failed with `client error (SendRequest): connection closed before
message completed`: 2671 host errors in 2 days on SNV. Each one was an ERROR line,
a status flip, a #484 backoff window that skips pushes, and possibly a lost line.

- Never build a second Resolume `reqwest::Client` with default pooling, and
  never re-enable the pool to save a connect. A LAN connect is sub-millisecond.
- Do not "fix" that error with a retry. hyper-util retries only requests that
  were never written. A request that was written may already have been
  executed by Arena, and a clip `connect` POST is not idempotent: a retry
  re-triggers the clip.
- Do not tune `pool_idle_timeout` either. Arena's close timing is irregular, so
  any timeout is a guess that still races.
- Test shape (`resolume/keepalive_tests.rs`): a raw tokio `TcpListener` mock
  answers the FIRST request on each connection with `Connection: keep-alive`
  and keeps the socket open. If a second request arrives on that connection, it
  closes the socket without answering. A mock that closes right after each
  response does NOT reproduce the bug: the FIN reaches hyper before the next
  checkout, and the pool drops the connection itself. The driver must run on
  `ResolumeRegistry::new()`'s own `client`, not a client the test builds,
  otherwise the test pins nothing. One test goes through `set_hosts` +
  `stage_update`, so the client that `spawn_host` hands each worker is
  pinned too.

## Worker loop

`run_host_worker`'s `select!` is `biased`, in this order: commands, the
follow-up deadline (a pending future when nothing is scheduled), then the
tick. When a command and a tick are both ready, the command runs first. A push that arrives while a
probe is in flight still waits for it, up to `LIVENESS_TIMEOUT` (5 s). The
interval uses `MissedTickBehavior::Delay`, so a long push burst does not leave
a queue of missed ticks that probe back to back.

## Testing

- `resolume/mapping_refresh_tests.rs` drives `tick()` and `dispatch_push()`
  directly against `MockArena`, a wiremock Arena with a swappable composition
  and an online flag. It counts the requests it received. Skip a backoff
  window with `driver.next_retry_at = None`, never with a sleep.
- `resolume/provisional_mapping_tests.rs` provides `DeckArena`: several
  decks, `select(i)`, `replace_decks` (new ids, so the old id is a 404),
  `set_loading(n)` (the next n composition GETs return the tag-less 27-clip
  "loading" composition) and `fail_deck_checks(status)`.
  `restart_arena_mid_load` reproduces the PP incident: 3 failed ticks, then a
  recovery fetch mid-load.
  `resolume/provisional_schedule_tests.rs` reuses `DeckArena`. It calls
  `run_follow_up` directly, the same call the worker's `select!` makes at the
  deadline, and asserts the deadline it set (`assert_follow_up`). Two
  worker-level tests sleep for real, about 2 s each, waiting for the first
  follow-up: `a_worker_started_while_arena_loads_…` and
  `the_worker_waits_for_the_follow_up_deadline`. Simulate an elapsed
  `LANE_REFETCH_RETRY` by setting
  `driver.provisional.lane_refetch = LaneRefetch::RetryAt(Instant::now())`,
  never by sleeping. DeckArena also has `fail_compositions(n)` (the next n
  composition GETs answer 500) and `set_lists_decks(false)` (bodies without
  `decks`).
- An UNMOUNTED wiremock route answers 404. That is how the tests model a stale
  id: never mount the old id's route.
- Every mock Arena must serve `/api/v1/product`: the embedded
  `mock_integrations/resolume.rs` does, and so does the Playwright
  `startMockResolume` (tests/e2e/support.ts), which also has
  `requestCount(method, path)`.
- The E2E `settings.spec.ts` "refresh mapping re-reads the composition only on
  demand" test proves it end to end: one composition read at the cold start,
  `/product` on the ticks, exactly one more read per button click.
