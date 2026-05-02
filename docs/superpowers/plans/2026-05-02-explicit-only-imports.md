# Explicit-Only Imports Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the auto-import-on-startup logic from `presenter-server` so deploys never touch the database. The Import Data GitHub Actions workflow becomes the only (re)populate path.

**Architecture:** Delete `ensure_seed_library` from `crates/presenter-server/src/state/mod.rs` and both call sites (production constructor and `in_memory` test helper). Move `sample_library()` to `#[cfg(test)]` and add a test-only `seed_sample_library()` helper for tests that need a populated state. Add an integration test that asserts a fresh `AppState` against an empty DB leaves libraries empty.

**Tech Stack:** Rust (tokio, anyhow, axum), SQLite via SeaORM.

**Spec:** `docs/superpowers/specs/2026-05-02-explicit-only-imports-design.md`

**Closes:** Issue #228 — `ensure_seed_library` race defeats deploy import detection.

---

## Context

User reported re-imports overwriting hand-curated playlists on every deploy. The cause is `ensure_seed_library` in `crates/presenter-server/src/state/mod.rs:691-696`:

```rust
async fn ensure_seed_library(&self) -> anyhow::Result<()> {
    if self.repository.fetch_libraries().await?.is_empty() {
        self.repository.upsert_library(&sample_library()).await?;
    }
    Ok(())
}
```

It is called at startup from two places:

- Line 373: `AppState::new` (production) — `state.ensure_seed_library().await?;`
- Line 524: `AppState::in_memory` (test helper) — `state.ensure_seed_library().await?;`

`sample_library()` lives in `crates/presenter-server/src/state/seed.rs` and creates a hardcoded "Sample Library" with 2 slides — a toy fixture, NOT real ProPresenter data. The Import Data GitHub Actions workflow is the real population path and is unaffected by this change.

Multiple tests in `crates/presenter-server/src/state/tests.rs` rely on `AppState::in_memory()` returning a state with the seed library populated (e.g. `seeded_state_contains_library` at line 10 asserts `libraries[0].name == "Sample Library"`; later tests grab `libraries[0].presentations[0]` for stage/timer/live tests). These need to keep working: provide a test-only `seed_sample_library()` helper and have existing tests call it explicitly.

**Pre-flight grep already done** — confirmed `ensure_seed_library` / `sample_library` are the only matches in the workspace. No `auto_import`, `bootstrap_libraries`, `import_libraries_from_disk`, or `seed_library_path` references exist.

---

## File Structure

### Modified Files
| File | Change |
|------|--------|
| `Cargo.toml` | Bump `[workspace.package].version` 0.4.50 → 0.4.51 |
| `crates/presenter-ui/Cargo.toml` | Bump `[package].version` 0.1.19 → 0.1.20 |
| `crates/presenter-server/src/state/mod.rs` | Remove `ensure_seed_library` method (lines 691-696) and both call sites (line 373 and 524). Update `use seed::sample_library;` and `pub use seed::TestBibleIngestion;` import block accordingly. |
| `crates/presenter-server/src/state/seed.rs` | Gate `sample_library()` under `#[cfg(test)]` and add a `seed_sample_library(state: &AppState)` test-only helper. |
| `crates/presenter-server/src/state/tests.rs` | Update tests that depend on auto-seeded data to call `seed_sample_library` explicitly. Convert `seeded_state_contains_library` into a regression guard that asserts a fresh `in_memory()` state leaves libraries empty. |
| `CLAUDE.md` | Replace line 266 wording about auto-import. |

### Lock files
- `Cargo.lock` — auto-updated by `cargo build` after version bumps.

---

## Task 1: Bump Version (Haiku)

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/presenter-ui/Cargo.toml`
- Modify: `Cargo.lock` (regenerated)

- [ ] **Step 1: Bump workspace version**

In `/home/newlevel/devel/presenter/presenter-dev2/Cargo.toml`, change:

```toml
version = "0.4.50"
```

to:

```toml
version = "0.4.51"
```

- [ ] **Step 2: Bump presenter-ui crate version**

In `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-ui/Cargo.toml`, change:

```toml
version = "0.1.19"
```

to:

```toml
version = "0.1.20"
```

- [ ] **Step 3: Regenerate Cargo.lock**

Run:

```bash
cargo check --workspace --all-targets 2>&1 | tail -20
```

Expected: clean check, `Cargo.lock` updated to reflect 0.4.51 and 0.1.20.

- [ ] **Step 4: Verify version visible in /healthz route at compile time**

Run:

```bash
grep -rn '"0.4.50"\|"0.1.19"' Cargo.toml Cargo.lock crates/presenter-ui/Cargo.toml 2>&1
```

Expected: NO matches. If any match remains, fix before commit.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock crates/presenter-ui/Cargo.toml
git commit -m "chore: bump version to 0.4.51 (#228)"
```

