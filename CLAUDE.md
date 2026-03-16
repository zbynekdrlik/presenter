# CLAUDE.md

> **Version:** 2025.7 | **Last Updated:** 2026-03-16

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

---

## Complete Implementation Policy (ABSOLUTE - NO EXCEPTIONS)

**ALWAYS implement 100% of the requested feature in ONE run. No phases. No skips. No shortcuts.**

### The Rule

When asked to implement a feature, rewrite, or replacement:

1. **Implement EVERYTHING** - Every function, every modal, every handler, every edge case
2. **No "partial implementation"** - If the feature has 10 parts, implement all 10
3. **No "tests for later"** - Write tests AND implementation together
4. **No removing tests to pass CI** - If tests fail, fix the code, not delete the tests
5. **No "phase 1, phase 2"** - Complete the entire feature in one session
6. **No "this can be added later"** - Add it NOW

### What This Means

- Asked to replace JS frontend with WASM? Implement **100% feature parity** - every modal, every drag-drop, every keyboard shortcut
- Asked to add a feature? Implement the **complete feature** with all edge cases
- Tests failing? **Fix the implementation**, not remove the tests
- Feature seems large? **Still do it all** - no excuses about scope

### Banned Behaviors

| Behavior                    | Why Banned             | Required Instead    |
| --------------------------- | ---------------------- | ------------------- |
| "Phase 1 complete"          | Incomplete delivery    | Complete ALL phases |
| Removing failing tests      | Hides missing features | Fix the code        |
| "Will add X later"          | Leaves gaps            | Add X NOW           |
| "Basic implementation done" | Incomplete             | Full implementation |
| Skipping edge cases         | Buggy result           | Handle ALL cases    |

**If you cannot complete 100% of a feature, say so BEFORE starting - never deliver partial work.**

---

## Autonomous Workflow (CRITICAL)

**Agents work autonomously. Do not ask for confirmation. Execute the full cycle:**

```
Code → Commit → Push to dev → Monitor CI → Fix failures → Repeat until green → STOP (user verifies on dev)
```

### The Loop (execute without asking)

1. **Sync with main, then commit and push** - No confirmation needed

   ```bash
   git fetch origin && git merge origin/main --no-edit && git add -A && git commit -m "..." && git push origin dev
   ```

   **CRITICAL: Always sync dev with main BEFORE pushing.** CI will fail fast if branch is behind main.

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

4. **When CI is green: STOP and report to user**
   - Provide the dev environment URL for the user to verify the feature manually
   - **Do NOT create a PR** — wait for the user to confirm the feature works on dev
   - **Do NOT test locally** unless the user asks

**Never ask "should I commit?" or "should I push?" - just do it.**

### PR Creation (ALWAYS automatic)

**ALWAYS create a pull request when CI is green.** This is non-negotiable:

1. Push to dev and wait for ALL CI workflows to be green
2. Create PR from dev → main immediately
3. Wait for PR checks to pass
4. Provide PR URL that is green and mergeable
5. Do NOT stop until PR is ready for merge

### Deliverables (ALWAYS provide at end of task)

When a task is complete, **always** provide:

- **PR URL:** Green and mergeable (user merges manually)
- **Dev environment:** http://10.77.8.134:8080/ui/operator (for user to verify)
- **CI status:** Confirm all workflows are green
- **What to test:** Brief list of what the user should verify

**A task is NOT complete until the PR is green and mergeable.**

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

**Note:** Test code (`#[cfg(test)]` modules) and WASM code (`presenter-ui` crate) are exempt from panic rules. WASM panics become browser-side JavaScript errors rather than server crashes.

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

### Runner Architecture

This project uses a **mixed runner strategy**:

- **GitHub-hosted runners** (`ubuntu-latest`): All compilation, linting, testing, and security scans. Free for public repos.
- **One bare-metal local runner** (`self-hosted`): E2E tests (need running binary) and deployments (SSH to LAN hosts).

**Local runner host:** `10.77.8.189`
**Local runner name:** `presenter-local`
**Local runner label:** `self-hosted`
**Config location:** `~/actions-runner/` on 10.77.8.189

The local runner does NOT compile Rust — it only runs pre-built artifacts downloaded from GitHub-hosted build jobs. It needs: Node.js 22, Playwright chromium, rsync, and SSH access to LAN deploy targets.

**Deploy workflows** use SSH from the local runner to application hosts (`10.77.9.205` for production, `10.77.8.134` for dev, `companion-pp.lan` for PP releases).

#### Runner Management

