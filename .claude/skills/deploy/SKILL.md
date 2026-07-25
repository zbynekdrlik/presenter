---
name: presenter-deploy
description: >
  Local build-deploy-iterate workflow and CLIProxyAPI login procedure for presenter on dev2.
  Use when building locally, deploying to dev, or setting up the Claude AI login.
triggers:
  - local build
  - deploy to dev
  - trunk build
  - CLIProxyAPI
  - claude login
  - presenter-dev.service
---

# Presenter Deploy Skill

## Local Build-Deploy-Iterate (Tier-1 Machine)

CI takes ~38 min per push. Iterate locally; push only when the feature works end-to-end.
This machine has `airuleset:local-builds=allowed` in CLAUDE.md => full builds are permitted.

Build order matters (WASM embedded into server at compile time via `include_dir!`):

1. **Format + lint** (mandatory before every push):
   `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`

2. **WASM I/F first** (server embeds dist/ at compile time):
   `bash scripts/build-ui.sh`  ← ALWAYS use this; it builds with **nightly +
   `-Zbuild-std` + target-cpu=mvp** (Safari-12 MVP wasm). Do NOT run a plain
   `trunk build` on stable — see the #465 note below.
   Manual equivalent: `RUSTUP_TOOLCHAIN=nightly trunk build --release` from
   `crates/presenter-ui` (the nightly + MVP `.cargo/config.toml` is what makes it work).

3. **Server** -- run from WORKSPACE ROOT, not a subcrate directory:
   `cd /home/newlevel/devel/presenter/presenter-dev2`
   Build the server binary (release mode, from workspace root).

4. **Deploy to dev*.* local machine -- no SSH needed):
   `sudo systemctl stop presenter-dev`
   `cp target/release/presenter-server /opt/presenter-dev/presenter-server`
   `sudo systemctl start presenter-dev`

5. **Verify**: `curl http://10.77.8.134:8080/healthz`

### Local WASM build — use `scripts/build-ui.sh`, NOT plain `trunk build` (#465 resolved)

The local WASM build **works** — via `scripts/build-ui.sh`. The #465 symptom
(`failed to find the __wbindgen_externref_table_alloc function`) only happens
when you run a **plain `trunk build` on the stable toolchain**: rustc 1.82+
enables wasm `reference-types` by default, emitting an externref table the
wasm-bindgen step can't resolve. The project's canonical build avoids this
entirely: `scripts/build-ui.sh` runs `RUSTUP_TOOLCHAIN=nightly trunk build`
with `crates/presenter-ui/.cargo/config.toml`'s `-Zbuild-std` +
`-Ctarget-cpu=mvp`, which recompiles std for the MVP wasm target (reference-types
OFF). So:

- **Build WASM locally with `bash scripts/build-ui.sh`** (verified working on dev2,
  rustc 1.96, wasm-bindgen 0.2.122). Then `cargo build --release -p presenter-server`
  embeds `dist/`, and Playwright E2E run locally. `RUSTFLAGS=-Ctarget-feature=-reference-types`
  does NOT help — the fix is the nightly + build-std-mvp path, not RUSTFLAGS.
- A plain `cd crates/presenter-ui && trunk build` on stable WILL fail by design —
  that is expected, not a bug. Use the script.
- Cheap Rust-only check (no wasm-bindgen): `cd crates/presenter-ui && cargo check
  --target wasm32-unknown-unknown` (+ clippy). `presenter-ui` is OUTSIDE the
  workspace (own `Cargo.lock`, version `0.1.x`): `cargo <cmd> -p presenter-ui` from
  root fails — run from `crates/presenter-ui/`.

## Running Playwright E2E locally (#461)

To validate a stage/UI change with the real Playwright specs before pushing
(saves a ~45-min CI cycle):

