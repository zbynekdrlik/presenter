---
paths:
  - "crates/presenter-ndi/src/manager/**"
---

# NdiManager active-map locking + the reservation activation pattern (#741)

`NdiManager.active` is a `tokio::sync::Mutex<HashMap<String, ActiveSource>>`. It is the
single choke point for the operator-dashboard status poll (`pipeline_snapshots_checked` →
also `/healthz`), `stop_*`, `periodic_reap`, `pipeline_snapshot(:id)`, AND every WHEP
POST/PATCH/DELETE (they lock it to clone the pipeline `Arc`).

## Iron rule: NEVER hold `active` across a slow pipeline op

Do not hold the `active` guard across `NdiPipeline::build` + `start()` + the ~8 s
streaming-ready wait, nor across `pipeline.stop().await` when it can be avoided. Holding it
across the 8 s wait stalls ALL of the callers above for 8 s per activation (#736 was the log
flood from that; #741 removed the contention). `MutexGuard` is `Send`, so the compiler will
NOT catch a hold-across-`.await` — you must confine the guard by hand (a `{ … }` block whose
tail moves the needed `Arc` out, or a helper that owns the whole lock scope).

## The reservation pattern (`manager/activation.rs`, #741)

`start_pipeline` / `rebuild_pipeline` do: **reserve under the lock** (check_active_entry →
build → `start()` → insert an `ActiveSource` in `Starting` state) → **RELEASE** → **wait
UNLOCKED** (`wait_for_streaming`) → **finalize under the lock** (`finalize_reservation` →
`Promote` / `Removed` / `Superseded`, keyed on `Arc::ptr_eq(slot.pipeline, ours)`; it stops
any removed/orphaned pipeline OUTSIDE the lock).

- The `Starting` entry is BOTH the per-source in-flight marker (a concurrent `start(A)`
  sees it via `check_active_entry` → Idempotent → `observe_in_flight`, never double-builds)
  AND what keeps the status reader #546-safe (a `Starting` slot classifies as `Connecting`,
  not the alarming "active in DB, no pipeline"). `check_active_entry` treats
  Streaming/Starting as Idempotent — do NOT change that; the reservation relies on it.
- The supervisor is spawned ONLY on `Promote` (after Streaming), then attached under a
  re-lock that re-checks `ptr_eq` (a concurrent op may have removed/replaced the slot in the
  gap — then abort the just-spawned supervisor and stop the orphan). `rebuild_pipeline`
  re-inserts `supervisor: None` (its calling supervisor re-subscribes via `state_watcher_for`).
- `PipelineStartError::Superseded` means "a concurrent deactivate/switch owns the outcome" →
  `activate_video_source` returns `Ok(source)` and publishes NOTHING (no stray status, no
  sibling reap). It has an exhaustive-match cost: adding it broke `Display` (lifecycle.rs)
  AND `ndi_status_for_start_error` (server integrations.rs) — grep `PipelineStartError::`
  before pushing any new variant.

## Testing the seam without libndi (Tier-0)

`NdiManager::try_new()` needs the NDI SDK → `None` on CI, so you cannot construct a manager
in a CI test. Test the PURE seam instead: `wait_for_streaming` with a `watch::channel(PipelineState)`,
and `finalize_reservation` / `check_active_entry` / `retain_only_active` against a bare
`HashMap<String, ActiveSource>` built from `NdiPipeline::stopped_for_test()`
(+ `set_state_for_test(&mut self)` BEFORE wrapping in `Arc`). The structural
"lock released during the wait" property is verified by design + a fresh-context review, not
by a libndi integration test — this crate cannot compile locally (Tier-0), so a careful
adversarial review is the pre-CI safety net for borrow/`Send`/exhaustive-match correctness.

**Deterministic concurrency tests use a per-source START-GATE on `FakeNdiControl`, NOT sleeps
(#745a).** `FakeNdiControl::gate_start(source_id)` parks that source's `start_pipeline` (after
recording its call, so the DB row is already written) and returns `(release, parked)` Notify
handles: `await parked` to know the activation is parked mid-flight, then race a second
activation, then `release.notify_one()`. Bound every wait in the test (`tokio::time::timeout`,
500 ms probe / 5 s join) so a lock regression fails loudly instead of hanging. tokio `Notify`
permits are stored, so there is no lost-wakeup even if `notify_one` fires before `notified()`.

## Supervisor ownership: the entry's `supervisor` is ALWAYS the live owner (#745c)

