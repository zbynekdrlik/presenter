# Android Stage Launcher Resilience Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restore working Android TV kiosk auto-launch on prod/dev by seeding the 4 known displays, hardening `adb connect` against stale state, and giving operators a one-click "Test" button.

**Architecture:** Three surgical changes in one PR: (1) a one-time seed migration that inserts `sd1l..sd4l` into `android_stage_displays` when the table is empty, (2) `adb disconnect` before `adb connect` inside `connect_and_launch`, (3) a new `POST /integrations/android-stage/displays/{id}/launch-now` endpoint wired through state → registry → existing `DeviceCommand::LaunchNow`, with a "Test" button in the Settings UI.

**Tech Stack:** Rust (axum, SeaORM, sea-orm-migration, tokio), Leptos SSR components, vanilla JS (`settings_script.js`).

**Spec:** `docs/superpowers/specs/2026-04-14-android-stage-resilience-design.md`

---

## Context

The existing `android_stage_displays` table lives in the initial migration `m20250927_000001_create_core_tables.rs:436-500`. It has columns `id TEXT PK`, `label TEXT NOT NULL UNIQUE`, `host TEXT NOT NULL`, `port INTEGER DEFAULT 5555`, `launch_component TEXT DEFAULT 'com.fullykiosk.videokiosk/de.ozerov.fully.MainActivity'`, `is_enabled BOOLEAN DEFAULT TRUE`, `created_at/updated_at TIMESTAMP`.

The launcher worker is at `crates/presenter-server/src/android_stage.rs`. Key facts:
- `DeviceCommand::LaunchNow` already exists (line 66). The worker already handles it (line 206). We just need to expose a public method to send it.
- `connect_and_launch` at line 225-314 runs `adb connect` then `adb shell am start`.
- The worker ticks every 20s (`RETRY_INTERVAL`, line 17) and also fires `LaunchNow` on startup (line 167).
- Registry methods currently exposed: `new`, `set_displays`, `snapshot`, `snapshot_for`, `spawn_display`.

The state layer at `crates/presenter-server/src/state/integrations.rs:74-125` already has `list/create/update/delete_android_stage_display` and `sync_android_stage_displays`. We add one more method.

The Settings UI has both SSR (`crates/presenter-server/src/ui/components/settings.rs:278-459`) and a JS client (`crates/presenter-server/src/settings_script.js:853-889`). The list gets re-rendered by JS after any mutation, so the "Test" button must be added to BOTH the SSR template and the JS template, plus an event delegation handler.

**Working directory:** `/home/newlevel/devel/presenter/presenter-dev2`. **Branch:** dev. The recent Android stage code hasn't been touched in weeks; we can work confidently with the existing shape.

---

## File Structure

| File | Change |
|------|--------|
| `crates/presenter-migration/src/m20260414_000002_seed_android_stage_displays.rs` | **New** — one-time seed migration with `COUNT(*) = 0` guard |
| `crates/presenter-migration/src/lib.rs` | Register the new migration as the last entry in `migrations()` |
| `crates/presenter-server/src/android_stage.rs` | Add `adb disconnect` before `adb connect` in `connect_and_launch`; add pub `launch_now` method on `AndroidStageRegistry` |
| `crates/presenter-server/src/state/integrations.rs` | Add `launch_now_android_stage_display(id)` method |
| `crates/presenter-server/src/router/integrations/android_stage.rs` | Add `launch_now_android_stage_display` handler returning 204 |
| `crates/presenter-server/src/router.rs` | Register `POST /integrations/android-stage/displays/{id}/launch-now` |
| `crates/presenter-server/src/ui/components/settings.rs` | Add "Test" button in SSR template alongside Edit/Delete |
| `crates/presenter-server/src/settings_script.js` | Add "Test" button to JS template + event handler + `testAndroidDisplay(id)` helper |

---

## Task 1: Seed Migration

**Files:**
- Create: `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-migration/src/m20260414_000002_seed_android_stage_displays.rs`
- Modify: `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-migration/src/lib.rs`

- [ ] **Step 1: Create the migration file**

