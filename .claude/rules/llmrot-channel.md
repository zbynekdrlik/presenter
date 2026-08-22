---
paths:
  - "deploy/llmrot-*.sh"
---

# llmrot channel — claudy → presenter CLIProxyAPI account supply (#730)

The `llmrot` channel lets **claudy** (fleet subscription manager on dev1) feed
live Claude subscription auth JSON into each presenter prod's on-device
**CLIProxyAPI** (127.0.0.1:18787, a child of `presenter-server`), so the AI
helper never stalls on a hit rate limit. Mirror of odoo-erp #4697, but
**simplified** (owner ruling #730): NO dedicated user, NO sudoers, NO root
script.

## Shape

- `deploy/llmrot-apply.sh` → installed as `<deploy-dir>/llmrot-apply`, runs as
  `newlevel` (the auth-dir owner) **without sudo**. Self-locates the deploy dir
  from `$0`; auth-dir = `<deploy-dir>/.cli-proxy-api`, port from
  `cli-proxy-api-config.yaml` (default 18787), log `<deploy-dir>/llmrot.log`
  (token-free), flock-serialized.
- Only entry is the sshd **forced command** on claudy's key in `newlevel`'s
  `~/.ssh/authorized_keys` (`command="…/llmrot-apply",restrict,…`). The key can
  run ONLY the script — `list` / `apply <name>` (STDIN=auth JSON) / `remove
  <name>`, `name=[a-z0-9_-]{1,32}`, all via `SSH_ORIGINAL_COMMAND`.
- `deploy/llmrot-provision.sh <deploy-dir>` — idempotent one-time installer
  (script + authorized_keys line). The three deploy workflows also refresh the
  script into the deploy dir on every deploy (stage-then-`mv -f`, new inode).

## Destinations (auto-detect: /opt/presenter-dev on dev2, /opt/presenter on prod)

| Host | tailscale (dev1→) | deploy-dir |
|---|---|---|
| dev2 dev (local) | 100.82.64.27 | /opt/presenter-dev |
| SNV prod (`presenter.lan`) | 100.122.204.47 | /opt/presenter |
| PP prod (`companion-pp.lan`) | 100.101.72.101 | /opt/presenter |

PP deploys via **release** only (not on main merge), so re-provision PP manually
if the script changes.

## Gotchas (cost time to (re)derive)

- **Reload = fsnotify hot-reload, NO restart.** CLIProxyAPI 7.2.130 watches the
  auth-dir; a new/removed `*.json` is picked up in ~1–2s (`events.go` "auth file
  changed … processing incrementally"). NEVER restart `presenter.service` as the
  apply mechanism (stage/operator outage). The apply/reload happens live —
  verify via the API probe, NOT the journal (running proxy is `debug:false`, so
  info reload lines never reach the journal relay).
- **The proxy REWRITES the auth file on load** (sorts keys, adds
  `disabled:false`) → `remove` must print the CURRENT on-disk JSON, not what was
  applied, so claudy reclaims the **rotated** refresh token. Verified: a
  round-tripped file comes back reformatted.
- **Atomic write is mandatory.** Temp must be same-fs + `mv`, and the temp/bak
  names must NOT end in `.json` (dotted `.tmp.`/`.bak.`) or the watcher grabs a
  half-written file ("ignoring empty auth file").
- **Rollback ONLY on proxy up→down** (a regression the apply caused). A
  429/limited completion is a legitimately rate-limited dying account → log,
  keep, exit 0. An already-down proxy is not our regression → keep + report
  `proxy=down`, don't roll back.
- **`completion=err` is normal here** — it means the current account is dead
  (expired/401), which is exactly the state that makes claudy supply a fresh
  one. It is NOT a channel bug.
- **`list`/`apply`/`remove` touch ONLY `llmrot-*.json`** — the owner's own
  `claude-*.json` accounts are invisible to the channel (can't be listed,
  overwritten, or deleted).

## Re-provision a host (from a clean committed tree)

```bash
sshpass -p '<pw>' ssh newlevel@<host> 'mkdir -p /tmp/llmrot'
sshpass -p '<pw>' scp deploy/llmrot-apply.sh deploy/llmrot-provision.sh newlevel@<host>:/tmp/llmrot/
sshpass -p '<pw>' ssh newlevel@<host> 'bash /tmp/llmrot/llmrot-provision.sh /opt/presenter'   # /opt/presenter-dev on dev2
```

## E2E test the channel (from dev1, using claudy's key)

```bash
ssh newlevel@100.104.8.125 "ssh -i ~/.ssh/llmrot_gk newlevel@<ts-ip> list"       # -> proxy=up … exit 0
ssh newlevel@100.104.8.125 "ssh -i ~/.ssh/llmrot_gk newlevel@<ts-ip> whoami"     # -> rejected (forced command)
# apply-invalid (reject) — NEVER push a real token through a presenter session:
ssh newlevel@100.104.8.125 "printf '{\"bad\":1}' | ssh -i ~/.ssh/llmrot_gk newlevel@<ts-ip> 'apply testbad'"  # exit 1
```

claudy side = `zbynekdrlik/claudy#153` (post "kanál LIVE" there when the channel
is up so claudy un-parks). claudy config uses **`user: newlevel`** (not `llmrot`).