---

## Task 2: Remove Auto-Import Logic (Sonnet)

**Files:**
- Modify: `crates/presenter-server/src/state/mod.rs` (lines 73, 373, 524, 691-696)
- Modify: `crates/presenter-server/src/state/seed.rs` (gate function)
- Modify: `crates/presenter-server/src/state/tests.rs` (update dependent tests)

### Pre-task grep verification

- [ ] **Step 1: Confirm no other auto-import references exist**

Run:

```bash
grep -rn -E "ensure_seed_library|auto_import|seed_library|bootstrap_libraries|import_libraries_from_disk|seed_libraries|first_time_import|present_seeds_path|seed_library_path" crates/ src/ 2>/dev/null
```

Expected matches: ONLY the three lines in `crates/presenter-server/src/state/mod.rs` (373, 524, 691) plus the function definition. Any additional matches must be reported back to the controller before proceeding.

### Code changes

- [ ] **Step 2: Remove the call site in `AppState::new` (production)**

In `crates/presenter-server/src/state/mod.rs` around line 373, delete the line:

```rust
        state.ensure_seed_library().await?;
```

The surrounding context should look like (BEFORE):

```rust
            local_public_ip,
        );
        state.ensure_seed_library().await?;

        // Pre-load group color cache from database
        let group_colors = state
```

AFTER:

```rust
            local_public_ip,
        );

        // Pre-load group color cache from database
        let group_colors = state
```

- [ ] **Step 3: Remove the call site in `AppState::in_memory` (test helper)**

In `crates/presenter-server/src/state/mod.rs` around line 524, delete the line:

```rust
        state.ensure_seed_library().await?;
```

AFTER:

```rust
            ableset_bridge.clone(),
        );
        state.ensure_demo_playlist().await?;
        state.sync_android_stage_displays().await?;
```

- [ ] **Step 4: Remove the `ensure_seed_library` method**

In `crates/presenter-server/src/state/mod.rs` around lines 691-696, delete:

```rust
    async fn ensure_seed_library(&self) -> anyhow::Result<()> {
        if self.repository.fetch_libraries().await?.is_empty() {
            self.repository.upsert_library(&sample_library()).await?;
        }
        Ok(())
    }
```

- [ ] **Step 5: Update the `use seed::*` import block**

In `crates/presenter-server/src/state/mod.rs` around line 73, change:

```rust
use seed::sample_library;
#[cfg(test)]
pub use seed::TestBibleIngestion;
```

to:

```rust
#[cfg(test)]
pub use seed::TestBibleIngestion;
#[cfg(test)]
pub(crate) use seed::seed_sample_library;
```

(The `sample_library` import is no longer needed because `ensure_seed_library` was removed. The new `seed_sample_library` helper is added in Step 6.)

- [ ] **Step 6: Gate `sample_library()` under `#[cfg(test)]` and add a test-only seeding helper**

`TestBibleIngestion` is already `#[cfg(test)]`-gated. The change here is to gate `sample_library()` and add a `seed_sample_library()` helper. Replace the entire contents of `crates/presenter-server/src/state/seed.rs` with:

