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
