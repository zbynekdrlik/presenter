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

## e2e-ndi lane NEVER compiles Rust — prebuild artifacts in the GH Build job (#519 lesson)

The self-hosted `e2e-ndi` lane (`timeout-minutes: 20`) must only RUN pre-built artifacts,
never `cargo build` on the runner (matches the runner architecture: the local runner doesn't
compile Rust). A **toolchain bump on dev2** (e.g. `rustc` stable auto-updated) invalidates the
runner-workspace Rust cache → a `cargo build … ndi_test_sender` step then cold-rebuilds the whole
gstreamer tree (~19 min) → blows the 20-min cap → GitHub cancels the job → deploys skipped → ALL
PRs blocked. This looks like a mystery cancellation with green logs. Fix (already in `pipeline.yml`):
the `Build` job (GH-hosted, deps hot) builds `cargo build --release -p presenter-ndi --features
test-helpers --bin ndi_test_sender` and ships it in `build-artifacts`; `e2e-ndi` just `cp`s
`_artifacts/ndi_test_sender target/release/` and runs it. If e2e-ndi ever cold-builds again, check
whether a new Rust-compiling step crept into that lane — don't raise the timeout.

## `quality-check.sh --strict` IS the CI "Quality Checks" gate — run it locally before push

CI runs `./scripts/dev/quality-check.sh --strict --against origin/main` (not just `fn_length_check.py`).
Run that EXACT command locally before pushing. Hard-fails: file >1000 prod lines, **fn >120 lines
(tests NOT exempt; >80 is only a warn)**, `continue-on-error` in workflows, and cargo-deny/cargo-audit
policy. Note: cargo-deny/cargo-audit can transiently FAIL then PASS on an immediate re-run (advisory
DB / lock timing) — re-run once before treating a deny/audit failure as real. A >120 fn added by a
feature → split a helper out (e.g. #514 split `extract_inbound_video` out of `post_client_stats`).

**`quality-check.sh` is NOT Tier-0-safe in ANY mode** (#586/#587 lesson, corrected #610) — it
unconditionally shells out to `cargo fmt --all --check`, `cargo clippy -p presenter-server --tests`,
and `cargo check` regardless of `--strict`; that flag only decides whether a failing check is fatal,
not whether these run. A worker under a "no local Rust build/check/clippy" constraint must NOT run
this script at all, in any mode; use the two underlying non-compiling checks directly instead:
`bash scripts/dev/count_prod_lines.sh <one-file-per-call>` and
`QC_TARGETS=<newline-separated-files> python3 scripts/dev/fn_length_check.py .` — both are pure
bash/Python and cover the two hard-fail gates (file size, function length) without touching cargo.
**`QC_TARGETS` is NEWLINE-separated, not comma-separated** (confirmed against the script's own
docstring: "newline-separated RELATIVE file paths") — a comma- or space-joined value matches zero
files and `fn_length_check.py` silently prints `{"violations": [], "warnings": []}`, which reads
exactly like a clean pass. Build the value with `printf '%s\n' file1 file2 ... | ...` or a small
wrapper script that reads a newline-delimited file into the env var, never a one-line
`QC_TARGETS="a,b,c"` (#647 lesson: a worker's own space-joined attempt silently returned an
empty-looking clean result before this was caught).

## RED-before-GREEN verification under Tier-0 — push RED alone, read `--log-failed` (#608)

A Tier-0 worker (no local `cargo test`) cannot run the new regression test locally to watch it
fail. The only way to satisfy "verify it fails for the right reason" is via CI itself: commit the
RED test(s), push ALONE (before the fix commit exists), poll the run, and once the `Test` job
reaches a terminal `failure`, read `gh run view <id> --log-failed | grep -iE "FAILED|assertion"` to
confirm the SPECIFIC new test names are the ones that failed (not some unrelated flake) and that
the failure is the expected assertion (e.g. status-code mismatch), not a compile error or panic
from something else. Only then commit + push the GREEN fix and poll again for a fully green run.
This costs 2 pipeline runs instead of 1, but is the only way this project's Tier-0 constraint and
`regression-test-first.md`'s RED-before-GREEN mandate can both be satisfied — do not skip the RED
push to save a CI cycle (#607, #608 both did exactly this: RED push failed on cue, GREEN push went
green). If the RED batch includes a coverage-only test alongside real bug-fix tests (a test that
should already pass because the underlying behavior isn't actually broken), confirm from the SAME
`--log-failed` output that it's absent from the failed-test list — that's your proof it wasn't
accidentally testing the wrong thing.

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

## Every "Setup SSH" step needs a retry-wrapped `ssh-keyscan`, never a bare one (#674)

All three "Setup SSH for deployment" steps (`deploy.yml`, `pipeline.yml`, `release.yml`) run under
the step's default `bash -e`. A **bare** top-level `ssh-keyscan $HOST >> known_hosts 2>/dev/null`
aborts the WHOLE step via `-e` the instant `ssh-keyscan` itself returns non-zero (a transient
connect timeout) -- BEFORE reaching any `[ ! -s known_hosts ]` diagnostic written to explain
exactly that failure. Hit live: run 31616035821 died in ~5s with a bare `exit code 1` and zero
explanation, discarding a ~20-minute build.

**The fix pattern, now applied to all three steps** -- wrap the scan in `if ...; then` (a command's
exit status inside `if` is exempt from `-e`'s abort behavior) with a bounded retry (5 attempts,
explicit `-T 5` timeout, 3s backoff), and run `ssh-keygen -R $HOST -f known_hosts` (best-effort)
BEFORE the scan so a re-deploy never accumulates duplicate host-key entries (the runner's
`known_hosts` is a PERSISTENT file across deploys, unlike an ephemeral GitHub-hosted runner's).
Reuse this exact shape for any NEW ssh-based deploy step added to these workflows -- never a bare
`ssh-keyscan ... 2>/dev/null` again.

## Dead `deb.nodesource.com` apt source breaks `--with-deps` browser install (#610)

The `NDI WebRTC E2E` job's "Install Playwright (branded Chrome for H.264)" step runs
`npx playwright install --with-deps chromium chrome`, which internally does `apt-get update` across
EVERY configured apt source — if ANY one source 403s, the whole `apt-get` call fails and Playwright
reports "Failed to install browsers" / exit code 100, even though every OTHER OS dependency would
have resolved fine.

**Symptom:** job fails at that exact step with `E: Failed to fetch https://deb.nodesource.com/...
403 Forbidden` / `The repository '...' is no longer signed.` Confirm it's a real dead endpoint (not
a transient blip) with `curl -sI https://deb.nodesource.com/node_22.x/dists/nodistro/InRelease` —
a Cloudflare/S3 `AccessDenied` on retry means the endpoint is genuinely gone, not flaky.

**Why it's safe to just delete the source:** this runner's actual node/npm come from **nvm**
(`NVM_BIN` is set in the runner's `~/actions-runner/.env`, e.g. `v24.12.0`), not the dpkg
`nodejs` package `deb.nodesource.com` provides. The nodesource apt source is leftover
provisioning cruft with no live consumer — check `which -a node npm` (nvm path first) and
`cat ~/actions-runner/.env` before assuming otherwise.

**Fix:**
```bash
sudo rm -f /etc/apt/sources.list.d/nodesource.list /usr/share/keyrings/nodesource.gpg
sudo apt-get update   # should now complete cleanly, no 403s
gh run rerun <run-id> --failed   # reruns only the failed job, others stay untouched
```

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
  is unrelated. **#654 post-mortem CORRECTION (verified against quality-check.sh source + run
  31068027673 log): the 800-line TARGET is WARN-only even under `--strict` — only >1000 prod
  lines hard-fails.** The round-1 "state/mod.rs exceeds target size (800): 868" line sat under
  `[quality] Warnings:`; the job's actual Failure was the serde convention check
  (`AbleSetMismatchAck` missing `#[serde(rename_all = "camelCase")]`). Do NOT split files
  defensively at 800 — the hard budget is the 1000 cap (an 989-line direct edit passed in #626/#649). Same batch's fn-growth trap: inlining a feature into an
  existing 100-119-line fn (`run_tracker` 110→126, `from_config` 119→122) flips it past the 120
  hard cap — extract a helper proactively when a touched fn is already >100 lines. And a meta-test
  that source-scans a directory (`state/*.rs`) counts its OWN pattern occurrences in test files —
  exclude `tests.rs`/`*_tests.rs` from such scans. Known offender: `state/mod.rs` (was ~1117, #486;
  re-split in #656 to 783 — split again before it approaches the 1000 hard cap). `resolume/tests.rs`
  was fixed in #487 (shared `mount_composition`/`mount_params`/`mount_clips`/`mount_full_composition`/
  `build_driver` helpers + `stage_all`/`stage_main_meta` builders → all fns now ≤120). Check before
  editing: `bash scripts/dev/count_prod_lines.sh <file>` and
  `QC_TARGETS=<file> python3 scripts/dev/fn_length_check.py .`.
  **#594 footgun: invoking `fn_length_check.py` with file PATHS as the argument (instead of `.` with
  `QC_TARGETS` set) silently reports `{"violations": [], "warnings": []}` no matter what — the
  script's only positional arg is `<repo_root>` to `os.walk`; a file path isn't a walkable directory,
  so it silently walks nothing.** Appending finding-4's test onto an existing 88-line test function
  once passed this false-negative local check, then failed CI's real `Quality Checks` job at 123
  lines. ALWAYS invoke it exactly as shown above (repo root as arg 1, target file(s) via the
  `QC_TARGETS` env var, newline-separated for multiple) — never pass a file path as argv[1].
  **Workaround when you must add code near an offender:** wire through a SMALL sibling file instead
  of the god-file (e.g. add the call in `state/integrations.rs`, not `state/mod.rs`), and put NEW
  tests in their OWN file (e.g. `resolume/latency_tests.rs`) so the bloated `tests.rs` stays out of
  the diff. Then `git diff --name-only origin/main...HEAD` must NOT list the offender.
  **The established split pattern (used by #486 for `resolume.rs`/`audit.rs`,
  `router/integrations/*`, and #590 for `repository/android_stage.rs`+`video_source.rs` and
  `router/bible/*`): one cohesive CRUD/handler group per sibling file, `use super::Repository;` (or
  `use super::super::AppError;` one level deeper for a router subdirectory) + its own
  `impl Repository { ... }` block or `pub(crate)` handler fns — inherent methods/handlers split
  across files are invisible to callers, so the parent file's public surface is unchanged and no
  route-table restructuring is needed. Before inventing a new split shape, grep for an existing
  sibling (`resolume.rs`, `router/integrations/android_stage.rs`) and copy its shape — don't design
  one from scratch.** #586/#587/#588/#589 (queued next, same 21-site sweep as #584/#590) will need
  this exact same shape again.

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

## gh CLI Quirk — `gh pr edit` Fails on This Repo

`gh pr edit <N> --body-file ...` dies with `GraphQL: Projects (classic) is being deprecated ... (repository.pullRequest.projectCards)` — the repo has a legacy Projects-classic reference and gh's edit path always queries projectCards. Workaround (works every time):

```bash
gh api -X PATCH repos/zbynekdrlik/presenter/pulls/<N> -F "body=@body.md"
```

## presenter-ui Is a Workspace `exclude` — Root fmt/clippy/test NEVER See It

`crates/presenter-ui` (WASM) is in `[workspace] exclude`, so `cargo fmt --all`,
`cargo clippy --workspace`, and `cargo test` from the root silently skip it. Run its
checks FROM THE CRATE DIR: `cd crates/presenter-ui && cargo fmt --check && cargo clippy
--all-targets -- -D warnings && cargo test --lib`. CI's Format job checks it explicitly
(second step, `working-directory: crates/presenter-ui`) — an unformatted presenter-ui
file now fails CI, so always `cargo fmt` inside the crate before pushing UI changes.

## After a `--merge` (merge-commit) PR, ALWAYS merge main back into dev before continuing (#591)

`gh pr merge <N> --merge` creates a NEW merge commit that lands ONLY on `main` — it is
NOT automatically present on `dev`'s own history (dev's tip stays the pre-merge commit
your PR was built on; only `main` gains the two-parent merge commit). Continuing
straight to a post-release task on `dev` (e.g. the standard version-bump-after-release
commit) WITHOUT first pulling that merge commit back in makes `dev` genuinely 1 commit
behind `main` from the Branch Sync Check's point of view (`git rev-list --count
HEAD..origin/main`) — even though every LINE of content already matches. The check
fails with the exact fix in its own error message, but it costs a wasted CI cycle if
you don't do it proactively:

```bash
git fetch origin
git checkout dev
git merge origin/main -m "Merge main (PR #N merge commit) back into dev"
git push origin dev
```

This resolves as a clean, conflict-free merge (the content already matches — you're
just re-uniting the two DAG branches at the merge commit), so it's always safe to run
immediately after ANY `--merge`-style PR merge, before starting the next commit on dev
(version bump or otherwise). `git rev-list --count origin/dev..origin/main` should read
`0` before you push anything else.

## Pipeline run can appear MINUTES late after a push (GitHub delay)

A `git push` to dev normally spawns the Pipeline run within seconds — but after GitHub
infra hiccups the run can appear MINUTES later (observed 2026-08-07: push at 07:0x, run
created ~07:20, then ran normally to success). A missing run right after push is NOT a
lost event: query by exact SHA before re-triggering anything:

```bash
gh api "repos/zbynekdrlik/presenter/actions/runs?head_sha=$(git rev-parse HEAD)" \
  --jq '.total_count, (.workflow_runs[] | "\(.id) \(.name) \(.status)")'
```

Only if it stays absent for ~15+ min consider `gh workflow run pipeline.yml --ref dev`.

## CI waiters are a convenience, NEVER the source of truth (2026-08-11)

The background `gh run view` poll loop this project uses to wait out a ~40-min Pipeline
**gets killed by the harness at unpredictable moments** (3 kills in one session on
2026-08-11; the notification reads `was stopped`, distinct from the `stopped by Claude`
wording of a deliberate `TaskStop`, and distinct from a non-zero exit — so it is external
task-cleanup, not a crash of the command). The CI run itself is completely unaffected.

The failure mode to avoid is waiting BLIND on a success-only signal: a dead waiter sends
neither "done" nor "failed", so a session that trusts silence hangs forever.

Protocol, every single wake-up:

1. Re-derive the run's state from the DURABLE resource: `gh run view <id> --json status,conclusion`.
2. Only if it is still `in_progress`, relaunch **one** fresh waiter. Never rerun/cancel the
   CI run itself — nothing is wrong with it.
3. Never infer "still running" from the absence of a notification.

A killed waiter costs nothing when this is followed; it cost 3 × ~1 min of re-derivation.

## `cargo fmt --all` does NOT cover presenter-ui — CI checks it separately (2026-08-11)

The Format job runs TWO steps: `cargo fmt --all -- --check` at the workspace root, then a
second `cargo fmt --check` **inside `crates/presenter-ui`**. That crate is workspace-`exclude`d
(own `Cargo.lock`), so the root `--all` sweep silently skips it — a local `cargo fmt --all --check`
can be perfectly clean while CI's Format job fails with `Diff in .../presenter-ui/src/...`.

Before any push that touches `crates/presenter-ui`, run BOTH:

```bash
cargo fmt --all -- --check
(cd crates/presenter-ui && cargo fmt --check)
```

Same trap applies to `cargo check`/`clippy` on that crate (`cargo <cmd> -p presenter-ui` from
the root fails — run it from `crates/presenter-ui/`, see the deploy skill).

## `cargo fmt`/`cargo check` for presenter-ui from inside a nested `.claude/worktrees/` checkout — FIXED (#669, 2026-08-12)

**Historical trap (2026-08-11 → 2026-08-12), now fixed — kept for context.** `cd crates/presenter-ui
&& cargo fmt` (or `cargo check`/`cargo metadata`/`cargo locate-project`) used to fail there with
`current package believes it's in a workspace when it's not: current: .../crates/presenter-ui/
Cargo.toml  workspace: /home/.../presenter-dev2/Cargo.toml` — it named the repo's **MAIN checkout**
as the workspace, not the worktree's own root, even though the worktree root has its own valid
`[workspace]` + `exclude = ["crates/presenter-ui"]`.

Root cause: cargo's workspace-root search does NOT stop at the first ancestor `[workspace]` that
EXCLUDES the current package — it treats "excluded here" as "keep climbing", not "standalone,
done". Since `.claude/worktrees/<name>/` is filesystem-nested *under* the main checkout, cargo kept
climbing straight past the worktree root and found the outer main-tree `Cargo.toml` next — which
did NOT exclude the (now oddly-nested) path, so it errored as "not in workspace.members". This was
invisible in the MAIN checkout (nothing to climb past) and invisible in CI (flat clone, no nesting)
— it only bit a worktree-isolated agent.

**The fix:** `crates/presenter-ui/Cargo.toml` now carries its own explicit `[workspace]` table
(#669). That makes cargo's very first manifest check return `Root` for the crate immediately, so
the ancestor walk that caused the collision never runs at all — immune to nesting depth, and it
also fixes `rust-analyzer`/IDE tooling in a worktree. Both `cargo fmt --check` and
`cargo check --target wasm32-unknown-unknown` now work normally from `crates/presenter-ui/` inside
ANY nested worktree, same as from the main checkout. Regression proof (CI's own `Format` job
checks out non-nested and can never catch a regression here):
`scripts/dev/check-presenter-ui-worktree-fmt.sh` creates a throwaway nested worktree, runs
`cargo fmt --check` inside it, asserts exit 0, and cleans up unconditionally.
- The rest of the workspace (`presenter-server`, `presenter-core`, etc.) was never affected —
  `cargo check --workspace --tests` from the worktree root has always worked normally.

## A fresh worktree can inherit an OLD, unrelated `git stash` conflict (#641)

Observed once in a fleet-dispatched `isolation: "worktree"` worker: `git status` showed
`UU docs/autopilot-log.md` (unmerged) despite never running `git stash`/`git merge`
myself. `git stash list` had a weeks-old entry (`"leftover autopilot-log from prev
cycle"`) from a long-abandoned session; worktree setup apparently tried to pop it
against current `dev`, and it conflicted because `dev` had moved on since that stash was
made. The conflict markers land in the file's tail (`<<<<<<< Updated upstream` /
`=======` / `>>>>>>> Stashed changes`) — check for them with `grep -n '^<<<<<<<\|^=======\|^>>>>>>>'`
on any file `git status` reports as unmerged at the START of a worktree session, before
assuming it's your own doing. Resolve by keeping the real committed ("Updated upstream")
content and dropping the stray stash side if it's redundant (cross-check: the stash's
info is usually ALREADY recorded properly elsewhere in the file, since these logs are
append-only). Leave the shared stash entry itself alone (`stash list`/`stash drop`
touches the WHOLE repo's shared `.git`, which other concurrent worktrees in the same
fleet round may still reference) — fix the file content only.