```rust
#[cfg(test)]
use presenter_core::{
    Library, LibraryId, Presentation, PresentationId, Slide, SlideContent, SlideGroup, SlideId,
    SlideText,
};

#[cfg(test)]
#[async_trait::async_trait]
pub trait TestBibleIngestion {
    async fn ingest_default_translations(
        &self,
    ) -> anyhow::Result<Vec<presenter_bible::BibleImportSummary>>;
}

#[cfg(test)]
pub(crate) fn sample_library() -> Library {
    // These unwrap calls are safe because the sample data uses known-valid strings
    // that are well within the character limits
    let main1 = SlideText::new("Welcome to service")
        .unwrap_or_else(|_| SlideText::new("Welcome").unwrap_or_else(|_| unreachable!()));
    let trans1 = SlideText::new("Vitajte")
        .unwrap_or_else(|_| SlideText::new("").unwrap_or_else(|_| unreachable!()));
    let stage1 = SlideText::new("Stage cue")
        .unwrap_or_else(|_| SlideText::new("").unwrap_or_else(|_| unreachable!()));

    let main2 = SlideText::new("Let's worship")
        .unwrap_or_else(|_| SlideText::new("Worship").unwrap_or_else(|_| unreachable!()));
    let trans2 = SlideText::new("Poďme chváliť")
        .unwrap_or_else(|_| SlideText::new("").unwrap_or_else(|_| unreachable!()));
    let stage2 = SlideText::new("Cue")
        .unwrap_or_else(|_| SlideText::new("").unwrap_or_else(|_| unreachable!()));

    let presentation = Presentation::new(
        "Welcome",
        vec![
            Slide::new(
                0,
                SlideContent::new(main1, trans1, stage1, Some(SlideGroup::new("Intro"))),
            )
            .with_id(SlideId::new()),
            Slide::new(1, SlideContent::new(main2, trans2, stage2, None)).with_id(SlideId::new()),
        ],
    )
    .unwrap_or_else(|_| {
        Presentation::new("Welcome", vec![])
            .unwrap_or_else(|_| unreachable!("empty presentation should be valid"))
    })
    .with_id(PresentationId::new());

    Library::new("Sample Library", vec![presentation])
        .unwrap_or_else(|_| {
            Library::new("Sample", vec![])
                .unwrap_or_else(|_| unreachable!("empty library should be valid"))
        })
        .with_id(LibraryId::new())
}

#[cfg(test)]
pub(crate) async fn seed_sample_library(
    state: &super::AppState,
) -> anyhow::Result<()> {
    state.repository.upsert_library(&sample_library()).await?;
    Ok(())
}
```

The whole file becomes test-only. If `presenter-bible` is no longer pulled into `presenter-server`'s non-test build because of this gating, that's expected — it's a dev-dep / test-only path now.

- [ ] **Step 7: Update `state/bible.rs` if `TestBibleIngestion` reference broke**

In `crates/presenter-server/src/state/bible.rs:665`, the type reference is:

```rust
ingestion: std::sync::Arc<dyn super::seed::TestBibleIngestion + Send + Sync>,
```

This line is already inside a `#[cfg(test)]` block (verify by reading 5-10 lines above line 665). If it is NOT under `#[cfg(test)]`, gate it with `#[cfg(test)]` so the trait reference works only in test builds.

Run:

```bash
sed -n '655,670p' crates/presenter-server/src/state/bible.rs
```

If the surrounding code is not test-gated, report back to the controller before proceeding.

- [ ] **Step 8: Update existing tests in `state/tests.rs`**

In `crates/presenter-server/src/state/tests.rs`:

a) Replace the test `seeded_state_contains_library` (lines 9-15) with a regression guard:

```rust
#[tokio::test]
async fn empty_state_does_not_auto_seed_library() {
    // Regression guard for issue #228: server startup must NOT auto-import any
    // library. The Import Data workflow is the ONLY (re)populate path.
    let state = AppState::in_memory().await.unwrap();
    let libraries = state.libraries().await.unwrap();
    assert!(
        libraries.is_empty(),
        "expected empty libraries on fresh state, found {}",
        libraries.len()
    );
}
```

b) For every OTHER test in this file that calls `AppState::in_memory().await.unwrap()` and then accesses `state.libraries().await.unwrap()[0]` (or otherwise depends on the seed library being present), insert this line immediately after `AppState::in_memory().await.unwrap()`:

```rust
    super::seed_sample_library(&state).await.unwrap();
```

To find every affected test, run:

```bash
grep -nE "AppState::in_memory|state\.libraries\(\)\.await" crates/presenter-server/src/state/tests.rs
```

Inspect each match. If a test grabs `libraries[0]` (or a presentation by index) without first seeding, add the helper call. If a test creates its own data and does not depend on the seed, leave it alone.

- [ ] **Step 9: Build and run unit tests**

```bash
cargo test -p presenter-server --lib 2>&1 | tail -40
```