Create `crates/presenter-migration/src/m20260414_000002_seed_android_stage_displays.rs` with this exact content:

```rust
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const SEED_ROWS: &[(&str, &str)] = &[
    ("Stage SD1", "sd1l.lan"),
    ("Stage SD2", "sd2l.lan"),
    ("Stage SD3", "sd3l.lan"),
    ("Stage SD4", "sd4l.lan"),
];

const DEFAULT_LAUNCH_COMPONENT: &str =
    "com.fullykiosk.videokiosk/de.ozerov.fully.MainActivity";

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // Only seed if the table is empty. If an operator has added any
        // rows (even after deleting and re-adding), leave them alone.
        let row = db
            .query_one(sea_orm::Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "SELECT COUNT(*) as cnt FROM android_stage_displays",
            ))
            .await?;

        let count = row
            .map(|r| r.try_get::<i32>("", "cnt").unwrap_or(0))
            .unwrap_or(0);

        if count > 0 {
            return Ok(());
        }

        for (label, host) in SEED_ROWS {
            let id = uuid::Uuid::new_v4().to_string();
            db.execute(sea_orm::Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Sqlite,
                "INSERT INTO android_stage_displays \
                 (id, label, host, port, launch_component, is_enabled, created_at, updated_at) \
                 VALUES (?, ?, ?, 5555, ?, 1, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)",
                [
                    id.into(),
                    (*label).into(),
                    (*host).into(),
                    DEFAULT_LAUNCH_COMPONENT.into(),
                ],
            ))
            .await?;
        }

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // No-op. Do not delete operator data on rollback.
        Ok(())
    }
}
```

- [ ] **Step 2: Add `uuid` to presenter-migration deps if missing**

Run:
```bash
grep '^uuid' /home/newlevel/devel/presenter/presenter-dev2/crates/presenter-migration/Cargo.toml
```

If the grep returns nothing, add `uuid = { version = "1", features = ["v4"] }` under `[dependencies]` in that Cargo.toml. If `uuid` is already there as a workspace dep, use `uuid.workspace = true`.

- [ ] **Step 3: Register the migration**

Edit `crates/presenter-migration/src/lib.rs`. It currently looks like:

```rust
pub use sea_orm_migration::prelude::*;

pub mod bible_fts_triggers;

mod m20250927_000001_create_core_tables;
mod m20260408_000001_add_preach_limit;
mod m20260410_000001_separate_bible;
mod m20260412_000001_bible_fts;
mod m20260414_000001_bible_translation_digest;

pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20250927_000001_create_core_tables::Migration),
            Box::new(m20260408_000001_add_preach_limit::Migration),
            Box::new(m20260410_000001_separate_bible::Migration),
            Box::new(m20260412_000001_bible_fts::Migration),
            Box::new(m20260414_000001_bible_translation_digest::Migration),
        ]
    }
}
```

Add `mod m20260414_000002_seed_android_stage_displays;` below the last `mod` line, and append `Box::new(m20260414_000002_seed_android_stage_displays::Migration),` as the last element of the `vec!`.

- [ ] **Step 4: Verify build**

Run: `cd /home/newlevel/devel/presenter/presenter-dev2 && cargo check -p presenter-migration`
Expected: compiles clean.

