# Resolume click-path instrumentation + timer flicker fix

**Issues:** #273 (resolume latency on slide click), #267 (resolume timer glitches)

**Date:** 2026-05-03

**Branch:** dev (workspace 0.4.56)

## Problem

The user reports two perceived issues during live worship services:

1. **#273** — Click-to-slide latency on Resolume "is still big". Anecdotal target: at least 100 ms reduction.
2. **#267** — The Resolume timer flickers; the user suspects empty text being sent between seconds.

Production data from `GET /integrations/resolume/hosts` on win-resolume (2026-05-03) shows `lastLatencyMs ≈ 0.47 ms` per text-parameter PUT. That means parallelizing the four sequential PUTs in `handle_stage` would save under 2 ms and dropping `TRIGGER_DELAY = 35 ms` would save 35 ms — combined, far below the 100 ms target. The latency is not in the resolume driver. It is somewhere else on the click path.

For #267, code reading rules out empty-string sends (`format_countdown_text` always returns non-empty). The two plausible causes are (a) `last_timer_payload` being reset on transient errors so the same value gets re-PUT after a network blip, and (b) sequential PUTs to multiple `#timer` clips landing in different render frames.

## Goal

Ship one PR that:

- Instruments the entire slide-click path with timing logs so the next PR can target the real bottleneck.
- Fixes the two confirmed root causes of #267 in the resolume worker.

The latency optimization itself is **deferred to a follow-up PR** that will be informed by the instrumentation data.

## Non-goals

- No parallelization of `handle_stage`'s sequential calls in this PR.
- No change to `TRIGGER_DELAY` in this PR.
- No change to `MAPPING_CACHE_TTL` in this PR.
- No new HTTP endpoint for metrics; logs only.

## Architecture

### One structured log line per click

Emit a single `tracing::info!` "stage click timing" line at the end of `update_stage_state`. The line carries a correlation UUID and timing fields for each step on the click path. Output goes to tracing INFO, which is already piped to journald on dev and production.

A second log line "resolume stage timing" is emitted from each resolume host worker after handling that click's `StageUpdate`. Both lines share the correlation ID so they can be joined with grep.

### Click-path checkpoints (in `update_stage_state` and downstream)

| Field | Measures |
|---|---|
| `correlation_id` | UUID v4 generated at the entry of `update_stage_state` |
| `t_validate_ms` | `presentation_detail` + slide membership checks |
| `t_db_write_ms` | `upsert_stage_state` |
| `t_build_ctx_ms` | `build_stage_context` (this is where extra DB queries can hide) |
| `t_resolume_enqueue_ms` | total time inside `resolume_registry.stage_update` (the `try_send` fanout to host workers) |
| `t_live_publish_ms` | `live_hub.publish` for the `LiveEvent::Stage` |
| `t_total_ms` | wall time from start of `update_stage_state` to last broadcast |

The correlation ID is plumbed into `StageUpdate` (a new `correlation_id: Option<Uuid>` field) so the worker can echo it back.

### Resolume worker checkpoints (in `handle_stage`)

| Field | Measures |
|---|---|
| `correlation_id` | echoed from `StageUpdate` |
| `t_queue_wait_ms` | from when `try_send` was called (recorded in `StageUpdate.enqueued_at: Instant`) to when `handle_stage` starts |
| `t_ensure_mapping_ms` | `ensure_mapping` (covers cache hit and the `GET /api/v1/composition` if it misses) |
| `t_main_ms` | `update_lane_text(main, ...)` total |
| `t_trans_ms` | `update_lane_text(translation, ...)` total |
| `t_song_ms` | `update_metadata_targets(song_name, ...)` total — `0` when deduped |
| `t_band_ms` | `update_metadata_targets(band_name, ...)` total — `0` when deduped |
| `t_trigger_delay_ms` | `TRIGGER_DELAY` constant |
| `t_trigger_ms` | `trigger_clips` total |
| `t_total_ms` | wall time from worker pickup to end of `handle_stage` |

### How to read the data

After clicking 5 slides on dev:

```bash
sudo journalctl -u presenter-dev --since "5 minutes ago" \
  | grep -E "stage click timing|resolume stage timing"
```

Each click produces two lines (one server-side, one per resolume host). Numbers reveal the dominant cost. The follow-up PR targets that.

## #267 timer-flicker fix

Bundled in the same PR because both causes are confirmed and small.

### Cause 1: `last_timer_payload` reset on transient errors

In `crates/presenter-server/src/resolume/driver.rs` the field `last_timer_payload` is reset to `None` in three places: connection lost, mapping cache invalidated, and on any `record_error`. The first and third cases reset the dedup state on transient blips, so the next tick (which carries the same formatted text) re-PUTs the same value. Resolume's text parameter then re-renders mid-frame, visible as a flicker.