Expected: all tests pass. The new `empty_state_does_not_auto_seed_library` test must be in the output and PASS.

If any test fails because it still depends on the seed library, add `super::seed_sample_library(&state).await.unwrap();` to the test body.

- [ ] **Step 10: Run clippy**

```bash
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all 2>&1 | tail -30
```

Expected: zero warnings. If any unused-import or dead-code warnings appear (e.g. `sample_library` flagged as unused under non-test cfg), fix in this commit.

- [ ] **Step 11: Run fmt**

```bash
cargo fmt --all
```

- [ ] **Step 12: Commit**

```bash
git add crates/presenter-server/src/state/mod.rs crates/presenter-server/src/state/seed.rs crates/presenter-server/src/state/tests.rs crates/presenter-server/src/state/bible.rs
git commit -m "fix(server): remove auto-import-on-startup; closes #228

The presenter-server no longer calls ensure_seed_library on startup.
Server startup runs schema migrations and serves traffic; it never
touches the libraries table. The Import Data GitHub Actions workflow
remains the only (re)populate path.

Tests that depend on the canonical fixture now call the test-only
seed_sample_library helper explicitly. The new
empty_state_does_not_auto_seed_library test guards against future
re-introduction of auto-import logic."
```

---

## Task 3: Add Integration Test for Empty-DB Boot (Sonnet)

**Files:**
- Create: `crates/presenter-server/tests/empty_db_startup.rs`

The unit-level regression guard in Task 2 covers `AppState::in_memory()`. This task adds an end-to-end integration test that boots `AppState::new` against a temp-file SQLite DB and asserts the libraries endpoint returns empty — covering the production code path explicitly.

- [ ] **Step 1: Check if integration tests directory exists and inspect existing patterns**

```bash
ls crates/presenter-server/tests/ 2>/dev/null
```

If empty or missing, the test file is the first integration test. If other `.rs` files exist there, read one to see the project's integration-test pattern (especially how they construct `AppState`).

- [ ] **Step 2: Write the integration test**

Create `crates/presenter-server/tests/empty_db_startup.rs` with:

```rust
//! Integration test guarding issue #228: server startup must never auto-import
//! libraries. A fresh `AppState::new` against an empty SQLite database must
//! leave the libraries table empty.

use presenter_persistence::Repository;
use presenter_server::AppState;

#[tokio::test]
async fn fresh_appstate_against_empty_db_leaves_libraries_empty() {
    let tmp = tempfile::NamedTempFile::new().expect("temp file");
    let url = format!("sqlite://{}?mode=rwc", tmp.path().display());
    let repository = Repository::connect(&url).await.expect("connect");

    let state = AppState::new(repository, None, None)
        .await
        .expect("AppState::new");

    let libraries = state.libraries().await.expect("libraries");
    assert!(
        libraries.is_empty(),
        "fresh AppState must not auto-seed libraries (found {})",
        libraries.len()
    );
}
```

**Note on `AppState::new` signature**: the actual signature has more parameters than the simplified call above. Before writing the test, run:

```bash
grep -n "pub async fn new\|pub(crate) async fn new\|impl AppState" crates/presenter-server/src/state/mod.rs | head -20
```

and inspect the real signature at line ~250-360 of `state/mod.rs`. Adapt the test's `AppState::new(...)` call to the real signature, passing `None` / sensible defaults / `Default::default()` for every parameter that does not affect library seeding. The point of the test is the assertion at the end — the constructor invocation is just plumbing.

If `AppState` is not `pub` outside the crate, place this test as an integration-style test inside `crates/presenter-server/src/state/tests.rs` instead (in addition to the unit test from Task 2), constructing the state via `AppState::new` with a temp-file DB. Either location is acceptable — the goal is exercising the production constructor path.

- [ ] **Step 3: Add `tempfile` to dev-dependencies if missing**

```bash
grep -A 20 "\[dev-dependencies\]" crates/presenter-server/Cargo.toml | head -25
```

If `tempfile` is not present, add it:

