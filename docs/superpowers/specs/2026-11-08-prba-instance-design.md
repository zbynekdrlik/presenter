# PRBA — third Presenter site, hosted on dev1 first (#918)

## Intent

PRBA is a new church location that will use Presenter the same way SNV and PP do. It has no
Linux PC yet. So that its people can start now — open `prba.newlevel.media`, create songs,
build playlists, prepare Bible presentations — we run a PRBA instance at our place (dev1). When
PRBA gets its own Linux PC (an N100 or similar) the instance, with its data, moves there.

## Goals

- A separate Presenter instance for PRBA: own SQLite database, own libraries folder, own systemd
  service, own public URL `https://prba.newlevel.media`.
- Seeded ONCE from a copy of the SNV database (songs, libraries, playlists, Bibles, settings).
- Updated with every release: a merge to `main` deploys SNV AND PRBA in the same workflow run.
- Visible version in the UI footer like every instance (`v0.4.x`).

## Non-goals (now)

- No outputs: no Resolume, no Android stage TVs, no NDI, no Companion, no AbleSet, no OSC, no VBAN
  listening. Web preparation only.
- No ongoing song sync with SNV or PP (the #555 sync stays SNV↔PP only).
- No login (owner decision): the tunnel is public, without Cloudflare Access.
- No 2-week rollout lag yet — that comes with the move to PRBA's own PC (#918 later checklist item).

## Anti-goals (failure even if it "works")

- PRBA pushing anything to SNV devices (Resolume walls, stage TVs) because the copied DB still
  lists them.
- PRBA editing SNV/PP songs through the sync peer.
- A PRBA deploy or crash disturbing anything else on dev1 (other projects, other users' services).
- A PRBA deploy failure blocking or failing the SNV deploy — SNV is production.
- Re-seeding from SNV on a later deploy and wiping PRBA's own edits.

## Constraints

- One code base: the same branch, the same release on SNV, PP and PRBA (owner). Only data and
  configuration differ.
- One unix account per project (airuleset #1184): PRBA runs under its own account on dev1, not
  `newlevel`.
- dev1 = `100.104.8.125` (tailscale; the LAN name does not resolve from dev2). Ryzen 7 5800X,
  16 threads, 31 GB RAM, ~272 GB disk free, GStreamer 1.24 base/good/bad installed, `cloudflared`
  installed at `/usr/local/bin/cloudflared` but no tunnel running, `newlevel` has passwordless
  sudo. Port 8080/8081 are free (8123, 8765, 8788–8804, 8890, 8898, 8977 are taken).
- The deploy runs from the self-hosted runner on dev2 and reaches hosts by SSH (`deploy.yml`
  pattern, `.claude/rules/deploy-workflow-ssh.md`: write `Host deploy-target` before ssh-keyscan).
- Prod SNV DB is read only for us; the seed copy is taken with a read-only/online backup
  (`sqlite3 -readonly … ".backup"`), never by writing on SNV.

## Decided with the owner

| Topic | Decision |
|---|---|
| Host now | dev1 |
| Code | same branch and releases as SNV/PP |
| Data | own DB, one-time copy of SNV |
| Update cadence now | with every merge to `main`, together with SNV |
| Access | `prba.newlevel.media`, public, no login |
| Later | move to PRBA's PC; 2-week lag behind SNV |

## Design (builder's calls marked)

### Instance on dev1

- Account `presenter-prba` (system user, no login shell). Deploy dir `/opt/presenter-prba`
  owned by it. Service `presenter-prba.service` (`User=presenter-prba`), `PRESENTER_PORT=8081`,
  `PRESENTER_DB_URL=sqlite:///opt/presenter-prba/presenter.db`,
  `PRESENTER_COMPANION_ENABLED=0`, no `PRESENTER_SYNC_PEER_URL`, no
  `PRESENTER_ANDROID_STAGE_URL`, `PRESENTER_LISTEN_RECORDINGS_DIR=/opt/presenter-prba/recordings`,
  `PRESENTER_BUILD_CHANNEL=release` (it is a release build like SNV).
- Unit file `scripts/deploy/presenter-prba.service`, derived from `scripts/deploy/presenter.service`
  without the H.264 encoder wait and hardware bits. *(builder's call which lines stay)*
- AI: the same `/etc/presenter-prba/ai.env` mechanism as SNV (OpenRouter key + model from
  secrets), so the AI features work for PRBA's people. *(my call — tell the owner)*

### One-time seed (script, run once by hand, idempotent guard)

`scripts/ops/seed_prba_from_snv.sh`:

1. Refuse to run if `/opt/presenter-prba/presenter.db` already exists (never re-seed).
2. On SNV: `sqlite3 -readonly /opt/presenter/presenter.db ".backup /tmp/prba-seed.db"`, copy it to
   dev1, delete it on SNV.
3. On the copy, clear every device and per-site runtime table: `resolume_hosts`,
   `resolume_push_audit`, `android_stage_displays`, `video_sources`, `ableset_settings`,
   `able_set_settings`, `osc_settings`, `stream_outputs` *(builder: check which `stream_*` rows
   point at SNV hardware vs. are reusable design data)*, `stage_state`, `ui_usage`,
   `interpreter_tips`, `settings_audit`, and the `listen.*` / sync keys in `app_settings`.
4. Copy `/opt/presenter/libraries` from SNV to `/opt/presenter-prba/libraries`.
5. Bibles come from the normal deploy step (`Import Bible translations`), as on SNV.
6. Print the row counts of the cleared tables (all 0) and of `presentations`/`libraries`.

The deploy never seeds: it only migrates the existing DB, exactly like SNV.

### Deploy

- A new job `deploy-prba` in `deploy.yml`, `needs: [build, deploy]` — runs after SNV succeeds,
  reuses the SNV build artifact, never runs before or instead of SNV.
- Same steps as SNV's deploy where they apply (stage, validate schema probe, swap, `/healthz`,
  Bible import, RUST_LOG check) with `DEPLOY_HOST=100.104.8.125`, `DEPLOY_DIR=/opt/presenter-prba`,
  `SERVICE_NAME=presenter-prba`. Skipped steps: Companion module, NDI self-heal, stage URL drop-in,
  sync peer drop-in, TURN.
- Reuse by extraction, not copy-paste, where the SNV job's steps are shell blocks
  *(builder's call: a reusable workflow / composite action vs. a parameterized script; keep the SNV
  job's behavior byte-identical)*.
- A PRBA failure marks the run red but cannot undo or block the SNV deploy (it runs after it).

### Public URL

- Cloudflare tunnel on dev1 for `prba.newlevel.media` → `http://localhost:8081`, public, no Access
  app. Tunnel token as a new secret `CLOUDFLARED_CREDS_PRBA` + vars like SNV's
  (`CLOUDFLARED_TUNNEL_ID_PRBA`, `CLOUDFLARED_HOSTNAME_PRBA`).
- The tunnel and DNS record need a Cloudflare action (token or dashboard) —
  `.claude/rules/cloudflare-access.md` and the `cloudflare-api-tokens` skill. *(builder: if no
  token with tunnel rights exists, that is an owner action)*
- A separate cloudflared service name on dev1 so it does not collide with any other tunnel there
  *(e.g. `cloudflared-prba.service`)*.

### Verification

- `https://prba.newlevel.media/healthz` → `version` = the released version, `channel=release`.
- Playwright on `https://prba.newlevel.media/ui/operator`: footer shows the version; create a test
  song, see it in the list, delete it; zero console errors.
- SNV unchanged: its `/healthz` version and its Resolume/stage devices untouched.
- PRBA DB: device tables empty after the first real deploy.

## Answers to the builder check

- **Library rsync + ProPresenter import** (answered by `deploy.yml`): the rsync only copies the
  `.pro` files from git into `libraries/`; `import_propresenter` runs ONLY when the DB file is
  missing or empty ("Existing database found - preserving presentations"). So keeping both steps
  never touches PRBA's own edits. **Superseded by the owner (2026-11-08):** „iba teraz na začiatku
  sa tam nakopíruje zo SNV a potom už oni si tam budú upravovať vlastnú" — PRBA skips BOTH steps;
  no SNV song file reaches PRBA after the one-time seed.
- **First deploy vs. seed** *(my call)*: `deploy-prba` FAILS with a clear `::error::` when
  `/opt/presenter-prba/presenter.db` is missing — it must never create or import a fresh DB on
  PRBA. Order: dev1 setup → seed → merge that enables `deploy-prba`.
- **SSH** *(my call)*: the runner deploys as `newlevel@100.104.8.125` with the same deploy key
  (`DEPLOY_SSH_KEY`) added to `newlevel`'s `authorized_keys` on dev1, and uses sudo for the
  `presenter-prba` paths/service, like SNV. No sudo narrowing for now (newlevel already has full
  sudo on dev1).
- **Seed runs from dev2** *(my call)*: dev2 reaches SNV (`presenter.lan`, key auth) and dev1
  (tailscale); the script pulls the backup from SNV and pushes it to dev1.
- **`PRESENTER_DB_URL`**: `sqlite:///opt/presenter/presenter.db` is what SNV already uses
  (`deploy.yml`), so the absolute form is valid.
- **Leftover SNV data** *(my call)*: after clearing the tables listed above, the seed dumps every
  `app_settings` key and fails if a key naming a host, URL, IP, port or sync peer remains; the
  plan includes a table-by-table audit of the schema (49 tables on SNV today).
- **A red `deploy-prba`** *(my call)*: the run turns red, SNV stays deployed; the release report
  names which job failed. Nothing else keys on it today.
- **Binary on dev1** *(builder)*: start-test the release binary with `PRESENTER_STARTUP_MODE=validate`
  first; install missing GStreamer runtime packages with apt (system packages on dev1 are fine).
- **Backup** *(my call)*: the same pre-migration DB backup as SNV (5 retained in
  `/opt/presenter-prba/backups/`) — PRBA's DB is the only copy of its data.
- **Cloudflare credential**: check for a token with tunnel + DNS rights (`cloudflare-api-tokens`
  skill). If none exists, creating the tunnel is an owner action, asked at that step.

## Left to the builder

- Exact unit file contents, the reuse mechanism for the deploy steps, which `stream_*` data to keep.
- Whether dev1 needs extra GStreamer packages for the binary to start (`ldd` + start test).
- Docs: add PRBA to the "Deployed Instances" table in `CLAUDE.md` and to `docs/architecture.md`.