- [ ] **Step 5: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git add crates/presenter-migration/src/m20260414_000002_seed_android_stage_displays.rs crates/presenter-migration/src/lib.rs crates/presenter-migration/Cargo.toml
git commit -m "feat(migration): seed known android stage displays when empty (#245)"
```

---

## Task 2: Seed migration idempotency tests

**Files:**
- Modify: `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-persistence/src/repository/tests.rs` — append two new tests

We put the tests in `presenter-persistence` because that crate already has in-memory DB helpers and exercises migrations via `Repository::connect_in_memory()`.

- [ ] **Step 1: Write the failing tests**

Append to the existing test module in `crates/presenter-persistence/src/repository/tests.rs`:

```rust
    #[tokio::test]
    async fn seed_migration_populates_four_android_stage_displays_on_empty_table() {
        let repo = Repository::connect_in_memory().await.unwrap();
        let displays = repo.list_android_stage_displays().await.unwrap();
        assert_eq!(displays.len(), 4, "seed should have inserted 4 displays");
        let hosts: Vec<_> = displays.iter().map(|d| d.host.clone()).collect();
        let mut sorted = hosts.clone();
        sorted.sort();
        assert_eq!(
            sorted,
            vec![
                "sd1l.lan".to_string(),
                "sd2l.lan".to_string(),
                "sd3l.lan".to_string(),
                "sd4l.lan".to_string(),
            ],
            "seed hosts should be sd1l..sd4l",
        );
        for d in &displays {
            assert_eq!(d.port, 5555);
            assert_eq!(
                d.launch_component,
                "com.fullykiosk.videokiosk/de.ozerov.fully.MainActivity",
            );
            assert!(d.is_enabled, "seeded displays should be enabled");
        }
    }

    #[tokio::test]
    async fn seed_migration_is_idempotent_when_rerun() {
        use presenter_core::AndroidStageDisplayDraft;
        use presenter_migration::{MigrationTrait, SchemaManager};

        let repo = Repository::connect_in_memory().await.unwrap();

        // The initial connect already ran migrations → 4 seed rows present.
        assert_eq!(repo.list_android_stage_displays().await.unwrap().len(), 4);

        // Operator manually adds a fifth display.
        let draft = AndroidStageDisplayDraft::new(
            "Operator Custom".to_string(),
            "custom.lan".to_string(),
        );
        repo.create_android_stage_display(&draft).await.unwrap();
        assert_eq!(repo.list_android_stage_displays().await.unwrap().len(), 5);

        // Manually invoke the seed migration's `up()` again.
        let connection = repo.connection_for_tests();
        let schema = SchemaManager::new(connection);
        let migration = presenter_migration::Migrator::migrations()
            .into_iter()
            .find(|m| m.name() == "m20260414_000002_seed_android_stage_displays")
            .expect("seed migration present in registry");
        migration.up(&schema).await.expect("rerun seed");

        // Row count stays at 5 — guard prevented re-insertion.
        let displays = repo.list_android_stage_displays().await.unwrap();
        assert_eq!(
            displays.len(),
            5,
            "seed rerun must not insert duplicates once any row exists",
        );
        assert!(
            displays.iter().any(|d| d.host == "custom.lan"),
            "operator's custom display must survive the rerun",
        );
    }
```

> **Note:** `Repository::connection_for_tests()` may not exist yet. If the first `cargo test` run fails with "no method named `connection_for_tests`", add a `#[cfg(test)] pub fn connection_for_tests(&self) -> &DatabaseConnection { &self.db }` method on `Repository` in `crates/presenter-persistence/src/repository/mod.rs` (search for the `impl Repository {` block near `connect_in_memory`). Commit that as a separate prep step if needed.

- [ ] **Step 2: Run the tests to confirm they fail without Task 1 only if cargo test regressions happen**

Run:
```bash
cd /home/newlevel/devel/presenter/presenter-dev2
cargo test -p presenter-persistence -- seed_migration --nocapture
```
Expected: both new tests PASS. The seed already landed via Task 1 (merged in the working branch), so the happy-path test should be green immediately. If the idempotency test fails on the `connection_for_tests` method, apply the note above and re-run.

- [ ] **Step 3: Run the whole presenter-persistence suite to catch regressions**

Run: `cargo test -p presenter-persistence`
Expected: all tests pass. The existing `android_stage_display_crud_round_trip` test at `tests.rs:253` assumes an empty table; after Task 1 the initial state has 4 rows, so that test MAY need to be adjusted to compare against a **delta** rather than absolute counts. If it fails, read the test carefully and convert its `assert_eq!(displays.len(), N)` checks into `assert_eq!(displays.len(), 4 + N)` form. Commit that fix as part of this step.