```toml
[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 4: Run the new integration test**

```bash
cargo test -p presenter-server --test empty_db_startup 2>&1 | tail -20
```

Expected: PASS. If the test fails because it found a non-empty libraries table, the auto-import removal is incomplete — return to Task 2.

- [ ] **Step 5: Run the full presenter-server test suite to confirm no regressions**

```bash
cargo test -p presenter-server 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/presenter-server/tests/empty_db_startup.rs crates/presenter-server/Cargo.toml Cargo.lock
git commit -m "test(server): add integration test guarding empty-DB startup (#228)"
```

---

## Task 4: Update CLAUDE.md and Docs (Haiku)

**Files:**
- Modify: `CLAUDE.md` (line 266 and surrounding context)

- [ ] **Step 1: Update the Deploy Safety section in CLAUDE.md**

In `/home/newlevel/devel/presenter/presenter-dev2/CLAUDE.md`, find the `### Deploy Safety` section (around line 262). Replace this block:

```markdown
### Deploy Safety

- Deploys NEVER delete the database — only binaries and service files are updated
- Database is backed up automatically before each deploy (5 retained in `backups/`)
- First-time deploys auto-import; subsequent deploys preserve all user data
- Data import is a separate, explicit "Import Data" workflow in GitHub Actions
```

with:

```markdown
### Deploy Safety

- Deploys NEVER delete the database — only binaries and service files are updated
- Database is backed up automatically before each deploy (5 retained in `backups/`)
- Imports happen only via the explicit Import Data workflow. Deploys never touch the database.
- New server installations start with an empty libraries table. Run the Import Data workflow once after first deploy to populate it.
```

- [ ] **Step 2: Verify no other auto-import references in docs**

Run:

```bash
grep -rn -i "auto-import\|first-time deploy\|ensure_seed_library" docs/ CLAUDE.md crates/*/README.md 2>/dev/null
```

Expected matches: only the spec at `docs/superpowers/specs/2026-05-02-explicit-only-imports-design.md` (which describes the historical context) and the plan at `docs/superpowers/plans/2026-05-02-explicit-only-imports.md` (this file). NO matches in CLAUDE.md, README files, or other live docs.

If any other live doc references auto-import or first-time auto-import, update its wording to match the new policy: "Imports happen only via the explicit Import Data workflow."

- [ ] **Step 3: Commit**

```bash
git add CLAUDE.md
git commit -m "docs: update CLAUDE.md to reflect explicit-only imports (#228)"
```

---

## Task 5: Local Checks, Push, CI Monitor, Dev Verification, Open PR (Controller)

This task is handled by the controller (the agent driving the plan), not by an implementer subagent. All steps run locally on this dev2 machine where Rust builds are allowed (per CLAUDE.md "Local Build Policy").

### Local pre-push checks

- [ ] **Step 1: Format check**

```bash
cargo fmt --all --check
```

Expected: zero output (all formatted).

- [ ] **Step 2: Clippy**

```bash
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all 2>&1 | tail -30
```

Expected: zero warnings.

- [ ] **Step 3: Full test suite**

```bash
cargo test -p presenter-server 2>&1 | tail -25
```

Expected: all green, including `empty_state_does_not_auto_seed_library` and `fresh_appstate_against_empty_db_leaves_libraries_empty`.

- [ ] **Step 4: Push**

```bash
git push origin dev
```

- [ ] **Step 5: Monitor CI to terminal state**

Per `core/ci-monitoring.md`: ONE background `sleep + gh run view` per cycle, no polling loops, no `gh run watch`.

```bash
RUN_ID=$(gh run list --branch dev --limit 1 --json databaseId --jq '.[0].databaseId')
echo "Monitoring run $RUN_ID"
# Background single sleep + status read
```

If any job fails: `gh run view $RUN_ID --log-failed`, fix root cause in ONE commit, push, monitor again.

### Dev verification (after CI deploys)

- [ ] **Step 6: Verify dev shows v0.4.51**

Open `http://10.77.8.134:8080/healthz` and confirm `{"version":"0.4.51", "channel":"dev"}`.

- [ ] **Step 7: Manual empty-DB verification on dev**

Per spec section "Manual verification on dev":

```bash
# Backup current dev DB
sudo systemctl stop presenter-dev
sudo cp /opt/presenter-dev/presenter.db /tmp/presenter-dev.db.backup.$(date +%s)

# Truncate libraries (DEV ONLY — never run on prod)
sudo sqlite3 /opt/presenter-dev/presenter.db 'DELETE FROM libraries;'

# Restart service
sudo systemctl start presenter-dev

# Wait for startup, then assert libraries endpoint returns empty
sleep 3
curl -s http://10.77.8.134:8080/libraries
# Expected: [] (empty array)

# Restore backup
sudo systemctl stop presenter-dev
sudo cp /tmp/presenter-dev.db.backup.* /opt/presenter-dev/presenter.db
sudo systemctl start presenter-dev

# Confirm libraries are back
sleep 3
curl -s http://10.77.8.134:8080/libraries | head -200
```

