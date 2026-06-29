---
name: presenter-ci
description: >
  CI / self-hosted runner management for presenter on dev2 — runner restart, GPU wedge recovery,
  and probe process cleanup. Use when CI jobs are failing, cancelled, or the e2e-ndi lane is stuck.
triggers:
  - runner
  - self-hosted
  - GPU wedge
  - e2e-ndi
  - presenter-local
  - nvh264enc pipeline failure
  - headless Chrome orphan
---

# Presenter CI Skill

## Runner Management

Local runner host: `10.77.8.134` (same machine as dev server), label `self-hosted`/`local`.

```bash
# Check runner status
cd ~/actions-runner && sudo ./svc.sh status

# View runner logs
sudo journalctl -u actions.runner.zbynekdrlik-presenter.presenter-local -f

# Restart runner
cd ~/actions-runner && sudo ./svc.sh stop && sudo ./svc.sh start

# Re-register (if token expired)
gh api -X POST repos/zbynekdrlik/presenter/actions/runners/registration-token --jq '.token'
cd ~/actions-runner && ./config.sh --url https://github.com/zbynekdrlik/presenter \
  --token "$TOKEN" --name presenter-local --labels self-hosted,local
sudo ./svc.sh install && sudo ./svc.sh start
```

CI has TWO runs per push: "PR Automation" (fast, Label/Validate) and "Pipeline" (real
Test→Build→E2E→deploy). `gh run list --limit 1` often returns PR Automation — filter:
`gh run list --branch dev --limit 5 | grep Pipeline`.

## GPU Wedge Recovery (#445)

**dev2's single RTX 5050 is SHARED** between Presenter CI runner (`presenter-local`) and
bakerion-prod GPU services. A bakerion OOM can leave the GPU wedged in `NV_ERR_RESET_REQUIRED`.

**Symptoms:** `nvidia-smi` shows util stuck at 100% with NO compute process, encoderCount=0,
~2 MiB mem. `dmesg | grep -iE 'nvrm|reset required'` confirms. The `e2e-ndi` lane fails with
`build encoder (nvh264enc)` / pipeline 500 → Deploy-to-Dev skipped → ALL open PRs blocked.

**Diagnosis first:** a UI-only diff cannot cause an `nvh264enc` failure. When e2e-ndi fails
on encoder init, check `nvidia-smi` before concluding code regression.

**Recovery (do NOT reboot first):**
- `nvidia-smi --gpu-reset` refuses on the primary GPU.
- Use `recover-hung-gpu` skill — PCIe function-level reset (FLR) over sysfs; no reboot needed.
- Reloading nvidia kernel modules also works (confirmed 2026-06-21; uptime unchanged).
- Reboot clears it but kills any Claude session on dev2 (gated — needs approval).

**Prevention:** never run two GPU processes simultaneously (bakerion inference + presenter e2e-ndi).
Do NOT set `EXCLUSIVE_PROCESS` — it breaks NVDEC, which the NDI decode path needs.

**Preflight (the e2e-ndi lane now fails FAST on a wedge):** `scripts/ci/gpu-preflight.sh` runs
between "Start synthetic NDI sender" and "Run NDI WebRTC E2E". It exits non-zero with an actionable
"run the recover-hung-gpu skill" message instead of letting all 6 tests fail opaquely on a 500.
Detection logic is unit-tested with mock facts in `tests/ci/gpu-preflight.test.sh` (wired into the
"Run CI shell tests" step — no real GPU needed on the hosted runner). Run it manually:
`bash scripts/ci/gpu-preflight.sh` (live) or `GPU_PREFLIGHT_FAKE=1 GPU_PREFLIGHT_FAKE_NVENC=missing
... bash scripts/ci/gpu-preflight.sh` (mock).