`check_active_entry` is 3-way: `Idempotent | RebuildDead(Option<JoinHandle<()>>) | Vacant`. It
MOVES a removed dead entry's supervisor into `RebuildDead(..)` (it still NEVER aborts it — the
self-rebuild task calls it). Invariant to preserve: **`ActiveSource.supervisor` is `Some(live
owner)` after every promote AND every rebuild**, `None` only transiently during a not-yet-promoted
`Starting` reservation. That is what lets every abort path (`stop_pipeline`/`stop_all`/
`retain_only_active`/`start_pipeline` reactivate) reach the owning supervisor.

- `rebuild_pipeline` on `RebuildDead(carried)` RE-INSERTS `supervisor: carried` (was `None`, the
  double-watch bug); on `Vacant` it returns `Ok(())` WITHOUT building (source gone →
  `resubscribe_after_rebuild`'s `state_watcher_for` finds no entry → `SupervisorStep::Exit`; the
  30 s reconnect ticker owns recovery per the DB). `start_pipeline` on `RebuildDead(Some h)`
  ABORTS `h` before building (operator reactivate → no double-watch).
- A FAILED rebuild (`Err`) exits the supervisor (`handle_dead_state` → `SupervisorStep::Exit`),
  never loops — retrying off the old watcher's unseen `Stopped` echo could re-attach the orphaned
  supervisor to a concurrently-promoted pipeline. The ticker re-drives per the DB.
- **Tier-0 TRAP:** matching `StateCheckOutcome` with `if let StateCheckOutcome::Idempotent = …`
  compiles UNCHANGED when the enum is later widened and SILENTLY DROPS the new variant's payload
  (the carried handle) — no compiler backstop on this no-local-compile crate. ALWAYS a total
  `match`, never `if let`, on this enum. (See the general Tier-0 `if let` trap in `quality-gates.md`.)

Pre-existing activation concurrency edge cases: #745 fixed (a) reap-vs-DB ordering (an
`AppState.activation_lock` serializing `activate_video_source`) + (c) the rebuilt-entry
double-watch (above); (b) was descoped (the 30 s reconnect ticker IS the backstop, conditional
on `hw_h264_encoder()`); the reconnect-ticker-vs-deactivate revive TOCTOU was #747 (FIXED).

## The reconnect ticker's read+activate is ONE critical section (#747)

`activation_lock` (a server-side `Arc<Mutex<()>>` in `AppState`, NOT the manager's `active`
map) covers `activate_video_source` / `deactivate_video_sources` / `delete_video_source`. But
the 30 s NDI auto-reconnect ticker (`state/background_tasks.rs`) used to **read**
`get_active_video_source()` OUTSIDE that lock and then activate the stale id — so an operator
deactivate committing in the read→activate gap was undone: the ticker revived the source the
operator just turned off (~within one tick). `repository.activate_video_source`'s #375
idempotency guard does NOT absorb it — it only early-returns when the row is ALREADY active, so
post-deactivate it re-flips the row to active.

The fix routes the ticker through `AppState::reconnect_active_video_source`, which takes
`activation_lock` FIRST and holds it across BOTH the DB re-read AND the activation. To take the
non-reentrant tokio `Mutex` exactly once, `activate_video_source` is split into a public
lock-taking wrapper + a private `activate_video_source_locked` (assumes the lock is held);
`reconnect_active_video_source` and the public wrapper both call `_locked`.

- **Re-read the ACTIVE SOURCE under the lock, never re-check a caller-supplied id.** Re-reading
  `get_active_video_source()` also makes a concurrent SWITCH correct (reconnect the NEW winner,
  never the stale one); a per-id `is_active` re-check would only handle deactivate.
- **The re-check is TICKER-ONLY, never baked into `activate_video_source` for all callers** —
  the operator's own activate deliberately targets an INACTIVE source (turning it ON), so a
  blanket skip-if-inactive would no-op the operator's explicit activate.
- **The one-shot startup restore (`restore_active_ndi_source`, `state/mod.rs`) is deliberately
  NOT routed through this** — it runs during AppState construction before the router serves any
  request, so no concurrent operator deactivate is possible and the TOCTOU cannot occur there.

## Tier-0 TOCTOU test technique: hold `activation_lock` in the test

A TOCTOU can only be shown with concurrency. The `FakeNdiControl` start-gate parks at
`start_pipeline`, which is AFTER the read+lock — too late to expose a read-that-raced-the-lock.
Instead, the test HOLDS `activation_lock` directly (white-box — the `state::integrations::tests`
module is a descendant of `crate::state`, so the private field is in scope), spawns the
reconnect, lets it reach its steady blocking state, commits the operator's deactivate at the
**repository** level while the lock is held (the state-level `deactivate_video_sources` would
DEADLOCK on the held lock), then releases. Pre-fix the reconnect's read raced ahead of the lock
and revived the source; post-fix its read is under the lock, sees the source inactive, returns
`Ok(None)`. Assert on committed DB state after the join; bound every wait with
`tokio::time::timeout`.