**Fix:** Only reset `last_timer_payload` when the mapping itself changes (different param IDs for the `#timer` clip). Keep the value across transient errors and across `record_error` calls. The next valid tick will compare correctly and skip the PUT.

The same logic applies to `last_song_name_payload` and `last_band_name_payload` for the same reason. Reset all three only when the mapping resolves to different param IDs.

### Cause 2: sequential multi-clip PUTs

When two or more `#timer` clips are mapped, `handle_timer` does:

```rust
for target in &mapping.timer {
    self.update_clip_text(target, &text, &endpoint).await?;
}
```

Each PUT lands in a different Resolume render frame, so the two timer surfaces are momentarily out of sync — a flicker.

**Fix:** Parallelize the loop using `FuturesUnordered`, the same pattern `trigger_clips` already uses (driver.rs). This applies to `handle_timer`, `update_lane_text`, and `update_metadata_targets`.

The `update_lane_text` and `update_metadata_targets` change is a minor cleanup that aligns with the existing `trigger_clips` style. It is not a latency fix in this PR — production data shows N ≈ 0.5 ms — but it is the right shape and prevents future flicker if more clips per lane get added.

## File-level changes

| File | Change |
|---|---|
| `crates/presenter-server/src/resolume/mod.rs` | Add `enqueued_at: Instant` and `correlation_id: Option<Uuid>` to `StageUpdate`. Plumb the ID into `stage_update`. |
| `crates/presenter-server/src/state/broadcasting.rs` | Generate a `Uuid::new_v4()` per click, pass into `StageUpdate`, time each step with `Instant::now()`, emit "stage click timing" `tracing::info!` line. |
| `crates/presenter-server/src/state/mod.rs` (`update_stage_state`) | Wrap with timing measurement; receive correlation ID at entry; thread to `broadcast_stage_resolution`. |
| `crates/presenter-server/src/resolume/handlers.rs` | In `handle_stage`: record per-step durations, emit "resolume stage timing" line. In `handle_timer`: parallelize multi-clip loop. In `update_lane_text` and `update_metadata_targets`: parallelize multi-clip loops with `FuturesUnordered`. |
| `crates/presenter-server/src/resolume/driver.rs` | Stop resetting `last_timer_payload`, `last_song_name_payload`, `last_band_name_payload` on `record_error` and on the `endpoint`-clear path. Reset only when mapping resolves to different param IDs. |
| `crates/presenter-server/src/resolume/tests.rs` | New tests: dedup state preserved across transient errors; multi-clip timer issued in parallel; instrumentation log line shape sanity. |

## Tests

### Unit tests (presenter-server, mockito + wiremock as already used)

1. `last_timer_payload_preserved_across_transient_error`
   - Set up driver with mapping resolved.
   - Send timer "12:30" → assert `last_timer_payload = Some("12:30")`.
   - Force a transient `record_error` (e.g. simulate fetch failure).
   - Assert `last_timer_payload` still equals `Some("12:30")`.
   - Send the same timer "12:30" again → assert no PUT issued (mocked HTTP server records 0 new calls).

2. `mapping_change_resets_dedup`
   - Set up driver with mapping A.
   - Send timer "12:30" → dedup populated.
   - Force mapping refresh that resolves to mapping B (different `#timer` param IDs).
   - Assert all three `last_*_payload` reset to `None`.

3. `multi_clip_timer_issued_in_parallel`
   - Mock Resolume mapping with two `#timer` clips.
   - Use a wiremock with a 50 ms delay on PUT responses.
   - Send one timer frame.
   - Assert total wall time < 75 ms (sequential would be > 100 ms). Tolerance: 25 ms.

4. `multi_clip_lane_issued_in_parallel`
   - Same shape as test 3 for `update_lane_text`.

### Manual verification (post-deploy)

- Deploy to dev.
- Click 5 worship slides through the operator UI.
- `sudo journalctl -u presenter-dev --since "2 minutes ago" | grep -E "stage click timing|resolume stage timing"`.
- Paste a sample log line into the PR description so the follow-up PR has data to work from.

## Out of scope (already deferred)

- Latency optimization itself (parallelizing `handle_stage`'s top-level calls, dropping `TRIGGER_DELAY`, raising `MAPPING_CACHE_TTL`). The follow-up PR will be informed by the instrumentation data and can be near-trivial in code if the data points to one of these.
- A diagnostics HTTP endpoint surfacing recent click metrics. Logs are enough for now.

## Acceptance

- All new and existing tests pass on CI.
- Dev deployment shows the two timing log lines for every slide click.
- The PR description contains a sample of real log output from clicking 5 slides on dev.
- Mutation testing on the touched files passes (no surviving mutants in the dedup logic or the timing field assignments).
- Browser console clean during E2E.