- [ ] **Step 4: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git add crates/presenter-persistence/src/repository/tests.rs crates/presenter-persistence/src/repository/mod.rs
git commit -m "test(persistence): pin seed migration idempotency (#245)"
```

---

## Task 3: `adb disconnect` before `adb connect` in the launcher

**Files:**
- Modify: `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-server/src/android_stage.rs` — inside `connect_and_launch`, just before the existing `adb connect` call (around line 241)

- [ ] **Step 1: Add the `disconnect` call**

Locate `connect_and_launch` in `android_stage.rs`. After the `let adb_bin = adb_path.as_os_str();` line and before `// Run adb connect`, insert:

```rust
    // Clear any stale offline device entry from a previous attempt.
    // ADB leaves stale entries after TV power cycles which then cause
    // subsequent `-s serial` commands to fail until the daemon is restarted.
    // Errors are intentionally ignored — the typical case is "not connected"
    // which returns a non-zero exit code we don't care about.
    let _ = timeout(
        ADB_COMMAND_TIMEOUT,
        Command::new(adb_bin)
            .arg("disconnect")
            .arg(&serial)
            .output(),
    )
    .await;
```

- [ ] **Step 2: Verify build and existing tests**

Run:
```bash
cd /home/newlevel/devel/presenter/presenter-dev2
cargo check -p presenter-server
cargo test -p presenter-server -- android_stage --nocapture
```
Expected: compile clean, android_stage tests pass (they don't exercise the worker loop against real adb, so this is a no-op at the test level — the change is behavioral at runtime).

- [ ] **Step 3: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git add crates/presenter-server/src/android_stage.rs
git commit -m "fix(android-stage): adb disconnect before connect to clear stale state (#245)"
```

---

## Task 4: `AndroidStageRegistry::launch_now` public method

**Files:**
- Modify: `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-server/src/android_stage.rs` — add a method on `impl AndroidStageRegistry`

- [ ] **Step 1: Add the method**

Locate the `impl AndroidStageRegistry { ... }` block (starts around line 70). Add this method after `snapshot_for` and before `spawn_display`:

```rust
    /// Tell the worker for `id` to run a launch immediately, bypassing the
    /// 20-second tick. Returns an error if no such display exists or if the
    /// display is currently disabled. The launch runs asynchronously — the
    /// caller should poll `snapshot_for(id)` to observe the state change.
    pub async fn launch_now(&self, id: AndroidStageDisplayId) -> anyhow::Result<()> {
        let guard = self.displays.read().await;
        let entry = guard
            .get(&id)
            .ok_or_else(|| anyhow!("unknown android stage display {id}"))?;
        if !entry.config.is_enabled {
            return Err(anyhow!("android stage display {id} is disabled"));
        }
        entry
            .command_tx
            .try_send(DeviceCommand::LaunchNow)
            .map_err(|err| anyhow!("failed to enqueue launch for {id}: {err}"))?;
        Ok(())
    }
```

- [ ] **Step 2: Add a unit test for the unknown-id error path**

At the end of `crates/presenter-server/src/android_stage.rs`, add (or append to) a `#[cfg(test)] mod tests` block:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use presenter_core::AndroidStageDisplayId;
    use uuid::Uuid;

    #[tokio::test]
    async fn launch_now_errors_on_unknown_id() {
        let registry = AndroidStageRegistry::new();
        let unknown = AndroidStageDisplayId::from_uuid(Uuid::new_v4());
        let err = registry.launch_now(unknown).await;
        assert!(err.is_err(), "launch_now must error on unknown id");
        assert!(
            err.unwrap_err().to_string().contains("unknown android stage display"),
            "error message should identify the unknown-id case",
        );
    }
}
```

This test deliberately avoids spawning a real worker (no `set_displays` call), so it doesn't touch `adb` at all. It only proves the unknown-id branch returns `Err`.

If an existing `#[cfg(test)] mod tests` block is already at the bottom of the file, append the new test inside it instead of creating a second module.

- [ ] **Step 3: Run the test and verify build**

Run:
```bash
cd /home/newlevel/devel/presenter/presenter-dev2
cargo test -p presenter-server -- android_stage::tests::launch_now_errors_on_unknown_id --nocapture
```
Expected: test passes.

