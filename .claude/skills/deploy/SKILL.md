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
