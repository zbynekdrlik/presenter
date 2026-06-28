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

## Mutation Gate

Fixed 2026-06-21 (PR #435, issue #439): dedicated `mutation-warm` bootstrap job + 16 shards.
The gate now passes on normal small PRs. Full-tree catch-up is on-demand via `/mutation-sweep`.

## Quality-Check Gate Landmines (#483 lessons)

The `quality-check.sh --strict --against origin/main` gate (the "Quality Checks" job) hard-fails
on the **changed-file set only** — but since the #407/#482 `count_prod_lines.sh` fix it now counts
correctly (past `#[cfg(test)] mod tests;`). Two pre-existing-debt landmines:

- **File-size (>1000 prod lines) + fn-length (>120 lines, tests NOT exempt):** TOUCHING an
  already-over-cap file/function pulls it into the diff and HARD-FAILS your PR — even if your edit
  is unrelated. Known offenders: `state/mod.rs` (~1117 prod lines, #486), `resolume/tests.rs`
  (test fns 168/166 lines, #487). Check before editing: `bash scripts/dev/count_prod_lines.sh <file>`
  and `QC_TARGETS=<file> python3 scripts/dev/fn_length_check.py .`.
  **Workaround when you must add code near an offender:** wire through a SMALL sibling file instead
  of the god-file (e.g. add the call in `state/integrations.rs`, not `state/mod.rs`), and put NEW
  tests in their OWN file (e.g. `resolume/latency_tests.rs`) so the bloated `tests.rs` stays out of
  the diff. Then `git diff --name-only origin/main...HEAD` must NOT list the offender.

- **Diff-scoped mutation gate (`cargo mutants --in-diff`, 16 shards, blocking):** mutates only
  CHANGED lines. Refactoring a function (e.g. extracting `handle_stage` under the 120 cap) marks ALL
  its lines changed → drags pre-existing EDGE logic into mutation scope. Kill survivors HONESTLY (no
  `exclude_re` for code that carries behavior):
  - pure telemetry helpers (`count_clips`, an `as_str`, a `duration_ms(Duration)->f64`) → make
    `pub(super)` + unit-test the exact output (kills `replace-body` + arithmetic mutants).
  - side-effect/audit/wiring fns (writer task, `record_*`, `attach_*`) → ONE end-to-end test that
    asserts the observable effect (a DB row appears) kills all the `-> ()` / `-> Ok(())` no-op mutants.
  - untestable guards (`if TRIGGER_DELAY.as_millis() > 0` — TRIGGER_DELAY is 0 in test builds) → just
    DROP the guard (the bare op is a no-op at 0).
  - log a `Duration` via `?d` instead of `d.as_secs_f64() * 1000.0` to remove arithmetic mutants from
    a behavioral fn (or route the `* 1000.0` through a tested `duration_ms` helper).
  - **VERIFY LOCALLY before re-pushing** (CI mutation is ~13 min sharded but a full re-push is ~30 min):
    `git diff origin/main...HEAD > /tmp/pr.diff && cargo mutants --in-diff /tmp/pr.diff --baseline=skip --test-tool=nextest --jobs 4 -- --all-targets`.
    Watch `mutants.out/missed.txt` (must stay empty). Local cold-build is slow (~50 min for ~50 mutants)
    but cheaper than a failed CI cycle. cargo-mutants does NOT mutate `#[cfg(test)]`/`#[test]` code.
