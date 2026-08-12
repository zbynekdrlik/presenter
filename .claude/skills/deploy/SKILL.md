---
name: presenter-deploy
description: >
  Push-to-CI iterate workflow (Tier-0: CI builds/tests/deploys, only cheap checks run
  locally), the HOTFIX MODE local-build exception, and the CLIProxyAPI login procedure
  for presenter on dev2. Use when pushing a change, deploying to dev, or setting up the
  Claude AI login.
triggers:
  - local build
  - deploy to dev
  - trunk build
  - CLIProxyAPI
  - claude login
  - presenter-dev.service
---

# Presenter Deploy Skill

## Push-to-CI Iterate (Tier-0 Machine)

**#627: this section used to say "Tier-0" in its heading while its own numbered steps
prescribed exactly the local build+deploy loop Tier-0 bans — `block-tier0-local-build.sh`
fails step 2/3 immediately, by design.** This is the corrected, actually-runnable
sequence.

This project is Tier-0 (#592): heavy compilation (`cargo build`, `cargo build
--release`, `trunk build`, `wasm-pack build`, a test run that RUNS tests) happens ONLY
on GitHub-hosted runners. Locally, only cheap checks are allowed/expected before every
push (`.claude/skills/local-builds`):

```bash
cargo fmt --all --check
cargo check --workspace --tests
cargo clippy --workspace --all-targets -- -D warnings   # Rust-only, no codegen/link
```

`presenter-ui` is OUTSIDE the workspace (its own `Cargo.lock`, WASM target) — its cheap
check is separate, see the "WASM build mechanics" note below.

CI (`pipeline.yml`, push to `dev`, ~38 min) then does the FULL loop for you: WASM build
(`scripts/build-ui.sh` under nightly + `-Zbuild-std`), the server release build, E2E, and
the `presenter-dev` systemd swap on the self-hosted runner. There is no separate manual
"build it yourself, then deploy it yourself" step for routine work — the ONLY sanctioned
exception is an explicitly user-declared production emergency (**HOTFIX MODE**, below),
which still lands its commit through these same CI gates retroactively.

1. **Cheap checks** (above), then push to `dev`.
2. **Monitor CI to a terminal state** (`ci-monitoring` global rule): `gh run list` /
   `gh run view <id> --json status,conclusion`. On failure: `gh run view <id>
   --log-failed` — fix the root cause and push again, never re-run a local build to
   "see if it passes" (it can't; the hook blocks it).
3. **Verify the CI-deployed result** — the same binary CI just swapped in, not one you
   built by hand: `curl http://10.77.8.134:8080/healthz`, then open
   `http://10.77.8.134:8080/ui/operator` for a functional check.

## Disk budget — incremental compilation disabled (#585)

`target/` dirs grew to **55 GB** on dev2 (41 GB workspace `target/` + 14 GB
`crates/presenter-ui/target/`, 84% disk usage), **29 GB of it pure
`incremental/` scratch cache** (20 GB `target/debug/incremental` + 5.0 GB
`crates/presenter-ui/target/debug/incremental` + 4.3 GB
`crates/presenter-ui/target/wasm32-unknown-unknown/{debug,release}/incremental`).
Incremental compilation only pays off on repeated small edits to the SAME
crate — our local runs are dominated by full `cargo test` / `cargo clippy
--all-targets` sweeps and `build-ui.sh`, where it added disk + I/O for
little wall-clock benefit (and this disk is shared with bakerion-ai's own
`target/`).

**Fixed as of v0.4.209**: `incremental = false` in BOTH `[build]` and
`[profile.dev]` of the repo-root `.cargo/config.toml` (a profile-level
`incremental` setting overrides `[build]`'s, so both must agree — Cargo's
own incremental decision comes ONLY from `[build]`/`[profile.*]`), plus the
identical `[build]` entry in `crates/presenter-ui/.cargo/config.toml`
(duplicated defensively — Cargo discovers config by DIRECTORY ANCESTRY, not
workspace membership, so the root config actually DOES merge into that
crate's build when `build-ui.sh` `cd`s there first; the duplicate just keeps
the setting attached to the crate if it's ever built from elsewhere).
**#594 correction:** the `CARGO_INCREMENTAL` `[env]` entries in both files
do NOT protect Cargo's own build — Cargo's `[env]` table only sets variables
for processes Cargo *spawns*, it does not read `[env]` back for its own
config (verified empirically on cargo 1.97.0). They're harmless to keep
(they do affect a nested cargo invocation spawned by a build script) but
real protection against a shell-exported `CARGO_INCREMENTAL` would need a
guard in `scripts/dev/quality-check.sh` / `scripts/build-ui.sh` instead.
Verified locally: a full `cargo test --workspace` +
`bash scripts/build-ui.sh` after purging leaves `incremental/` either
absent or an EMPTY 4 KB placeholder dir (cargo/trunk still `mkdir`s the
slot; it just never writes cache content into it) under every `target/`
tree — no more multi-GB growth.

**Purge one-liner** (safe any time — these are pure scratch caches, never
committed source, and Cargo regenerates whatever it still needs):
```bash
rm -rf target/debug/incremental target/release/incremental \
  crates/presenter-ui/target/debug/incremental \
  crates/presenter-ui/target/release/incremental \
  crates/presenter-ui/target/wasm32-unknown-unknown/debug/incremental \
  crates/presenter-ui/target/wasm32-unknown-unknown/release/incremental
```
Check current disk budget: `df -h /` and `du -sh target crates/presenter-ui/target`.
If `target/` disk usage is climbing again despite the fix, suspect a NEW
per-profile `incremental = true` override slipping into either
`.cargo/config.toml` before reaching for the purge — the setting, not the
purge, is the actual fix; purging just reclaims space already spent.

### WASM build mechanics — reference knowledge, NOT a local prescription (#465 resolved)

**`scripts/build-ui.sh` is what CI's build job runs** (and, if ever needed, HOTFIX MODE
below) — it is not a routine local step under Tier-0. This is WHY it exists and what it
avoids, kept here so a CI build-job failure is legible:

The #465 symptom (`failed to find the __wbindgen_externref_table_alloc function`) happens
when you run a **plain `trunk build` on the stable toolchain**: rustc 1.82+ enables wasm
`reference-types` by default, emitting an externref table the wasm-bindgen step can't
resolve. `scripts/build-ui.sh` avoids this entirely: it runs `RUSTUP_TOOLCHAIN=nightly
trunk build` with `crates/presenter-ui/.cargo/config.toml`'s `-Zbuild-std` +
`-Ctarget-cpu=mvp`, which recompiles std for the MVP wasm target (reference-types OFF).
`RUSTFLAGS=-Ctarget-feature=-reference-types` does NOT help — the fix is the nightly +
build-std-mvp path, not RUSTFLAGS.

- A plain `cd crates/presenter-ui && trunk build` on stable WILL fail by design (and is
  ALSO Tier-0 hook-blocked regardless of toolchain) — that is expected, not a bug.
- **The one thing actually allowed locally:** a cheap Rust-only check, no wasm-bindgen, no
  trunk: `cd crates/presenter-ui && cargo check --target wasm32-unknown-unknown` (+
  `cargo clippy --target wasm32-unknown-unknown --all-targets -- -D warnings`).
  `presenter-ui` is OUTSIDE the workspace (own `Cargo.lock`, version `0.1.x`): `cargo
  <cmd> -p presenter-ui` from root fails — run from `crates/presenter-ui/`.

## Playwright E2E — how CI runs them (#461, #627: no supported local loop under Tier-0)

**There is no supported push-free local Playwright loop on this box.** `startTestServer`
needs a FRESH `presenter-server`/`presenter-importer` binary built WITH the E2E feature
flags — building one is exactly the `cargo build --release` the Tier-0 hook blocks, and a
stale/old binary tests stale/old code, which defeats the point. Default flow: push to
`dev`, let CI's `e2e` job build + run the specs on the self-hosted runner, read `gh run
view <id> --log-failed` on a red run rather than trying to reproduce it with a hand-built
binary.

**If HOTFIX MODE is active** (explicit user declaration only — see below) and a genuine
local E2E run is warranted, the mechanics CI itself uses are:

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

**Spec-writing knowledge (applies wherever the spec runs — CI, same as any sanctioned
local run):**

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

### Proving RED→GREEN — default is commit order + CI, not a local rebuild (#568/#569, #627)

**Default (every normal PR, this Tier-0 box):** `regression-test-first` requires RED
before GREEN in COMMIT ORDER — commit the new failing test(s) first (source still
unfixed), commit the fix second, push once, and let CI prove both states: the test file
alone (checked out at the RED commit) would fail against the not-yet-fixed source, and
CI's actual run at the final (GREEN) commit passes. This needs no local build at all.

**If HOTFIX MODE is active** and a genuine local rebuild is already underway (per the
Playwright section above), you can go one step further and PROVE the RED failure
locally before committing: `git stash push -- <the fix files, NOT the new test
files>` (leaves the new spec(s) staged/committed, reverts only the production code),
rebuild WASM+server, run the new spec(s) and confirm they fail for the RIGHT reason,
then `git stash pop`, rebuild again, and confirm green. Catches two real classes of
mistakes cheaply: (1) a test that can't actually fail (a tautology, or testing the wrong
thing) stays green even with the fix reverted; (2) a fix so complete you forgot to also
verify the OLD behavior was real (confirms you're not just adding a passing assertion to
already-working code). This is a HOTFIX-mode bonus, not something to reach for on a
normal Tier-0 PR — it needs the same local build the Playwright section above gates.

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

## `delete_library` resurrection bug — FIXED as of v0.4.207/#578 (was previously live)

Historical note: before #578 landed, `DELETE /libraries/{id}` did a plain hard
`Entity::delete_by_id` with NO sync tombstone — deleting a library that the peer
(PP<->SNV) had already pulled a copy of could resurrect it forever on the next sync
cycle. **This is fixed since v0.4.207**: `delete_library` now SOFT-deletes the library
row AND its live presentations in one transaction (`repository/library.rs`), so the
deletion propagates like any other edit under LWW. It is now safe to delete a library
directly via `/libraries/{id}` for cleanup — no more presentation-first workaround
needed. (v0.4.208 also closed the 3 review gaps found on that fix: search no longer
surfaces a tombstoned library by name, a missing/already-deleted library 404s instead
of 500, and the library's favorite row is cleaned up on delete — see #578 comments.)

## `cargo test -p presenter-server --lib <test>` FAILS — it's a binary-only crate

`presenter-server`'s `Cargo.toml` has no `[lib]` section and no `src/lib.rs` — only
`src/main.rs`. Running `cargo test -p presenter-server --lib <path>` errors immediately
with `error: no library targets found in package` (no compile, no test run — a fast
false negative that looks like "the test doesn't exist"). Use `--bin presenter-server`
instead:
```bash
cargo test -p presenter-server --bin presenter-server router::tests::my_test -- --nocapture
```
A bare `cargo test -p presenter-server` (no `--lib`/`--bin`) also works — cargo infers
the single binary target. Only add `--bin presenter-server` when you need to pass a
specific test-name filter alongside other flags.

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

## CLIProxyAPI deploy step: version-guard + the local-vs-remote interpolation trap (#660)

The "Deploy CLIProxyAPI" step in all three workflows (`deploy.yml`, `pipeline.yml`, `release.yml`)
downloads the vendored binary from `github.com/router-for-me/CLIProxyAPI/releases` and now
(since #660) tracks the installed version in a `.cliproxy-version` marker file next to the binary,
re-downloading whenever the pinned `CLIPROXY_VERSION` differs from that marker -- **not just when
the binary is missing**. Before this fix, bumping the version string in the workflow source did
NOTHING on an already-provisioned host: the old guard was `if [ ! -f cli-proxy-api ]`, so prod ran
the same March-2026 build of v6.9.1 for months despite the workflow having been edited since.

**The trap for anyone editing this step again:** `ssh deploy-target "..."` with a **double-quoted**
string is interpolated by the LOCAL runner shell BEFORE being sent as the ssh command argument --
`$SOME_VAR` inside it expands on the RUNNER, not the remote host, even though the whole string then
executes remotely. This step now uses a real remote heredoc instead
(`ssh deploy-target << 'REMOTE_SCRIPT' ... REMOTE_SCRIPT`, single-quoted delimiter) specifically so
`$CURRENT_VERSION` (read from the remote host's own `.cliproxy-version` file) is evaluated on the
REMOTE side -- a locally-interpolated double-quoted string can never do this since the remote file
doesn't exist on the runner. `${{ env.DEPLOY_DIR }}`-style GH Actions expressions still substitute
fine either way (GitHub's own templating runs before ANY shell sees the text, regardless of quoting).

**Upstream facts worth not re-deriving** (confirmed live 2026-08-12 via `gh api
repos/router-for-me/CLIProxyAPI/releases/latest` + its issue tracker): CLIProxyAPI DOES ship a
built-in Claude OAuth auto-refresh subsystem (`core auth auto-refresh started (interval=15m0s)`).
It had real, matching bugs -- "auto-refresh fails silently despite valid refresh_tokens" and
"Claude OAuth refresh can stampede and replay 429s" -- both filed and CLOSED (fixed) in April 2026.
Our March-2026 vendored build predated both fixes. Presenter's own code has NO refresh logic of its
own (`ai/proxy.rs`'s `refresh_token` string appears only in a test fixture) -- it relies entirely on
the vendored binary's own auto-refresh. If tokens are STILL found dying silently after a deploy
running a version newer than these fixes, the next step is a self-driven refresh loop in
`ai/proxy.rs`/a sibling module, not another vendor bump -- see #675 (follow-up, filed contingent on
that observation).
