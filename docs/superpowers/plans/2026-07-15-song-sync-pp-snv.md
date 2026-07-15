# Implementation Plan — Two-Way Song Sync Between Presenter Instances (PP ↔ SNV)

**Issue:** #555 (feat(sync): two-way song sync between PP and SNV instances — newest edit wins)
**Spec (contract):** `docs/superpowers/specs/2026-07-15-song-sync-pp-snv-design.md`
**Date:** 2026-07-15

> **For agentic workers: REQUIRED SUB-SKILL `superpowers:subagent-driven-development`.**
> Execute this plan task-by-task with that skill. Each task is independently
> committable and CI-safe (workspace compiles + tests pass at every commit).

## Goal

Songs (presentations) created, edited, renamed, or deleted on one Presenter
instance propagate automatically to the other over Tailscale, reconciled
last-write-wins, with deletes going to a restorable 30-day trash.

## Architecture

Each instance runs a symmetric periodic **pull loop** (30 s + a ~2 s debounced
nudge after every local song mutation) that reads its peer's `/sync/manifest`,
pulls any presentation whose peer `updatedAt` is strictly newer (or is unknown
locally), and upserts it by a new cross-instance `sync_id`, applying **adopt-by-name**
for transitional identity mismatches and storing the **peer's** timestamp so an
applied change is never re-broadcast (no echo). Identity + LWW live on the
`presentations` table via three new columns (`updated_at`, `sync_id`, `deleted_at`);
deletes are soft (the trash). The sync engine is a self-contained module
(`state/sync.rs`) that reuses the AbleSet-tracker background-task pattern; the wire
protocol is two read endpoints plus a status endpoint, all serving both directions.

## Tech stack

Rust workspace: `presenter-core` (domain), `presenter-persistence` (SeaORM/SQLite),
`presenter-migration` (idempotent incremental migrations), `presenter-importer`
(`.pro` protobuf), `presenter-server` (Axum + Leptos SSR + background tasks),
`presenter-ui` (Leptos/WASM). HTTP peer calls use `reqwest` (already a server dep,
`json` feature). Tests: `cargo test` (Rust) + Playwright (`tests/e2e`).

## Global constraints (apply to EVERY task)

- **Banned in prod code** (all crates except `presenter-ui` WASM): `unwrap()`,
  `expect()`, `panic!`, `std::thread::sleep`. Use `?` / `anyhow` / `ok_or_else` and
  `tokio::time::sleep`. Test modules (`#[cfg(test)]`) are exempt.
- **File/function caps (CI-enforced):** no prod file > 1000 lines, no function > 120
  lines. `state/mod.rs` is already 795 lines — add the ABSOLUTE MINIMUM there (one
  struct field, one init line, one spawn call); put every new `impl AppState` sync
  method in `state/sync.rs`. New tests go in their OWN files, never appended to a big
  existing `tests.rs`.
