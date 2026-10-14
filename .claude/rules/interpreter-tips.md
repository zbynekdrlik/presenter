---
paths:
  - "crates/presenter-core/src/interpreter_tips.rs"
  - "crates/presenter-migration/src/m20261014_000001_create_interpreter_tips.rs"
  - "crates/presenter-persistence/src/entities/interpreter_tip.rs"
  - "crates/presenter-persistence/src/repository/interpreter_tips.rs"
  - "crates/presenter-server/src/state/interpreter_tips.rs"
  - "crates/presenter-server/src/state/interpreter_tips/**"
  - "crates/presenter-server/src/router/interpreter_tips.rs"
  - "crates/presenter-ui/src/pages/interpreter.rs"
  - "crates/presenter-ui/src/pages/sepkar.rs"
  - "crates/presenter-ui/src/components/interpreter/**"
  - "crates/presenter-ui/src/ws/passive.rs"
  - "crates/presenter-ui/src/state/interpreter.rs"
  - "crates/presenter-ui/styles/interpreter.css"
  - "crates/presenter-ui/styles/sepkar.css"
  - "tests/e2e/interpreter-tips.spec.ts"
---

# Šepkár tips (#852 spec §9, units S1a/S1b)

- **One source for the rules:** core `interpreter_tips.rs` (limits, `clean_text`, `validate_tip`,
  `is_active`, `find_recent_duplicate`). The server is the only validator; the pages read the same
  constants for counters but never block a send.
- **Site-local:** `interpreter_tips` has NO sync columns and the SNV ↔ PP sync only knows
  presentations + libraries — never add the table to `state/sync.rs`. The dev deploy copies prod's
  DB (tips included) to dev.
- **Dates:** `service_date` is TEXT `YYYY-MM-DD` (the server's LOCAL date, `TipClock`). Filter by
  equality / lexical order (active list, duplicate check, retention `service_date < today − 30`);
  compare timestamps in Rust, never in SQL.
- **Refusals:** validation + rate limits are decided in the state layer → `TipRefusal`, mapped in
  `router/interpreter_tips.rs::map_tip_error` (422 Slovak rule / 429 + `Retry-After`). Unknown id
  (404) and another day's restore (409) are `RepositoryError` through the central mapping. Never
  route the state refusals through `RepositoryError`.
- **Create order:** validate → duplicate → rate limit → insert → publish. A 422 or a duplicate never
  uses a rate slot (pinned by `invalid_tips_are_422_with_the_slovak_rule`). Duplicate check, rate
  limit and insert run under `create_lock`, one create at a time: without it two quick taps both
  read the list before either insert and both pass (pinned by
  `two_creates_of_the_same_tip_at_once_store_one_row`). The lock is NOT `publish_lock`, and the
  publish happens after the lock is released.
- **Duplicate:** the pair AND its note (a bare pair keys on the pair alone, a note-only tip on the note),
  lower-cased, whitespace collapsed, sent LESS than 2 min ago by anyone and NOT dismissed → 200 +
  `duplicate: true` + the earlier tip; no row, no event. A hidden tip may be sent again, and a resend
  with a corrected or added note is a new tip.
- **Rate limit:** exact sliding window per (client IP, lower-cased name) 5 / 60 s + 30 / 60 s overall,
  tokio `Instant`, in memory (reset on restart). One WARN per 60 s per key with `suppressed`. The
  limiter itself is pure (`state/interpreter_tips/rate_limit.rs`, the caller passes `now`), so its
  tests use explicit instants and never sleep. A key that still holds refused-but-unreported tips
  (`suppressed > 0`) is never forgotten: its next WARN carries the count.
- **Dismiss / dismiss-all** are single conditional `UPDATE … WHERE dismissed_at IS NULL` calls in the
  repository (first time wins, no read-then-write transaction).
- **Live event:** `InterpreterTips` = FULL active list, newest first, capped at 20, after every write.
  Publishes hold `publish_lock` (two writes could otherwise end on the older list). GET is not capped.
- **Boundary + retention:** one minute task from `spawn_background_tasks` (never in validate mode):
  it publishes once at start (seeds the published ids, so a restart mid-service still sends the
  boundary event), then each minute runs retention once per service date and the tick, which
  re-publishes when the active ids changed. The tick reads nothing while the last published list is
  empty — EXCEPT after a failed publish: `publish_interpreter_tips` sets `publish_stale` on a failure
  (and clears it on success, both under `publish_lock`), and the tick publishes while it is set, so a
  failed seed or create publish is never lost behind an empty `published` (pinned by
  `a_failed_publish_is_retried_by_the_next_tick_even_when_nothing_was_published`). Tick failures WARN on #484 power-of-two streaks and log ONE recovery line (pinned by
  `a_failing_boundary_check_warns_on_power_of_two_streaks_and_logs_the_recovery`, which takes the
  table away with `ALTER TABLE … RENAME` through `repository().connection()`).
- **Clock in tests:** `test_support::ManualTipClock::at("…+02:00")` + `set_interpreter_tip_clock`;
  hub assertions via `test_support::last_tips_event(&mut rx)` (heartbeats interleave). A test that
  sends more than 5 tips varies the name or the `x-forwarded-for` IP.
