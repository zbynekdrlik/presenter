# CLAUDE.md

> **Version:** 2025.4 | **Last Updated:** 2025-12-31

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

### Deliverables (ALWAYS provide at end of task)

When a task is complete, **always** provide these links:

- **Dev environment:** http://10.77.9.191:8080/ui/operator (verify healthy)
- **PR to main:** GitHub PR URL (must be green and mergeable)
- **Production environment:** http://10.77.9.191/ui/operator (verify healthy)

---

## Git Rules

### Branch Policy (STRICT)

**Only two branches exist:** `main` and `dev`. No exceptions.

- `dev` - All development happens here
- `main` - Production releases only (merged from dev by human)

**Before starting any work**, verify no stale branches exist:

```bash
git fetch --prune && git branch -r | grep -v -E '(main|dev|HEAD)'
```

If any branches exist, delete them first: `git push origin --delete <branch>`

### PR Policy (STRICT)

**Only ONE pull request at a time.** No exceptions. Maximum 1 PR open in the entire repo.

- Before creating a new PR, close any existing open PRs
- The **only** valid PRs are `dev → main` for releases
- If any unexpected PRs exist (Dependabot, stale branches, etc.), close them immediately

**NEVER create a PR until ALL CI workflows are green.** This is non-negotiable:

1. Push to dev
2. Wait for **every** CI workflow to complete with `conclusion: success`
3. Verify with: `gh run list --limit 10 --json name,status,conclusion`
4. Only when **all** are green, then create the PR with `gh pr create`
5. After PR creation, wait for **all PR-triggered checks** to go green too
6. Verify PR is mergeable: `gh pr view <number> --json mergeable,mergeStateStatus`

**A PR that is not green and mergeable is unacceptable. NEVER leave a PR in a failing state.**

**Dependabot is DISABLED.** Dependencies are managed manually via the dev branch. The `.github/dependabot.yml` file has been removed. Do not re-enable it.

**Check before any PR work:**

```bash
gh pr list --state open
```