If the `curl` after truncation returns ANY library, the fix is incomplete — investigate immediately.

### Open PR

- [ ] **Step 8: Open PR from dev to main**

```bash
gh pr create --base main --head dev --title "fix(server): explicit-only imports — stop auto-import on deploy (#228)" --body "$(cat <<'EOF'
## Summary

Removes the auto-import-on-startup logic from `presenter-server` so deploys never touch the database. Closes #228 (`ensure_seed_library` race defeats deploy import detection).

After this change, the **only** way to (re)populate libraries is the manual Import Data GitHub Actions workflow. Operators' hand-curated playlists, presentation renames, and other custom edits stay safe across every deploy.

## What changed

- Removed `ensure_seed_library` and both call sites (`AppState::new` and `AppState::in_memory`).
- Gated the canonical `sample_library` fixture and added a test-only `seed_sample_library` helper for tests that need a populated state.
- Added `empty_state_does_not_auto_seed_library` (unit) and `fresh_appstate_against_empty_db_leaves_libraries_empty` (integration) tests as regression guards.
- Updated CLAUDE.md Deploy Safety section to reflect the new policy.

## Production data treatment

**Untouched.** The user's existing production data stays exactly as-is. The next deploy after merge is guaranteed not to call any import code.

## Test plan

- [x] `cargo test -p presenter-server` — all green, both new regression tests pass
- [x] `cargo clippy --workspace --all-targets -- -D warnings -W clippy::all` — zero warnings
- [x] Manual dev verification: truncate libraries → restart → confirm `/libraries` returns `[]` → restore backup
- [x] CI green on push to dev

Closes #228
EOF
)"
```

- [ ] **Step 9: Confirm PR is mergeable + clean**

```bash
PR_NUM=$(gh pr list --head dev --base main --json number --jq '.[0].number')
gh api repos/zbynekdrlik/presenter/pulls/$PR_NUM --jq '{mergeable: .mergeable, mergeable_state: .mergeable_state}'
```

Expected: `{"mergeable": true, "mergeable_state": "clean"}`.

If `mergeable_state` is anything other than `"clean"` (BEHIND, DIRTY, BLOCKED, UNSTABLE), investigate and fix per `pr-merge-policy.md` and `autonomous-quality-discipline.md`. Never bypass branch protection.

### Pre-completion gate

- [ ] **Step 10: Run `/plan-check`**

Audit every requirement in this plan and the original spec. Every item must be `[x]`.

- [ ] **Step 11: Run `/review`** on the diff

Address every 🔴, 🟡, AND 🔵 finding inside the PR's diff. Re-run until both audits return `0 🔴 0 🟡 0 🔵`.

- [ ] **Step 12: Send completion report**

Per `core/completion-report.md`. Include:

- `✅ CI: green` (with run id)
- `✅ /plan-check: N/N fulfilled`
- `✅ /review: clean — 0 🔴 0 🟡 0 🔵`
- `✅ Deploy: dev shows v0.4.51 at /healthz; manual /libraries empty-DB verification passed`
- `🌐 Dev:  http://10.77.8.134:8080/ui/operator`
- `🌐 Prod: http://10.77.9.205/ui/operator`
- `[presenter] PR #N: <full title>` + URL
- Wait for explicit "merge it" before merging.

---

## Verification Summary

| Check | How to verify |
|-------|---------------|
| Auto-import code removed | `grep -rn ensure_seed_library crates/` returns nothing |
| Production startup leaves DB alone | `fresh_appstate_against_empty_db_leaves_libraries_empty` integration test passes |
| Test helper still works for tests that need seed | `cargo test -p presenter-server` all green |
| CLAUDE.md updated | `grep "First-time deploys auto-import" CLAUDE.md` returns nothing |
| Dev manual verification | After truncating dev libraries + restart, `curl /libraries` returns `[]` |
| Production data untouched | The merge → deploy never calls any import code path; existing playlists and renames preserved |
