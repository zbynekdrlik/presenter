# CLAUDE.md

> **Version:** 2025.7 | **Last Updated:** 2026-03-16

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Local Build Policy

**This project is Tier-0 (CI-only builds).** Heavy builds (`cargo build`, `cargo clippy`, `cargo test`) run on GitHub-hosted runners, not on this machine. The `block-tier0-local-build.sh` hook enforces this — do NOT add a `local-builds=allowed` marker to disarm it. Only lint and cheap compile checks may run locally, per the global `local-builds` skill. Dev2 is the self-hosted runner host for E2E + deploys, but compilation happens on GitHub's runners.

<!-- Global rules applied via airuleset modules (~/devel/airuleset/):
     core/complete-planned-work, core/completion-report, core/autonomous-verification,
     core/ci-monitoring, core/ci-push-discipline, core/pr-merge-policy,
     core/tdd-workflow, core/git-fetch-first, core/version-bumping,
     git/two-branch-workflow, git/commit-conventions,
     ci/test-strictness, ci/no-continue-on-error, ci/e2e-real-user-testing,
     ci/browser-console-zero-errors, quality/rust-web-stack, quality/mvp-philosophy,
     quality/architecture-first, quality/security-basics,
     deploy/post-deploy-verification, deploy/ssh-deployment
-->

---

## Playbook Router