## Integrating parallel worker branches — union-merge purely additive conflicts

When two worktree-agent branches both APPEND at the same spot (a `mod x;` list, a `.merge(x::routes())`
chain, an `AppState` field + its `new()` init, a playbook section), the conflict is a pure both-sides
union. Resolve mechanically from the index stages instead of hand-editing:

```bash
for f in $(git diff --name-only --diff-filter=U); do
  git show :1:"$f" > base; git show :2:"$f" > ours; git show :3:"$f" > theirs
  git merge-file --union ours base theirs && cp ours "$f" && git add "$f"
done
```

Then grep the result for BOTH sides (e.g. both `mod` lines, both `.merge(` calls) and for `<<<<<<<`.
Gotcha: a paragraph both sides EDITED differently (not appended) is kept TWICE by `--union` — check
prose files for a doubled sentence after the merge (stream-graphics.md, PR-4 round of #718).

### Rehearse the NEXT wave's merges while the current wave's CI runs

Serial integration leaves the main session idle for ~45 min per Pipeline. Use that time: dispatch a
sonnet agent to `git worktree add -b rehearsal-<wave> <scratchpad>/rehearsal dev`, merge the held
worker branches there in the planned order, union-resolve, run the non-compiling gates (`cargo fmt
--check` in BOTH the root and `crates/presenter-ui`, `count_prod_lines.sh`, `fn_length_check.py`,
`npx tsc --noEmit -p tsconfig.json`, `npm run test:companion`), write a recipe file (per merge:
conflicted files + exact resolution), then `git worktree remove --force` + `git branch -D` the
rehearsal branch. The real merge onto `dev` then replays the recipe in 2–3 commands (PR-6 of #718:
3 predicted additive conflicts, zero surprises). Typical catch: rustfmt `reorder_modules` wants
`pub mod` lines alphabetical after a union merge (`stream_editor` before `stream_output`) — the
gate reports it before CI does.
