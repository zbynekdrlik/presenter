# CLAUDE.md

> **Version:** 2025.7 | **Last Updated:** 2026-03-16

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Local Build Policy

**Local Rust builds (cargo build, cargo clippy, cargo test) are ALLOWED on this machine.** This is the powerful dev2 build machine. Run fmt, clippy, test, and build locally before pushing. The global airuleset default (CI-only) does NOT apply here.

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
3. Merge: human merges, main deploys, production shows `vX.Y.Z`
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

#### Runner Management

```bash
# Check runner status (runs locally — same machine as dev)
cd ~/actions-runner && sudo ./svc.sh status

# View runner logs
sudo journalctl -u actions.runner.zbynekdrlik-presenter.presenter-local -f

# Restart runner
cd ~/actions-runner && sudo ./svc.sh stop && sudo ./svc.sh start

# Re-register (requires fresh token)
gh api -X POST repos/zbynekdrlik/presenter/actions/runners/registration-token --jq '.token'
cd ~/actions-runner && ./config.sh --url https://github.com/zbynekdrlik/presenter --token "$TOKEN" --name presenter-local --labels self-hosted,local
sudo ./svc.sh install && sudo ./svc.sh start
```

### Workflows

| Workflow                | Trigger                    | Purpose                                                   |
| ----------------------- | -------------------------- | --------------------------------------------------------- |
| `pipeline.yml`          | Push to `dev`              | Full pipeline: checks → build → E2E → deploy-dev          |
| `deploy.yml`            | Push to `main`             | Build on GH runner, deploy to production via SSH          |
| `release.yml`           | GitHub Release published   | Build release, upload tarball, deploy to companion-pp.lan |
| `security-schedule.yml` | Weekly (Sunday) + manual   | Scheduled vulnerability scanning                          |
| `import-data.yml`       | Manual (workflow_dispatch) | Re-import ProPresenter/Bible data                         |
| `pr-labeler.yml`        | PR opened/edited           | Auto-label PRs by changed paths                           |

**Pipeline dependency chain:** `branch-sync → [fmt, clippy, companion, version-check, security] → [test, quality] → coverage → build → e2e → deploy-dev`

Deploy-dev **cannot run** unless every check, test, build, and E2E job succeeds.

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
| `RUST_LOG`                    | `info,tower_http=debug` | Tracing filter                                                       |

---

## Database Policy

### Schema Changes (Pre-release)

Schema is mutable during pre-release:

1. **New columns:** Add an incremental migration (e.g., `m20260408_000001_add_column.rs`) that uses `ALTER TABLE ADD COLUMN` with an idempotent guard (check `pragma_table_info` first). Register it in `lib.rs`. This ensures existing databases are upgraded automatically on startup.
2. **New tables:** May be added to the initial migration (uses `if_not_exists()`) or as an incremental migration.
3. **Destructive schema changes** (column renames, type changes): Add an incremental migration. If data must be re-imported, manually trigger the Import Data workflow after deploy.
4. The server auto-migrates on startup via `Repository::connect()`

### Deploy Safety

- Deploys NEVER delete the database — only binaries and service files are updated
- Database is backed up automatically before each deploy (5 retained in `backups/`)
- First-time deploys auto-import; subsequent deploys preserve all user data
- Data import is a separate, explicit "Import Data" workflow in GitHub Actions

### Manual Import

To re-import source data (ProPresenter libraries, Bibles):

1. Go to Actions > "Import Data" > Run workflow
2. Select environment (dev/production) and import type
3. Default mode (`--keep`) preserves existing data
4. `--purge` mode replaces all libraries (WARNING: destroys playlists via FK cascade)

### Library Management

ProPresenter libraries are stored in `data/libraries/` as the single source of truth.

**To update songs:**

1. Export from ProPresenter on Mac
2. Copy `.pro` files to `data/libraries/<LIBRARY_NAME>/`
3. Commit and push to dev
4. Deploy syncs libraries to servers via `rsync`
5. Run Import Data workflow if needed

Deploy workflows automatically sync `data/libraries/` to `/opt/presenter*/libraries/` on target servers.

---

## Code Quality Standards

### Format & Lint (CI enforces)

```bash
cargo fmt --check                                              # Must pass
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all  # Must pass (zero warnings allowed)
```

### Naming Conventions

- `snake_case` for module names
- `PascalCase` for types
- `mod.rs` for orchestration/re-exports only

---

## Anti-Patterns (Never Do These)

- **Don't delete production databases** - Use incremental migrations for schema changes
- **Don't use arbitrary sleeps in tests** - Use retry-with-assert helpers

---

## Documentation Standards

### When to Update Docs

- New features: Update relevant docs and CLAUDE.md
- Architecture changes: Add/update ADR in `docs/adr/`
- Configuration changes: Update `docs/configuration.md`

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

The user accesses the server from another machine on the same network. **Always provide LAN IP URLs, never localhost.**

**Production LAN IP:** `10.77.9.205`
**Dev LAN IP:** `10.77.8.134`

When providing URLs, use:

- http://10.77.9.205/ui/operator (Production)
- http://10.77.8.134:8080/ui/operator (Dev)
- http://10.77.9.205/stage (Production stage)
- http://10.77.8.134:8080/stage (Dev stage)

**Never use localhost** — always use the LAN IP.