**Two false-positive landmines when diagnosing a wedge (both cost time on #445):**
- **Stale GStreamer registry cache hides `nvh264enc` even after the GPU recovers.** `gst-inspect-1.0
  nvh264enc` can report "missing" against a healthy GPU because `~/.cache/gstreamer-1.0/registry.
  x86_64.bin` was written while wedged. ALWAYS `rm -f ~/.cache/gstreamer-1.0/registry.x86_64.bin`
  then re-probe — that re-scans nvcodec against the CURRENT GPU. (The preflight does this itself.)
- **`dmesg` keeps pre-recovery `NV_ERR_RESET_REQUIRED` lines after a driver/module reload.** The ring
  buffer is NOT cleared by the FLR/reload, so `dmesg | grep -i 'reset required'` matches OLD lines on
  a now-healthy GPU. dmesg is a HINT, not a live wedge signal — confirm with a fresh `gst-inspect`
  probe + `nvidia-smi` (compute process present? util/mem?). The preflight uses dmesg only to enrich
  the message, never as a standalone trigger.

## Probe / Headless Chrome Cleanup

After manual NDI/WHEP verification, stale processes starve the e2e-ndi CI lane:

```bash
# Audit (should be ~0 at rest)
pgrep -c -x chrome
pgrep -fc 'node .*\.mjs'

# Check before killing — confirm the Chrome root ancestor
# If it traces to actions-runner (pgrep -f actions-runner) → LIVE CI JOB, leave it
# If it traces to tmux/bash/Claude shell → leftover probe, kill it
for p in $(pgrep -x chrome); do setsid sh -c "kill -9 $p"; done
# Kill leftover node probes: target by PID only
```

**NEVER `pkill -f`** with a path that matches your own shell command (exit-144 trap).

Do NOT kill: `presenter-dev` service (ports 8080/8091), `python3 -m remoteos` (:8092),
or unrelated python http servers (e.g. n8n docs :8099).

A cancelled e2e-ndi caused by overload is a legitimate ONE rerun after cleanup:
`gh run rerun <id> --failed`.

## NDI E2E Lane Split — load-sensitive latency is on-demand (#386)

The `e2e-ndi` self-hosted lane is REQUIRED on every PR but split by load-sensitivity:

- **Per-PR `e2e-ndi` lane** runs the load-INSENSITIVE NDI guards (decode / freeze / console /
  straggler / reactivate / reload) in `ndi-webrtc-synthetic.spec.ts`. A PR still fails if NDI video
  is actually broken. Selector: `--grep "@synthetic-ndi" --grep-invert "@latency-ndi" --project chrome-video`.
- **On-demand `ndi-latency.yml`** (`workflow_dispatch`) runs the load-SENSITIVE glass-to-glass
  latency assertion (`ndi-latency.spec.ts`, tag `@latency-ndi`: median ≤350ms / p95 ≤600ms /
  ≥300 samples / freeze <1s). It builds release binaries fresh + starts `ndi_test_sender`, same
  self-hosted setup as the per-PR lane. Selector: `--grep "@latency-ndi" --project chrome-video`.

**Why:** the latency assert is a timing measurement; concurrent CPU load on the shared dev2 runner
(bakerion cargo-mutants / rebuilds) starves the in-browser rVFC sampling loop + GPU encoder → median
crosses the bound on otherwise-healthy code (issue #386 had median 161-168ms quiet vs 394ms under
load against the 350 cap). Bounds are NOT loosened and the test is NOT skipped — it just runs where
load can't corrupt the measurement. Same on-demand pattern as the #488 mutation full-sweep.

**Tag scheme:** `@synthetic-ndi` keeps the GitHub-hosted `e2e` job excluding the NDI tests (no SDK
there); `@video-codec` routes to the real-Chrome (H.264) `chrome-video` Playwright project;
`@latency-ndi` moves the latency test OUT of the per-PR lane and INTO `ndi-latency.yml`. Playwright
ANDs the project's own `grep` with the CLI `--grep`/`--grep-invert`. **Run `ndi-latency.yml` after any
NDI/WebRTC pipeline change** (encoder, downscale, fanout, WHEP) — it is the strict latency guard.

Run it: `gh workflow run ndi-latency.yml --ref dev` (then `gh run watch <id>` on a quiet box), or the
Actions tab → "NDI Glass-to-Glass Latency (on-demand)" → Run workflow.

**Local latency check (quiet box):** `cargo build -p presenter-ndi --features test-helpers --bin
ndi_test_sender && PRESENTER_NDI_TEST_NAME=PRESENTER-TEST ./target/debug/ndi_test_sender &` then
`NDI_RUNTIME_DIR_V6=/usr/lib/ndi PRESENTER_SKIP_MOCK_INTEGRATIONS=1 npx playwright test --grep
"@latency-ndi" --project chrome-video --reporter=line` (needs `/usr/bin/google-chrome` for H.264 —
bundled Chromium has none).

## Mutation Gate

**Removed from the per-PR pipeline (#488, 2026-06-28).** Mutation testing no longer runs on every
`dev` push — the `mutation-warm` + sharded `mutation` jobs (and their `mutation-warm-bootstrap`
self-test) are gone from `pipeline.yml`, so the pipeline (checks → build → e2e → deploy-dev) reaches
deploy faster. Mutation is now **on-demand only** via `mutation-full.yml` (`/mutation-sweep`), which
runs the full-tree sweep and files surviving mutants as `test-quality` issues. The `[profile.mutants]`
in `Cargo.toml` stays — the sweep binds it via `profile = "mutants"` in `.cargo/mutants.toml` (not
auto-detect; removing either would drop the sweep back to the `test` profile).

History: the per-PR gate was fixed twice (#430 diff-scoping, #435/#439 `mutation-warm` + 16 shards)
but the user decided (2026-06-28) the per-PR cost was too high for the MVP/autopilot backlog.

## Quality-Check Gate Landmines (#483 lessons)

The `quality-check.sh --strict --against origin/main` gate (the "Quality Checks" job) hard-fails
on the **changed-file set only** — but since the #407/#482 `count_prod_lines.sh` fix it now counts
correctly (past `#[cfg(test)] mod tests;`). Two pre-existing-debt landmines:

- **File-size (>1000 prod lines) + fn-length (>120 lines, tests NOT exempt):** TOUCHING an
  already-over-cap file/function pulls it into the diff and HARD-FAILS your PR — even if your edit
  is unrelated. Known offender STILL open: `state/mod.rs` (~1117 prod lines, #486). `resolume/tests.rs`
  was fixed in #487 (shared `mount_composition`/`mount_params`/`mount_clips`/`mount_full_composition`/
  `build_driver` helpers + `stage_all`/`stage_main_meta` builders → all fns now ≤120). Check before
  editing: `bash scripts/dev/count_prod_lines.sh <file>` and
  `QC_TARGETS=<file> python3 scripts/dev/fn_length_check.py .`.
  **Workaround when you must add code near an offender:** wire through a SMALL sibling file instead
  of the god-file (e.g. add the call in `state/integrations.rs`, not `state/mod.rs`), and put NEW
  tests in their OWN file (e.g. `resolume/latency_tests.rs`) so the bloated `tests.rs` stays out of
  the diff. Then `git diff --name-only origin/main...HEAD` must NOT list the offender.

- **Mutation survivors (on-demand sweep only since #488 — NOT a per-PR gate anymore):** mutation no
  longer blocks PRs; the full-tree `/mutation-sweep` (`mutation-full.yml`) files survivors as
  `test-quality` issues. When you DO work a survivor (from a sweep, or proactively before a refactor
  that widens scope), kill it HONESTLY — same techniques as the old diff gate (no `exclude_re` for
  code that carries behavior):
  - pure telemetry helpers (`count_clips`, an `as_str`, a `duration_ms(Duration)->f64`) → make
    `pub(super)` + unit-test the exact output (kills `replace-body` + arithmetic mutants).
  - side-effect/audit/wiring fns (writer task, `record_*`, `attach_*`) → ONE end-to-end test that
    asserts the observable effect (a DB row appears) kills all the `-> ()` / `-> Ok(())` no-op mutants.
  - untestable guards (`if TRIGGER_DELAY.as_millis() > 0` — TRIGGER_DELAY is 0 in test builds) → just
    DROP the guard (the bare op is a no-op at 0).
  - log a `Duration` via `?d` instead of `d.as_secs_f64() * 1000.0` to remove arithmetic mutants from
    a behavioral fn (or route the `* 1000.0` through a tested `duration_ms` helper).
  - **Verify a fix locally** (no CI mutation gate to lean on anymore — #488):
    `git diff origin/main...HEAD > /tmp/pr.diff && cargo mutants --in-diff /tmp/pr.diff --baseline=skip --test-tool=nextest --jobs 4 -- --all-targets`.
    Watch `mutants.out/missed.txt` (must stay empty). Local cold-build is slow (~50 min for ~50 mutants).
    cargo-mutants does NOT mutate `#[cfg(test)]`/`#[test]` code.

## Testing patterns for driver behavior (logs / timing / backoff) — #484 lessons

When a fix is about WHEN something happens (log frequency, retry spacing, backoff), make the
decision a PURE helper and unit-test it — don't bury it in the async path:

- **Decision as a pure `pub(super)` fn** (`backoff_interval(consecutive_failures)->Duration`,
  `should_log_error(consecutive_failures)->bool` in `resolume/driver.rs`) → deterministic unit tests
  pin the exact schedule with NO sleeping (mirrors `duration_ms`/`count_clips` from #483). Strong
  mutation killer too.
- **Assert on actual ERROR-log frequency** with a minimal scoped subscriber: a tiny
  `struct ErrorCounter` impl `tracing::Subscriber` that bumps an `AtomicUsize` in `event()` when
  `*event.metadata().level()==Level::ERROR`, installed via `tracing::subscriber::set_default(...)`
  (keep the `DefaultGuard` alive). Works because `#[tokio::test]` defaults to current-thread → the
  thread-local default captures events across `.await`. Lets a test prove "N failures → bounded
  ERROR lines, not N" against the real `error!` call (RED on the unconditional log). See
  `resolume/backoff_tests.rs`.
- **Time-based behavior without wall-clock sleep:** `#[tokio::test(start_paused = true)]` +
  `tokio::time::advance(d)` — but this needs tokio's `test-util` feature. Add it to the crate's
  dev-deps only: `tokio = { workspace = true, features = ["test-util"] }` (additive, test build only;
  default behavior unchanged when not paused, so other `tokio::time::sleep` tests are unaffected).
  The driver's `next_retry_at`/`in_backoff` use `tokio::time::Instant`, so the paused clock + advance
  drive them deterministically.
- Keep new behavior tests in a SELF-CONTAINED file (`resolume/backoff_tests.rs`, registered
  `#[cfg(test)] mod backoff_tests;` in `resolume/mod.rs`) so the over-cap `tests.rs` debt never blocks
  you and the fn-length gate stays green.