```bash
# SSH to runner host
ssh 10.77.8.189

# Check runner status
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

## Testing & CI (CRITICAL - READ CAREFULLY)

### Absolute Rules (NO EXCEPTIONS)

1. **ALL CI workflows MUST be green** - Every workflow, every job, every check
2. **ALL tests MUST pass** - Unit, integration, E2E, security scans - every single one
3. **NEVER skip tests** - No `.skip()`, `.only()`, `#[ignore]`, `testIgnore`, or any mechanism
4. **Fix failures IMMEDIATELY** - CI failures block everything until resolved
5. **E2E tests are PRIMARY** - They are the acceptance gate, not optional
6. **VERIFY changes work BEFORE claiming completion** - See below

### Verify Before Claiming Complete (CRITICAL)

**NEVER claim a feature works without verifying it actually works.** This is non-negotiable:

1. **Edit the correct files** - Before editing, verify which files are actually used by checking imports (`mod` declarations in Rust, `import` statements in TypeScript). Do not edit orphaned/unused files.
2. **Run targeted tests** - After making changes, run tests that specifically exercise the changed code path.
3. **Verify the behavior** - For user-facing changes, manually verify on dev environment or write a test that proves the behavior.
4. **Check existing tests still pass** - Run the full test suite (`cargo test --workspace`) before pushing.

**Common mistakes to avoid:**

- Editing files that aren't included in the module tree (e.g., `parse.rs` when `mod parsers;` imports `parsers.rs`)
- Claiming code changes fix an issue without running tests that exercise the fix
- Pushing changes without verifying CI will pass
- Assuming database changes take effect without re-importing data

**The test must fail before your fix, and pass after.** If you cannot demonstrate this, you have not verified the fix.

### CI Failure Policy (ABSOLUTE)

**ALL CI must be green. Zero exceptions. Zero excuses.**

- Every CI failure is YOUR responsibility, regardless of when introduced
- Stop all other work immediately when ANY workflow fails
- Fix the root cause - no workarounds, no skips, no "pre-existing issue" excuses
- If a workflow cannot pass, FIX IT or REMOVE IT
- NEVER classify failures as "non-blocking" or "acceptable"
- When uncertain, ask the user - never assume
- `gh run list` must show ALL workflows as `success` before claiming "CI is green"

**Diagnosis:** `gh run view --log-failed`

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

### E2E Coverage Mandate (NO EXCEPTIONS)

**Every change MUST include comprehensive E2E tests.** This is non-negotiable:

1. **All new features** must have E2E tests covering the full user workflow
2. **All bug fixes** must have E2E tests that reproduce the bug and verify the fix
3. **All UI changes** must have E2E tests verifying the visual/interactive behavior
4. **Do NOT skip E2E paths** to save effort — cover ALL user-facing paths affected by the change
5. **A PR without E2E tests for changed functionality is UNACCEPTABLE and must not be submitted**

**Before submitting any PR**, verify:

- Every changed/added feature has at least one E2E test exercising it
- Edge cases are covered (empty states, error states, boundary conditions)
- Tests verify the actual user-visible behavior, not implementation details

### Critical Self-Review of Test Quality

**Be ruthlessly critical of your own test implementation.** Before marking tests as done:

1. **Challenge coverage completeness**: Ask "what user paths did I NOT test?" and add them
2. **Challenge test robustness**: Ask "could this test pass even if the feature is broken?" — if yes, strengthen assertions
3. **Challenge test isolation**: Verify tests don't depend on execution order or shared state
4. **Challenge assertion quality**: Prefer specific assertions (`toHaveText("exact value")`) over weak ones (`toBeVisible()`)
5. **Challenge edge cases**: Test with empty data, maximum data, special characters, rapid interactions
6. **Never take shortcuts**: Do not reduce test scope to make implementation easier or faster
7. **Tests must fail when the feature breaks**: If you can imagine a regression that wouldn't be caught, add a test for it

**The goal is tests that a human reviewer would trust as proof the feature works correctly.**

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
| `PRESENTER_PORT`              | 80                      | Server port             |
| `PRESENTER_DB_URL`            | `sqlite://presenter.db` | Database connection     |
| `PRESENTER_COMPANION_ENABLED` | 0                       | Enable Companion socket |
| `PRESENTER_COMPANION_PORT`    | 18175                   | Companion listen port   |
| `RUST_LOG`                    | `info,tower_http=debug` | Tracing filter          |

---

## Database Policy

### Schema Changes (Pre-release)

Schema is mutable during pre-release:

1. **Never add incremental migrations** - Edit initial migration directly
2. The server auto-migrates on startup via `Repository::connect()`
3. For destructive schema changes, manually trigger the Import Data workflow after deploy

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
cargo fmt --check                                              # Must pass
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all  # Must pass (zero warnings allowed)
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

**Production LAN IP:** `10.77.9.205`
**Dev LAN IP:** `10.77.8.134`

When providing URLs, use:

- http://10.77.9.205/ui/operator (Production)
- http://10.77.8.134:8080/ui/operator (Dev)
- http://10.77.9.205/stage (Production stage)
- http://10.77.8.134:8080/stage (Dev stage)

**Never use localhost** — always use the LAN IP.
