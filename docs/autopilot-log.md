# Autopilot Log

Terse per-issue record of autonomous cycles (issue #, commits, tests, decisions).

---

## 2026-06-16 — #374 CI: function-length gate self-test (regression guard for abs/rel no-op)

- **Scope (rescoped by ticket-validator):** only the remaining-open item — add a self-test so the function-length gate's abs/rel no-op bug can't silently return. Gate fix (`5f5fe9f`) + the 4 over-cap NDI/WebRTC fns were already resolved (PR #368); not re-touched.
- **Version:** 0.4.131 → **0.4.132** (commit `18849e2`), after merging main into dev (FF to `58dbe9b`).
- **Implementation:**
  - Extracted the embedded checker out of `quality-check.sh` §7 into standalone `scripts/dev/fn_length_check.py` (identical behavior) so the test exercises the exact CI code (no drift). Commit `f7960d8`.
  - Added the self-test, then aligned it to the repo's `tests/ci/*.test.sh` convention → **`tests/ci/fn-length-gate.test.sh`**, run via the Test job's existing "Run CI shell tests" step. Commit `4b11f37`.
- **RED→GREEN proof:** test passes against the real (fixed) checker; reintroducing the abs/rel bug (`rel`→`path` in the relpath compare) makes the test FAIL (exit 1); restoring it passes again. Asserts: gate fires on a relative-path over-cap fixture, buggy abs-path variant no-ops, absolute target no-ops, unscoped scan fires.
- **PR #399** dev→main, merged `2cf145a`. Dev pipeline all green (incl. self-test step); deploy-dev verified `/healthz` = 0.4.132. Main Deploy → prod.
- **Side findings (filed):** `#404` — prod android stage relaunch fails (Fully Kiosk `launchComponent` activity missing on sd1-sd4; TVs still show prod `/stage` via TCL browser). Verified dev presenter has ZERO android-stage config (no prod/dev watchdog conflict); cleaned up stray dev2 adb client connections to sd1-sd4.
- **NDI audit (user concern):** current `dev` confirmed intact — every NDI stutter-debug experiment (VP8/480p/sw-encode/silent-audio/playout-delay/RTCP-SR/lite/diag) fully reverted, zero residue; Resolume/audio/HW-H264/30s-reaper/watchdogs all live (verified by grep + clean cargo check/clippy).

## #370 — NDI: source-switch leaks old source's pipeline + encoder (2026-06-16)
- **Bug (rescoped from validator PARTIAL):** switching active video source (deactivate A → activate B) started B's pipeline but never stopped A's. `repository.activate_video_source` flips sibling rows `is_active=false` without notifying the manager; `manager.start_pipeline` only touches the new id; no reaper covers a whole orphaned source pipeline. → two source pipelines = two `nvh264enc` encoders after every switch (NVENC contention). "Two encoders per source" was already gone (PR #390 single shared H264); deactivate-all + delete-active already tear down.
- **Fix:** `retain_only_active(active, keep_id)` (manager.rs) removes+stops every active-map entry whose source_id != keep; `NdiManager::stop_other_pipelines(keep_id)` (lifecycle.rs); `activate_video_source` (integrations.rs) calls it AFTER the new source confirms Streaming (no stage gap; start-failure returns early without reaping).
- **RED→GREEN:** RED `5d90b29` (no-op stub + tests, FAIL left:2/4 right:1), GREEN `8b52d03`. Tests: `crates/presenter-ndi/src/manager.rs` `stop_other_pipelines_tests::{activate_switch_leaves_exactly_one_source, activate_reaps_all_stale_siblings, reactivate_same_source_keeps_it}`.
- **Version:** 0.4.132 → 0.4.133 (`fa6205c`). Local: fmt/clippy clean, 48 ndi + 251 server tests pass.
- **Live-verify caveat:** dev has no video sources (`/healthz ndi_pipelines:[]`), prod has a single source — a real 2-source switch can't be exercised on a live target; the CI regression test is the behavioral proof.

## #384 — Stage layout resets to default on server restart — persist it in DB (2026-06-16)
- **Bug:** selected stage layout (`POST /stage/layout`) lived only in `AppState.stage_layout` RwLock (init to `DEFAULT_STAGE_LAYOUT_CODE` `worship-snv`); `set_stage_layout_code` wrote RwLock + live event only, never the DB. Every `presenter.service` restart/deploy silently reset the layout → blanked NDI stage displays (false "TVs not connecting" alarms, found during PR #382 prod testing).
- **Fix:** persist via existing `app_settings` k/v table, key `feature.stage.layout` (same mechanism as Companion `feature.companion.*`, which intentionally bypass `settings_audit` — only typed singleton tables are audited). `set_stage_layout_code` (`state/stage_display.rs`) writes the code after the RwLock update (failed write logs+continues, never aborts the live switch). `from_config` (`state/mod.rs`) loads it on startup (pure read, seeds RwLock; falls back to default if unset/unknown). NO migration: `app_settings` already exists (core migration); only a new key added.
- **RED→GREEN:** RED `48963a9` (`stage_layout_persists_across_restart` fails: after restart layout=`worship-snv` not `timer`), GREEN `f75e226`. Tests in `crates/presenter-server/src/state/tests.rs`: `stage_layout_persists_across_restart` (2nd AppState from same on-disk DB) + `stage_layout_load_on_startup_writes_no_audit_rows` (no-audit invariant).
- **Version:** 0.4.134 → 0.4.135 (`9751f1c`). Local: fmt/clippy clean, server 253 + persistence 50 (incl. `second_startup_writes_no_audit_rows`) + migration 3 pass.
- **PR #411** dev→main — opened, NOT merged (supervisor drives CI→merge→deploy; project CI ~30-40 min).
