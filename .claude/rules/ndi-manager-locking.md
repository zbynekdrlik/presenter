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
on `hw_h264_encoder()`); the reconnect-ticker-vs-deactivate revive TOCTOU is #747.
