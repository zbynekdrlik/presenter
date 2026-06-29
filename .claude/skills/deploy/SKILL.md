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

## PP location (companion-pp.lan) — release + manual recovery

PP is upgraded via a **GitHub Release** (`gh release create vX.Y.Z --target main --generate-notes`, X.Y.Z = current main version) → `release.yml` builds + `deploy-pp` SSH-deploys. SSH from dev2: `newlevel@companion-pp.lan` (creds in memory `project-pp-location-upgrade`; no `sqlite3` CLI on the box — use `python3 -c "import sqlite3; ..."`).

⚠️ **`release.yml` deploy-pp is currently BROKEN for PP (#469):** it hard-fails the VA-API check (`vah264enc not available` — PP has no GPU/NDI) AND stops the service + swaps the binary BEFORE that check, so a failure leaves PP **DOWN**. Until #469 is fixed, expect to finish a PP release by hand.

**Manual recovery (binary is already deployed when it fails):**
1. Backup: `cp /opt/presenter/presenter.db /opt/presenter/backups/presenter-prerelease-$(date +%Y%m%d-%H%M%S).db`
2. Schema-validate on a COPY: run the new binary with `PRESENTER_DB_URL=sqlite:///tmp/x.db PRESENTER_PORT=18099`, poll `/healthz` (migrations apply) + `/libraries/summary` (data survives), kill + rm the copy.
3. `sudo systemctl start presenter`; verify `/healthz` version, `/libraries/summary` non-empty, `/stage` + `/ui/operator` = 200.

Deploy is DB-safe — ProPresenter import is skipped on an existing DB ("preserving presentations"). DBs predating `video_sources` (PP) lack that table → `/integrations/video-sources` 500s; proper fix is an idempotent incremental migration (#468).

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