- **Cite functions by NAME, re-read before editing.** A concurrent PR (#552/#553) is
  shifting line numbers in `crates/presenter-server/src/state/slides/edit_ops.rs` and
  bumping the version. Never trust a line number from this plan; grep the function name
  and read it fresh.
- **Do not touch UI files another agent is editing** beyond the specific additions in
  Task 14; if a settings file conflicts, re-read and re-apply the minimal change.
- **TDD:** for each behavior change, write the failing test and run it (expect FAIL)
  BEFORE the implementation step in the same task, then make it GREEN. (Feature work —
  RED/GREEN commit ordering is not mandatory here, but keep test-first where natural.)
- **camelCase serde** on every wire DTO (`#[serde(rename_all = "camelCase")]`), matching
  the repo convention.
- **Local gate** (this is the powerful dev2 build box — local builds ARE allowed here):
  `cargo fmt --all` · `cargo clippy --workspace --all-targets -- -D warnings -W clippy::all`
  · `cargo test` · (UI) `cd crates/presenter-ui && cargo test --lib`.

---

## Task 1 — Version bump + workspace deps

**Files**
- Modify: `Cargo.toml` (workspace `[workspace.package].version`, `[workspace.dependencies].uuid`)

**Steps**
- [ ] `git fetch origin && git merge origin/main` (sync base first).
- [ ] Determine the next free version — do NOT hard-code. Run:
  ```bash
  grep -m1 '^version = ' Cargo.toml            # current dev version
  gh release list -L 1                          # latest published release
  ```
  Pick the next patch strictly greater than BOTH (the concurrent #552/#553 PR may have
  already bumped dev past `0.4.199`; if dev is e.g. `0.4.200`, use `0.4.201`). Set
  `[workspace.package].version` to that value.
- [ ] The UUIDv5 backfill/identity needs the `v5` uuid feature. Edit the workspace uuid
  dep line (currently `uuid = { version = "1.8", features = ["serde", "v4"] }`) to:
  ```toml
  uuid = { version = "1.8", features = ["serde", "v4", "v5"] }
  ```
  (`reqwest` is already a server dep with `json` — no change needed.)
- [ ] Run: `cargo build -p presenter-server` → expect success (compiles with new feature).
- [ ] Commit: `chore(sync): bump version and enable uuid v5 for cross-instance identity (#555)`

---

## Task 2 — Schema migration: `updated_at`, `sync_id`, `deleted_at`

Adds the three columns with idempotent guards (per DB policy), backfills `updated_at`
from `created_at`, backfills `sync_id` deterministically (UUIDv5 of `library_name/name`,
with a v4 fallback on the rare in-DB collision so the unique index always holds), and
creates the `sync_id` unique index.

**Files**
- Create: `crates/presenter-migration/src/m20260715_000001_add_presentation_sync_columns.rs`
- Modify: `crates/presenter-migration/src/lib.rs` (register the migration)
- Modify: `crates/presenter-migration/Cargo.toml` (add `presenter-core` dep for the shared id helper — see Task 6; if you do Task 6 first this is already present)
- Create (Task 6 also touches it): `crates/presenter-core/src/sync.rs` (shared `sync_id_for_name`)

**Steps**
- [ ] First add the shared deterministic id helper to `presenter-core` so migration,
  persistence, AND importer all compute the SAME id (drift here breaks convergence).
  Create `crates/presenter-core/src/sync.rs`:
  ```rust
  //! Cross-instance song identity (#555). One canonical implementation used by the
  //! importer (from the `.pro` UUID), the create path, the LWW apply path, and the
  //! backfill migration — so two instances derive identical `sync_id`s with zero
  //! coordination. NEVER change the namespace or the name format without a data
  //! migration: it would re-pair every existing song.
  use uuid::Uuid;

  /// Fixed project namespace for song sync ids. A stable random UUID — do not change.
  pub const SYNC_ID_NAMESPACE: Uuid = Uuid::from_u128(0x9f1c7a2e_5b34_4d8e_9a11_6c2f0b7d4e15);

  /// Deterministic `sync_id` for a song that has no `.pro` UUID: UUIDv5 over
  /// `"<library_name>/<name>"`. Both sites compute this identically for the same
  /// repertoire, so existing identical songs pair up automatically.
  pub fn sync_id_for_name(library_name: &str, name: &str) -> String {
      let key = format!("{library_name}/{name}");
      Uuid::new_v5(&SYNC_ID_NAMESPACE, key.as_bytes()).to_string()
  }
  ```
- [ ] Register the module in `crates/presenter-core/src/lib.rs`: add `pub mod sync;` and
  re-export: `pub use sync::{sync_id_for_name, SYNC_ID_NAMESPACE};` (follow the existing
  `pub mod` / `pub use` pattern in that file).
- [ ] Add `presenter-core.workspace = true` under `[dependencies]` in
  `crates/presenter-migration/Cargo.toml` (no cycle: core depends on neither migration nor
  persistence).
- [ ] Write the migration. Mirror `m20260629_000001_add_stage_active_entry_index.rs`
  (pragma guard) and `m20260506_000001_normalize_text_to_nfc.rs` (row-iteration backfill):
  ```rust
  use presenter_core::sync_id_for_name;
  use sea_orm::{ConnectionTrait, Statement};
  use sea_orm_migration::prelude::*;
  use std::collections::HashSet;

  #[derive(DeriveMigrationName)]
  pub struct Migration;

  async fn column_missing<C: ConnectionTrait>(db: &C, column: &str) -> Result<bool, DbErr> {
      let sql = format!(
          "SELECT COUNT(*) AS cnt FROM pragma_table_info('presentations') WHERE name='{column}'"
      );
      let row = db
          .query_one(Statement::from_string(sea_orm::DatabaseBackend::Sqlite, sql))
          .await?;
      Ok(row
          .map(|r| r.try_get::<i32>("", "cnt").unwrap_or(0) == 0)
          .unwrap_or(true))
  }

  #[async_trait::async_trait]
  impl MigrationTrait for Migration {
      async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
          let db = manager.get_connection();

          // 1. updated_at — nullable at DB level (SQLite can't ADD NOT NULL without a
          //    default), backfilled from created_at, always set by every insert going
          //    forward, so no NULL row ever exists (entity type is NOT NULL).
          if column_missing(db, "updated_at").await? {
              db.execute(Statement::from_string(
                  sea_orm::DatabaseBackend::Sqlite,
                  "ALTER TABLE presentations ADD COLUMN updated_at TEXT",
              ))
              .await?;
              db.execute(Statement::from_string(
                  sea_orm::DatabaseBackend::Sqlite,
                  "UPDATE presentations SET updated_at = created_at WHERE updated_at IS NULL",
              ))
              .await?;
          }

          // 2. deleted_at — genuinely nullable (the trash marker).
          if column_missing(db, "deleted_at").await? {
              db.execute(Statement::from_string(
                  sea_orm::DatabaseBackend::Sqlite,
                  "ALTER TABLE presentations ADD COLUMN deleted_at TEXT",
              ))
              .await?;
          }

          // 3. sync_id — cross-instance identity. Backfill deterministically; on the
          //    rare same-library+same-name in-DB collision, fall back to a fresh v4 so
          //    the unique index below always holds.
          if column_missing(db, "sync_id").await? {
              db.execute(Statement::from_string(
                  sea_orm::DatabaseBackend::Sqlite,
                  "ALTER TABLE presentations ADD COLUMN sync_id TEXT",
              ))
              .await?;

              let rows = db
                  .query_all(Statement::from_string(
                      sea_orm::DatabaseBackend::Sqlite,
                      "SELECT p.id AS id, COALESCE(l.name, '') AS lib_name, p.name AS name \
                       FROM presentations p LEFT JOIN libraries l ON l.id = p.library_id",
                  ))
                  .await?;
              let mut used: HashSet<String> = HashSet::new();
              for row in rows {
                  let id: String = row.try_get("", "id")?;
                  let lib_name: String = row.try_get("", "lib_name")?;
                  let name: String = row.try_get("", "name")?;
                  let mut sid = sync_id_for_name(&lib_name, &name);
                  if !used.insert(sid.clone()) {
                      sid = uuid::Uuid::new_v4().to_string();
                      used.insert(sid.clone());
                  }
                  db.execute(Statement::from_sql_and_values(
                      sea_orm::DatabaseBackend::Sqlite,
                      "UPDATE presentations SET sync_id = ? WHERE id = ? AND sync_id IS NULL",
                      [sid.into(), id.into()],
                  ))
                  .await?;
              }
          }

          // Unique index — enforces one row per identity. Idempotent (IF NOT EXISTS).
          db.execute(Statement::from_string(
              sea_orm::DatabaseBackend::Sqlite,
              "CREATE UNIQUE INDEX IF NOT EXISTS idx_presentations_sync_id \
               ON presentations(sync_id)",
          ))
          .await?;
          Ok(())
      }

      async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
          // Non-destructive additive migration; SQLite pre-3.35 has no DROP COLUMN. No-op.
          Ok(())
      }
  }
  ```
- [ ] Register in `crates/presenter-migration/src/lib.rs`: add `mod m20260715_000001_add_presentation_sync_columns;` with the others and
  `Box::new(m20260715_000001_add_presentation_sync_columns::Migration),` at the END of the
  `migrations()` vec.
- [ ] Write the migration test FIRST inside the migration file (`#[cfg(test)] mod tests`),
  modeled on the `normalize_text_to_nfc` test: create a `sqlite::memory:` DB with the
  pre-migration `presentations` + `libraries` schema (id, library_id, name, search_name,
  created_at), seed two rows (same lib+name to exercise the collision fallback, plus a
  distinct one), run `Migration.up` via `SchemaManager`, then assert:
  (a) `updated_at == created_at` for every row; (b) each row has a non-empty `sync_id`;
  (c) the distinct row's `sync_id == sync_id_for_name(lib, name)` (determinism);
  (d) the two colliding rows have DIFFERENT `sync_id`s; (e) re-running `up` is a no-op
  (idempotent). Use `Statement`-level SELECTs like the NFC test does.
- [ ] Run: `cargo test -p presenter-migration` → RED first (no migration), then GREEN.
- [ ] Commit: `feat(sync): add updated_at/sync_id/deleted_at migration with deterministic backfill (#555)`

---

## Task 3 — Entity fields + populate every insert site

Add the three columns to the SeaORM `presentation::Model`. This makes the two existing
`ActiveModel { … }` literals fail to compile until they set the new fields — that is the
compile-time proof every insert site is covered. App-created and imported songs both get a
fresh `Uuid::new_v4()` `sync_id` here; Task 6 refines the importer/upsert to use the `.pro`
UUID / deterministic id.

**Files**
- Modify: `crates/presenter-persistence/src/entities.rs` (`presentation` mod `Model`)
- Modify: `crates/presenter-persistence/src/repository/presentation.rs` (`create_presentation`)
- Modify: `crates/presenter-persistence/src/repository/library.rs` (`upsert_library`)
- Create: `crates/presenter-persistence/src/repository/sync_tests.rs` (new test file)
- Modify: `crates/presenter-persistence/src/repository/mod.rs` (register `mod sync_tests;` under `#[cfg(test)]`)

**Steps**
- [ ] In `entities.rs`, extend `pub mod presentation`'s `Model`:
  ```rust
  pub struct Model {
      #[sea_orm(primary_key, auto_increment = false)]
      pub id: String,
      pub library_id: String,
      pub name: String,
      pub search_name: String,
      pub created_at: DateTimeWithTimeZone,
      // #555 song sync:
      pub updated_at: DateTimeWithTimeZone,
      pub sync_id: String,
      pub deleted_at: Option<DateTimeWithTimeZone>,
  }
  ```
- [ ] In `create_presentation` (`repository/presentation.rs`), set the new fields on the
  insert `ActiveModel` (uses `chrono::Utc` — already imported):
  ```rust
  presentation_entity::Entity::insert(presentation_entity::ActiveModel {
      id: Set(presentation_uuid.to_string()),
      library_id: Set(library_uuid.clone()),
      name: Set(name.to_string()),
      search_name: Set(fold_query(name)),
      created_at: Set(Utc::now().into()),
      updated_at: Set(Utc::now().into()),
      sync_id: Set(uuid::Uuid::new_v4().to_string()),
      deleted_at: Set(None),
  })
  ```
- [ ] In `upsert_library` (`repository/library.rs`), set the new fields on the per-presentation
  insert `ActiveModel` (Task 6 changes `sync_id` to prefer the domain value):
  ```rust
  let pres_model = presentation_entity::ActiveModel {
      id: Set(presentation.id.to_string()),
      library_id: Set(library.id.to_string()),
      name: Set(presentation.name.clone()),
      search_name: Set(fold_query(&presentation.name)),
      created_at: Set(Utc::now().into()),
      updated_at: Set(Utc::now().into()),
      sync_id: Set(uuid::Uuid::new_v4().to_string()),
      deleted_at: Set(None),
  };
  ```
- [ ] Create `repository/sync_tests.rs` with a test-first assertion. This file grows across
  later tasks. It reads new columns via DIRECT entity queries (in-crate `crate::entities`
  access; `repo.db` is `pub(crate)`) so each task's tests compile WITHOUT depending on a
  later task's repo method:
  ```rust
  //! #555 song-sync repository tests: identity, LWW apply, soft-delete, trash.
  //! Add further `use` imports (ColumnTrait/QueryFilter/etc.) in the task that first needs
  //! them — keep the file clippy-clean (`-D warnings` forbids unused imports) at every commit.
  use crate::entities::presentation as presentation_entity;
  use crate::Repository;
  use presenter_core::{PresentationId, Slide, SlideContent, SlideText};
  use sea_orm::EntityTrait;

  async fn repo() -> Repository {
      Repository::connect_in_memory().await.expect("in-memory repo")
  }

  fn slide(order: u32, main: &str) -> Slide {
      Slide::new(
          order,
          SlideContent::new(
              SlideText::new(main).unwrap(),
              SlideText::new("").unwrap(),
              SlideText::new("").unwrap(),
              None,
          ),
      )
  }

  /// Direct row read (test-only) — used before the sync read methods exist.
  async fn row(repo: &Repository, id: PresentationId) -> presentation_entity::Model {
      presentation_entity::Entity::find_by_id(id.to_string())
          .one(&repo.db)
          .await
          .unwrap()
          .expect("presentation row exists")
  }

  #[tokio::test]
  async fn create_presentation_persists_sync_id_and_updated_at() {
      let repo = repo().await;
      let lib = repo.create_library("Songs").await.unwrap();
      let (_, _, pres) = repo
          .create_presentation(lib.id, "New Song", Some(&[slide(0, "verse")]))
          .await
          .unwrap();
      let model = row(&repo, pres.id).await;
      assert!(!model.sync_id.is_empty(), "create must assign a sync_id");
      assert!(model.deleted_at.is_none(), "a new song is not trashed");
      // updated_at is NOT NULL (the entity type guarantees it deserialized).
      let _ = model.updated_at;
  }
  ```
- [ ] Register the test module: in `repository/mod.rs`, under the existing `#[cfg(test)] mod tests;`
  add `#[cfg(test)] mod sync_tests;`.
- [ ] Run: `cargo test -p presenter-persistence --lib` → GREEN (compiles because every insert
  site now sets the fields).
- [ ] Commit: `feat(sync): add sync columns to presentation entity and all insert sites (#555)`

---

## Task 4 — `updated_at` bumped by every mutation path

Every song mutation must bump `presentations.updated_at` so LWW has a clock. Rename and
slide-content edits currently touch NO timestamp; structural slide ops go through
`replace_presentation_slides`.

**Files**
- Modify: `crates/presenter-persistence/src/repository/presentation.rs`
  (`rename_presentation`, `update_slide_content`, `update_slide_content_with_metadata`,
  `replace_presentation_slides`)
- Modify: `crates/presenter-persistence/src/repository/sync_tests.rs` (add tests)

**Steps**
- [ ] Add a small private helper to `presentation.rs` (keeps each mutation under the fn cap):
  ```rust
  /// Bump a presentation's `updated_at` to now, on any connection. Every local song
  /// mutation calls this so LWW sync has a monotone clock (#555).
  async fn touch_presentation<C: sea_orm::ConnectionTrait>(
      conn: &C,
      presentation_id: &str,
  ) -> anyhow::Result<()> {
      use presentation_entity::Column;
      presentation_entity::Entity::update_many()
          .col_expr(Column::UpdatedAt, Expr::value(chrono::Utc::now().to_rfc3339()))
          .filter(Column::Id.eq(presentation_id))
          .exec(conn)
          .await?;
      Ok(())
  }
  ```
  (Store timestamps as RFC3339 TEXT — SQLite `DateTimeWithTimeZone` round-trips as text.)
- [ ] `rename_presentation`: after the existing `update_many` succeeds, add the bump on the
  same connection:
  ```rust
  touch_presentation(&self.db, &id).await?;
  ```
- [ ] `update_slide_content` and `update_slide_content_with_metadata`: after the slide
  `update_many` succeeds (rows_affected checked), bump the parent presentation:
  ```rust
  touch_presentation(&self.db, &presentation_id.to_string()).await?;
  ```
- [ ] `replace_presentation_slides`: inside the existing `txn`, after the slide re-insert loop
  and BEFORE `txn.commit()`, add:
  ```rust
  touch_presentation(&txn, &presentation_id.to_string()).await?;
  ```
- [ ] Add tests to `sync_tests.rs` (test-first) — each asserts `updated_at` strictly
  increases across a mutation. Read `updated_at` via the DIRECT `row(...)` helper already in
  `sync_tests.rs` (no dependency on a later task); use
  `tokio::time::sleep(Duration::from_millis(5))` between reads so the RFC3339 subsecond
  differs, then compare as `DateTime<Utc>`. Cover `rename_presentation`,
  `update_slide_content_with_metadata`, and one path through `replace_presentation_slides`
  (call the repo method directly here with two slides):
  ```rust
  #[tokio::test]
  async fn rename_bumps_updated_at() {
      let repo = repo().await;
      let lib = repo.create_library("Songs").await.unwrap();
      let (id, _, _) = repo
          .create_presentation(lib.id, "Old", Some(&[slide(0, "a")]))
          .await
          .unwrap();
      let before: chrono::DateTime<chrono::Utc> = row(&repo, id).await.updated_at.into();
      tokio::time::sleep(std::time::Duration::from_millis(5)).await;
      repo.rename_presentation(id, "New").await.unwrap();
      let after: chrono::DateTime<chrono::Utc> = row(&repo, id).await.updated_at.into();
      assert!(after > before, "rename must bump updated_at");
  }
  ```
  Repeat the same before/after shape for `update_slide_content_with_metadata` (edit the
  song's slide) and `replace_presentation_slides` (pass a fresh 2-slide vec).
- [ ] Run: `cargo test -p presenter-persistence --lib` → RED then GREEN.
- [ ] Commit: `feat(sync): bump presentation updated_at on every local mutation (#555)`

---

## Task 5 — Soft delete + list-query filtering

`DELETE /presentations/{id}` becomes a soft delete: set `deleted_at` + `updated_at`, remove
the song's playlist entries (preserving today's user-visible "gone from playlists"), and
clear its stage-layout markers. Every list/lookup query filters `deleted_at IS NULL`.

**Files**
- Modify: `crates/presenter-persistence/src/repository/presentation.rs` (`delete_presentation`,
  `fetch_presentation_detail`, `fetch_first_presentation_detail`)
- Modify: `crates/presenter-persistence/src/repository/library.rs` (`fetch_libraries`,
  `list_library_summaries`)
- Modify: `crates/presenter-persistence/src/repository/search.rs` (both
  `presentation_entity::Entity::find()` sites)
- Modify: `crates/presenter-persistence/src/repository/playlist.rs`
  (`fetch_presentation_names_*` query)
- Modify: `crates/presenter-persistence/src/repository/sync_tests.rs` (tests)

**Steps**
- [ ] Rewrite `delete_presentation` as a soft delete in one transaction:
  ```rust
  #[instrument(skip_all)]
  pub async fn delete_presentation(&self, presentation_id: PresentationId) -> anyhow::Result<()> {
      use crate::entities::playlist_entry;
      let id = presentation_id.to_string();
      let txn = self.db.begin().await?;

      // Soft-delete: mark trashed + bump the clock (syncs like any edit under LWW).
      let now = chrono::Utc::now().to_rfc3339();
      let result = presentation_entity::Entity::update_many()
          .col_expr(presentation_entity::Column::DeletedAt, Expr::value(now.clone()))
          .col_expr(presentation_entity::Column::UpdatedAt, Expr::value(now))
          .filter(presentation_entity::Column::Id.eq(id.clone()))
          .filter(presentation_entity::Column::DeletedAt.is_null())
          .exec(&txn)
          .await?;
      if result.rows_affected == 0 {
          return Err(anyhow!("presentation not found"));
      }

      // Preserve today's behavior: a deleted song leaves every playlist.
      playlist_entry::Entity::delete_many()
          .filter(playlist_entry::Column::PresentationId.eq(id.clone()))
          .exec(&txn)
          .await?;

      // #515 markers go with the (now-hidden) song.
      crate::entities::slide_stage_layout::Entity::delete_many()
          .filter(crate::entities::slide_stage_layout::Column::PresentationId.eq(id))
          .exec(&txn)
          .await?;

      txn.commit().await?;
      Ok(())
  }
  ```
- [ ] Add `.filter(presentation_entity::Column::DeletedAt.is_null())` to the presentation
  `find()` in EACH of these (re-read each function by name first):
  `fetch_presentation_detail` (the `find_by_id` — chain `.filter(...)` on a `find().filter(Column::Id.eq(...))` form, OR after fetching check `deleted_at.is_none()` and return `None`),
  `fetch_first_presentation_detail`, `fetch_libraries` (the batch presentations query),
  `list_library_summaries` (the batch presentations query), both sites in `search.rs`, and the
  names query in `playlist.rs`.
  For `fetch_presentation_detail` which uses `find_by_id(...).one(...)`, the cleanest guard:
  ```rust
  let pres_model = presentation_entity::Entity::find()
      .filter(presentation_entity::Column::Id.eq(presentation_id.to_string()))
      .filter(presentation_entity::Column::DeletedAt.is_null())
      .one(&self.db)
      .await?;
  ```
- [ ] Tests (test-first) in `sync_tests.rs`: after `delete_presentation`, assert the song is
  ABSENT from `fetch_libraries` / `list_library_summaries` / `search_presenter` /
  `fetch_presentation_detail` (returns `None`), but the ROW STILL EXISTS with a non-null
  `deleted_at` (read via the direct `row(&repo, id)` helper — `model.deleted_at.is_some()`);
  and assert a playlist entry that referenced it is gone (create a playlist + entry first, or
  query `playlist_entry::Entity` directly).
- [ ] Run: `cargo test -p presenter-persistence --lib` → RED then GREEN.
- [ ] Commit: `feat(sync): soft-delete presentations to a trash and hide them from lists (#555)`

---

## Task 6 — Importer `sync_id` from the `.pro` UUID

Two instances importing the SAME `.pro` file must converge on the same identity. Carry the
protobuf `raw.uuid` (`proto::Presentation.uuid: Option<Uuid>`, `Uuid.string`) through the
domain `Presentation` into the persisted `sync_id`.

**Files**
- Modify: `crates/presenter-core/src/presentation.rs` (add `sync_id` field + builder)
- Modify: `crates/presenter-importer/src/lib.rs` (`presentation_from_proto`)
- Modify: `crates/presenter-persistence/src/repository/library.rs` (`upsert_library` — prefer domain `sync_id`)
- Modify: `crates/presenter-importer/src/lib.rs` tests (assert sync_id from uuid)

**Steps**
- [ ] Add an optional identity to the domain `Presentation` (default `None`,
  `#[serde(default)]` so existing JSON stays valid):
  ```rust
  #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
  #[serde(rename_all = "camelCase")]
  pub struct Presentation {
      pub id: PresentationId,
      pub name: String,
      pub slides: Vec<Slide>,
      /// #555 cross-instance identity. `None` for app-created songs (a fresh v4 is
      /// assigned at persist time); `Some` when imported from a `.pro` file (its UUID).
      #[serde(default)]
      pub sync_id: Option<String>,
  }
  ```
  In `Presentation::new`, set `sync_id: None`. Add a builder:
  ```rust
  pub fn with_sync_id(mut self, sync_id: impl Into<String>) -> Self {
      self.sync_id = Some(sync_id.into());
      self
  }
  ```
  Fix every other `Presentation { … }` struct-literal construction (grep
  `Presentation {` across the workspace) to add `sync_id: None` — most construction goes
  through `::new`, so this is minimal.
- [ ] In `presentation_from_proto` (`presenter-importer`), attach the `.pro` UUID:
  ```rust
  let presentation = Presentation::new(nfc::to_nfc(&raw.name), slides)?;
  let presentation = match raw.uuid.as_ref() {
      Some(u) if !u.string.trim().is_empty() => presentation.with_sync_id(u.string.trim()),
      _ => presentation,
  };
  Ok(presentation)
  ```
- [ ] In `upsert_library` (`repository/library.rs`), prefer the domain `sync_id`, else the
  deterministic name-based id (so a re-import without a `.pro` UUID still pairs across
  sites), replacing the plain `Uuid::new_v4()` from Task 3:
  ```rust
  sync_id: Set(presentation
      .sync_id
      .clone()
      .filter(|s| !s.trim().is_empty())
      .unwrap_or_else(|| presenter_core::sync_id_for_name(&library.name, &presentation.name))),
  ```
- [ ] Test (test-first) in `presenter-importer/src/lib.rs` tests: extend the existing
  `presentation_from_proto_*` fixture to set `raw.uuid = Some(proto::Uuid { string: "PRO-UUID-123".into() })`
  and assert `presentation.sync_id.as_deref() == Some("PRO-UUID-123")`.
- [ ] Add a persistence test in `sync_tests.rs`: build a `Library` (via `Library::new` +
  `Presentation::new(...).with_sync_id("PRO-UUID-123")`), `upsert_library`, then read the row
  via the direct `row(&repo, pres.id)` helper (or query `presentation_entity::Entity` by name)
  and assert `sync_id == "PRO-UUID-123"`; and a second presentation WITHOUT a sync_id upserts
  with `presenter_core::sync_id_for_name(lib_name, name)`.
- [ ] Run: `cargo test -p presenter-core -p presenter-importer -p presenter-persistence --lib`
  → RED then GREEN.
- [ ] Commit: `feat(sync): persist .pro UUID as cross-instance sync_id on import (#555)`

---

## Task 7 — Sync read layer: manifest + full-content repository methods

**Files**
- Create: `crates/presenter-persistence/src/repository/sync.rs`
- Modify: `crates/presenter-persistence/src/repository/mod.rs` (`mod sync;` + `pub use`)
- Modify: `crates/presenter-persistence/src/lib.rs` (re-export the sync structs)
- Modify: `crates/presenter-persistence/src/repository/sync_tests.rs`

**Steps**
- [ ] Create `repository/sync.rs` with the DT-agnostic structs (all `pub`) and read methods:
  ```rust
  use super::util::{parse_uuid, to_domain_slide, RepositoryError};
  use super::Repository;
  use crate::entities::{library, presentation as presentation_entity, slide as slide_entity};
  use chrono::{DateTime, Utc};
  use presenter_core::Slide;
  use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
  use tracing::instrument;

  /// One manifest row — identity + timestamps, ALL songs including trashed.
  #[derive(Debug, Clone)]
  pub struct SyncManifestRow {
      pub sync_id: String,
      pub library_name: String,
      pub name: String,
      pub updated_at: DateTime<Utc>,
      pub deleted_at: Option<DateTime<Utc>>,
  }

  /// Full synced content for one song.
  #[derive(Debug, Clone)]
  pub struct SyncPresentation {
      pub sync_id: String,
      pub library_name: String,
      pub name: String,
      pub updated_at: DateTime<Utc>,
      pub deleted_at: Option<DateTime<Utc>>,
      pub slides: Vec<Slide>,
  }

  impl Repository {
      #[instrument(skip_all)]
      pub async fn list_sync_manifest(&self) -> anyhow::Result<Vec<SyncManifestRow>> {
          let rows = presentation_entity::Entity::find()
              .find_also_related(library::Entity)
              .all(&self.db)
              .await?;
          let mut out = Vec::with_capacity(rows.len());
          for (p, lib) in rows {
              out.push(SyncManifestRow {
                  sync_id: p.sync_id,
                  library_name: lib.map(|l| l.name).unwrap_or_default(),
                  name: p.name,
                  updated_at: p.updated_at.into(),
                  deleted_at: p.deleted_at.map(Into::into),
              });
          }
          Ok(out)
      }

      #[instrument(skip_all)]
      pub async fn fetch_sync_presentation(
          &self,
          sync_id: &str,
      ) -> anyhow::Result<Option<SyncPresentation>> {
          let Some(p) = presentation_entity::Entity::find()
              .filter(presentation_entity::Column::SyncId.eq(sync_id))
              .one(&self.db)
              .await?
          else {
              return Ok(None);
          };
          let library_name = library::Entity::find_by_id(p.library_id.clone())
              .one(&self.db)
              .await?
              .map(|l| l.name)
              .unwrap_or_default();
          let slides = slide_entity::Entity::find()
              .filter(slide_entity::Column::PresentationId.eq(p.id.clone()))
              .order_by_asc(slide_entity::Column::Position)
              .all(&self.db)
              .await?
              .into_iter()
              .map(to_domain_slide)
              .collect::<Result<Vec<_>, RepositoryError>>()?;
          Ok(Some(SyncPresentation {
              sync_id: p.sync_id,
              library_name,
              name: p.name,
              updated_at: p.updated_at.into(),
              deleted_at: p.deleted_at.map(Into::into),
              slides,
          }))
      }
  }
  ```
- [ ] In `repository/mod.rs`: add `mod sync;` and
  `pub use sync::{SyncManifestRow, SyncPresentation};` (the apply/trash types are added in
  Tasks 8–9 — extend this `pub use` then).
- [ ] In `lib.rs`, extend the re-export: `pub use repository::{DatabaseSettings, Repository, SyncManifestRow, SyncPresentation};`.
- [ ] Run: `cargo test -p presenter-persistence --lib`
  → GREEN.
- [ ] Add a read test: seed a library + one song + one trashed song; assert
  `list_sync_manifest` returns BOTH (trashed with `deleted_at.is_some()`), and
  `fetch_sync_presentation(sync_id)` returns the full slides for the live one.
- [ ] Commit: `feat(sync): manifest + full-content sync read methods (#555)`

---

## Task 8 — LWW apply with adopt-by-name (the reconciliation core)

**Files**
- Create: `crates/presenter-persistence/src/repository/sync_apply.rs`
- Modify: `crates/presenter-persistence/src/repository/mod.rs` (`mod sync_apply;` + `pub use`)
- Modify: `crates/presenter-persistence/src/lib.rs` (re-export)
- Modify: `crates/presenter-persistence/src/repository/sync_tests.rs`

**Steps**
- [ ] Create `sync_apply.rs`. First a PURE, unit-testable decision function, then the
  transactional apply. Apply stores the PEER's `updated_at` (never `now()`) → no echo.
  Preserves the local presentation id on update (playlist refs intact). Adopt-by-name for
  transitional identity mismatches.
  ```rust
  use super::util::build_slide_active_model;
  use super::Repository;
  use crate::entities::{library, presentation as presentation_entity, slide as slide_entity};
  use crate::SyncPresentation;
  use chrono::{DateTime, Utc};
  use presenter_core::search::fold_query;
  use sea_orm::{
      sea_query::Expr, ColumnTrait, EntityTrait, QueryFilter, Set, TransactionTrait,
  };
  use tracing::{info, instrument};

  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub enum SyncApplyOutcome {
      Created,
      Updated,
      AdoptedByName,
      SkippedNotNewer,
  }

  impl SyncApplyOutcome {
      /// Did this apply WRITE to the DB? (Drives the no-echo/audit counts.)
      pub fn wrote(self) -> bool {
          !matches!(self, SyncApplyOutcome::SkippedNotNewer)
      }
  }

  /// LWW: apply the peer row iff it is strictly newer than what we hold (or unknown).
  /// `local` is `None` when we have no matching song at all.
  pub fn sync_should_apply(peer: DateTime<Utc>, local: Option<DateTime<Utc>>) -> bool {
      match local {
          None => true,
          Some(local) => peer > local,
      }
  }

  impl Repository {
      #[instrument(skip_all, fields(sync_id = %incoming.sync_id, name = %incoming.name))]
      pub async fn apply_sync_presentation(
          &self,
          incoming: &SyncPresentation,
      ) -> anyhow::Result<SyncApplyOutcome> {
          let txn = self.db.begin().await?;

          // Ensure a library with the peer's library name exists; reuse or create.
          let lib = library::Entity::find()
              .filter(library::Column::Name.eq(incoming.library_name.clone()))
              .one(&txn)
              .await?;
          let library_id = match lib {
              Some(l) => l.id,
              None => {
                  let id = uuid::Uuid::new_v4().to_string();
                  library::Entity::insert(library::ActiveModel {
                      id: Set(id.clone()),
                      name: Set(incoming.library_name.clone()),
                      search_name: Set(fold_query(&incoming.library_name)),
                      created_at: Set(Utc::now().into()),
                  })
                  .exec(&txn)
                  .await?;
                  id
              }
          };

          // 1. Match by sync_id.
          let by_sync = presentation_entity::Entity::find()
              .filter(presentation_entity::Column::SyncId.eq(incoming.sync_id.clone()))
              .one(&txn)
              .await?;

          if let Some(existing) = by_sync {
              let local_updated: DateTime<Utc> = existing.updated_at.into();
              if !sync_should_apply(incoming.updated_at, Some(local_updated)) {
                  txn.commit().await?;
                  info!("sync skip (not newer)");
                  return Ok(SyncApplyOutcome::SkippedNotNewer);
              }
              Self::write_synced_row(&txn, &existing.id, &library_id, incoming).await?;
              txn.commit().await?;
              info!("sync updated");
              return Ok(SyncApplyOutcome::Updated);
          }

          // 2. Adopt-by-name: same name in the same-named library, unknown sync_id.
          let by_name = presentation_entity::Entity::find()
              .filter(presentation_entity::Column::LibraryId.eq(library_id.clone()))
              .filter(presentation_entity::Column::Name.eq(incoming.name.clone()))
              .one(&txn)
              .await?;
          if let Some(existing) = by_name {
              let local_updated: DateTime<Utc> = existing.updated_at.into();
              if !sync_should_apply(incoming.updated_at, Some(local_updated)) {
                  // Local wins; the peer will adopt OUR sync_id when it pulls us.
                  txn.commit().await?;
                  info!("sync skip (adopt-by-name, local newer)");
                  return Ok(SyncApplyOutcome::SkippedNotNewer);
              }
              Self::write_synced_row(&txn, &existing.id, &library_id, incoming).await?;
              txn.commit().await?;
              info!("sync adopted-by-name");
              return Ok(SyncApplyOutcome::AdoptedByName);
          }

          // 3. Unknown → create with the peer's identity + timestamps.
          let new_id = uuid::Uuid::new_v4().to_string();
          presentation_entity::Entity::insert(presentation_entity::ActiveModel {
              id: Set(new_id.clone()),
              library_id: Set(library_id.clone()),
              name: Set(incoming.name.clone()),
              search_name: Set(fold_query(&incoming.name)),
              created_at: Set(Utc::now().into()),
              updated_at: Set(incoming.updated_at.into()),
              sync_id: Set(incoming.sync_id.clone()),
              deleted_at: Set(incoming.deleted_at.map(Into::into)),
          })
          .exec(&txn)
          .await?;
          Self::replace_slides(&txn, &new_id, incoming).await?;
          txn.commit().await?;
          info!("sync created");
          Ok(SyncApplyOutcome::Created)
      }

      /// Update an existing local row IN PLACE (preserving its id + playlist refs):
      /// name, search_name, library, sync_id (adopt), deleted_at, and the PEER's
      /// updated_at (never now() — that is what prevents echo). Then replace slides.
      async fn write_synced_row<C: sea_orm::ConnectionTrait>(
          conn: &C,
          local_id: &str,
          library_id: &str,
          incoming: &SyncPresentation,
      ) -> anyhow::Result<()> {
          use presentation_entity::Column;
          let deleted = incoming
              .deleted_at
              .map(|d| Expr::value(d.to_rfc3339()))
              .unwrap_or_else(|| Expr::value(Option::<String>::None));
          presentation_entity::Entity::update_many()
              .col_expr(Column::Name, Expr::value(incoming.name.clone()))
              .col_expr(Column::SearchName, Expr::value(fold_query(&incoming.name)))
              .col_expr(Column::LibraryId, Expr::value(library_id))
              .col_expr(Column::SyncId, Expr::value(incoming.sync_id.clone()))
              .col_expr(Column::UpdatedAt, Expr::value(incoming.updated_at.to_rfc3339()))
              .col_expr(Column::DeletedAt, deleted)
              .filter(Column::Id.eq(local_id))
              .exec(conn)
              .await?;
          Self::replace_slides(conn, local_id, incoming).await
      }

      /// Wholesale slide replacement carrying the peer's slide ids (global v4 uniqueness
      /// makes id collisions a non-issue).
      async fn replace_slides<C: sea_orm::ConnectionTrait>(
          conn: &C,
          presentation_id: &str,
          incoming: &SyncPresentation,
      ) -> anyhow::Result<()> {
          slide_entity::Entity::delete_many()
              .filter(slide_entity::Column::PresentationId.eq(presentation_id))
              .exec(conn)
              .await?;
          for (index, slide) in incoming.slides.iter().enumerate() {
              let active = build_slide_active_model(slide, presentation_id, index as i32);
              slide_entity::Entity::insert(active).exec(conn).await?;
          }
          Ok(())
      }
  }
  ```
- [ ] `repository/mod.rs`: add `mod sync_apply;` and extend the sync `pub use` to
  `pub use sync_apply::{sync_should_apply, SyncApplyOutcome};`. Re-export both from `lib.rs`.
- [ ] Unit tests (test-first) in `sync_apply.rs` `#[cfg(test)]` for the pure fn — the full
  matrix: newer → true, older → false, equal → false, unknown(None) → true. Plus in
  `sync_tests.rs`, apply-level tests: create-from-unknown, update-when-newer,
  skip-when-older, adopt-by-name (seed a local song with a DIFFERENT sync_id but same
  name+library, apply a newer peer with a new sync_id → assert the LOCAL row's id is
  UNCHANGED but its sync_id is now the peer's and content updated), and peer-timestamp
  preservation (after apply, `updated_at == incoming.updated_at`, NOT ~now).
- [ ] Run: `cargo test -p presenter-persistence --lib` → RED then GREEN.
- [ ] Commit: `feat(sync): LWW apply with adopt-by-name and peer-timestamp preservation (#555)`

---

## Task 9 — Trash: list, restore, 30-day prune

**Files**
- Modify: `crates/presenter-persistence/src/repository/sync.rs` (or a new
  `repository/trash.rs` if `sync.rs` nears the size cap — keep files < 1000 lines)
- Modify: `crates/presenter-persistence/src/repository/mod.rs` + `lib.rs` (re-export `TrashedPresentation`)
- Modify: `crates/presenter-persistence/src/repository/sync_tests.rs`

**Steps**
- [ ] Add the trash struct + methods (put in `sync.rs`; if it would exceed ~600 lines, make a
  new `repository/trash.rs` and register it):
  ```rust
  /// A soft-deleted song, for the trash UI.
  #[derive(Debug, Clone)]
  pub struct TrashedPresentation {
      pub id: String,
      pub sync_id: String,
      pub name: String,
      pub library_name: String,
      pub deleted_at: DateTime<Utc>,
  }

  impl Repository {
      #[instrument(skip_all)]
      pub async fn list_trashed_presentations(&self) -> anyhow::Result<Vec<TrashedPresentation>> {
          let rows = presentation_entity::Entity::find()
              .filter(presentation_entity::Column::DeletedAt.is_not_null())
              .order_by_desc(presentation_entity::Column::DeletedAt)
              .find_also_related(library::Entity)
              .all(&self.db)
              .await?;
          let mut out = Vec::with_capacity(rows.len());
          for (p, lib) in rows {
              if let Some(deleted) = p.deleted_at {
                  out.push(TrashedPresentation {
                      id: p.id,
                      sync_id: p.sync_id,
                      name: p.name,
                      library_name: lib.map(|l| l.name).unwrap_or_default(),
                      deleted_at: deleted.into(),
                  });
              }
          }
          Ok(out)
      }

      #[instrument(skip_all)]
      pub async fn restore_presentation(
          &self,
          presentation_id: presenter_core::PresentationId,
      ) -> anyhow::Result<()> {
          use presentation_entity::Column;
          let now = Utc::now().to_rfc3339();
          let result = presentation_entity::Entity::update_many()
              .col_expr(Column::DeletedAt, Expr::value(Option::<String>::None))
              .col_expr(Column::UpdatedAt, Expr::value(now))
              .filter(Column::Id.eq(presentation_id.to_string()))
              .filter(Column::DeletedAt.is_not_null())
              .exec(&self.db)
              .await?;
          if result.rows_affected == 0 {
              return Err(anyhow::anyhow!("no trashed presentation to restore"));
          }
          Ok(())
      }

      /// Hard-delete songs trashed longer than `retain`. FK cascade removes slides;
      /// stage-layout markers were cleared at soft-delete time. Returns rows removed.
      #[instrument(skip_all)]
      pub async fn prune_deleted_presentations(
          &self,
          retain: chrono::Duration,
      ) -> anyhow::Result<u64> {
          let cutoff = (Utc::now() - retain).to_rfc3339();
          let res = presentation_entity::Entity::delete_many()
              .filter(presentation_entity::Column::DeletedAt.is_not_null())
              .filter(presentation_entity::Column::DeletedAt.lt(cutoff))
              .exec(&self.db)
              .await?;
          Ok(res.rows_affected)
      }
  }
  ```
  (Import `Expr`, `QueryOrder`, `sea_query` as needed; add `use sea_orm::sea_query::Expr;`.)
- [ ] Re-export `TrashedPresentation` via `mod.rs` + `lib.rs`.
- [ ] Tests (test-first): soft-delete then `list_trashed_presentations` shows it;
  `restore_presentation` clears `deleted_at`, bumps `updated_at`, and the song reappears in
  `fetch_libraries`; `prune_deleted_presentations(Duration::days(30))` removes a row whose
  `deleted_at` you set to 31 days ago (soft-delete then directly UPDATE its deleted_at to an
  old timestamp via a repo test helper or a raw statement) but KEEPS a freshly-trashed one.
- [ ] Run: `cargo test -p presenter-persistence --lib` → RED then GREEN.
- [ ] Commit: `feat(sync): trash list, restore, and 30-day prune (#555)`

---

## Task 10 — Config: `PRESENTER_SYNC_PEER_URL`

**Files**
- Modify: `crates/presenter-server/src/config.rs` (new `SyncConfig`, wired into `ServerConfig`)

**Steps**
- [ ] Mirror `NetworkConfig` (which reads `PRESENTER_LOCAL_PUBLIC_IP`). Add:
  ```rust
  #[derive(Debug, Clone, Default)]
  pub struct SyncConfig {
      /// Peer instance base URL for song sync (#555). Unset/empty → sync disabled.
      pub peer_url: Option<String>,
  }

  impl SyncConfig {
      pub fn load() -> Self {
          let peer_url = std::env::var("PRESENTER_SYNC_PEER_URL")
              .ok()
              .map(|s| s.trim().to_string())
              .filter(|s| !s.is_empty());
          Self { peer_url }
      }
  }
  ```
  Add `pub sync: SyncConfig,` to `ServerConfig` and `sync: SyncConfig::load(),` in its loader
  (next to `network: NetworkConfig::load()`).
- [ ] Test (test-first, mirroring the existing `PRESENTER_LOCAL_PUBLIC_IP` env test):
  unset → `None`; empty/whitespace → `None`; `"  http://100.101.72.101 "` → trimmed `Some`.
  Serialize env access with the same `#[serial]`/lock pattern the existing config test uses
  (re-read that test — it saves/restores the env var).
- [ ] Run: `cargo test -p presenter-server config` → RED then GREEN.
- [ ] Commit: `feat(sync): read PRESENTER_SYNC_PEER_URL config (#555)`

---

## Task 11 — Sync engine + AppState wiring + mutation nudges

The pull loop (30 s interval + ~2 s debounced nudge, oneshot shutdown), the status
snapshot, the coordinator field on `AppState`, and the nudge calls on every local mutation.
Keep `state/mod.rs` additions minimal; put all logic + `impl AppState` methods in
`state/sync.rs`.

**Files**
- Create: `crates/presenter-server/src/state/sync.rs`
- Modify: `crates/presenter-server/src/state/mod.rs` (module decl, one struct field + init, one
  spawn call in `from_config`)
- Modify: `crates/presenter-server/src/state/presentations.rs` (nudge on create/rename/delete)
- Modify: `crates/presenter-server/src/state/slides/edit_ops.rs` (nudge on the 5 slide ops)

**Steps**
- [ ] Create `state/sync.rs`:
  ```rust
  //! #555 song-sync engine: a symmetric pull loop against the peer instance, plus a
  //! debounced nudge after local mutations. Reuses the AbleSet-tracker background-task
  //! shape (interval + oneshot shutdown). Applied rows carry the PEER timestamp → no echo.
  use std::sync::Arc;

  use chrono::{DateTime, Utc};
  use presenter_persistence::{SyncApplyOutcome, SyncPresentation};
  use serde::{Deserialize, Serialize};
  use tokio::sync::{mpsc, oneshot, Mutex, RwLock};
  use tokio::time::{interval, Duration, MissedTickBehavior};
  use tracing::{info, warn};

  use super::AppState;

  const SYNC_INTERVAL: Duration = Duration::from_secs(30);
  const NUDGE_DEBOUNCE: Duration = Duration::from_secs(2);

  /// Wire DTOs (both directions). camelCase to match the repo convention.
  #[derive(Debug, Clone, Serialize, Deserialize)]
  #[serde(rename_all = "camelCase")]
  pub struct SyncManifestEntryDto {
      pub sync_id: String,
      pub library_name: String,
      pub name: String,
      pub updated_at: DateTime<Utc>,
      pub deleted_at: Option<DateTime<Utc>>,
  }

  #[derive(Debug, Clone, Serialize, Deserialize)]
  #[serde(rename_all = "camelCase")]
  pub struct SyncPresentationDto {
      pub sync_id: String,
      pub library_name: String,
      pub name: String,
      pub updated_at: DateTime<Utc>,
      pub deleted_at: Option<DateTime<Utc>>,
      pub slides: Vec<presenter_core::Slide>,
  }

  impl From<SyncPresentationDto> for SyncPresentation {
      fn from(d: SyncPresentationDto) -> Self {
          SyncPresentation {
              sync_id: d.sync_id,
              library_name: d.library_name,
              name: d.name,
              updated_at: d.updated_at,
              deleted_at: d.deleted_at,
              slides: d.slides,
          }
      }
  }

  /// Operator-facing status (AbleSet status pattern).
  #[derive(Debug, Clone, Default, Serialize)]
  #[serde(rename_all = "camelCase")]
  pub struct SyncStatus {
      pub enabled: bool,
      pub peer_url: Option<String>,
      pub peer_version: Option<String>,
      pub peer_healthy: bool,
      pub last_run: Option<DateTime<Utc>>,
      pub last_success: Option<DateTime<Utc>>,
      pub last_error: Option<String>,
      pub pulled_last_cycle: usize,
      pub applied_last_cycle: usize,
  }

  /// Clonable handle stored on AppState. The receiver is taken once by the loop.
  #[derive(Clone)]
  pub struct SyncCoordinator {
      nudge_tx: mpsc::Sender<()>,
      nudge_rx: Arc<Mutex<Option<mpsc::Receiver<()>>>>,
      status: Arc<RwLock<SyncStatus>>,
  }

  impl Default for SyncCoordinator {
      fn default() -> Self {
          Self::new()
      }
  }

  impl SyncCoordinator {
      pub fn new() -> Self {
          let (tx, rx) = mpsc::channel(1);
          Self {
              nudge_tx: tx,
              nudge_rx: Arc::new(Mutex::new(Some(rx))),
              status: Arc::new(RwLock::new(SyncStatus::default())),
          }
      }

      /// Non-blocking; a full channel already has a nudge pending.
      pub fn nudge(&self) {
          let _ = self.nudge_tx.try_send(());
      }

      pub async fn snapshot(&self) -> SyncStatus {
          self.status.read().await.clone()
      }
  }

  impl AppState {
      /// Fire-and-forget nudge after a local song mutation.
      pub(crate) fn nudge_sync(&self) {
          self.sync.nudge();
      }

      pub async fn sync_status_snapshot(&self) -> SyncStatus {
          self.sync.snapshot().await
      }

      /// Start the pull loop against `peer_url`. Called once from `from_config` when the
      /// env var is set. Returns the shutdown sender (dropped-on-exit is fine in prod).
      pub(crate) fn spawn_sync_task(&self, peer_url: String) -> oneshot::Sender<()> {
          let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();
          let state = self.clone();
          let coordinator = self.sync.clone();
          let rx_slot = coordinator.nudge_rx.clone();
          let status = coordinator.status.clone();

          tokio::spawn(async move {
              let mut nudge_rx = match rx_slot.lock().await.take() {
                  Some(rx) => rx,
                  None => {
                      warn!("sync task already started; not starting a second loop");
                      return;
                  }
              };
              {
                  let mut s = status.write().await;
                  s.enabled = true;
                  s.peer_url = Some(peer_url.clone());
              }
              let client = reqwest::Client::builder()
                  .timeout(Duration::from_secs(15))
                  .build()
                  .unwrap_or_default();

              let mut ticker = interval(SYNC_INTERVAL);
              ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

              loop {
                  tokio::select! {
                      _ = &mut shutdown_rx => {
                          info!("sync loop shutting down");
                          break;
                      }
                      _ = ticker.tick() => {
                          run_and_record(&state, &peer_url, &client, &status).await;
                      }
                      maybe = nudge_rx.recv() => {
                          if maybe.is_none() { break; }
                          // Debounce: coalesce a burst of edits into one cycle.
                          tokio::time::sleep(NUDGE_DEBOUNCE).await;
                          while nudge_rx.try_recv().is_ok() {}
                          run_and_record(&state, &peer_url, &client, &status).await;
                      }
                  }
              }
          });
          shutdown_tx
      }
  }

  async fn run_and_record(
      state: &AppState,
      peer_url: &str,
      client: &reqwest::Client,
      status: &Arc<RwLock<SyncStatus>>,
  ) {
      let started = Utc::now();
      let peer_version = fetch_peer_version(client, peer_url).await;
      match run_sync_cycle(state, peer_url, client).await {
          Ok((pulled, applied)) => {
              let mut s = status.write().await;
              s.last_run = Some(started);
              s.last_success = Some(Utc::now());
              s.last_error = None;
              s.peer_healthy = peer_version.is_some();
              s.peer_version = peer_version;
              s.pulled_last_cycle = pulled;
              s.applied_last_cycle = applied;
          }
          Err(err) => {
              warn!(?err, "sync cycle failed");
              let mut s = status.write().await;
              s.last_run = Some(started);
              s.last_error = Some(err.to_string());
              s.peer_healthy = false;
              s.peer_version = peer_version;
          }
      }
  }

  async fn fetch_peer_version(client: &reqwest::Client, peer_url: &str) -> Option<String> {
      let resp = client.get(format!("{peer_url}/healthz")).send().await.ok()?;
      let json: serde_json::Value = resp.json().await.ok()?;
      json.get("version").and_then(|v| v.as_str()).map(|s| s.to_string())
  }

  /// One reconciliation pass against the peer. Returns (pulled, applied). Directly
  /// callable from the integration test (bypasses the loop).
  pub(crate) async fn run_sync_cycle(
      state: &AppState,
      peer_url: &str,
      client: &reqwest::Client,
  ) -> anyhow::Result<(usize, usize)> {
      let repo = state.repository();

      // Index our local identities → updated_at for the LWW gate.
      let local = repo.list_sync_manifest().await?;
      let mut local_map = std::collections::HashMap::new();
      for row in &local {
          local_map.insert(row.sync_id.clone(), row.updated_at);
      }

      let peer_manifest: Vec<SyncManifestEntryDto> = client
          .get(format!("{peer_url}/sync/manifest"))
          .send()
          .await?
          .error_for_status()?
          .json()
          .await?;

      let mut pulled = 0usize;
      let mut applied = 0usize;
      for entry in peer_manifest {
          let local_updated = local_map.get(&entry.sync_id).copied();
          if !presenter_persistence::sync_should_apply(entry.updated_at, local_updated) {
              continue;
          }
          pulled += 1;
          let dto: SyncPresentationDto = client
              .get(format!("{peer_url}/sync/presentations/{}", entry.sync_id))
              .send()
              .await?
              .error_for_status()?
              .json()
              .await?;
          match repo.apply_sync_presentation(&dto.into()).await {
              Ok(outcome) => {
                  if outcome.wrote() {
                      applied += 1;
                  }
                  info!(sync_id = %entry.sync_id, name = %entry.name, ?outcome, "sync applied");
              }
              Err(err) => warn!(?err, sync_id = %entry.sync_id, "sync apply failed"),
          }
      }
      Ok((pulled, applied))
  }
  ```
  > Do NOT use `unwrap()` in prod paths. The one `unwrap_or_default()` on the reqwest client
  > is acceptable (it degrades to a default client), but if clippy/policy objects, build the
  > client with `?` at spawn time and log-and-return on error instead.
- [ ] In `state/mod.rs` (MINIMAL — re-read first):
  - Add `pub(crate) mod sync;` to the module list (near the other `mod` lines).
  - Add ONE field to `struct AppState`:
    ```rust
    /// #555 song-sync coordinator (nudge channel + status). Loop spawned only when
    /// PRESENTER_SYNC_PEER_URL is set.
    sync: sync::SyncCoordinator,
    ```
  - In `new_with_heartbeat`'s `Self { … }` literal, add `sync: sync::SyncCoordinator::new(),`.
  - In `from_config`, AFTER `state.spawn_background_tasks();`, add:
    ```rust
    if let Some(peer_url) = config.sync.peer_url.clone() {
        tracing::info!(%peer_url, "song sync enabled");
        let _ = state.spawn_sync_task(peer_url);
    }
    ```
    (The returned shutdown sender is intentionally dropped — the loop runs for the process
    lifetime; the `oneshot` closes on drop which is a clean shutdown-on-exit.)
- [ ] Wire the 30-day trash prune tick into `spawn_background_tasks` (`state/mod.rs`), mirroring
  the existing WAL-checkpoint ticker — a low-frequency loop so months of use don't grow the
  table. `Repository::prune_deleted_presentations` lands in Task 9; if executing strictly in
  order, add this tick in Task 9's follow-up or here once the method exists:
  ```rust
  let prune_state = self.clone();
  tokio::spawn(async move {
      let mut ticker = interval(TokioDuration::from_secs(6 * 3600));
      ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
      loop {
          ticker.tick().await;
          match prune_state
              .repository
              .prune_deleted_presentations(chrono::Duration::days(30))
              .await
          {
              Ok(n) if n > 0 => tracing::info!(pruned = n, "pruned trashed songs older than 30 days"),
              Ok(_) => {}
              Err(err) => warn!(?err, "trash prune failed"),
          }
      }
  });
  ```
  (`interval`, `TokioDuration`, `MissedTickBehavior`, `warn` are already imported in
  `state/mod.rs`.)
- [ ] Add the nudge after each successful local mutation. In `state/presentations.rs`
  (re-read by name): `create_presentation` (after the repo create), `rename_presentation`
  (after repo rename), `delete_presentation` (after repo delete) — add `self.nudge_sync();`.
  In `state/slides/edit_ops.rs` (re-read by name — line numbers are shifting): after the
  repository call succeeds in `update_slide_content`, `insert_blank_slide`, `duplicate_slide`,
  `delete_slide`, `reorder_slides`, add `self.nudge_sync();` (place it right after the
  `self.repository.<...>().await?` / before/after the broadcast — either is fine).
- [ ] Unit test the pure `sync_should_apply` is already covered in Task 8; add a small
  `state/sync.rs` `#[cfg(test)]` test that `SyncCoordinator::new().nudge()` does not panic and
  `snapshot()` returns defaults. Full engine behavior is proven by Task 13.
- [ ] Run: `cargo test -p presenter-server --lib` and `cargo build -p presenter-server`
  → GREEN. Verify `state/mod.rs` stays under 1000 lines (`wc -l`).
- [ ] Commit: `feat(sync): pull-loop engine, coordinator wiring, and post-mutation nudge (#555)`

---

## Task 12 — HTTP routes: manifest, content, status, trash, restore

**Files**
- Create: `crates/presenter-server/src/router/sync.rs`
- Modify: `crates/presenter-server/src/router.rs` (`mod sync;` + route registration)
- Modify: `crates/presenter-server/src/router/tests.rs` (route-shape tests — new tests appended
  here follow the existing `oneshot` pattern; this is the designated router test file)

**Steps**
- [ ] Create `router/sync.rs` with all five handlers (keeps `router/presentations.rs`
  untouched — reduces conflict with concurrent edits):
  ```rust
  use axum::{extract::{Path, State}, http::StatusCode, Json};
  use serde::Serialize;
  use tracing::instrument;

  use super::AppError;
  use crate::state::sync::{SyncManifestEntryDto, SyncPresentationDto, SyncStatus};
  use crate::state::AppState;
  use presenter_core::PresentationId;

  #[derive(Debug, Serialize)]
  #[serde(rename_all = "camelCase")]
  pub(super) struct TrashedPresentationDto {
      pub(super) id: String,
      pub(super) sync_id: String,
      pub(super) name: String,
      pub(super) library_name: String,
      pub(super) deleted_at: chrono::DateTime<chrono::Utc>,
  }

  #[instrument(skip_all)]
  pub(super) async fn get_sync_manifest(
      State(state): State<AppState>,
  ) -> Result<Json<Vec<SyncManifestEntryDto>>, AppError> {
      let rows = state.repository().list_sync_manifest().await?;
      Ok(Json(
          rows.into_iter()
              .map(|r| SyncManifestEntryDto {
                  sync_id: r.sync_id,
                  library_name: r.library_name,
                  name: r.name,
                  updated_at: r.updated_at,
                  deleted_at: r.deleted_at,
              })
              .collect(),
      ))
  }

  #[instrument(skip_all)]
  pub(super) async fn get_sync_presentation(
      Path(sync_id): Path<String>,
      State(state): State<AppState>,
  ) -> Result<Json<SyncPresentationDto>, AppError> {
      match state.repository().fetch_sync_presentation(&sync_id).await? {
          Some(p) => Ok(Json(SyncPresentationDto {
              sync_id: p.sync_id,
              library_name: p.library_name,
              name: p.name,
              updated_at: p.updated_at,
              deleted_at: p.deleted_at,
              slides: p.slides,
          })),
          None => Err(AppError::not_found(format!("sync presentation {sync_id} not found"))),
      }
  }

  #[instrument(skip_all)]
  pub(super) async fn get_sync_status(
      State(state): State<AppState>,
  ) -> Result<Json<SyncStatus>, AppError> {
      Ok(Json(state.sync_status_snapshot().await))
  }

  #[instrument(skip_all)]
  pub(super) async fn list_trash(
      State(state): State<AppState>,
  ) -> Result<Json<Vec<TrashedPresentationDto>>, AppError> {
      let rows = state.repository().list_trashed_presentations().await?;
      Ok(Json(
          rows.into_iter()
              .map(|r| TrashedPresentationDto {
                  id: r.id,
                  sync_id: r.sync_id,
                  name: r.name,
                  library_name: r.library_name,
                  deleted_at: r.deleted_at,
              })
              .collect(),
      ))
  }

  #[instrument(skip_all)]
  pub(super) async fn restore_presentation(
      State(state): State<AppState>,
      Path(id): Path<String>,
  ) -> Result<StatusCode, AppError> {
      let uuid = super::parse_uuid("presentationId", &id)?;
      state.restore_presentation(PresentationId::from_uuid(uuid)).await?;
      Ok(StatusCode::NO_CONTENT)
  }
  ```
  > `AppError::not_found` / `super::parse_uuid` follow the existing `router/presentations.rs`
  > usage — mirror them exactly. Add a thin `AppState::restore_presentation` wrapper in
  > `state/presentations.rs` that calls `self.repository.restore_presentation(id)` then
  > `self.nudge_sync();` (restore is a local change that must propagate).
- [ ] In `router.rs`: add `mod sync;` near the other `mod` decls, and register the routes
  (place the trash routes with the presentation routes; the `/sync/*` routes anywhere in the
  builder chain). Note the STATIC `/presentations/trash` MUST be registered BEFORE the
  dynamic `/presentations/{id}` route so matchit doesn't swallow it (same lesson as the
  video-sources `/status` route):
  ```rust
  .route("/sync/manifest", get(sync::get_sync_manifest))
  .route("/sync/presentations/{sync_id}", get(sync::get_sync_presentation))
  .route("/integrations/sync/status", get(sync::get_sync_status))
  .route("/presentations/trash", get(sync::list_trash))
  .route("/presentations/{id}/restore", post(sync::restore_presentation))
  ```
- [ ] Router tests (in `router/tests.rs`, following the `oneshot` pattern already there):
  `GET /sync/manifest` → 200 + JSON array; `GET /integrations/sync/status` → 200 + JSON with
  `enabled:false` (no peer configured in tests); `GET /presentations/trash` → 200 empty array;
  create+delete a song then `GET /presentations/trash` shows it and `POST
  /presentations/{id}/restore` → 204 and it leaves the trash.
- [ ] Run: `cargo test -p presenter-server` → RED then GREEN.
- [ ] Commit: `feat(sync): sync + trash HTTP endpoints (#555)`

---

## Task 13 — Integration test: two AppStates syncing over real HTTP (CORE DELIVERABLE)

Two full `AppState`s (separate in-memory DBs), each serving its real router on an ephemeral
`127.0.0.1:0` port, driven through `run_sync_cycle` in both directions. Proves the whole
matrix from the spec.

**Files**
- Create: `crates/presenter-server/src/state/sync_integration_tests.rs`
- Modify: `crates/presenter-server/src/state/mod.rs` (register `#[cfg(test)] mod sync_integration_tests;`)

**Steps**
- [ ] Register the module in `state/mod.rs` next to `#[cfg(test)] mod tests;`:
  `#[cfg(test)] mod sync_integration_tests;`.
- [ ] Write the full test file:
  ```rust
  //! #555 end-to-end sync proof: two independent AppStates, each on a real ephemeral
  //! HTTP port, reconciled via `run_sync_cycle` both ways.
  use std::net::SocketAddr;

  use presenter_core::{LibraryId, PresentationId, Slide, SlideContent, SlideText};

  use crate::router::build_router;
  use crate::state::sync::run_sync_cycle;
  use crate::state::AppState;

  fn slide(order: u32, main: &str) -> Slide {
      Slide::new(
          order,
          SlideContent::new(
              SlideText::new(main).unwrap(),
              SlideText::new("").unwrap(),
              SlideText::new("").unwrap(),
              None,
          ),
      )
  }

  /// Bind the state's real router on 127.0.0.1:0 and return its base URL.
  async fn serve(state: AppState) -> String {
      let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
      let addr: SocketAddr = listener.local_addr().unwrap();
      let router = build_router(state);
      tokio::spawn(async move {
          axum::serve(listener, router).await.unwrap();
      });
      format!("http://{addr}")
  }

  async fn make_song(state: &AppState, lib: &str, name: &str, text: &str) -> (LibraryId, PresentationId) {
      let library = state.create_library(lib).await.unwrap();
      let (id, _, pres, _) = state
          .create_presentation(library.id, name, Some(&[slide(0, text)]))
          .await
          .unwrap();
      (library.id, pres.id)
  }

  fn client() -> reqwest::Client {
      reqwest::Client::builder().build().unwrap()
  }

  #[tokio::test]
  async fn create_propagates_a_to_b() {
      let a = AppState::in_memory().await.unwrap();
      let b = AppState::in_memory().await.unwrap();
      let a_url = serve(a.clone()).await;
      make_song(&a, "Songs", "Amazing Grace", "verse 1").await;

      // B pulls from A.
      let (_pulled, applied) = run_sync_cycle(&b, &a_url, &client()).await.unwrap();
      assert!(applied >= 1, "B must import the song created on A");

      let libs = b.libraries().await.unwrap();
      assert!(libs.iter().any(|l| l.presentations.iter().any(|p| p.name == "Amazing Grace")));
  }

  #[tokio::test]
  async fn edit_and_rename_propagate() {
      let a = AppState::in_memory().await.unwrap();
      let b = AppState::in_memory().await.unwrap();
      let a_url = serve(a.clone()).await;
      let (_lib, id) = make_song(&a, "Songs", "Song", "old text").await;
      run_sync_cycle(&b, &a_url, &client()).await.unwrap();

      // Rename + edit on A.
      a.rename_presentation(id, "Renamed").await.unwrap();
      let detail = a.presentation_detail(id).await.unwrap().unwrap().2;
      let sid = detail.slides[0].id;
      a.update_slide_content(id, sid, "new text".into(), "".into(), "".into(), None, None)
          .await
          .unwrap();

      run_sync_cycle(&b, &a_url, &client()).await.unwrap();
      let libs = b.libraries().await.unwrap();
      let p = libs.iter().flat_map(|l| &l.presentations).find(|p| p.name == "Renamed").unwrap();
      assert_eq!(p.slides[0].content.main.value(), "new text");
  }

  #[tokio::test]
  async fn delete_to_trash_and_restore_propagate() {
      let a = AppState::in_memory().await.unwrap();
      let b = AppState::in_memory().await.unwrap();
      let a_url = serve(a.clone()).await;
      let (_lib, id) = make_song(&a, "Songs", "Temp", "x").await;
      run_sync_cycle(&b, &a_url, &client()).await.unwrap();

      // Delete on A → soft delete → syncs as an edit.
      a.delete_presentation(id).await.unwrap();
      run_sync_cycle(&b, &a_url, &client()).await.unwrap();
      assert!(!b.libraries().await.unwrap().iter().any(|l| l.presentations.iter().any(|p| p.name == "Temp")));
      let trash = b.repository().list_trashed_presentations().await.unwrap();
      assert!(trash.iter().any(|t| t.name == "Temp"), "B shows it in trash");

      // Restore on A → propagates back to live.
      tokio::time::sleep(std::time::Duration::from_millis(5)).await;
      a.restore_presentation(id).await.unwrap();
      run_sync_cycle(&b, &a_url, &client()).await.unwrap();
      assert!(b.libraries().await.unwrap().iter().any(|l| l.presentations.iter().any(|p| p.name == "Temp")));
  }

  #[tokio::test]
  async fn lww_newer_edit_wins_in_a_two_sided_conflict() {
      // Same song imported on both, then edited on both — the newer edit wins.
      let a = AppState::in_memory().await.unwrap();
      let b = AppState::in_memory().await.unwrap();
      let a_url = serve(a.clone()).await;
      let b_url = serve(b.clone()).await;
      let (_la, ia) = make_song(&a, "Songs", "Conflict", "A original").await;
      // B pulls A so both share the sync_id.
      run_sync_cycle(&b, &a_url, &client()).await.unwrap();

      // Edit B first, then A later (A is newer).
      let bp = b.libraries().await.unwrap().into_iter().flat_map(|l| l.presentations).find(|p| p.name == "Conflict").unwrap();
      let bsid = bp.slides[0].id;
      b.update_slide_content(bp.id, bsid, "B edit".into(), "".into(), "".into(), None, None).await.unwrap();
      tokio::time::sleep(std::time::Duration::from_millis(10)).await;
      let ap = a.presentation_detail(ia).await.unwrap().unwrap().2;
      a.update_slide_content(ia, ap.slides[0].id, "A edit (newer)".into(), "".into(), "".into(), None, None).await.unwrap();

      // Reconcile both ways; A's newer edit must win everywhere.
      run_sync_cycle(&a, &b_url, &client()).await.unwrap();
      run_sync_cycle(&b, &a_url, &client()).await.unwrap();

      for st in [&a, &b] {
          let p = st.libraries().await.unwrap().into_iter().flat_map(|l| l.presentations).find(|p| p.name == "Conflict").unwrap();
          assert_eq!(p.slides[0].content.main.value(), "A edit (newer)");
      }
  }

  #[tokio::test]
  async fn fully_synced_pair_produces_zero_writes_on_next_cycle() {
      // The no-echo guard.
      let a = AppState::in_memory().await.unwrap();
      let b = AppState::in_memory().await.unwrap();
      let a_url = serve(a.clone()).await;
      make_song(&a, "Songs", "Steady", "text").await;
      run_sync_cycle(&b, &a_url, &client()).await.unwrap(); // B imports

      // Next cycle: nothing newer on A → zero applied (and zero pulled).
      let (pulled, applied) = run_sync_cycle(&b, &a_url, &client()).await.unwrap();
      assert_eq!(applied, 0, "a settled pair must not re-apply (no echo)");
      assert_eq!(pulled, 0, "a settled pair must not re-pull");
  }
  ```
  > `build_router` is `pub`; `AppState::in_memory` is `#[cfg(test)]` pub; `run_sync_cycle` is
  > `pub(crate)` — all reachable from this in-crate test module. `AppState::in_memory` does
  > NOT spawn the sync loop (only `from_config` does), so the test drives cycles manually and
  > deterministically. `unwrap()` is allowed here (test code).
- [ ] Run: `cargo test -p presenter-server sync_integration` → all green.
- [ ] Commit: `test(sync): two-instance integration test over real HTTP (#555)`

---

## Task 14 — Trash UI section + API client + Playwright E2E

**Files**
- Create: `crates/presenter-ui/src/pages/settings/trash.rs`
- Modify: `crates/presenter-ui/src/pages/settings/mod.rs` (register + render the card)
- Modify: `crates/presenter-ui/src/api/mod.rs` (`pub mod sync;`)
- Create: `crates/presenter-ui/src/api/sync.rs` (client fns + DTOs)
- Create: `tests/e2e/song-trash.spec.ts`

**Steps**
- [ ] API client `crates/presenter-ui/src/api/sync.rs` (mirror `api/ndi.rs`):
  ```rust
  use serde::Deserialize;

  use super::{get_json, post_no_content, ApiError};

  #[derive(Debug, Clone, Deserialize)]
  #[serde(rename_all = "camelCase")]
  pub struct TrashedSongDto {
      pub id: String,
      pub name: String,
      pub library_name: String,
      pub deleted_at: String,
  }

  pub async fn list_trash() -> Result<Vec<TrashedSongDto>, ApiError> {
      get_json("/presentations/trash").await
  }

  pub async fn restore_song(id: &str) -> Result<(), ApiError> {
      post_no_content(&format!("/presentations/{id}/restore"), &serde_json::json!({})).await
  }
  ```
  Add `pub mod sync;` to `api/mod.rs`. (Confirm `post_no_content` exists in `api/mod.rs`; the
  ndi client imports it — reuse it.)
- [ ] Settings card `crates/presenter-ui/src/pages/settings/trash.rs` (mirror the simpler
  structure of an existing card; use `super::ToastHandle`, `super::format_timestamp`, the
  `confirm` modal, and a `RwSignal<Vec<TrashedSongDto>>`):
  ```rust
  //! #555 "Zmazané piesne" (trash) card: soft-deleted songs with restore.
  use leptos::prelude::*;

  use super::ToastHandle;
  use crate::api::sync::{self, TrashedSongDto};

  #[component]
  pub fn TrashCard(toast: ToastHandle) -> impl IntoView {
      let items = RwSignal::new(Vec::<TrashedSongDto>::new());

      let reload = move || {
          leptos::task::spawn_local(async move {
              if let Ok(list) = sync::list_trash().await {
                  items.set(list);
              }
          });
      };
      reload();

      let restore = move |id: String| {
          leptos::task::spawn_local(async move {
              match sync::restore_song(&id).await {
                  Ok(()) => {
                      toast.show("Pieseň obnovená", "success");
                      if let Ok(list) = sync::list_trash().await {
                          items.set(list);
                      }
                  }
                  Err(err) => toast.show(&format!("Obnovenie zlyhalo: {err}"), "error"),
              }
          });
      };

      view! {
          <section class="settings__card" data-role="trash-card">
              <header class="settings__card-header">
                  <div>
                      <h2>"Zmazané piesne"</h2>
                      <p>"Obnov omylom zmazanú pieseň (uchované 30 dní)."</p>
                  </div>
              </header>
              <div class="settings__list" data-role="trash-list">
                  <Show
                      when=move || !items.get().is_empty()
                      fallback=|| view! { <p class="settings__empty">"Kôš je prázdny."</p> }
                  >
                      <For
                          each=move || items.get()
                          key=|item| item.id.clone()
                          children=move |item: TrashedSongDto| {
                              let id = item.id.clone();
                              let restore = restore.clone();
                              view! {
                                  <div class="settings__row" data-role="trash-row">
                                      <div>
                                          <span class="settings__row-title">{item.name.clone()}</span>
                                          <span class="settings__row-sub">
                                              {item.library_name.clone()}" · "
                                              {super::format_timestamp(&item.deleted_at)}
                                          </span>
                                      </div>
                                      <button
                                          class="settings__btn"
                                          data-role="restore-btn"
                                          on:click=move |_| restore(id.clone())
                                      >"Obnoviť"</button>
                                  </div>
                              }
                          }
                      />
                  </Show>
              </div>
          </section>
      }
  }
  ```
  > `restore` closure is used inside `For` children — wrap it as needed for Leptos `Send/Sync`
  > (it captures only `Copy` signals + a `String`, mirroring the video-sources card's
  > `activate`/`delete_source` closures; follow that exact pattern if the borrow checker
  > complains). Re-check reactive-closure rules against the `ui` skill.
- [ ] Register in `settings/mod.rs`: `mod trash;`, `use trash::TrashCard;`, and add
  `<TrashCard toast=toast />` to the `<main class="settings__main">` list (after
  `<VideoSourcesCard .../>`). Re-read the file first (another agent may be editing it).
- [ ] Playwright E2E `tests/e2e/song-trash.spec.ts` (starts its own server via
  `startTestServer` — no `PRESENTER_SYNC_PEER_URL`, so sync stays disabled): create a song via
  the API, `DELETE /presentations/{id}`, open `/ui/settings`, assert a `[data-role="trash-row"]`
  with the song name is visible, click `[data-role="restore-btn"]`, assert the row disappears
  and the song is back in `/libraries/summary`. Assert zero console errors (per the project's
  browser-console-zero-errors rule). Model it on an existing settings spec.
- [ ] Run: `cd crates/presenter-ui && cargo test --lib` (host build of the UI card logic) and
  the Playwright spec locally (`npm run test:playwright -- song-trash`). GREEN.
- [ ] Commit: `feat(sync): trash settings section + restore UI + E2E (#555)`

---

## Task 15 — Deploy workflow env vars + configuration docs

**Files**
- Modify: `.github/workflows/deploy.yml` (SNV — peer is PP)
- Modify: `.github/workflows/release.yml` (PP — peer is SNV)
- Modify: `docs/configuration.md` (new env-var row)

**Steps**
- [ ] Both workflows already write a systemd drop-in for `PRESENTER_ANDROID_STAGE_URL`
  (`printf '[Service]\nEnvironment=...' | ssh ... tee /etc/systemd/system/presenter.service.d/stage-url.conf`).
  Mirror that with a `sync-peer.conf` drop-in. In `deploy.yml` (deploys to SNV; peer = PP
  tailscale IP), add a step next to the stage-url step:
  ```yaml
  - name: Configure song-sync peer (PP)
    run: |
      printf '[Service]\nEnvironment=PRESENTER_SYNC_PEER_URL=%s\n' "http://100.101.72.101" |
        ssh deploy-target "sudo tee /etc/systemd/system/presenter.service.d/sync-peer.conf >/dev/null && sudo systemctl daemon-reload"
  ```
- [ ] In `release.yml` (deploys to PP; peer = SNV tailscale IP), add the analogous step:
  ```yaml
  - name: Configure song-sync peer (SNV)
    run: |
      printf '[Service]\nEnvironment=PRESENTER_SYNC_PEER_URL=%s\n' "http://100.122.204.47" |
        ssh deploy-target "sudo tee /etc/systemd/system/presenter.service.d/sync-peer.conf >/dev/null && sudo systemctl daemon-reload"
  ```
  > Place each new step BEFORE the `systemctl enable/start` step so the drop-in is in place
  > when the service (re)starts. Re-read the exact step ids / anchors in each workflow; match
  > the surrounding indentation and the `deploy-target` SSH alias already used.
- [ ] `docs/configuration.md`: add a row to the environment-variable table (find the table
  with `PRESENTER_LOCAL_PUBLIC_IP` / `PRESENTER_ANDROID_STAGE_URL`):
  ```
  | `PRESENTER_SYNC_PEER_URL` | unset | Base URL of the peer Presenter instance for two-way song sync (#555). Set per env in the deploy units over Tailscale: SNV → `http://100.101.72.101` (PP), PP → `http://100.122.204.47` (SNV). Unset → sync disabled (dev + E2E). |
  ```
  (Also add the same row to the CLAUDE.md env table if the implementer is updating docs there;
  optional — configuration.md is the canonical reference.)
- [ ] Run: `yamllint`/`actionlint` if available, else visually verify YAML. `cargo build` is
  unaffected.
- [ ] Commit: `feat(sync): deploy peer URLs + document PRESENTER_SYNC_PEER_URL (#555)`

---

## Task 16 — Verification (full local gate + post-deploy plan)

**Steps**
- [ ] `git fetch origin && git merge origin/main` (resolve any drift from the concurrent PR;
  re-run affected tests).
- [ ] Full local gate (this is the dev2 build box — local builds allowed):
  ```bash
  cargo fmt --all --check
  cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
  cargo test
  (cd crates/presenter-ui && cargo test --lib)
  npm run test:playwright -- song-trash
  ```
  All must pass. Confirm no prod file exceeds 1000 lines and no function exceeds 120 lines
  (the file/function-limit CI check): `wc -l` the touched files, especially `state/mod.rs`
  (must be < 1000) and `repository/sync.rs` / `sync_apply.rs`.
- [ ] Spec-coverage self-check — verify every spec item is implemented:
  migration (updated_at backfill + sync_id UUIDv5 backfill + deleted_at + unique index) ✓T2;
  entity+repo updated_at bump every mutation ✓T3–T4; importer sync_id from raw.uuid ✓T6;
  soft-delete + playlist-entry removal + list filtering ✓T5; 30-day prune ✓T9 (+ wire a prune
  tick — see next box); restore + trash list endpoints ✓T9/T12; `/sync/manifest` +
  `/sync/presentations/{sync_id}` ✓T7/T12; adopt-by-name preserving local id ✓T8;
  peer-timestamp no-echo ✓T8/T13; sync loop 30s + 2s debounce + oneshot ✓T11;
  `/integrations/sync/status` ✓T11/T12; trash UI + Playwright ✓T14; deploy env vars ✓T15;
  docs row ✓T15; integration matrix ✓T13.
- [ ] **Confirm the 30-day prune tick actually RUNS:** verify Task 11 added the periodic
  `repository.prune_deleted_presentations(chrono::Duration::days(30))` ticker to
  `state/mod.rs::spawn_background_tasks` (the 6 h loop). If it is missing, add it now per the
  Task 11 snippet, re-run `cargo test -p presenter-server`, and commit the fix.
- [ ] **CI:** push `dev`, monitor every job to terminal state (fmt, clippy, companion,
  version-check, security, test, quality, coverage, build, e2e, deploy-dev) until all green;
  fix root causes, never rerun blindly.
- [ ] **Post-deploy verification (from the spec):** after dev deploy is green, and after the
  SNV deploy (merge to main) + PP release cut, verify on the LIVE targets: create a test song
  on SNV (`http://10.77.9.205/ui/operator`) → within ~1 min observe it on PP
  (`http://companion-pp.lan`) via the real UI; create one on PP → observe on SNV (both
  directions). Then delete it on one side and restore from the trash section
  (`/ui/settings` → "Zmazané piesne") on the other. Confirm `GET /integrations/sync/status`
  reports `enabled:true`, `peerHealthy:true`, a recent `lastSuccess`, and no `lastError` on
  BOTH instances. Read results with your own tools (curl over tailscale / Playwright), never
  ask the user to test.
- [ ] Write the completion report (auto-merge default flow per project policy; SNV deploys on
  merge to main, PP requires a Release cut — note that in the report).

---

## Notes for the implementer

- **Timestamp storage:** all new datetime columns are SQLite TEXT (RFC3339). Write with
  `chrono::Utc::now().to_rfc3339()` in `col_expr`, and the SeaORM `DateTimeWithTimeZone`
  entity type round-trips them. Compare in Rust as `DateTime<Utc>` (`.into()`).
- **Determinism is load-bearing:** `sync_id_for_name` (namespace + `"lib/name"`) must be
  byte-identical on both instances. It lives ONCE in `presenter-core::sync` — never duplicate
  or re-parameterize it.
- **No echo depends on one line:** the apply path stores `incoming.updated_at`, never
  `now()`. If you ever "bump on apply", the two instances will ping-pong forever. Task 13's
  `fully_synced_pair_produces_zero_writes_on_next_cycle` is the guard — keep it green.
- **Adopt-by-name preserves the local presentation id** (playlist references stay intact) —
  it updates the existing row's `sync_id`, it does not delete+recreate.
- **`state/mod.rs` is near the size cap** — every sync method except the field/init/spawn
  lives in `state/sync.rs`. Verify `wc -l crates/presenter-server/src/state/mod.rs` < 1000
  after Task 11.
