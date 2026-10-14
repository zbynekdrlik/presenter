---
paths:
  - "crates/presenter-core/src/interpreter_tips.rs"
  - "crates/presenter-migration/src/m20261014_000001_create_interpreter_tips.rs"
  - "crates/presenter-persistence/src/entities/interpreter_tip.rs"
  - "crates/presenter-persistence/src/repository/interpreter_tips.rs"
  - "crates/presenter-server/src/state/interpreter_tips.rs"
  - "crates/presenter-server/src/state/interpreter_tips/**"
  - "crates/presenter-server/src/router/interpreter_tips.rs"
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
  tests use explicit instants and never sleep.
- **Dismiss / dismiss-all** are single conditional `UPDATE … WHERE dismissed_at IS NULL` calls in the
  repository (first time wins, no read-then-write transaction).
- **Live event:** `InterpreterTips` = FULL active list, newest first, capped at 20, after every write.
  Publishes hold `publish_lock` (two writes could otherwise end on the older list). GET is not capped.
- **Boundary + retention:** one minute task from `spawn_background_tasks` (never in validate mode):
  it publishes once at start (seeds the published ids, so a restart mid-service still sends the
  boundary event), then each minute runs retention once per service date and the tick, which
  re-publishes when the active ids changed. The tick reads nothing while the last published list is
  empty. Tick failures WARN on #484 power-of-two streaks and log ONE recovery line (pinned by
  `a_failing_boundary_check_warns_on_power_of_two_streaks_and_logs_the_recovery`, which takes the
  table away with `ALTER TABLE … RENAME` through `repository().connection()`).
- **Clock in tests:** `test_support::ManualTipClock::at("…+02:00")` + `set_interpreter_tip_clock`;
  hub assertions via `test_support::last_tips_event(&mut rx)` (heartbeats interleave). A test that
  sends more than 5 tips varies the name or the `x-forwarded-for` IP.
- **Client IP** = `integrations::extract_client_ip` (#793). `sender_ip` / `user_agent` are audit
  columns and never appear in a DTO.
- **Logs:** INFO `presenter::interpreter::tip` (`action` create/dismiss/restore/dismiss_all, `id`,
  `sender_name`, `client_ip`, `chars`, `duplicate`), pinned with `tracing_capture::Capture`. The
  routes are person-triggered (no timer polling), so `router/request_log.rs` quiet lists stay as they
  are.
