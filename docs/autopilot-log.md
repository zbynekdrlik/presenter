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

## #404 — Prod android stage relaunch fails (dead Fully Kiosk launchComponent) (2026-06-16)
- **Bug (confirmed live):** all 4 prod displays (sd1-sd4) stuck `state:"error"` — seeded `launch_component` `com.fullykiosk.videokiosk/de.ozerov.fully.MainActivity` activity doesn't exist; periodic `adb shell am start -n <component>` failed every attempt (Error type 3). TVs actually run `com.tcl.browser` opened via VIEW intent at the stage URL.
- **Fix (variant B, per supervisor decision on #404):** `launch_component` is now a browser PACKAGE (default `com.tcl.browser`; `DEFAULT_LAUNCH_COMPONENT`→`DEFAULT_LAUNCH_PACKAGE`). `validate()` relaxed to accept a bare package; legacy `package/activity` still valid + launcher extracts the package before `/`. New env `PRESENTER_ANDROID_STAGE_URL` (read once at `AndroidStageRegistry::new`); launcher fires `am start -a android.intent.action.VIEW -d <url> <package>`. Unset/empty URL → structured WARN + skip (records error so dashboard shows the misconfig, no broken intent). Per-env in deploy units (prod `http://10.77.9.205/stage`, dev `http://10.77.8.134:8080/stage`); documented in CLAUDE.md. Incremental migration `m20260616_000001` rewrites only dead-value rows to `com.tcl.browser` (idempotent, leaves customised rows, never edits the seed). `connect_and_launch` refactored → `adb_connect` + `adb_launch` helpers (all functions ≤120 cap; was 124). Settings UI labels/placeholders updated off the dead component.
- **RED→GREEN:** RED `5fd25fd` (launcher VIEW-intent/package-extraction/skip + validate-bare-package tests fail to compile / assert dead value), GREEN `3f8e29e`. Tests: `crates/presenter-server/src/android_stage.rs::tests::{build_launch_args_emits_view_intent_with_url_and_package, build_launch_args_extracts_package_from_legacy_component, build_launch_args_skips_when_url_unset, launch_package_strips_legacy_activity_suffix}`; `crates/presenter-core/src/android_stage_display.rs::tests::{validate_accepts_bare_package_name, validate_still_accepts_legacy_package_activity, validate_rejects_shell_metacharacters_in_package, default_launch_component_is_bare_browser_package}`; `crates/presenter-migration/src/m20260616_000001_fix_android_stage_launch_package.rs::tests::{migration_replaces_dead_component_and_is_idempotent, migration_no_op_when_table_missing}`.
- **Version:** 0.4.135 → 0.4.136 (`15173ee`). Local: fmt/clippy clean, quality-check `--strict` exit 0 (connect_and_launch 124→59 lines), core 5 + migration 2 + server android_stage 6 + persistence android_stage/seed 4 pass.
- **PR** dev→main — opened, NOT merged (supervisor drives CI→merge→deploy).

## #402 — Stage TVs: keep-screen-awake (sd1 has 5-min screen_off_timeout) (2026-06-16)
- **Validated STILL_VALID:** zero `wakeLock`/`WakeLock` matches in source (only build artifacts); sd1 Tesla = `screen_off_timeout=300000` → risk of black stage after 5 min idle. Feature never existed.
- **Feature:** new `crates/presenter-ui/src/components/stage/wake_lock.rs`. `should_acquire(held_live, visible)` pure decision fn + 3 host unit tests. `wasm_impl` (`target_arch="wasm32"`): acquires `navigator.wakeLock.request('screen')` on `StagePage` mount, re-acquires on `visibilitychange`→visible (browser auto-releases when hidden). Graceful `leptos::logging::warn!` on reject/unsupported, never panics. Wired into `StagePage`; added stable `VisibilityState` web-sys feature.
- **Design decision:** reached the API via RAW JS interop (`js_sys::Reflect`+`Function.call`), NOT web-sys typed `WakeLock` bindings — those need `--cfg=web_sys_unstable_apis`, which flips `HtmlElement::set_scroll_top` to `f64` and breaks 6 unrelated slide-scroll call sites (`slide_list_scroll.rs`/`ai.rs`). Raw interop keeps the change self-contained; no cfg flag, only the stable `VisibilityState` feature added.
- **Tests (feature, same PR):** `crates/presenter-ui/src/components/stage/wake_lock.rs::tests::{acquires_when_visible_and_no_live_lock, does_not_reacquire_when_already_held_live, does_not_acquire_when_hidden}`. No E2E (wakeLock spying is non-deterministic cross-browser; per maintainers, do not add flaky E2E).
- **Version:** 0.4.136 → 0.4.137 (`5687f0d`). Local: `cargo test --lib wake_lock` 3/3; clippy `--lib` host + `--target wasm32` clean; `cargo check --lib --target wasm32-unknown-unknown` (real run target) compiles clean; fmt clean; quality-check `--strict --against origin/main` exit 0.
- **Out-of-repo follow-ups (noted in PR, ops handles):** set `screen_off_timeout=max` on sd1 via adb; fix prod `stage-watchdog.sh` stale "VP8" comment (script not in repo).
- **PR #414** dev→main — opened, NOT merged (supervisor drives CI→merge→deploy).

---

## 2026-06-20 — Batch: #364 + #366 (one PR, one CI cycle) → PR #427, merge 5f8e5f9

**Version:** 0.4.140 → 0.4.141 (`9b91706`).

### #364 — quality-check.sh placeholder/TODO gate was a silent no-op (BUG FIX, RED→GREEN)
- **Root cause:** `crates/**/*.rs` rg include glob is version-fragile (older rg misses nested files) + both rg calls ended in `2>/dev/null || true` (swallowed real scan errors → gate passed silently). No self-test.
- **Fix:** extracted scan into `scripts/dev/placeholder_check.sh` — scans explicit relative `crates` path via `--type rust`/`--type ts`, branches on rg exit code (0 match / 1 no-match / ≥2 error) so a real scan error exits 2 loudly. quality-check.sh §16 calls it + maps exit code. Self-test `tests/ci/placeholder-gate.test.sh` wired into CI "Run CI shell tests".
- **Regression test:** `tests/ci/placeholder-gate.test.sh` — RED `4716b45` (test only, gate absent / old shallow-glob no-ops), GREEN `723881f` (robust gate fires on deeply-nested marker, fails loudly on scan error). `Closes #364`.
- **Review fixes:** `456e200` (rel scan path fixes spaced-checkout false-positive in comment filter; meaningful RED-pin), `a65000f` (assertion 3 → genuine differential pin vs $GATE), `5407f6b` (CI: install ripgrep in Test job — runner lacked it, self-test failed on missing rg).

### #366 — consolidate cargo-audit/deny advisory-ignore config (REFACTOR, no behavior change) — `a840fe7`
- Moved RUSTSEC-2026-0097 + RUSTSEC-2026-0173 from CI `--ignore` flags into `.cargo/audit.toml` (file cargo-audit actually reads); dropped `--ignore` from pipeline.yml + security-schedule.yml (kept in sync); added cross-ref comment to deny.toml; deleted dead repo-root `audit.toml` (empirically proven never read). RUSTSEC-2026-0173 stays ignored (blocked by #367). Verified: `cargo audit --deny warnings` (no flags) + `cargo deny check advisories / bans licenses sources` all green.

**Audits:** /review (code-review skill) + /requesting-code-review (deep) — all findings fixed. CI all 13 required checks + Mutation advisory green; PR mergeable+clean. Deployed v0.4.141: dev DOM `v0.4.141 (dev)`, prod DOM `v0.4.141`, 0 console errors.
