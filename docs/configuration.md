# Configuration Reference

All environment variables and feature flags for Presenter.

## Environment Variables

### Server

| Variable                    | Default                     | Description                                                                                                                                                        |
| --------------------------- | --------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `PRESENTER_PORT`            | `80`                        | HTTP server port                                                                                                                                                   |
| `PRESENTER_DB_URL`          | `sqlite://presenter_dev.db` | SQLite connection string                                                                                                                                           |
| `RUST_LOG`                  | `info,tower_http=debug`     | Tracing filter                                                                                                                                                     |
| `PRESENTER_LOCAL_PUBLIC_IP` | unset                       | Church's outbound public IP. When set, `/api/network-mode` classifies a tunnel request with matching `CF-Connecting-IP` as `local` (LAN). See `cloudflare-tunnel-setup.md`. |
| `PRESENTER_SYNC_PEER_URL`   | unset                       | Base URL of the peer Presenter instance for two-way song sync (#555). Set per env in the deploy units over Tailscale: SNV → `http://100.101.72.101` (PP), PP → `http://100.122.204.47` (SNV). Unset → sync disabled (dev + E2E). |
| `PRESENTER_STREAM_ASSETS_DIR` | unset                     | Override for the stream-graphics asset directory (#708). Unset → the `stream-assets/` sibling of the `PRESENTER_DB_URL` sqlite file (i.e. `$DEPLOY_DIR/stream-assets`), else `./stream-assets` (cwd). The server creates it at startup. |

### AI Assistant

| Variable                             | Default                        | Description                                                                                                                                          |
| ------------------------------------- | ------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| `PRESENTER_AI_API_URL`                | `http://127.0.0.1:18787/v1`     | OpenAI-compatible chat completions endpoint — the on-device CLIProxyAPI bundled proxy, which listens on `127.0.0.1:18787`. The raw env-var fallback literal in code is a placeholder (`http://localhost:8787/v1`) that the server self-heals to the proxy's actual configured port (**18787**) at startup, so the effective default is the row's value. Overridden by a persisted `/ai/settings` value if one is saved. |
| `PRESENTER_AI_API_KEY`                | unset                           | Bearer token for the AI provider, if required.                                                                                                      |
| `PRESENTER_AI_MODEL`                  | `claude-opus-4-6`               | Model name sent on every chat completion request.                                                                                                   |
| `PRESENTER_AI_CONTEXT_BUDGET_BYTES`   | `300000`                        | Conservative byte-size ceiling on the request-side conversation, enforced on every agent-loop iteration (#665). Invalid/zero falls back to default. |
| `PRESENTER_AI_MAX_TOKENS`             | `8192`                          | Cap on the PROVIDER's own reply size, sent as `max_tokens` on every chat completion request (#665). Invalid/zero falls back to default.             |
| `PRESENTER_AI_IDLE_CLEAR_MINUTES`     | `30`                            | Idle window after which the shared AI conversation is auto-cleared on the next `/ai/chat` call (#665). Invalid/zero falls back to default.          |

#### AI subscription pool (llmrot channel) — #730

The AI assistant runs against the on-device **CLIProxyAPI** bundled proxy
(`127.0.0.1:18787`), which load-balances across the Claude account auth files in
its auth-dir (`$DEPLOY_DIR/.cli-proxy-api/*.json`). A single account can hit its
rate limit (SNV saw a real `rate_limit_error` on 2026-08-16) and stall the
helper. To keep at least one live subscription available, **claudy** (the fleet
subscription manager on dev1, repo `zbynekdrlik/claudy`) feeds dying-but-live
accounts into each presenter prod over a narrow, keyed SSH channel — the
`llmrot` channel (mirror of odoo-erp #4697), owned and driven by claudy.

- **Transport:** claudy's public key (`llmrot-claudy@dev1`) is authorized in the
  deploy user's (`newlevel`) `~/.ssh/authorized_keys` with a **forced command**
  (`command="$DEPLOY_DIR/llmrot-apply",no-pty,no-port-forwarding,no-agent-forwarding,no-X11-forwarding`),
  so the key can run ONLY `llmrot-apply` — nothing else on the box.
- **Script:** [`deploy/llmrot-apply.sh`](../deploy/llmrot-apply.sh) is installed
  as `$DEPLOY_DIR/llmrot-apply` (runs as `newlevel`, **no sudo** — the auth-dir is
  newlevel-owned) and refreshed on every deploy. Commands (all via
  `SSH_ORIGINAL_COMMAND` + STDIN/STDOUT, never argv/log/repo):
  `list` (per-account `name<TAB>mtime` + a `proxy=… completion=… accounts=N`
  summary, no tokens), `apply <name>` (STDIN = raw CLIProxyAPI auth JSON, ≤64 KiB,
  validated + atomically installed as `llmrot-<name>.json`, hot-reloaded by the
  proxy's fsnotify watcher with **no restart**; rolled back only if the proxy goes
  down), and `remove <name>` (prints the current rotated auth JSON to STDOUT for
  reclaim, then deletes).
- **Provisioning (one-time, per host):** run
  [`deploy/llmrot-provision.sh`](../deploy/llmrot-provision.sh) from a clean
  checkout as `newlevel` — it installs the script and writes the authorized_keys
  forced-command line idempotently. Destinations: dev2 dev (`/opt/presenter-dev`,
  ts `100.82.64.27`), SNV (`/opt/presenter`, ts `100.122.204.47`), PP
  (`/opt/presenter`, ts `100.101.72.101`).

### Companion Integration

| Variable                      | Default | Description                      |
| ----------------------------- | ------- | -------------------------------- |
| `PRESENTER_COMPANION_ENABLED` | `0`     | Enable Companion WebSocket (0/1) |
| `PRESENTER_COMPANION_PORT`    | `18175` | Companion listener port          |
| `PRESENTER_COMPANION_TOKEN`   | (none)  | Shared secret for authentication |

**Precedence:** when `PRESENTER_COMPANION_ENABLED` / `PRESENTER_COMPANION_PORT` are
explicitly SET in the environment, they win over the runtime settings toggle
(`POST /settings/features`) on every boot — the env value is re-persisted over the
stored setting at startup. Leave them UNSET on hosts where the UI toggle should
survive restarts (production leaves them unset; the dev instance pins `=0`
deliberately, so a runtime enable on dev is reset by the next service restart).

### Stage Display

| Variable                          | Default | Description                    |
| --------------------------------- | ------- | ------------------------------ |
| `PRESENTER_HEARTBEAT_INTERVAL_MS` | `1500`  | Stage heartbeat frequency (ms) |

### Android Stage Launchers

| Variable                    | Default      | Description            |
| --------------------------- | ------------ | ---------------------- |
| `PRESENTER_ANDROID_ADB_BIN` | `adb` (PATH) | Custom adb binary path |

### Data & Backup

ProPresenter libraries are stored in git at `data/libraries/` and synced to deployment servers during deploy. No runtime configuration is needed - libraries are imported from `$DEPLOY_DIR/libraries` on each target.

| Location     | Path                           |
| ------------ | ------------------------------ |
| Git repo     | `data/libraries/`              |
| Production   | `/opt/presenter/libraries`     |
| Dev          | `/opt/presenter-dev/libraries` |
| PP (release) | `/opt/presenter/libraries`     |

Bible files are similarly stored in `data/bibles/` and synced during deploy.

#### Stream-graphics assets (#708)

Uploaded stream-graphics images are stored content-addressed on disk as
`$DEPLOY_DIR/stream-assets/<sha256>.<ext>` (a sibling of `presenter.db`), with a
`stream_assets` metadata row per image. Unlike `video_sources` (wiped on every dev deploy),
these are authored content and **survive deploys**: the deploy `rsync --delete` steps are
scoped to the `$DEPLOY_DIR/libraries/` subdirectory only — never the deploy-dir root — so
`stream-assets/` is never in any delete scope, and the binary is delivered via `scp` (which
deletes nothing). The directory is created by the server at startup (and on demand by the
upload handler); if ever lost, every asset is re-uploadable from the editor.

| Location     | Path                              |
| ------------ | --------------------------------- |
| Production   | `/opt/presenter/stream-assets`     |
| Dev          | `/opt/presenter-dev/stream-assets` |
| PP (release) | `/opt/presenter/stream-assets`     |

#### Stream-graphics OBS browser source (#717)

The stream-graphics output (epic #718 — the Resolume replacement) renders as a
transparent web page you add to OBS as a **Browser Source**. It replaces a
Resolume composition: one exclusive base scene + independently-toggled overlay
scenes, all driven from Bitfocus Companion.

Add it in OBS: **Sources → + → Browser**, then:

| Setting            | Value                                          |
| ------------------ | ---------------------------------------------- |
| URL                | `http://<presenter-host>/stream/<output-name>` |
| Width              | `1920`                                         |
| Height             | `1080`                                         |
| Custom CSS         | *(leave empty — none needed)*                  |
| Shutdown when not visible | unchecked (keep it live so it stays in sync) |

- `<output-name>` is the output **slug**. The migration seeds one output whose
  slug is `stream`, so the default URL is
  `http://<presenter-host>/stream/stream` (e.g. production
  `http://10.77.9.205/stream/stream`, dev `http://10.77.8.134:8080/stream/stream`).
- The page is **transparent** by design — no green screen, no chroma key. OBS
  composites it directly over your camera/video below it. Do NOT add a colour
  key filter.
- It is **chrome-free**: no version label, no controls (they would leak onto the
  broadcast). Operate it from `/ui/stream` (the editor) and from Companion.
- Fonts (Inter, Bebas Neue, Oswald) are **bundled** with the app, so the source
  renders identically even with no internet at the rig.
- 1920×1080 is assumed by the layout maths (`Frame` percentages map 1:1 to the
  16:9 canvas). Other sizes still work but element positions are relative to the
  actual browser-source dimensions.

Companion controls the show over the WebSocket (see **Companion WebSocket**
below): `stream_scene_set` switches the exclusive base scene by name,
`stream_overlay_on`/`_off`/`_toggle` flip overlays independently, and
`stream_clear` blanks the output back to transparent.

## Feature Flags

Managed via Settings UI (`/ui/settings`):

### Companion WebSocket

- **Enable/Disable**: Toggle WebSocket server
- **Port**: Configure listener port
- **Token**: Set authentication secret

### Android Stage Launchers

- **Device Roster**: Configure ADB endpoints
- **Health Reporting**: Per-device status

## Runtime Profiles

| Environment | Port    | Database                          | Service                 | Notes                     |
| ----------- | ------- | --------------------------------- | ----------------------- | ------------------------- |
| Production  | 80      | `/opt/presenter/presenter.db`     | `presenter.service`     | Release build from `main` |
| Dev deploy  | 8080    | `/opt/presenter-dev/presenter.db` | `presenter-dev.service` | Release build from `dev`  |
| Local dev   | 80      | `presenter_dev.db` (cwd)          | N/A                     | `cargo run`               |
| E2E testing | dynamic | temp DB                           | N/A                     | Playwright tests          |

## CI/CD Variables

GitHub Actions repository variables:

| Variable        | Purpose                                   |
| --------------- | ----------------------------------------- |
| `RUNNER_LABEL`  | Custom runner label (e.g., `self-hosted`) |

## Related

- [Runbook](ops/runbook.md) - Operational procedures
- [Settings README](settings/README.md) - Feature flag details
