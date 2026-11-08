# PRBA Instance on dev1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A third, data-separate Presenter instance for PRBA on dev1, reachable at `https://prba.newlevel.media`, seeded once from SNV and updated by every merge to `main` right after SNV.

**Architecture:** The SNV deploy job in `deploy.yml` becomes a reusable workflow `deploy-site.yml` (`workflow_call`) with per-site inputs; `deploy.yml` calls it twice — SNV, then PRBA (`needs:` SNV). The SNV call keeps every step it has today; hardware/site-only steps are gated by a `hardware: true` input. A one-time seed script copies SNV's DB to dev1 with every device table cleared.

**Tech Stack:** GitHub Actions (self-hosted runner on dev2), bash, sqlite3, systemd, cloudflared, Python (unittest) for the seed checks and the workflow guard.

**Spec:** `docs/superpowers/specs/2026-11-08-prba-instance-design.md` (ticket #918)

## Global Constraints

- One code base: same branch, same release on SNV, PP and PRBA. PRBA reuses the SNV build artifact of the same run.
- dev1 = `100.104.8.125` (tailscale). Deploy as `newlevel` with sudo; the service runs as `presenter-prba` (system user, `/usr/sbin/nologin`).
- PRBA: dir `/opt/presenter-prba`, service `presenter-prba`, port `8081`, DB `sqlite:///opt/presenter-prba/presenter.db`, `PRESENTER_COMPANION_ENABLED=0`, no sync peer, no stage URL, recordings `/opt/presenter-prba/recordings`.
- SNV prod DB is read only: the seed uses `sqlite3 -readonly … ".backup"`.
- A PRBA deploy must never create or import a fresh DB: missing DB → `::error::` + exit 1.
- The SNV call of the reusable workflow must run exactly the steps it runs today (same order, same commands); `scripts/ci/check_deploy_workflows.py` (#795) keeps passing.
- `https://prba.newlevel.media` is public, no Cloudflare Access app.
- PP (`release.yml`) is untouched.

## Review Focus

1. Seed run twice (second run after PRBA users edited songs) → refuses, DB unchanged. Test in Task 1.
2. A device-like key survives in `app_settings` (e.g. a new site key added after this plan) → seed fails naming the key. Test in Task 1.
3. `deploy-prba` before the seed (DB missing) → job fails with the error, no `presenter.db` created. Test in Task 2 (guard script test).
4. SNV call drifting from today's behavior after the extraction → the step list of the SNV run equals today's list. Test in Task 2.
5. PRBA job red while SNV succeeded → SNV stays live and its `/healthz` version is the new one. Checked in Task 5 verification.

---

### Task 1: One-time seed script

**Files:**
- Create: `scripts/ops/seed_prba_from_snv.sh` (orchestration: backup on SNV, copy to dev2 scratch, clear, push to dev1, copy libraries)
- Create: `scripts/ops/prba_seed_clean.py` (pure step: clear tables + leftover-address check on a local DB file)
- Test: `scripts/ops/test_prba_seed_clean.py`

**Interfaces:**
- Produces: `prba_seed_clean.py <db_path>` — exit 0 and prints `cleared <table>=0` lines + `kept presentations=<n> libraries=<n>`; exit 1 with `leftover: <table>.<key>` on a remaining address-like setting.
- `CLEARED_TABLES = ["resolume_hosts", "resolume_push_audit", "android_stage_displays", "video_sources", "ableset_settings", "able_set_settings", "osc_settings", "stage_state", "ui_usage", "interpreter_tips", "settings_audit"]` plus `stream_outputs` rows (outputs point at SNV hardware; scenes/elements/fonts/nameplates/assets are kept as reusable design data).
- `app_settings`: delete keys starting `listen.` and `sync.`; then fail if any remaining value matches `\b\d{1,3}(\.\d{1,3}){3}\b`, `https?://`, or `\.lan\b`.

- [ ] **Step 1: Write failing tests** — build a temp sqlite DB with the tables above (minimal columns) holding one row each, `presentations` with 2 rows, `app_settings` with `listen.mics`, `sync.peer`, `ui.theme`.
  - `test_device_tables_are_emptied`: after run, every `CLEARED_TABLES` count = 0, `stream_outputs` = 0.
  - `test_songs_are_kept`: `presentations` count = 2.
  - `test_listen_and_sync_keys_go`: `app_settings` keys = `{"ui.theme"}`.
  - `test_leftover_address_fails`: add `app_settings('ndi.host','http://10.77.9.205')` → exit 1, stderr contains `leftover: app_settings.ndi.host`.
  - `test_missing_table_is_not_an_error`: drop `able_set_settings` → still exit 0.
- [ ] **Step 2:** `python3 -m unittest scripts/ops/test_prba_seed_clean.py -v` → FAIL (module missing).
- [ ] **Step 3:** Implement `prba_seed_clean.py` (stdlib `sqlite3`, `set`-based table existence check, one transaction, `VACUUM` after).
- [ ] **Step 4:** Tests → PASS.
- [ ] **Step 5:** Implement `seed_prba_from_snv.sh` (`set -euo pipefail`):
  1. `ssh newlevel@100.104.8.125 test ! -e /opt/presenter-prba/presenter.db` else `echo "PRBA DB exists — refusing to re-seed" >&2; exit 1`.
  2. `ssh newlevel@presenter.lan "sqlite3 -readonly /opt/presenter/presenter.db '.backup /tmp/prba-seed.db'"`, `scp` it to a `mktemp -d`, `ssh … rm /tmp/prba-seed.db`.
  3. `python3 scripts/ops/prba_seed_clean.py <local copy>`.
  4. `scp` to `newlevel@100.104.8.125:/tmp/`, then `sudo install -o presenter-prba -g presenter-prba -m 640` into `/opt/presenter-prba/presenter.db`.
  5. `rsync -a newlevel@presenter.lan:/opt/presenter/libraries/` via dev2 into `/opt/presenter-prba/libraries/`, `chown -R presenter-prba:`.
- [ ] **Step 6:** Commit `feat(prba): one-time seed from SNV with SNV devices cleared (#918)`.

### Task 2: Reusable deploy workflow + PRBA job

**Files:**
- Create: `.github/workflows/deploy-site.yml` (`on: workflow_call`), the body of today's `deploy` job moved here.
- Modify: `.github/workflows/deploy.yml` — `deploy` job → `uses: ./.github/workflows/deploy-site.yml` with SNV inputs; new `deploy-prba` job, `needs: [build, deploy]`, PRBA inputs.
- Create: `scripts/deploy/presenter-prba.service`
- Create: `scripts/deploy/require_seeded_db.sh` + `scripts/deploy/test_require_seeded_db.sh` (or a Python test beside `check_deploy_workflows.py`)
- Modify: `scripts/ci/check_deploy_workflows.py` + its test — include `deploy-site.yml` in the scanned set.

**Interfaces:**
- `deploy-site.yml` inputs: `site` (string, `SNV`|`PRBA`), `deploy_host`, `deploy_dir`, `service_name`, `unit_file` (path under `scripts/deploy/`), `hardware` (boolean). Secrets: `inherit`.
- `hardware: true` keeps: ADB, NDI SDK/avahi, GStreamer+VA-API, `wait-for-h264-encoder.sh`, stage-url drop-in, sync-peer drop-in, TURN, NDI fan-out self-heal, Companion module. `hardware: false` skips exactly those.
- Input `library_sync` (boolean, SNV `true`, PRBA `false`): gates "Sync ProPresenter libraries from git" and "Import ProPresenter (first deploy or schema reset)". Owner 2026-11-08: SNV data is copied ONCE at the start, afterwards PRBA edits its own — no SNV song files reach PRBA on any later deploy.
- Every hardcoded `/opt/presenter` in the moved steps becomes `${{ inputs.deploy_dir }}`; `presenter.service` → `${{ inputs.unit_file }}` / `${{ inputs.service_name }}`.
- `require_seeded_db.sh <db_path>` (runs over ssh before "Backup database" when `hardware == false`): exit 1 with `::error::PRBA database missing — run scripts/ops/seed_prba_from_snv.sh first` if the file is absent or empty.
- PRBA cloudflared step uses vars `CLOUDFLARED_TUNNEL_ID_PRBA` / `CLOUDFLARED_HOSTNAME_PRBA` and secret `CLOUDFLARED_CREDS_PRBA`, service `cloudflared-prba` (not the shared `cloudflared service install`, which would replace another tunnel on dev1).

- [ ] **Step 1: Failing tests**
  - `test_require_seeded_db`: missing path → exit 1 + the error text; empty file → exit 1; non-empty → exit 0.
  - In `test_check_deploy_workflows.py`: `test_snv_step_list_unchanged` — parse `deploy-site.yml` with `hardware: true` and assert the step-name list equals today's 33 names from `deploy.yml` (copy the list from `grep -n "      - name:"` lines 144–794 into the test as a literal).
  - `test_prba_skips_library_sync`: "Sync ProPresenter libraries from git" and "Import ProPresenter (first deploy or schema reset)" carry `if: inputs.library_sync`; the PRBA call passes `library_sync: false`.
  - `test_prba_skips_hardware_steps`: with `hardware: false` the names `Ensure ADB is installed`, `Ensure NDI SDK and avahi-daemon are installed`, `Configure TURN credentials`, `NDI fan-out self-heal (#768)`, `Deploy Companion module` are gated by `if: inputs.hardware`.
- [ ] **Step 2:** Run them → FAIL.
- [ ] **Step 3:** Move the job, parameterize, add the gate and the guard, write `presenter-prba.service` (from `presenter.service`: `User/Group=presenter-prba`, no `ExecStartPre`, no `SupplementaryGroups`, no `CAP_NET_BIND_SERVICE` (port 8081), `ReadWritePaths=/opt/presenter-prba`, the env lines from Global Constraints, `EnvironmentFile=-/etc/presenter-prba/ai.env`, same `CPUQuota/MemoryMax/TasksMax`).
- [ ] **Step 4:** Tests → PASS; `python3 scripts/ci/check_deploy_workflows.py` → exit 0; `actionlint` if installed.
- [ ] **Step 5:** Commit `feat(deploy): reusable site deploy; PRBA after SNV on every main merge (#918)`.

### Task 3: dev1 host setup + seed (live, by hand, from dev2)

- [ ] **Step 1:** On dev1: `sudo useradd --system --home /opt/presenter-prba --shell /usr/sbin/nologin presenter-prba`, `sudo install -d -o presenter-prba -g presenter-prba /opt/presenter-prba`.
- [ ] **Step 2:** Add the deploy key's public half to `newlevel@dev1:~/.ssh/authorized_keys` (derive it with `airuleset.py secret exec` from `DEPLOY_SSH_KEY`'s source — never print the private key). Verify: `ssh -i <key> newlevel@100.104.8.125 true` from the runner user on dev2.
- [ ] **Step 3:** Start-test the current release binary on dev1: copy `presenter-server` from SNV's `/opt/presenter/`, run `PRESENTER_STARTUP_MODE=validate PRESENTER_DB_URL=sqlite:///tmp/prba-probe.db PRESENTER_PORT=0`; install missing GStreamer/NDI runtime packages (`ldd` "not found" lines) with apt until `/healthz` answers. Remove the probe DB.
- [ ] **Step 4:** Run `scripts/ops/seed_prba_from_snv.sh` from dev2. Expected: all `cleared … =0`, `kept presentations=<SNV count>`.
- [ ] **Step 5:** Record on #918: counts, packages installed, key added (no secret values).

### Task 4: Cloudflare tunnel `prba.newlevel.media`

- [ ] **Step 1:** Look for a Cloudflare token with Tunnel + DNS edit rights (`cloudflare-api-tokens` skill, `airuleset.py secret inspect`). None → ask the owner (`❓`, `needs-owner-action`), stop this task until answered.
- [ ] **Step 2:** Create a remotely managed tunnel `presenter-prba`, ingress `prba.newlevel.media` → `http://localhost:8081`, proxied CNAME. No Access application.
- [ ] **Step 3:** Store the tunnel token as secret `CLOUDFLARED_CREDS_PRBA`, vars `CLOUDFLARED_TUNNEL_ID_PRBA`, `CLOUDFLARED_HOSTNAME_PRBA=prba.newlevel.media` (`gh secret set` / `gh variable set`, value piped from `secret exec`).
- [ ] **Step 4:** Record on #918 (tunnel id, hostname; never the token).

### Task 5: Release + verify

- [ ] **Step 1:** Docs: add PRBA to the "Deployed Instances" table in `CLAUDE.md` and to `docs/architecture.md`; playbook entry in `.claude/rules/deploy-workflow-ssh.md` (reusable site workflow, `hardware` input, PRBA guard).
- [ ] **Step 2:** Push dev, CI green, PR dev→main, gates clean, merge, wait for `deploy.yml` to terminal.
- [ ] **Step 3:** Verify: SNV `/healthz` = new version; `https://prba.newlevel.media/healthz` → same version, `channel=release`; read-only sqlite on dev1: device tables 0.
- [ ] **Step 4:** Playwright against `https://prba.newlevel.media/ui/operator`: footer shows `v<version>`, create song „PRBA test", visible in the list, delete it, zero console errors.
- [ ] **Step 5:** Tick #918 checklist with evidence; completion report.
