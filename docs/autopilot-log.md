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

---

## 2026-06-20 — #429 CI: make the Mutation Testing PR gate airuleset-compliant (joins PR #428)

**Version:** unchanged — dev already at 0.4.142 (> main 0.4.141). No bump.

### #429 — Mutation gate full-`--workspace` shard timed out at 45min (`|| true` fake-green) → every PR UNSTABLE (CI-FOUNDATION FIX)
- **Root cause:** `pipeline.yml` `mutation` job ran `cargo mutants --workspace --timeout 120 --no-shuffle --shard 1/12 ... || true` with `timeout-minutes: 45` — a full-tree shard that timed out at the 45-min cap → run conclusion `cancelled` → PR mergeStateStatus UNSTABLE, despite all required checks green. The `|| true` made it a fake-green advisory, never a real gate.
- **Fix (airuleset `ci/mutation-testing.md` two-tier shape):**
  - `pipeline.yml` `mutation` job → **diff-scoped REAL gate**: `timeout-minutes: 20` (hard cap), checkout `fetch-depth: 0`, install cargo-mutants+cargo-nextest via `taiki-e/install-action@v2` (prebuilt), run `git diff origin/${{ github.base_ref || 'main' }}...HEAD > pr.diff` then `cargo mutants --in-diff pr.diff --baseline=skip --test-tool=nextest --jobs 2 -- --all-targets`. Removed `|| true` + `--workspace --shard 1/12`.
  - `.cargo/mutants.toml`: `profile = "mutants"`, `test_workspace = false` (per-package tests only — `test_package = true` is invalid TOML; the boolean key is `test_workspace`), `exclude_globs` for migrations / proto vendor / companion protocol / build.rs.
  - `Cargo.toml`: `[profile.mutants]` `inherits = "test"`, `debug = "none"`.
  - `.github/workflows/mutation-full.yml`: `workflow_dispatch`-only full-tree sweep, sharded 1..8 on `ubuntu-latest`, survivors → `gh issue create` label `test-quality` (label auto-created). The `/mutation-sweep` target.
  - `.gitignore`: `mutants.out/` + `/pr.diff`.
