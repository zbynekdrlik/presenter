# CLAUDE.md

> **Version:** 2025.2 | **Last Updated:** 2025-12-30

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

---

## Autonomous Workflow (CRITICAL)

**Agents work autonomously. Do not ask for confirmation. Execute the full cycle:**

```
Code → Commit → Push to dev → Monitor CI → Fix failures → Repeat until green → Local test
```

### The Loop (execute without asking)

1. **Commit and push immediately** - No confirmation needed
   ```bash
   git add -A && git commit -m "..." && git push origin dev
   ```

2. **Monitor CI until completion**
   ```bash
   gh run watch
   ```

3. **If CI fails: fix and repeat from step 1**
   ```bash
   gh run view --log-failed  # See what failed
   # Fix the issue
   # Commit and push again
   ```

4. **When CI is green: test locally**
   ```bash
   cargo run -p presenter-server
   # Verify at http://localhost:80
   ```

**Never ask "should I commit?" or "should I push?" - just do it.**

---

## Git Rules

- **Always push to `dev` branch** - Never to main/master
- **Commit frequently** - Small commits, push often
- **CI validates everything** - Trust the pipeline
- **Only users merge PRs** - Agents prepare, users approve

### Banned Patterns (Production Code)

| Pattern | Why Banned | Alternative |
|---------|------------|-------------|
| `unwrap()` | Panics in production | Use `?`, `ok_or()`, or handle `None`/`Err` |
| `expect()` | Panics in production | Use `?` with context via `anyhow`/`thiserror` |
| `panic!` | Crashes the service | Return `Result` or `Option` |
| `std::thread::sleep` | Blocks async runtime | Use `tokio::time::sleep` |
| `.only` / `.skip` | Leaves incomplete test coverage | Remove before commit |

### File/Function Limits (Enforced by CI)

| Metric | Warning | Hard Fail |
|--------|---------|-----------|
| File lines | >800 | >1000 |
| Function lines | - | >60 |

---

## GitHub Actions (Primary CI/CD)

### Self-Hosted Runner

This project uses a **local self-hosted runner** to save GitHub Actions costs.

**Runner location:** This machine
**Runner label:** `self-hosted`

All workflows run on the local runner, providing:
- Faster builds (local caching)
- No GitHub minutes consumed
- Full access to local resources

### Workflows

| Workflow | Trigger | Purpose |
|----------|---------|---------|
| `ci.yml` | Push to `dev`/`main`, PRs | Format, lint, test, quality |
| `e2e.yml` | Push to `dev`/`main`, PRs | Playwright E2E tests |
| `security.yml` | Weekly + manual | Vulnerability scanning |
| `release.yml` | Release tags | Build artifacts, Docker image |

### Monitoring CI

```bash
# Watch current run
gh run watch

# List recent runs
gh run list

# View specific run
gh run view <run-id>

# Re-run failed jobs
gh run rerun <run-id> --failed
```

---

## Project Overview

Presenter is a monolithic Rust application for church worship services, providing lyrics display, Bible passages, timers, and stage displays.

**Key Documentation:**
- Domain specifics: `docs/functional-needs.md`
- System architecture: `docs/architecture.md`
- Configuration reference: `docs/configuration.md`

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

---

## Testing

GitHub Actions runs all tests automatically. For local debugging:

```bash
# Rust unit tests
cargo test

# Single test
cargo test test_name

# Playwright E2E
npm run test:playwright
npm run test:playwright:headed  # Browser visible

# View Playwright report
scripts/dev/show-playwright-report.sh
```

### E2E Test Policy

- E2E tests are the **primary acceptance mechanism**
- Ship new behavior only with E2E coverage
- Tests must be deterministic: fixed seeds, stable timeouts
- Prefer retry-with-assert poll helpers over arbitrary sleeps

---

## Architecture

### Workspace Crates

```
crates/
├── presenter-core/        # Domain logic (no server deps)
├── presenter-server/      # Axum HTTP/WS + Leptos SSR
├── presenter-persistence/ # SeaORM repository layer
├── presenter-migration/   # Schema evolution
├── presenter-importer/    # ProPresenter import
└── presenter-bible/       # Bible translation ingestion
```

### Key HTTP Endpoints

| Endpoint | Purpose |
|----------|---------|
| `/healthz` | Readiness probe |
| `/ui/operator` | Desktop control surface |
| `/ui/tablet` | Touch-optimized controller |
| `/ui/bible` | Bible search/trigger UI |
| `/stage` | HTML stage display |
| `/live/ws` | Live updates (timers, stage) |
| `/companion/ws` | Bitfocus Companion control |

---

## Environment Variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `PRESENTER_PORT` | 80 | Server port |
| `PRESENTER_DB_URL` | `sqlite://presenter.db` | Database connection |
| `PRESENTER_COMPANION_ENABLED` | 0 | Enable Companion socket |
| `PRESENTER_COMPANION_PORT` | 18175 | Companion listen port |
| `RUST_LOG` | `info,tower_http=debug` | Tracing filter |

---

## Database Policy (Pre-release)

Schema is mutable during pre-release:

1. **Never add incremental migrations** - Edit initial migration directly
2. Rebuild SQLite from scratch after schema changes
3. Keep `scripts/dev/refresh-dev-data.sh` up to date

---

## Commit Standards

### Conventional Commits (Required)

```
<type>(<scope>): <description>

feat(ui): add dark mode toggle
fix(timer): correct countdown display
docs(readme): update installation steps
refactor(state): split into feature modules
test(e2e): add playlist drag-drop tests
chore(deps): update tokio to 1.40
```

**Types:** `feat`, `fix`, `docs`, `style`, `refactor`, `test`, `chore`, `perf`, `ci`

---

## Code Quality Standards

### Format & Lint (CI enforces)

```bash
cargo fmt --check              # Must pass
cargo clippy -- -D warnings    # Must pass
```

### Naming Conventions

- `snake_case` for module names
- `PascalCase` for types
- `mod.rs` for orchestration/re-exports only

---

## Anti-Patterns (Never Do These)

- **Don't commit to main** - Always use `dev` branch
- **Don't skip CI** - Wait for green before testing locally
- **Don't add incremental migrations** - Edit initial migration directly
- **Don't defer E2E tests** - Add mocks if dependencies are hard to access
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