If any PRs exist (other than the one you're working on), close them first.

### Core Rules

- **Always push to `dev` branch** - Never to main/master
- **Commit frequently** - Small commits, push often
- **CI validates everything** - Trust the pipeline
- **Only users merge PRs** - Agents prepare, users approve
- **NEVER merge to main/master** - Only the repository owner (human) can merge PRs to main/master. Claude must never execute `git merge`, `gh pr merge`, or any command that merges code to main/master, even if explicitly asked. This requires the human to perform the merge manually.

### Banned Patterns (Production Code)

| Pattern              | Why Banned                      | Alternative                                   |
| -------------------- | ------------------------------- | --------------------------------------------- |
| `unwrap()`           | Panics in production            | Use `?`, `ok_or()`, or handle `None`/`Err`    |
| `expect()`           | Panics in production            | Use `?` with context via `anyhow`/`thiserror` |
| `panic!`             | Crashes the service             | Return `Result` or `Option`                   |
| `std::thread::sleep` | Blocks async runtime            | Use `tokio::time::sleep`                      |
| `.only` / `.skip`    | Leaves incomplete test coverage | Remove before commit                          |

**Note:** Test code (`#[cfg(test)]` modules) is exempt from panic rules.

### File/Function Limits (Enforced by CI)

| Metric         | Warning | Hard Fail | Exempt                                  |
| -------------- | ------- | --------- | --------------------------------------- |
| File lines     | >800    | >1000     | Migrations, tests                       |
| Function lines | >80     | >120      | Migrations, UI renders, router builders |

**Exempt patterns:**

- `m*_create_*.rs` - Migration files (declarative schema definitions)
- `render_*_ui` functions - Leptos component renders (HTML-like DSL)
- `build_router` functions - Route declarations

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

### Self-Hosted Runners

This project uses **3 Docker-based self-hosted runners** on a remote machine to save GitHub Actions costs.

**Runner host:** `10.77.8.189`
**Runner names:** `presenter-runner-1`, `presenter-runner-2`, `presenter-runner-3`
**Runner label:** `self-hosted`
**Config location:** `~/presenter-runners/` on 10.77.8.189

All workflows run on the Docker runners, providing:

- Faster builds (local caching, shared cargo volumes)
- No GitHub minutes consumed
- 3 parallel runners for concurrent workflow execution
- Reproducible environment (Docker image with Rust, Node.js, Playwright, Docker CLI)

**Deploy workflows** use SSH to deploy binaries from runners back to the application host (`10.77.9.191`).

#### Runner Management

```bash
# SSH to runner host
ssh 10.77.8.189

# Check runner status
cd ~/presenter-runners && docker compose ps

# View runner logs
docker compose logs -f presenter-runner-1

# Restart all runners (requires fresh registration token)
gh api -X POST repos/zbynekdrlik/presenter/actions/runners/registration-token --jq '.token'
# Update .env with new RUNNER_TOKEN, then:
docker compose down --timeout 5 && docker compose up -d

# Rebuild image after Dockerfile changes
docker build -t presenter-runner:latest .
docker compose down --timeout 5 && docker compose up -d
```

### Workflows

| Workflow            | Trigger                   | Purpose                                         |
| ------------------- | ------------------------- | ----------------------------------------------- |
| `ci.yml`            | Push to `dev`/`main`, PRs | Format, lint, test, quality                     |
| `e2e.yml`           | Push to `dev`/`main`, PRs | Playwright E2E tests                            |
| `version-check.yml` | Push to `dev`/`main`, PRs | Validate version format                         |
| `security.yml`      | Weekly + manual           | Vulnerability scanning                          |
| `deploy-dev.yml`    | Push to `dev`             | Deploy dev binary via SSH to /opt/presenter-dev |
| `deploy.yml`        | Push to `main`            | Deploy prod binary via SSH to /opt/presenter    |
| `release.yml`       | GitHub Release published  | Build and upload release artifacts              |

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

### Deployed Instances

Two instances run simultaneously on the self-hosted runner:

| Instance   | URL                     | Port | Service                 | Deploy Dir           | Branch |
| ---------- | ----------------------- | ---- | ----------------------- | -------------------- | ------ |
| Production | http://10.77.9.191      | 80   | `presenter.service`     | `/opt/presenter`     | `main` |
| Dev        | http://10.77.9.191:8080 | 8080 | `presenter-dev.service` | `/opt/presenter-dev` | `dev`  |

```bash
# Check both services
systemctl status presenter presenter-dev

# View dev logs
sudo journalctl -u presenter-dev -f

# Restart dev only
sudo systemctl restart presenter-dev
```

---

## Testing & CI (CRITICAL - READ CAREFULLY)

### Absolute Rules (NO EXCEPTIONS)

1. **ALL CI workflows MUST be green** - Every workflow, every job, every check
2. **ALL tests MUST pass** - Unit, integration, E2E, security scans - every single one
3. **NEVER skip tests** - No `.skip()`, `.only()`, `#[ignore]`, `testIgnore`, or any mechanism
4. **Fix failures IMMEDIATELY** - CI failures block everything until resolved
5. **E2E tests are PRIMARY** - They are the acceptance gate, not optional

### CI Failures = STOP EVERYTHING

When ANY CI workflow or check fails:

1. **Stop all other work immediately**
2. **Diagnose the failure** - `gh run view --log-failed`
3. **Fix the root cause** - Not workarounds, not skips
4. **Push and verify green** - Only then continue other work

**There is NO "fix later" or "known issue" or "flaky test" or "pre-existing issue" excuse. Fix it NOW.**

### No "Non-Blocking" Failures

**NEVER classify any CI failure as "non-blocking" or "acceptable":**

- No "pre-existing infrastructure issues" excuse
- No "this workflow was already broken" excuse
- No "nightly Rust is flaky" excuse
- No "security scans are optional" excuse

**If a workflow cannot pass, either FIX IT or REMOVE IT. Do not leave broken workflows in the repository.**

**When uncertain whether something should block merge, ALWAYS ask the user. Never assume.**

### Banned Test Patterns

| Pattern                       | Why Banned         | What To Do Instead       |
| ----------------------------- | ------------------ | ------------------------ |
| `.skip()` / `.only()`         | Hides failures     | Remove and fix the test  |
| `#[ignore]`                   | Skips silently     | Remove and fix the test  |
| `testIgnore` in config        | Hides entire files | Remove and fix the tests |
| `continue-on-error` for tests | Masks failures     | Remove, tests must pass  |
| Timeouts that "cancel"        | Hides slow tests   | Fix the slowness         |
| Conditional test runs         | Reduces coverage   | Run all tests always     |

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

### E2E Test Standards

- E2E tests are the **primary acceptance mechanism**
- Ship new behavior only with E2E coverage
- Tests must be deterministic: fixed seeds, stable timeouts
- Prefer retry-with-assert poll helpers over arbitrary sleeps
- **E2E timeout = build failure** - Optimize build caching, not extend timeouts

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
| `PRESENTER_PORT`              | 80                      | Server port             |
| `PRESENTER_DB_URL`            | `sqlite://presenter.db` | Database connection     |
| `PRESENTER_COMPANION_ENABLED` | 0                       | Enable Companion socket |
| `PRESENTER_COMPANION_PORT`    | 18175                   | Companion listen port   |
| `RUST_LOG`                    | `info,tower_http=debug` | Tracing filter          |

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

---

## User Preferences

### LAN IP (NOT localhost)

The user accesses the server from another machine on the same network. **Always provide LAN IP URLs, never localhost.**

**LAN IP:** `10.77.9.191` (default route interface)

When providing URLs, use:

- http://10.77.9.191/ui/operator (Production)
- http://10.77.9.191:8080/ui/operator (Dev)
- http://10.77.9.191/stage (Production stage)
- http://10.77.9.191:8080/stage (Dev stage)

**Never use localhost** — always use the LAN IP.