| Topic | Where to look |
|---|---|
| NDI pipeline, WebRTC testing, cleanup | `.claude/skills/ndi/SKILL.md` |
| Companion plugin, /etc/hosts, batching | `.claude/skills/companion/SKILL.md` |
| Runner management, GPU wedge, probe cleanup, branch-sync-after-merge | `.claude/skills/ci/SKILL.md` |
| Local build/deploy workflow, CLIProxyAPI login | `.claude/skills/deploy/SKILL.md` |
| llmrot channel (claudy → CLIProxyAPI live-subscription supply, #730) | `.claude/rules/llmrot-channel.md` (auto-loads on `deploy/llmrot-*.sh`) |
| Leptos/WASM frontend gotchas (view! macro, keyed `<For>`) | `.claude/skills/ui/SKILL.md` |
| Database schema, migrations, settings audit, library import | `.claude/skills/database/SKILL.md` |
| Repository refusal → HTTP 404/409/422 typed pattern (RepositoryError) | `.claude/rules/repository-error-pattern.md` (auto-loads on `repository/**`, `router/**`, `state/**`) |
| Stream-graphics subsystem (epic #718): serde tag, i32/i64 ids, config_revision vs activation, error taxonomy, test idiom | `.claude/rules/stream-graphics.md` (auto-loads on `**/*stream*.rs`) |
| Stream-graphics WASM operator editor `/ui/stream` (#713): def+events client model, generic ws/api reuse, reorder full-id-set, untracked handler reads, CI clippy-excludes presenter-ui | `.claude/rules/stream-editor-ui.md` (auto-loads on `pages/stream_editor.rs`, `components/stream_editor/**`) |
| Adding a `LiveEvent` variant → the one exhaustive match (`companion/variables.rs`) that breaks (E0004) | `.claude/rules/live-events.md` (auto-loads on `live.rs`, `companion/variables.rs`) |
| Sync/LWW invariants (synthetic tombstones, clock bumps, best-effort recon) | `.claude/rules/sync-lww.md` (auto-loads on `repository/sync*`, `library_sync*`, `state/sync.rs`) |
| AI-eval harness (lib.rs split, Cargo autobins gotcha, minimal pub widening) | `.claude/rules/ai-eval-harness.md` (auto-loads on `bin/ai_eval/**`, `lib.rs`, `scripts/dev/ai-eval/**`) |
| Local quality-gate gotchas (fn-length counts wrapped signature lines, file-size cap) | `.claude/rules/quality-gates.md` (auto-loads on `crates/**/*.rs`) |
| Down-dependency log floods → resolume #484 backoff + power-of-two log gate (AbleSet/Resolume/NDI/ADB pollers) | `.claude/rules/log-flood-backoff.md` (auto-loads on `ableset.rs`, `resolume/**`, `manager/**`, `android_stage.rs`) |
| Bible page DOM contract + async-effect E2E determinism (#727): collapsed/full book-item attribute parity, settle-signal over expect.poll, Tier-0 live-server repro | `.claude/rules/bible-page.md` (auto-loads on `pages/bible.rs`, `state/bible.rs`, `wasm-bible.spec.ts`) |
| Resolume port-drift tests (#744/#564): use `free_port_pair()` for a verified-free CONSECUTIVE port pair, never `free_port()+N`; drift target = configured_port+1 within the 5-port probe window | `.claude/rules/resolume-port-drift.md` (auto-loads on `resolume/port_drift*.rs`) |

## Always-Rules

- **Box dimensions** — NEVER change stage layout box sizes or positions without explicit user instruction. Explain the constraint and ask what dimensions they want.
- **Tablet triggers** — Tablet sends only `(presentation_id, slide_id)` to server; server looks up and broadcasts full data. NEVER reconstruct slide data in the tablet. Fixes go in WASM Rust (`tablet.rs`), not CSS.
- **Hardware features** — If a feature depends on external hardware/SDK (NDI, cameras, audio), ask the user about availability BEFORE implementing. Write tests for the real pipeline, not just the fallback path. Never report hardware features as done based only on CI green.
- **External API integration** — Always WebSearch + WebFetch official docs BEFORE writing integration code. Never guess API behavior. (AbleSet `/api/setlist` returns the full session; `internalMeta.skipped: true` marks songs not in the active set.)
- **Drag-drop** — A single middle-position E2E test is insufficient. Before claiming complete, verify in Playwright: empty list, drop above first entry, drop below last entry, middle-position drop.

---

## Project Overview

Presenter is a monolithic Rust application for church worship services, providing lyrics display, Bible passages, timers, and stage displays.

**Key Documentation:**

- Domain specifics: `docs/functional-needs.md`
- System architecture: `docs/architecture.md`
- Configuration reference: `docs/configuration.md`

---

## Versioning

The project uses **Semantic Versioning** with a single `X.Y.Z` format on both branches.

**Version location:** `Cargo.toml` workspace `[workspace.package].version` (always clean semver `X.Y.Z`)

**Build channel:** Dev vs production is distinguished via a compile-time `PRESENTER_BUILD_CHANNEL` env var (defaults to `"dev"`). Deploy workflows set this to `"dev"` or `"release"`.

**Version display:**

- `/healthz` returns `{"status":"ok","version":"0.1.2","channel":"dev"}`
- UI footer shows `v0.1.2 (dev)` or `v0.1.2` (release)

**CI enforcement:** `version-check.yml` validates version is valid semver and greater than the latest GitHub release.

**Release lifecycle:**

1. Dev work: version is `X.Y.Z`, push to dev, CI passes, dev deploy shows `vX.Y.Z (dev)`
2. Release: create PR dev->main, no version change needed, all checks green
3. Merge: Claude merges autonomously once every gate is green (user directive 2026-07-24), main deploys, production shows `vX.Y.Z`
4. Post-release: bump to next version on dev, continue

See `docs/architecture.md` for full versioning and release strategy.

---

## GitHub Actions (Primary CI/CD)

### Runner Architecture

This project uses a **mixed runner strategy**:

- **GitHub-hosted runners** (`ubuntu-latest`): All compilation, linting, testing, and security scans. Free for public repos.
- **One bare-metal local runner** (`self-hosted`): E2E tests (need running binary) and deployments (SSH to LAN hosts).

**Local runner host:** `10.77.8.134` (same machine as dev server)
**Local runner name:** `presenter-local`
**Local runner label:** `self-hosted`
**Config location:** `~/actions-runner/`

The local runner does NOT compile Rust — it only runs pre-built artifacts downloaded from GitHub-hosted build jobs. It needs: Node.js 22, Playwright chromium, rsync, and SSH access to LAN deploy targets.

**Deploy workflows** use SSH from the local runner to application hosts (`10.77.9.205` for production, `10.77.8.134` for dev, `companion-pp.lan` for PP releases).

Runner management commands (status, restart, re-register) live in the ci skill — see the Playbook Router.

### Workflows

| Workflow                | Trigger                    | Purpose                                                   |
| ----------------------- | -------------------------- | --------------------------------------------------------- |
| `pipeline.yml`          | Push to `dev`              | Full pipeline: checks → build → E2E → deploy-dev          |
| `deploy.yml`            | Push to `main`             | Build on GH runner, deploy to production via SSH          |
| `release.yml`           | GitHub Release published   | Build release, upload tarball, deploy to companion-pp.lan |
| `security-schedule.yml` | Weekly (Sunday) + manual   | Scheduled vulnerability scanning                          |
| `import-data.yml`       | Manual (workflow_dispatch) | Re-import ProPresenter/Bible data                         |
| `mutation-full.yml`     | Manual (workflow_dispatch) | On-demand full-tree mutation sweep (`/mutation-sweep`)    |
| `ndi-latency.yml`       | Manual (workflow_dispatch) | On-demand NDI glass-to-glass latency guard on a quiet box (#386) |
| `pr-labeler.yml`        | PR opened/edited           | Auto-label PRs by changed paths                           |

**Pipeline dependency chain:** `branch-sync → [fmt, clippy, companion, version-check, security] → [test, quality] → coverage → build → e2e → deploy-dev`

Deploy-dev **cannot run** unless every check, test, build, and E2E job succeeds.

**NDI E2E split (load-sensitivity, #386):** The per-PR `e2e-ndi` lane runs the load-INSENSITIVE
NDI guards (decode / freeze / console / straggler / reactivate / reload in
`ndi-webrtc-synthetic.spec.ts`) — PRs still fail if NDI video is actually broken. The
load-SENSITIVE glass-to-glass latency assertion (`ndi-latency.spec.ts`, tag `@latency-ndi`) is
NOT in the per-PR pipeline: on the shared dev2 runner, concurrent CPU load (another project's
cargo-mutants / rebuilds) starves the in-browser sampling loop + GPU encoder and false-reds the
timing bound. Its bounds are unchanged — it runs on-demand on a quiet box via `ndi-latency.yml`
(same pattern as the #488 mutation full-sweep). Run it after NDI/WebRTC pipeline changes.

---

## Build & Run

```bash
# Development server
cargo run -p presenter-server

# With environment variables
PRESENTER_PORT=8080 cargo run -p presenter-server

# Release build (what CI produces)
cargo build --release -p presenter-server
./target/release/presenter-server
```

### Deployed Instances

Three instances run on separate hosts:

| Instance   | URL                     | Host             | Port | Service                 | Deploy Dir           | Trigger        |
| ---------- | ----------------------- | ---------------- | ---- | ----------------------- | -------------------- | -------------- |
| Production | http://10.77.9.205      | 10.77.9.205      | 80   | `presenter.service`     | `/opt/presenter`     | push to `main` |
| Dev        | http://10.77.8.134:8080 | 10.77.8.134      | 8080 | `presenter-dev.service` | `/opt/presenter-dev` | push to `dev`  |
| PP         | http://companion-pp.lan | companion-pp.lan | 80   | `presenter.service`     | `/opt/presenter`     | GitHub Release |

```bash
# Check both services
systemctl status presenter presenter-dev

# View dev logs
sudo journalctl -u presenter-dev -f

# Restart dev only
sudo systemctl restart presenter-dev
```

---

## Banned Patterns (Production Code)

| Pattern              | Why Banned                      | Alternative                                   |
| -------------------- | ------------------------------- | --------------------------------------------- |
| `unwrap()`           | Panics in production            | Use `?`, `ok_or()`, or handle `None`/`Err`    |
| `expect()`           | Panics in production            | Use `?` with context via `anyhow`/`thiserror` |
| `panic!`             | Crashes the service             | Return `Result` or `Option`                   |
| `std::thread::sleep` | Blocks async runtime            | Use `tokio::time::sleep`                      |

**Note:** Test code (`#[cfg(test)]` modules) and WASM code (`presenter-ui` crate) are exempt from panic rules. WASM panics become browser-side JavaScript errors rather than server crashes.

---

## File/Function Limits (Enforced by CI)

| Metric         | Warning | Hard Fail | Exempt                                  |
| -------------- | ------- | --------- | --------------------------------------- |
| File lines     | >800    | >1000     | Migrations, tests                       |
| Function lines | >80     | >120      | Migrations, UI renders, router builders |

**Exempt patterns:**

- `m*_create_*.rs` - Migration files (declarative schema definitions)
- `render_*_ui` functions - Leptos component renders (HTML-like DSL)
- `build_router` functions - Route declarations

---

## Testing

### Test Commands

```bash
# Rust unit tests
cargo test

# Single test
cargo test test_name

# Playwright E2E (MUST pass before any merge)
npm run test:playwright
npm run test:playwright:headed  # Browser visible

# View Playwright report
scripts/dev/show-playwright-report.sh
```

### E2E Notes

- Tests must be deterministic: fixed seeds, stable timeouts
- Prefer retry-with-assert poll helpers over arbitrary sleeps
- **E2E timeout = build failure** - Optimize build caching, not extend timeouts

---

## Architecture

### Project Structure

```
data/
├── libraries/             # ProPresenter libraries (single source of truth)
└── bibles/                # Bible translation files
crates/
├── presenter-core/        # Domain logic (no server deps)
├── presenter-server/      # Axum HTTP/WS + Leptos SSR
├── presenter-persistence/ # SeaORM repository layer
├── presenter-migration/   # Schema evolution
├── presenter-importer/    # ProPresenter import
└── presenter-bible/       # Bible translation ingestion
```

### Key HTTP Endpoints

| Endpoint        | Purpose                      |
| --------------- | ---------------------------- |
| `/healthz`      | Readiness probe              |
| `/ui/operator`  | Desktop control surface      |
| `/ui/tablet`    | Touch-optimized controller   |
| `/ui/bible`     | Bible search/trigger UI      |
| `/stage`        | HTML stage display           |
| `/live/ws`      | Live updates (timers, stage) |
| `/companion/ws` | Bitfocus Companion control   |

---

## Environment Variables

| Variable                      | Default                 | Purpose                 |
| ----------------------------- | ----------------------- | ----------------------- |
| `PRESENTER_PORT`              | 80                      | Server port                                                          |
| `PRESENTER_DB_URL`            | `sqlite://presenter.db` | Database connection                                                  |
| `PRESENTER_COMPANION_ENABLED` | 0                       | Enable Companion socket                                              |
| `PRESENTER_COMPANION_PORT`    | 18175                   | Companion listen port                                                |
| `PRESENTER_LOCAL_PUBLIC_IP`   | unset                   | Church's public egress IP for LAN/WAN detection via Cloudflare Tunnel |
| `PRESENTER_ANDROID_STAGE_URL` | unset                   | Stage URL the launcher opens on every Android stage display via `am start -a VIEW -d <url> <package>`. Set per env in deploy units (prod `http://10.77.9.205/stage`, dev `http://10.77.8.134:8080/stage`). Unset → launcher warns + skips. |
| `RUST_LOG`                    | `info,tower_http=debug` | Tracing filter                                                       |

---

## Code Quality Standards

### Format & Lint (CI enforces)

Pre-push quality gate: `scripts/dev/quality-check.sh --strict --against origin/main` (what CI runs) — details in the ci skill.

---

## Documentation Standards

### ADR Process

1. Copy `docs/adr/template.md` to `docs/adr/NNNN-<slug>.md`
2. Fill all sections; set status to "Proposed"
3. Link from PR description
4. Change status to "Accepted" when implemented

---

## Design Philosophy

- **Reliability over breadth**: Offline-ready, sub-100ms latency
- **Church-specific**: Solve exact requirements for our workflows
- **Greenfield redesigns**: Treat redesigns as fresh starts
- **CI-first**: GitHub Actions validates everything, local testing confirms

---

## User Preferences

### LAN IP (NOT localhost)

The user accesses the server from another machine on the same network. **Always provide LAN IP URLs, never localhost.** (IPs are in the Deployed Instances table above.)

**Never use localhost** — always use the LAN IP.