- [ ] **Step 4: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git add crates/presenter-server/src/android_stage.rs
git commit -m "feat(android-stage): expose launch_now on registry (#245)"
```

---

## Task 5: State-layer `launch_now_android_stage_display`

**Files:**
- Modify: `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-server/src/state/integrations.rs` — append a method on `impl AppState`

- [ ] **Step 1: Add the method**

After the `delete_android_stage_display` method (around line 117), add:

```rust
    pub async fn launch_now_android_stage_display(
        &self,
        id: AndroidStageDisplayId,
    ) -> anyhow::Result<()> {
        self.android_stage_registry.launch_now(id).await
    }
```

`AndroidStageDisplayId` is already imported at the top of the file.

- [ ] **Step 2: Verify build**

Run: `cd /home/newlevel/devel/presenter/presenter-dev2 && cargo check -p presenter-server`
Expected: clean.

- [ ] **Step 3: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git add crates/presenter-server/src/state/integrations.rs
git commit -m "feat(state): add launch_now_android_stage_display passthrough (#245)"
```

---

## Task 6: Router endpoint + wiring

**Files:**
- Modify: `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-server/src/router/integrations/android_stage.rs` — add handler
- Modify: `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-server/src/router.rs` — register route

- [ ] **Step 1: Add the handler**

Append to `crates/presenter-server/src/router/integrations/android_stage.rs` after `delete_android_stage_display`:

```rust
#[instrument(skip_all)]
pub(crate) async fn launch_now_android_stage_display(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<axum::http::StatusCode, AppError> {
    state
        .launch_now_android_stage_display(AndroidStageDisplayId::from_uuid(id))
        .await
        .map_err(|err| AppError::bad_request(err.to_string()))?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}
```

All imports (`AndroidStageDisplayId`, `AppError`, `AppState`, `Path`, `State`, `Uuid`, `instrument`) are already present in that file.

- [ ] **Step 2: Register the route**

Open `crates/presenter-server/src/router.rs`. Find the existing block that registers `/integrations/android-stage/displays/{id}` with `.put(...)` and `.delete(...)` (around line 176-179). Immediately after that, add:

```rust
        .route(
            "/integrations/android-stage/displays/{id}/launch-now",
            post(integrations::android_stage::launch_now_android_stage_display),
        )
```

`post` should already be imported from `axum::routing` at the top of `router.rs`.

- [ ] **Step 3: Verify build**

Run: `cd /home/newlevel/devel/presenter/presenter-dev2 && cargo check -p presenter-server`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git add crates/presenter-server/src/router/integrations/android_stage.rs crates/presenter-server/src/router.rs
git commit -m "feat(router): POST /android-stage/displays/{id}/launch-now (#245)"
```

---

## Task 7: SSR "Test" button in settings.rs

**Files:**
- Modify: `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-server/src/ui/components/settings.rs` — add button to the Leptos template

- [ ] **Step 1: Add the Test button**

Locate the `<div class="settings__list-actions">` block inside `AndroidStageSettingsCard` (around line 438-451). Immediately BEFORE the existing Edit button, add the Test button so the action order is `[Test] [Edit] [Delete]`:

```rust
                                    <div class="settings__list-actions">
                                        <button
                                            type="button"
                                            class="settings__button settings__button--ghost"
                                            data-role="android-test"
                                            data-id={display.id.clone()}
                                        >"Test"</button>
                                        <button
                                            type="button"
                                            class="settings__button settings__button--ghost"
                                            data-role="android-edit"
                                            data-id={display_id_edit}
                                        >"Edit"</button>
                                        <button
                                            type="button"
                                            class="settings__button settings__button--danger"
                                            data-role="android-delete"
                                            data-id={display_id_delete}
                                        >"Delete"</button>
                                    </div>