1. **Rebuild the server after ANY WASM change.** The server embeds `dist/` at
   compile time, so `bash scripts/build-ui.sh` ALONE is not enough — the running
   binary still has the OLD WASM. Order: `build-ui.sh` → rebuild the server →
   run E2E. Skipping the server rebuild silently tests stale WASM.
2. **Build the test server WITH the E2E feature flags** (CI uses these):
   `cargo build --release -p presenter-server -p presenter-importer --features
   presenter-server/mock-integrations,presenter-server/test-helpers`.
   `startTestServer` picks the NEWER of `target/{debug,release}/presenter-server`.
3. **Free the fixed mock-resolume port 8091 first.** `mock-integrations` binds a
   HARD-CODED `127.0.0.1:8091` (`mock_integrations/resolume.rs`), and the deployed
   `presenter-dev.service` already holds 8091 — so a local E2E run dies on
   `mock-resolume failed to bind 127.0.0.1:8091 Address already in use` /
   `beforeAll hook timeout`. Fix: `sudo systemctl stop presenter-dev` → run the
   spec → `sudo systemctl start presenter-dev` (stopping the app you're testing
   is in-scope; restart right after). Run with `--workers=1` (specs serialize on
   the fixed ports).
   `npx playwright test <spec> -g "<title>" --reporter=line --workers=1`
4. Stage specs seed via the HTTP API: `POST /stage/layout {code}`, `POST
   /playlists`, `PUT /playlists/{id}/entries`, `POST /stage/state {presentationId,
   currentSlideId, playlistId}` (returns 204) → the matching playlist entry goes
   `is_active`. Each spec starts its own server via `startTestServer()`.
5. Read content INSIDE the operator's embedded stage preview iframe (#460) with
   `page.frameLocator("iframe.operator__stage-iframe").locator("<sel>")`. The
   stage's own selectors work: `.stage__current-slide .stage__slide-text` for the
   current line (valid in worship-snv AND api layouts — `ApiStage` wraps
   `WorshipSnv`), `.stage__bible-text`/`.stage__bible-reference` for a triggered
   verse (set `POST /stage/layout {code:"bible"}` first so the mirror renders it).

### Proving RED→GREEN locally with a real rebuild, not just commit order (#568/#569)

