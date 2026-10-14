---
paths:
  - "crates/presenter-server/src/ableset.rs"
  - "crates/presenter-server/src/resolume/**"
  - "crates/presenter-ndi/src/manager/**"
  - "crates/presenter-ndi/src/pipeline/build.rs"
  - "crates/presenter-ndi/src/pipeline/encoder_timing.rs"
  - "crates/presenter-server/src/state/integrations.rs"
  - "crates/presenter-server/src/state/background_tasks.rs"
  - "crates/presenter-server/src/android_stage.rs"
---

# Down-dependency log floods → the resolume-driver #484 backoff + power-of-two gate

When code polls or retries against an EXTERNAL dependency that can be unreachable (AbleSet HTTP,
Resolume HTTP, an NDI lock, an ADB device), a naive "log every failed attempt" loop floods journald
and can evict retention — real incidents: resolume 163,943 lines (#484), AbleSet 244,039 lines
(#735), `pipeline_snapshots` lock-timeout thousands of lines (#736). The canonical fix in this repo
is the pattern in `crates/presenter-server/src/resolume/driver.rs`:

- **`backoff_interval(consecutive_failures)`** — `Duration::ZERO`/base at 0, else exponential
  `base * 2^(n-1)` with `shift.min(32)` + `saturating_mul` (overflow-guarded), capped at a
  `BACKOFF_CAP`. Drives the *retry cadence* so a down host is not re-hit every tick.
- **`should_log_error(consecutive_failures)`** = `n > 0 && n.is_power_of_two()` — logs the 1st
  failure + power-of-two milestones only (~log2(N)+1 lines instead of N). Keep the log at WARN so a
  genuinely-down dependency stays visible on the default `info` `RUST_LOG`, but bounded.
- Reset the streak to 0 on any success/reachable response, and log ONE recovery line when a streak
  ends (gate the recovery log on `streak > 0`).

Reuse it verbatim, don't reinvent it. Both pure helpers are trivially unit-tested without sleeping
(assert the schedule + the power-of-two gate over 0..9). Existing reuses to copy from:
`ableset.rs` (`ableset_backoff_interval` / `should_log_ableset_failure`, #735) and
`manager/whep.rs` (`should_log_contention` on an `AtomicU32` streak field of `NdiManager`, #736 —
for a shared-across-callers counter use `AtomicU32` + `Ordering::Relaxed`, `fetch_add(1).saturating_add(1)`
for a 1-based count).

**comprehensive-logging discipline:** fix the flood by RATE-LIMITING, never by dropping the log
level to hide it or stripping the useful signal. The WARN keeps its full diagnostic text; only its
frequency changes.

**Deep vs shallow fix (bundling):** the log rate-limit is the bundle-safe fix for the flood. The
UNDERLYING contention/retry-storm (e.g. `#741`: `start_pipeline` holding `active` across an 8 s
wait; `#740`: the debug-keystore-per-CI-run behind `adb install -r` failing) is usually a
cross-cutting/concurrency or build change — split it to its own ticket, don't bundle it with the
log fix.

## Repeats driven by a TIMER, not a poll loop: count the streak where the OUTCOME is decided (#858)

SNV's silent Resolume made the 30 s NDI reconnect ticker rebuild a whole pipeline every tick and log
5 INFO lines each (600/h — the entire journal). The repeats were invisible to every log site: the
count has to live where the outcome is decided and travel IN the outcome.

- **The streak:** `NdiManager.silent_streaks` (`presenter-ndi/src/manager/silent_streak.rs`) counts
  consecutive `SourceSilent` outcomes per source and hands it out as
  `PipelineStartError::SourceSilent { attempt }`. It ends when the source reaches Streaming
  (`reset`, on every promote), on delete (`stop_pipeline` → `reset`), on deactivate
  (`stop_all` → `clear`) and on a switch / leftover reap (`stop_other_pipelines` →
  `retain_only(keep)`, which keeps the kept source's streak).
- **The gate:** `should_log_silent_attempt(attempt)` (= `attempt.is_power_of_two()`, pure, unit-tested)
  — INFO on attempts 1,2,4,8,..., DEBUG in between, on every line about a silent attempt (the
  server's #448 line carries `attempt=N`). A 24 h night is 12 INFO lines, not 14 400.
- **Build-time lines are DEBUG** (`pipeline built`, `pinned to system monotonic clock`, `timestamp guard
  installed`): a rebuild that never reaches Streaming has nothing worth an INFO line. The recovery
  line is ONE INFO `NDI pipeline streaming encoder=… timestamp_guard=… silent_attempts_before=N`,
  logged on every promote.
- **Recovery = ONE INFO line**, the manager's `NDI pipeline streaming`. The ticker's repair line
  (`re-activated the source's pipeline`) is DEBUG and reported only when the repair produced a
  STREAMING pipeline — `reconnect_active_video_source` returns `Some` only then.
- **Cadence unchanged.** A backoff on the 30 s repair would delay the picture after Resolume starts
  (acceptance: <= 30 s), and keeping a built pipeline waiting past the 8 s budget changes the #741
  reservation state machine — both rejected in #858. One VA-API init per 30 s on an idle box is accepted.
- **Asserting log lines in a test:** a scoped capture subscriber loses events when a sibling test's
  thread registers the callsite first while only one dispatcher is live (tracing-core caches the
  interest). Keep a SECOND live `Dispatch` for the test's duration — `capture_events()` in
  `state/integrations.rs` tests (`silent_streak_logs`).