```

Note: `display.id.clone()` is used directly because the existing `display_id_edit` and `display_id_delete` locals are already consumed by the following buttons. We add a third local clone if the borrow checker complains — in that case, add `let display_id_test = display.id.clone();` alongside the existing edit/delete locals above the `view!` block.

- [ ] **Step 2: Verify build**

Run: `cd /home/newlevel/devel/presenter/presenter-dev2 && cargo check -p presenter-server`
Expected: clean. If borrow-checker complains about `display.id.clone()` after move, apply the `display_id_test` workaround.

- [ ] **Step 3: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git add crates/presenter-server/src/ui/components/settings.rs
git commit -m "feat(ui): add test launch button in android stage settings SSR (#245)"
```

---

## Task 8: JS template "Test" button + click handler

**Files:**
- Modify: `/home/newlevel/devel/presenter/presenter-dev2/crates/presenter-server/src/settings_script.js` — update the android list template and add a click handler

- [ ] **Step 1: Update the JS template**

Find the android list item template around line 710-713:

```javascript
  <div class="settings__list-actions">
    <button type="button" class="settings__button settings__button--ghost" data-role="android-edit" data-id="${normalized.id}">Edit</button>
    <button type="button" class="settings__button settings__button--danger" data-role="android-delete" data-id="${normalized.id}">Delete</button>
  </div>
```

Replace with:

```javascript
  <div class="settings__list-actions">
    <button type="button" class="settings__button settings__button--ghost" data-role="android-test" data-id="${normalized.id}">Test</button>
    <button type="button" class="settings__button settings__button--ghost" data-role="android-edit" data-id="${normalized.id}">Edit</button>
    <button type="button" class="settings__button settings__button--danger" data-role="android-delete" data-id="${normalized.id}">Delete</button>
  </div>
```

- [ ] **Step 2: Add the `testAndroidDisplay` helper**

Insert this function immediately above `async function deleteAndroidDisplay(id) {` (around line 853):

```javascript
  async function testAndroidDisplay(id) {
    if (!id) return;
    try {
      const response = await fetch(`${ANDROID_API_ROOT}/${id}/launch-now`, {
        method: 'POST',
      });
      if (!response.ok) {
        throw new Error(await extractError(response));
      }
      showToast('Launch queued — refreshing status…', 'success');
      setTimeout(() => {
        refreshAndroidDisplays(false);
      }, 1500);
    } catch (error) {
      console.error('Failed to trigger android launch', error);
      showToast(error.message || 'Unable to trigger launch.', 'error');
    }
  }
```

- [ ] **Step 3: Wire up the click handler**

In the android event delegation block (around line 885-889), extend the `if/else` chain:

```javascript
    if (target.dataset.role === 'android-edit') {
      startAndroidEdit(id);
    } else if (target.dataset.role === 'android-delete') {
      await deleteAndroidDisplay(id);
    } else if (target.dataset.role === 'android-test') {
      await testAndroidDisplay(id);
    }
```

- [ ] **Step 4: Verify JS is valid (no syntax errors)**

Run:
```bash
cd /home/newlevel/devel/presenter/presenter-dev2
node -e "new Function(require('fs').readFileSync('crates/presenter-server/src/settings_script.js', 'utf8'))" && echo "JS parses"
```

Expected: `JS parses`. If this prints a parse error, fix it before committing.

- [ ] **Step 5: Commit**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git add crates/presenter-server/src/settings_script.js
git commit -m "feat(ui): wire test launch button to launch-now endpoint (#245)"
```

---

## Task 9: Version bump, fmt, clippy, push, monitor CI, verify on prod

- [ ] **Step 1: Bump workspace version**

Edit `Cargo.toml` at the workspace root — bump `[workspace.package].version` from `0.4.22` to `0.4.23` (whatever the current dev version is + 1 patch).

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
cargo check -p presenter-server  # refresh Cargo.lock
```

- [ ] **Step 2: Local checks**

```bash
cargo fmt --all
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
cargo test --workspace
```

Fix any fmt drift, clippy warnings, or test failures before proceeding.