`regression-test-first` requires RED before GREEN in commit order — but you can go one step
further and actually PROVE the RED failure locally before committing: `git stash push -- <the fix
files, NOT the new test files>` (leaves the new spec(s) staged/committed, reverts only the
production code), rebuild WASM+server (steps 2–3), run the new spec(s) and confirm they fail for
the RIGHT reason, then `git stash pop`, rebuild again, and confirm green. Costs ~2 extra
build-and-test cycles (~15 min combined on this box) but catches two real classes of mistakes
cheaply: (1) a test that can't actually fail (a tautology, or testing the wrong thing) stays green
even with the fix reverted; (2) a fix so complete you forgot to also verify the OLD behavior was
real (confirms you're not just adding a passing assertion to already-working code).

### GOTCHA — a stale/ambiguous `target/release/presenter-server` silently shadows your fresh fix (#558)

`startTestServer` picks the NEWER of `target/debug/presenter-server` /
`target/release/presenter-server` by mtime. If you only rebuilt `debug` (`cargo build -p
presenter-server`, no `--release`) but a release binary already exists from an earlier point,
E2E runs against WHICHEVER is newer — and if you can't account for why release's mtime moved
(another process touched it, a leftover from a previous session), your test can silently
exercise OLD code and produce a false result. When a result looks wrong/inconsistent with the
diff you just made: check both mtimes (`ls -la --time-style=full-iso target/{debug,release}/presenter-server`);
if in doubt, `rm target/release/presenter-server` to force the harness onto the `debug` binary
you just built, and rebuild release properly (`cargo build --release -p presenter-server ...`,
per step 2 above) before relying on it again.

### GOTCHA — Playwright `page.on("console")` ALSO captures IFRAME console (#460)

The operator header now embeds `<iframe src="/stage?preview=1">` on EVERY operator
page. Playwright's `page.on("console")` fires for the page AND all its child frames,
so anything the embedded `/stage` logs lands in the operator's console listener and
breaks every `expect(consoleMessages).toEqual([])` operator spec. The real stage
emits the `crbug.com/981419` wake-lock permissions warning in headless — so the
stage page SKIPS `start_wake_lock_guard()` in preview mode (`?preview=1`,
`stage.rs`). When embedding any page-in-page, gate its heavy/noisy side-effects
(wake lock, self-reload watchdogs, beacons) behind a preview flag, or the parent's
console-zero assertions fail.

### Stage-monitor count must EXCLUDE preview clients (#460)

Stage WS clients register in `StageConnections` by SENDING an inbound
`StagePresence` over `/live/ws` (driven client-side in `ws/stage.rs`), NOT by the
WS upgrade. The preview iframe is one more `/stage` WS client, so it would inflate
the operator's "N stage displays connected" count. Fix: it tags its socket
`/live/ws?surface=stage&preview=1` (`ws_url()` appends it when
`utils::window::url_flag_enabled("preview")`); the server (`router.rs` →
`live.rs::serve_websocket(preview)`) skips `connections.register(...)` for preview
sockets while still forwarding every live event. To embed another live stage view
without polluting the count, reuse `?preview=1`.

## PP location (companion-pp.lan) — release + manual recovery

PP is upgraded via a **GitHub Release** (`gh release create vX.Y.Z --target main --generate-notes`, X.Y.Z = current main version) → `release.yml` builds + `deploy-pp` SSH-deploys. SSH from dev2: `newlevel@companion-pp.lan` (creds in memory `project-pp-location-upgrade`; no `sqlite3` CLI on the box — use `python3 -c "import sqlite3; ..."`).

⚠️ **`release.yml` deploy-pp is currently BROKEN for PP (#469):** it hard-fails the VA-API check (`vah264enc not available` — PP has no GPU/NDI) AND stops the service + swaps the binary BEFORE that check, so a failure leaves PP **DOWN**. Until #469 is fixed, expect to finish a PP release by hand.

**Manual recovery (binary is already deployed when it fails):**
1. Backup: `cp /opt/presenter/presenter.db /opt/presenter/backups/presenter-prerelease-$(date +%Y%m%d-%H%M%S).db`
2. Schema-validate on a COPY: run the new binary with `PRESENTER_DB_URL=sqlite:///tmp/x.db PRESENTER_PORT=18099`, poll `/healthz` (migrations apply) + `/libraries/summary` (data survives), kill + rm the copy.
3. `sudo systemctl start presenter`; verify `/healthz` version, `/libraries/summary` non-empty, `/stage` + `/ui/operator` = 200.

Deploy is DB-safe — ProPresenter import is skipped on an existing DB ("preserving presentations"). DBs predating `video_sources` (PP) lack that table → `/integrations/video-sources` 500s; proper fix is an idempotent incremental migration (#468).

## ROLLBACK RUNBOOK — regression on prod after a release (#558-era, standing)

Escalating tiers; start at the lowest that covers the symptom. The v0.4.202+ schema
migration is ADDITIVE (updated_at/sync_id/deleted_at columns) — an older binary
ignores unknown columns, so a binary rollback WITHOUT a DB restore is always safe.
Every deploy also backs up the DB first (5 retained in `backups/` on each host).

**Tier 0 — sync kill switch (~1 min, keeps everything else live).** If the symptom
is sync-shaped (songs changing/disappearing across sites, trash weirdness), disable
the sync loop only:
```bash
# on the affected host(s) — SNV: sshpass -p '<pw in memory>' ssh newlevel@presenter.lan; PP: ...@companion-pp.lan
sudo rm /etc/systemd/system/presenter.service.d/sync-peer.conf   # the PRESENTER_SYNC_PEER_URL drop-in
sudo systemctl daemon-reload && sudo systemctl restart presenter
curl -s localhost/healthz   # still ok; /integrations/sync/status now enabled:false
```
Re-enable = restore the drop-in + restart. All other features keep running.

**Tier 1 — binary rollback to the previous release (~10 min, no data loss).**
Download the previous release tarball (`gh release download v<PREV> -p '*'`), extract
`presenter-server`, then on the host: stop service → replace `/opt/presenter/presenter-server`
→ start → verify `/healthz` shows the OLD version and `/ui/operator` works. New-schema
columns are ignored by the old binary; songs edited meanwhile keep working.

**Tier 1.5 — git revert (clean, slower ~40 min).** `git revert -m 1 <merge-sha>` on a
dev branch → PR → CI → merge → auto-deploy. Use when the regression is real but not
urgent enough for Tier 1's manual surgery.

**Tier 2 — DB restore (LOSES post-deploy edits — ASK THE USER FIRST, destructive).**
Only for actual data corruption: stop service → copy the newest pre-deploy backup from
`/opt/presenter/backups/` over `presenter.db` (rm -f the -wal/-shm) → start. If sync is
enabled, FIRST kill sync on BOTH hosts (Tier 0), restore, verify, then re-enable —
otherwise LWW can re-import the corruption from the peer.

## HOTFIX MODE — user-declared emergencies only (skips the full CI wait, never main protection)

Activate ONLY when the user explicitly declares hotfix mode. This is the fast path for
"prod is broken NOW"; it does NOT admin-merge or weaken main — the process catches up after.

1. Fix on `dev`, minimal targeted gate only: `cargo fmt --all --check`, `cargo clippy
   --workspace --all-targets -- -D warnings`, plus ONLY the tests covering the touched
   area (skip the full suite/E2E matrix).
2. COMMIT on dev (the deploy-from-clean-tree hook requires a clean committed tree).
3. Build locally on dev2 (this is the build box): `bash scripts/build-ui.sh && cargo build
   --release -p presenter-server`.
4. Manual deploy straight to the affected host(s) — standing-approved manual deploy of the
   app being worked on: scp the binary → stop service → swap → start → verify `/healthz`
   version + the FIXED behavior live in a real browser.
5. IMMEDIATELY AFTER the fire is out: push dev, let full CI run, open/ride the dev→main PR
   as normal — the hotfix commit goes through the ordinary gates retroactively; any CI
   failure found then is fixed forward. Never leave a hotfix deployed without its PR landing.

## Event-network cloudflared tunnel — QUIC gets blocked, force HTTP/2 (#562)

**Symptom:** `prsnv.newlevel.media` (the Cloudflare Tunnel in front of prod) went unreachable the
moment the SNV rig traveled to an event venue — while `systemctl status cloudflared` and the
Presenter service both looked completely healthy on the box itself. Cloudflare's edge showed the
generic error 1033 (tunnel not connected) to the public.

**Root cause:** `cloudflared`'s DEFAULT transport is QUIC (UDP). Many venue/event networks
(temporary Wi-Fi, restrictive corporate/venue firewalls) block outbound UDP entirely — `cloudflared`
then retries the QUIC handshake forever and never falls back on its own. The tunnel daemon, the
Presenter binary, and the LAN are all fine; only the OUTBOUND leg to Cloudflare's edge is silently
failing.

**Fix — force the HTTP/2 transport (TCP 7844), which venue networks pass:**

```bash
sudo mkdir -p /etc/systemd/system/cloudflared.service.d
sudo tee /etc/systemd/system/cloudflared.service.d/protocol-http2.conf >/dev/null <<'EOF'
[Service]
ExecStart=
ExecStart=/usr/bin/cloudflared tunnel --protocol http2 run
EOF
sudo systemctl daemon-reload
sudo systemctl restart cloudflared
# Verify: the tunnel reconnects and the public hostname resolves again.
curl -sI https://prsnv.newlevel.media/healthz
```

(The blank `ExecStart=` line before the real one is required — systemd drop-ins APPEND to
`ExecStart` by default; without clearing it first you get two conflicting `ExecStart` lines. Adjust
the `ExecStart=` binary path/tunnel name to match the actual unit if it differs — check
`systemctl cat cloudflared` first.)

**HTTP/2 is a full-fidelity transport — this drop-in is safe to leave in place permanently**, not
just as an event-mode toggle. It was applied LIVE on SNV prod on 2026-07-17 (hotfix mode, mid-event)
and is standing since. **Apply the same drop-in to PP's (`companion-pp.lan`) cloudflared BEFORE PP
ever travels to an event venue** — PP has not hit this yet only because it hasn't left the building.

**Diagnosis checklist when a Cloudflare-Tunnel-fronted host goes unreachable "from outside" but is
healthy locally:** `systemctl status cloudflared` (daemon up?) → `journalctl -u cloudflared -n 50`
(look for repeated QUIC handshake/registration retries, `context deadline exceeded`, or "no
`edge connections`") → if venue/event Wi-Fi is involved, suspect UDP-blocking FIRST, apply the
HTTP/2 drop-in above, and re-verify — don't chase the app/service layer when the daemon logs show
the tunnel itself never reconnecting.

## Post-deploy live verification on SNV/PP: NEVER delete a test library via `/libraries/{id}` (#578)

When you create a throwaway test library+presentation on SNV or PP to verify a live fix,
clean it up by deleting the **presentation** (`DELETE /presentations/{id}` — soft-delete,
tombstoned, propagates the deletion through the #555 sync loop) and only THEN, if you
want, the now-empty library. **Never delete the library first/only** — `delete_library`
does a plain hard `Entity::delete_by_id` with NO sync tombstone (pre-existing bug, filed
as #578), so if the peer (PP<->SNV) already pulled a copy of that presentation in the
~30s since you created it, the very next sync cycle sees "we never held this" on the
side that just hard-deleted it and RESURRECTS the whole library+presentation from the
peer — repeatably, forever, no matter how many times you delete the library again. Live
incident (2026-07-24/25, verifying #575/#571/#574): a canary library round-tripped
resurrection twice before switching to presentation-level delete converged it for good.
If you're left with an empty, obviously-test-named library shell on either side, that's
the safe end state — leave it (deleting it again just risks re-triggering the loop if
either side still holds a live copy of anything under that name).

## `cargo test --workspace 2>&1 | tail -N` SWALLOWS a real test failure's exit code

The exit code of a bash pipeline is the LAST command's (`tail`'s, which is almost always
0) — not `cargo test`'s. Piping a live `cargo test` run through `tail` for a shorter
paste ALSO throws away the real pass/fail signal; a failing suite silently reports
`EXIT_CODE: 0`. Redirect to a file instead and check the exit code of the redirect
itself, then `grep -n "^test result:\|FAILED"` the file afterward:
```bash
cargo test --workspace > /tmp/full_test.log 2>&1; echo "EXIT: $?"
grep -n "^test result:\|FAILED" /tmp/full_test.log
```

## CLIProxyAPI Login Flow

Use `cli-proxy-api -claude-login -no-browser` with callback URL paste.
PKCE and SSH tunnel approaches both fail (PKCE rejected by Anthropic endpoint;
SSH too complex for remote users).

1. Spawn: `cli-proxy-api -claude-login -no-browser -config <path>`
2. Read auth URL from stdout
3. User opens URL in browser, outhorizes
4. Browser redirects to `localhost:54545/callback` -- shows error page (expected)
5. User copies the full callback URL from the browser address bar, pastes into Presenter UI
6. Presenter forwards query string to `http://127.0.0.1:54545/callback` on the server
7. CLIProxyAPI builds token exchange internally, saves credentials
8. Restart proxy to pick up new credentials

The error page is expected -- the localhost redirect can't reach the server from the user's browser.