- **Client IP** = `integrations::extract_client_ip` (#793). `sender_ip` / `user_agent` are audit
  columns and never appear in a DTO.
- **Logs:** INFO `presenter::interpreter::tip` (`action` create/dismiss/restore/dismiss_all, `id`,
  `sender_name`, `client_ip`, `chars`, `duplicate`), pinned with `tracing_capture::Capture`. A
  duplicate create logs the REQUESTER's `sender_name` and `client_ip` plus `duplicate_of` (the earlier
  tip's id), never the earlier sender. The
  routes are person-triggered (no timer polling), so `router/request_log.rs` quiet lists stay as they
  are.

## Pages (S1b): /ui/sepkar and /ui/interpreter

- **Socket = `ws::passive::use_passive_websocket`** (per event, presence-free, "connected" on the
  first frame, 5 s silent → reconnect, back after > 30 s hidden → reconnect, every attempt numbered
  so an older socket / timer stands down). Never `use_live_websocket` (its latest-value slot
  coalesces) and never the stage socket (it registers a stage display).
- **Resync** = `GET /interpreter/tips` on load and on every `connects` change; `FetchGuard` lets only
  the newest fetch land and drops it when a live list or an own write arrived meanwhile.
- **The interpreter page is not a stage layout.** `?layout=tv` shows a notice until C5b. Areas render
  only with data: `data-transcript="absent"` makes the side column the whole page; C5a adds its slots
  to `TabletLayout`, never a second page.
- **Tips area:** keyed `<For>` by id (tips are immutable). Highlight = `data-new` fixed at mount + a
  CSS animation (no timer). Swipe = pointer events with `touch-action: pan-y`; the card captures the
  pointer so a mouse drag that leaves it still ends, but NEVER on a press that starts on ✕ (capture
  would retarget that click). A hide is optimistic: `pending_dismiss` keeps the tip out of every list
  until its POST answered, and a Späť pressed meanwhile waits for it (`restore_after`). The undo
  offer is a ticketed 5 s `Timeout` (`try_` signal calls inside timers).
- **Errors:** the tips client's writes use `api::post_json_refusal`: a non-2xx whose body IS the
  server's JSON error body is `ApiError::Refused(code, message)`, any other body (a proxy / gateway
  page) stays `ApiError::Status`. `state::interpreter::failure_status` maps that to the helpers'
  input (the text is empty for a `Status`), so the tablet's `tip_error_text` (server text only for a
  404 / 409 / 422 / 429 refusal) and the šepkár page's `sepkar_error_text` (verbatim for a 422 / 429
  refusal only; the fields are kept) show a fixed Slovak line for a proxy page, never its text. The
  other `*_detail` helpers and their `ApiError::Status` callers are unchanged.
- **Šepkár page:** never blocks a send. The page's ROOT element is `.sepkar__root` — the `<body>` itself
  carries the class `sepkar` (like `interpreter`), so a bare `.sepkar` rule would also style the body.
  A success clears only the fields still holding what was sent (the šepkár may have typed on) and
  refocuses Slovo only when none was touched. Enter: name → Slovo → Preklad; Preklad / Poznámka send
  (an IME-composition Enter never does). User text is a text node, never HTML. A failed "Dnes" fetch
  shows „Zoznam sa nepodarilo načítať." (only the NEWEST fetch may set or clear it) and never the
  "žiadne tipy" text; row states come from `row_states`: only the newest 20 ACTIVE tips are "na
  obrazovke" (the interpreter's list is capped), further active ones read "mimo obrazovky (max. 20)".
- **CSS** is bundled globally: scope to `body.sepkar` / `body.interpreter`. `tablet.css` sets
  `html { overflow: hidden }`, so `body.sepkar` is the scroll box. Dimmed rows keep ≥ 4.5:1 (dim the
  text colour, never an `opacity` on the row).
- **E2E (`interpreter-tips.spec.ts`):** one server per file, `beforeEach` dismiss-all, a distinct
  sender name per test, ≤ 30 accepted tips per minute across the file (21 today). The šepkár list
  shows ALL of today's tips, dismissed ones too, so every word a test looks for there is unique across
  the file. Measure a delivery bound (T-S1, 1 s) with `expect.poll(…, { intervals: [25] })`, never
  `toBeVisible`, whose own back-off (~0 / 100 / 350 / 850 ms) would blur it. A client fetch failure
  without console noise = a malformed 200 (`route.fulfill({ status: 200, body: "not-json" })`). 422 / 429 come from the REAL server (empty / over-long note, a 6th tip in a minute):
  Chrome logs each as "Failed to load resource", so that test asserts exactly one 422 line and one 429
  line and an otherwise empty console. Device profiles + fit helpers (`PHONE_390`, `TABLET_1180`,
  `assertNoHorizontalOverflow`, `assertMinFontSize`, `assertMinTapTarget`, `touchDrag`) live in
  `support.ts`. Lost frames: `page.routeWebSocket` + `close({code: 4000})`.
