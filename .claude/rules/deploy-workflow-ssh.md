---
paths:
  - ".github/workflows/deploy.yml"
  - ".github/workflows/pipeline.yml"
  - ".github/workflows/release.yml"
  - ".github/workflows/import-data.yml"
  - "scripts/ci/check_deploy_workflows.py"
---

# Deploy workflow SSH alias safety (#795)

**Why this exists:** the ONE self-hosted runner (`presenter-local`) shares a single
`~/.ssh/config` across every job. In the v0.4.288 release, release.yml's PP
keyscan failed, the setup step exited before rewriting `Host deploy-target`, and
the `if: always()` "Start service" step (#469) then ran `ssh deploy-target` against
the PREVIOUS job's alias — SNV prod (10.77.9.205). Harmless only because auth failed.

## Rules (enforced by `scripts/ci/check_deploy_workflows.py` in the `quality` job)

- **Every SSH setup step that defines `Host deploy-target` has `id: ssh_setup` and
  does `rm -f ~/.ssh/config` + `cat > ~/.ssh/config` (truncate, never `>>`) BEFORE
  any `ssh-keyscan`.** A later failure then leaves an alias pointing at THIS job's
  `DEPLOY_HOST`, never another job's host. Extra aliases for the same job
  (`prod-server`, `companion-snv`/`companion-pp`) are APPENDED after it with `>>`.
- **A remote step (`ssh deploy-target`, `scp`/rsync `deploy-target:`) whose `if:`
  uses `always()` / `failure()` / `cancelled()` MUST also require
  `steps.ssh_setup.outcome == 'success'`.** The #469 "always bring the service back"
  intent only applies once the job actually touched the host. Steps with no status
  function get implicit `success()` and are skipped after any failure — no gate needed.
  The gate must be AND-ed: a condition containing `||` is rejected (an OR-ed gate
  gates nothing). Single or double quotes around `success` are both accepted.
- **Any executable mention of `deploy-target` counts as remote use** (backslash
  continuations are joined, so `ssh -o X \` + `deploy-target "..."` is caught).
- **The checker ignores shell comment lines** (comments freely mention
  `ssh-keyscan` / `ssh deploy-target`). Run it locally:
  `python3 -m unittest discover -s scripts/ci -p 'test_check_deploy_workflows.py'`
  and `python3 scripts/ci/check_deploy_workflows.py` (delete `scripts/ci/__pycache__`
  afterwards — it is not gitignored).
- Adding a new deploy workflow/job with a `deploy-target` alias: it is picked up
  automatically (every `.github/workflows/*.yml` is scanned).
- **Replace a host's known_hosts entry only AFTER a successful `ssh-keyscan`.** The shared
  runner's `~/.ssh/known_hosts` is also what local sessions on dev2 use. Removing the entry
  (`ssh-keygen -R`) BEFORE a scan that then fails (the host is offline) leaves the host out of
  the file, so every later `ssh` to it fails "Host key verification failed" (PP internet
  outage 2026-10-14). Scan into a `mktemp` file, and only when it is non-empty run
  `ssh-keygen -R` + append. Recovery by hand: compare the fingerprint of a fresh scan with
  `ssh-keygen -F <host> -f ~/.ssh/known_hosts.old` before re-adding it.
- **Staged deploy (deploy.yml): until "Swap staged binaries", the tools in the deploy dir are the
  PREVIOUS release's** (invariants 3–4 of the checker, #851 D1). Before the swap run only
  `staging.new/<tool>`; `ingest_bibles` runs only after the swap AND after the step that waits for
  `/healthz` — it opens the DB with its own build's migrations, so earlier it would migrate the DB
  under the old service (an aborted deploy leaves it unable to boot) or race the new service's
  migration run. pipeline.yml / release.yml stop the service and copy the binaries first, so the
  rule does not apply there.
- **No Companion deploy in a dev-push workflow** (invariant 5, #850). Companion runs only on the
  production hosts; `scripts/deploy/deploy_companion_module.sh` runs from deploy.yml (SNV) and
  release.yml (PP) as the "Deploy Companion module" step, through the job's `deploy-target`
  alias. pipeline.yml (every dev push) never runs it, `repin-presenter-connection.py` or
  `systemctl stop|start|restart companion` (running the repin unit-test file is fine).
- **The SEB / Roháček / SEVP / Milosť modules come from the private repo (#859).** A job that
  scps them or sets `PRESENTER_BIBLE_*` to them has the step "Check out the private Bible modules
  (#859)" right after its main checkout (`path: data/bibles-private`,
  `persist-credentials: false`) and reads `data/bibles-private/<file>`; release.yml
  `build-release` (the public tarball) never checks it out. Enforced by
  `scripts/ci/check_public_bibles.py`; details in `.claude/rules/bible-import.md`.