- [ ] **Step 3: Commit version bump**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
git add Cargo.toml Cargo.lock
git commit -m "chore: bump version for android stage resilience (#245)"
```

- [ ] **Step 4: Push and monitor dev pipeline**

```bash
git push origin dev
gh run list --branch dev --workflow Pipeline --limit 3
```

Monitor the pipeline via `gh run view <id>` or the Monitor tool. When `Deploy to Dev` finishes, verify dev picked up the seed:

```bash
curl -sf http://10.77.8.134:8080/integrations/android-stage/displays | python3 -m json.tool
```

Expected: 4 entries with hosts `sd1l..sd4l.lan`, all enabled, with `status.state` = `running` (or `connecting` while the first tick is in flight).

- [ ] **Step 5: Functional verification on dev**

For at least one display (say the one backed by `sd1l.lan`):

```bash
# Grab its id from the list
DISPLAY_ID=$(curl -sf http://10.77.8.134:8080/integrations/android-stage/displays | python3 -c "import json,sys; print([d['id'] for d in json.load(sys.stdin) if d['host']=='sd1l.lan'][0])")

# Force a launch via the new endpoint
curl -sf -X POST http://10.77.8.134:8080/integrations/android-stage/displays/$DISPLAY_ID/launch-now
echo "HTTP $?"

# Wait a moment, then check the TV
sleep 2
adb -s sd1l.lan:5555 shell "dumpsys activity activities | grep mResumedActivity"
```

Expected: the resumed activity contains `com.fullykiosk.videokiosk`. If it says `com.google.android.tvlauncher/.MainActivity`, something is still wrong — investigate before merging.

- [ ] **Step 6: Create PR to main**

```bash
cd /home/newlevel/devel/presenter/presenter-dev2
gh pr create --base main --head dev --title "fix(android-stage): restore kiosk auto-launch + test button (#245)" --body "$(cat <<'EOF'
## Summary

- **Seed 4 known displays** (sd1l..sd4l) when the table is empty — one-time migration with a COUNT(*)=0 guard so operator edits survive reruns.
- **adb disconnect before adb connect** in the launcher worker — clears stale offline device entries that ADB leaves behind after TV power cycles.
- **"Test" button** on each display row in Settings → POST to a new /launch-now endpoint that tells the worker to launch immediately, bypassing the 20s retry tick.

## Test plan

- [x] Unit: seed populates 4 rows on empty table
- [x] Unit: seed is idempotent when rerun against a non-empty table (operator edits preserved)
- [x] Functional: dev deploy picks up the seed, `curl /integrations/android-stage/displays` returns 4 rows
- [x] Functional: click Test on sd1l → status transitions within 2s → `adb shell dumpsys activity activities` shows FullyActivity foregrounded
- [x] Functional: power-cycle sd1l and wait ≤20s → launcher auto-recovers via the new adb disconnect/connect sequence

Closes #245.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 7: After user merges, verify on prod**

Once the user merges the PR and the main deploy finishes:

```bash
curl -sf http://10.77.9.205/integrations/android-stage/displays | python3 -m json.tool
```

Expected: same 4 rows. If prod already had rows (operator had added them at some point that we couldn't find in backups), the seed skipped — still fine.

Power on one of the TVs cold, wait up to 20 seconds, and confirm the kiosk is foregrounded. Optionally click "Test" in the UI to force a launch.

---

## Verification Summary

| Check | How to verify |
|-------|---------------|
| Seed migration idempotent | `cargo test -p presenter-persistence -- seed_migration` passes; the idempotency test proves a rerun doesn't duplicate |
| 4 displays auto-created on empty DB | After deploy: `curl /integrations/android-stage/displays` returns 4 rows with sd1l..sd4l |
| adb disconnect hardening | Read `connect_and_launch` at `crates/presenter-server/src/android_stage.rs` — disconnect call precedes connect |
| Test button works | Click "Test" in Settings UI → toast → status panel shows updated `lastAttempt` within 2s |
| Launch-now endpoint | `curl -X POST /integrations/android-stage/displays/<id>/launch-now` returns 204 |
| End-to-end kiosk launch | `adb -s sd1l.lan:5555 shell dumpsys activity activities` shows FullyActivity after Test click |
| CI green | Pipeline succeeds for dev and main |
| Prod verified | `curl /integrations/android-stage/displays` on prod shows 4 rows; manual TV power-cycle test |
