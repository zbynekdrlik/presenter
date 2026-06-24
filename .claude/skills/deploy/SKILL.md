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
   `bash scripts/build-ui.sh`
   Manual equivalent: `cd crates/presenter-ui && trunk build --release`

3. **Server** -- run from WORKSPACE ROOT, not a subcrate directory:
   `cd /home/newlevel/devel/presenter/presenter-dev2`
   Build the server binary (release mode, from workspace root).

4. **Deploy to dev*.* local machine -- no SSH needed):
   `sudo systemctl stop presenter-dev`
   `cp target/release/presenter-server /opt/presenter-dev/presenter-server`
   `sudo systemctl start presenter-dev`

5. **Verify**: `curl http://10.77.8.134:8080/healthz`

### ⚠️ Known issue — local WASM build currently BROKEN (#465)

`trunk build` fails at wasm-bindgen with `failed to find the __wbindgen_externref_table_alloc function`: the trunk-cached wasm-bindgen 0.2.122 CLI is incompatible with rustc 1.96 (reference-types enabled by default). `RUSTFLAGS=-Ctarget-feature=-reference-types` does NOT fix it. **CI builds WASM fine** (it deploys), so until #465 is fixed:

- Validate Rust locally with the cheap path: `cd crates/presenter-ui && cargo check --target wasm32-unknown-unknown` (and `cargo clippy ... --target wasm32-unknown-unknown -- -D warnings`). These skip wasm-bindgen, so they work.
- For UI changes that need a real built artifact, **push and let CI build it**, then verify on the live dev server (`10.77.8.134:8080`) with Playwright. Don't fight the local trunk build.
- `presenter-ui` is OUTSIDE the workspace (own `Cargo.lock`, version `0.1.x`): `cargo <cmd> -p presenter-ui` from root fails — run from `crates/presenter-ui/`.

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