- **Diff mutation result (local, verified):** 3 mutants in the PR diff — 1 CaughtMutant (`stage_display_snapshot` match-arm, killed by #383 unit tests), 2 Unviable, **0 survivors** → no new tests needed. `Closes #429`.

**Cycle:** local fmt + clippy (`-D warnings`) + `cargo test --workspace` (all green) + quality-check `--strict --against origin/main` (exit 0) + diff-scoped `cargo mutants` (0 survivors). PR #428 body extended with `Closes #429`.

## 2026-06-21 — batch #392 + #421 (Android stage launcher) — PR #432, v0.4.143

- **#392** (provisioning bug): `bootstrap-host.sh` did not install `adb` (hard runtime dep of the Android Stage Launcher). RED `caa6abe` (tests/ci/bootstrap-adb.test.sh asserts `android-tools-adb` in APT_PACKAGES — failed against adb-less bootstrap) → GREEN `0d4b4f9` (added the package; updated runbook.md; added "Ensure ADB is installed" step to release.yml PP deploy, mirroring deploy.yml — closes PP coverage gap). CI self-test wired into pipeline.yml "Run CI shell tests".
- **#421** (test-addition gated on DI refactor): keep-alive wiring untested (bare `Command::new(adb_bin)`, no seam). Seam refactor `3d0cd79` (`trait AdbRunner`; `ProcessAdbRunner` prod impl; `&dyn AdbRunner` threaded through run_device_worker/connect_and_launch/adb_connect/adb_launch/adb_foreground_package; `async-trait` moved dev-dep→dep). Wiring test `4d705ba` (FakeAdbRunner; tests: tick_skips_am_start_when_browser_foreground, tick_fires_am_start_when_backgrounded, tick_fires_am_start_when_foreground_unknown, launch_now_always_fires_am_start_without_probing, worker_launch_now_command_forces_am_start). Proven non-tautological: bypassing `if !force_launch` failed all 3 tick tests (reverted).
- **Mutation:** diff-scoped `cargo mutants` first run = 1 survivor (`adb_connect -> Ok(())`). Killed by `61208d9` (launch_aborts_when_adb_connect_fails + connect_calls assertion); targeted re-run = 1 caught → **0 survivors**.
- **Reviews:** /review correctness (2 cosmetic notes — timeout wording, fact still surfaced), /review ci-ops ([]), requesting-code-review (Ready to merge, 0 Critical/Important).
- Decision: the two cosmetic timeout-message notes left as-is — the propagated io error message still names "adb command timed out", so the timeout fact is preserved on the operator dashboard's last_error.

**Cycle:** local fmt + clippy (`-D warnings`) + `cargo test --workspace` (277 passed, 0 fail) + diff-scoped `cargo mutants` (0 survivors after the kill). One push, one Pipeline CI run, PR #432 (`Closes #392`, `Closes #421`).

### Follow-up on PR #432 — mutation gate timed out (#430), sharded + warmed

- **Root cause (#430):** the single-job diff-scoped `mutation` gate (#429) hit its 20-min `timeout-minutes` cap. cargo-mutants builds the `mutants` profile in an isolated dir that the `ci` rust-cache (keyed for the debug/test profile in `target/`) does NOT warm, so the heavy dep graph (gstreamer/NDI/leptos/seaorm) cold-built from scratch; CI run 27887309677 logged "Found 11 mutants" then cancelled at 20m16s with ZERO mutants tested → PR mergeStateStatus UNSTABLE (all REQUIRED checks green; mutation is non-required, deploy-dev does not `needs` it, so v0.4.143 already deployed to dev).
- **Fix (airuleset `ci/mutation-testing.md` "Budget overrun = setup bug" — SHARD + WARM, never raise the cap):**
  - `pipeline.yml` `mutation` job → 4-job matrix `shard: [0,1,2,3]` (cargo-mutants `--shard k/4` is 0-indexed; `4/4` is rejected). `--in-diff` then `--shard` splits the 11 diff mutants 3+3+3+2.
  - Dedicated `mutants`-keyed `Swatinem/rust-cache` (separate from `ci`) + a `cargo build --tests --profile mutants --workspace --all-targets` warm step, then `cargo mutants --in-place` reuses the warm `target/` directly (dropped `--jobs 2` — in-place serializes). `timeout-minutes: 20` UNCHANGED (now per shard).
  - `.gitignore`: added `mutants.out.old/` (cargo-mutants rotates the previous run aside).
- **Local verification (warm):** cold warm-build 7m21s (8-core dev2 under GPU load); full-diff `cargo mutants --in-place` = **11 mutants in 6m: 9 caught, 2 unviable, 0 survivors** (exit 0). With 4 shards each ≈ warm build + ~3 mutants → comfortably under the 20-min cap. No new tests needed — the seam diff is fully covered.
- **PR #432 body** extended with `Closes #430` (keeps `Closes #392`, `Closes #421`).
- 2026-06-21 batch#3 PR #432 (merge cc80c06) — Closes #392 (adb in bootstrap-host.sh + release.yml adb step + lock test), #421 (AdbRunner seam + keep-alive wiring integration test), #430 (mutation gate: 4-way shard + warm mutants-profile cache, fits 20-min cap). main Deploy success, v0.4.143 dev+prod (healthz verified). Filed #433 (deploy.yml set -e twin).

## batch#4 — #394 verse-split + #433 deploy.yml set -e (v0.4.144)

- 2026-06-21 batch#4 — bump 0.4.144 (c1668d8c). One PR closes #394 + #433.
- **#394 (verse-split bug, path A — deterministic composer layer, no agent.rs/LLM-prompt changes):**
  - RED: `9a5550bb` — new/updated tests assert a verse is never split mid-text. GREEN: `703d562b`.
  - `compose.rs` `compose_bible_items_into_slides`: (a) merge consecutive `Verse` items sharing the SAME number into one whole line/slide; (b) lone oversized verse kept WHOLE (`would_overflow` returns false on empty acc); (c) review-found edge — when a same-number continuation fragment would over-pack a slide already holding an earlier verse, `flush_keeping_last` flushes the earlier verses so the growing verse lands whole on its own slide (`df940402`). Closure → `VerseAccumulator` struct; extracted `build_reference_label` + `group_reference` to keep both composer fns < 120-line cap.
  - `bible_validator.rs` `is_lone_whole_verse`: validator ACCEPTS a lone whole verse over the char limit (single verse-prefixed line + non-empty reference; autofit shrinks). Multi-verse over-pack + oversized emphasis still rejected.
  - Tests updated to new behavior: `create_bible_presentation_accepts_lone_oversized_verse_whole` (was `_rejects_oversized_single_verse`), `length_rule_accepts_lone_verse_one_char_over_limit` (was `_rejects_slide_one_char_over_limit`), `length_rule_accepts_lone_oversized_verse` (was `_rejects_slide_well_over_limit`), `compose_items_single_verse_longer_than_limit_kept_whole_on_own_slide` (renamed). New: `_same_verse_number_split_is_merged_whole_*`, `_same_number_merge_then_next_verse_overflows_*`, `_same_number_fragment_after_earlier_verse_moves_growing_verse_to_own_slide`, `_same_number_fragment_that_still_fits_stays_*`, `length_rule_rejects_multi_verse_slide_over_limit`.
  - `bible_presentation.rs`: refreshed 2 stale comments (validator no longer guarantees `main.len() <= limit`); extracted `parse_bible_items` to keep `create_bible_presentation` < 120 (`635b5511`). Doc comment on `compose_bible_items_into_slides` refreshed (`7a5c3872`).
  - Filed #434 (stale agent.rs prompt step 8 — "separate slides" now wrong; blocked by fn-length gate on `build_system_prompt`/`run_agent`, out of scope).
- **#433 (deploy.yml ADB `set -e` twin):** `fafe0f57` — `set -e` in deploy.yml ADB heredoc mirroring release.yml 47d466a; new `tests/ci/adb-install-set-e.test.sh` (genuine guard: fails when set -e removed). Review found a THIRD gap → `ae8e6cbe` added `set -e` to pipeline.yml deploy-dev ADB step + extended the test to all 3 workflows.
- Reviews: `/code-review` + `/requesting-code-review` both clean (0 critical/important; one minor doc-comment fixed).

## Batch #5 (2026-06-21) — #437 + #438 + #436 (one PR, v0.4.145)

- **Version:** bumped `0.4.144 -> 0.4.145` (Cargo.toml workspace, first commit `3fbc7154`).
- **#437 (bug — AI model default was retired `claude-opus-4-20250514`, 404'd prod 2026-06-21):**
  RED `40b3147c` (`ai::tests::default_model_is_not_retired` — asserts default != retired ID, == `claude-opus-4-6`) → GREEN `70a46330` (`Closes #437`). Extracted `DEFAULT_AI_MODEL` const = `claude-opus-4-6` (newest Opus the on-device CLIProxyAPI catalog serves; 4-8 not in catalog). Updated retired UI placeholder `crates/presenter-ui/src/pages/ai.rs:297` → `claude-opus-4-6`.
- **#438 (bug — `/ai/status` reported `claudeAuthenticated:true` for expired/dead OAuth token):**
  RED `61564d81` (`ai::proxy::tests::expired_token_is_not_authenticated` + fresh/mixed/none cases) → GREEN `6ecc3691` (`Closes #438`). `is_claude_authenticated()` now parses each `claude-*.json` token's RFC3339 `expired` field; provably-expired → not authenticated, unparseable → fail-open. WARN log per expired token + aggregate. Offline freshness check only (MVP, no live probe). Added `TokenValidity` enum.
- **#436 (behavior — hide song number on NDI fullscreen):** `13b2736d` (`Closes #436`). Added `hide_song_number` prop to `StatusBar` (mirrors `hide_live`), gated song-number `<div data-role="song-number">` render + its autofit on it, `NdiFullscreen` passes `hide_song_number=true`. Other layouts unchanged. E2E `tests/e2e/ndi-fullscreen-song-number.spec.ts` (absent on ndi-fullscreen, present on worship-snv, zero console errors). Synced presenter-ui Cargo.lock presenter-core 0.4.141→0.4.145.
- Local gates: fmt clean, clippy `--workspace --all-targets -D warnings` clean, full workspace tests green (293 server bin + persistence ok, no SIGSEGV), presenter-ui wasm32 check + 118 lib tests green.
- Note: RED tests are in-file `#[cfg(test)] mod tests` inside `ai/mod.rs`/`ai/proxy.rs` (standard Rust convention) — the filename-based `pre-push-test-check` RED-before-GREEN gate can't see them, so push used `[no-test: ...]` with the honest reason that the RED tests exist and precede the fixes (verified failing→passing).

_RED-before-GREEN verified: RED 40b3147c (#437), 61564d81 (#438) precede their GREEN fixes._

### batch#5 review hardening (post-review, same PR #440)
- `/code-review` (high) + deep second-pass review found 2 issues, both fixed in-PR:
  - `4149e304` proxy.rs: skip non-regular-file auth-dir entries (a `claude`-named subdir would EISDIR→Unknown→fail-open, masking expired tokens). +regression test `claude_named_subdir_does_not_grant_auth`.
  - `7cbd1727` ndi-fullscreen E2E: replaced blind `waitForTimeout(1500)` with a positive snapshot-arrival anchor (assert #042 on worship-snv first, then absence on ndi-fullscreen). Verified locally: 2 passed; removing `hide_song_number` makes the HIDDEN test fail (Received: 1).
- Filed airuleset hook gap (Rust in-file `#[cfg(test)]` tests undetectable by filename-based RED-before-GREEN gate) — see filed issue.
