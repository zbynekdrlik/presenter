# Bible Import Speedup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Cut the ~40 min bible import on every deploy down to ~0s when source archives are unchanged and <2 min when they actually change.

**Architecture:** Two layers. (1) A `source_digest` column on `bible_translations` lets the ingest binary SHA-256 the source archive and skip the whole import when the hash matches. (2) The repository's `replace_bible_translation_passages` drops the `bible_passage_fts` triggers inside its transaction, bulk-inserts passages, bulk-populates FTS via one `INSERT ... SELECT`, and recreates the triggers — eliminating the per-row FTS trigger cost that dominates wall time.

**Tech Stack:** Rust, SeaORM, SQLite FTS5, sha2 crate.

**Spec:** `docs/superpowers/specs/2026-04-14-bible-import-speedup-design.md`

---

## Context

The bible import goes through `crates/presenter-importer/src/bin/ingest_bibles.rs` → `BibleIngestionService::ingest` → `BibleScraper::scrape` (reads file bytes, parses) → `Repository::replace_bible_translation_passages` (single transaction, batched `insert_many`). Per-row `INSERT` into `bible_passages` fires the three FTS5 triggers from migration `m20260412_000001_bible_fts.rs`, which is the bottleneck.

**Key existing code:**
- `crates/presenter-importer/src/bin/ingest_bibles.rs` — CLI entry point
- `crates/presenter-importer/src/bible.rs` — `BibleIngestionService::ingest(spec)` and `ingest_specs(specs)`
- `crates/presenter-bible/src/lib.rs:185-231` — `BibleScraper::scrape(spec)` (reads bytes + delegates to `parsers::build_ingestion_batch`)
- `crates/presenter-bible/src/parsers.rs:22` — `pub(crate) fn build_ingestion_batch(bytes, spec)`
- `crates/presenter-persistence/src/repository/bible.rs:52-123` — `replace_bible_translation_passages`
- `crates/presenter-persistence/src/entities.rs:332-357` — `bible_translation` entity module, table name `bible_translations`
- `crates/presenter-persistence/src/repository/util.rs:192` — `to_domain_translation` helper
- `crates/presenter-persistence/src/repository/util.rs:368` — `BIBLE_INSERT_CHUNK = 500`
- `crates/presenter-migration/src/lib.rs` — migration registry
- `crates/presenter-migration/src/m20260408_000001_add_preach_limit.rs` — reference pattern for `ALTER TABLE ADD COLUMN` with `pragma_table_info` guard
- `crates/presenter-migration/src/m20260412_000001_bible_fts.rs` — the FTS5 triggers we're going to drop/recreate inside the import transaction

**Table naming reminder:** the Rust module alias is `bible_translation` (singular) but the SQLite table is `bible_translations` (plural). Migrations touch the plural table name; entity code touches the module alias.

---

## File Structure

| File | Change |
|------|--------|
| `crates/presenter-migration/src/m20260414_000001_bible_translation_digest.rs` | **New** — `ALTER TABLE bible_translations ADD COLUMN source_digest TEXT NULL` with `pragma_table_info` guard |
| `crates/presenter-migration/src/lib.rs` | Register the new migration |
| `crates/presenter-persistence/src/entities.rs:332-342` | Add `pub source_digest: Option<String>` field to `bible_translation::Model` |
| `crates/presenter-persistence/src/repository/bible.rs:52-123` | Rework `replace_bible_translation_passages` to drop/recreate FTS triggers inside the transaction and bulk-populate FTS in one statement. Add `get_bible_source_digest` and `set_bible_source_digest` helpers. Preserve the existing digest across calls unless the caller provides a new one. |
| `crates/presenter-persistence/src/repository/util.rs:192` | Update `to_domain_translation` if it has to become aware of the new column (check only — likely unaffected since domain `BibleTranslation` doesn't carry the digest) |
| `crates/presenter-bible/src/lib.rs:185-231` | Add `BibleScraper::scrape_with_bytes(spec, &[u8])`. Refactor `scrape(spec)` to read bytes and delegate. Add a sibling helper `read_local_file_bytes(spec)` on `BibleTranslationSpec` (or as free fn) so the ingest binary can read bytes without touching the provider. |
| `crates/presenter-importer/src/bible.rs` | Add `BibleIngestionService::ingest_with_bytes(spec, &[u8])`. `ingest(spec)` delegates to it after reading bytes. |
| `crates/presenter-importer/src/bin/ingest_bibles.rs` | Read local file bytes, compute digest, query stored digest, skip or ingest+persist digest per spec. |
| `crates/presenter-importer/Cargo.toml` | Add `sha2 = "0.10"` dependency |

---

## Task 1: Add `source_digest` migration

**Files:**
- Create: `crates/presenter-migration/src/m20260414_000001_bible_translation_digest.rs`
- Modify: `crates/presenter-migration/src/lib.rs`

- [ ] **Step 1: Create the migration file**

```rust
// crates/presenter-migration/src/m20260414_000001_bible_translation_digest.rs
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let result = db
            .query_one(sea_orm::Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "SELECT COUNT(*) as cnt FROM pragma_table_info('bible_translations') WHERE name='source_digest'",
            ))
            .await?;

        let has_column = result
            .map(|row| row.try_get::<i32>("", "cnt").unwrap_or(0) > 0)
            .unwrap_or(false);

        if !has_column {
            db.execute(sea_orm::Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "ALTER TABLE bible_translations ADD COLUMN source_digest TEXT NULL",
            ))
            .await?;
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // SQLite DROP COLUMN support is limited; non-destructive addition — no-op.
        let _ = manager;
        Ok(())
    }
}
```

- [ ] **Step 2: Register the migration**

In `crates/presenter-migration/src/lib.rs`, add the module and entry. Full new file:

```rust
pub use sea_orm_migration::prelude::*;

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

- [ ] **Step 3: Verify build**

Run: `cargo check -p presenter-migration`
Expected: compiles clean.

- [ ] **Step 4: Commit**

```bash
git add crates/presenter-migration/src/m20260414_000001_bible_translation_digest.rs crates/presenter-migration/src/lib.rs
git commit -m "feat(migration): add source_digest column to bible_translations (#243)"
```

---

## Task 2: Expose `source_digest` in the entity and patch existing ActiveModel literal

**Files:**
- Modify: `crates/presenter-persistence/src/entities.rs:332-342`
- Modify: `crates/presenter-persistence/src/repository/bible.rs` — the `bible_translation::ActiveModel { ... }` literal inside `replace_bible_translation_passages` (around the "let translation_model = bible_translation::ActiveModel {" block)

- [ ] **Step 1: Add field to `bible_translation::Model`**

Locate the `bible_translation` module in `entities.rs` and update `Model`:

```rust
#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "bible_translations")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub code: String,
    pub name: String,
    pub language: String,
    pub show_in_dashboard: bool,
    pub source: Option<String>,
    pub created_at: DateTimeWithTimeZone,
    pub source_digest: Option<String>,
}
```

- [ ] **Step 2: Patch the existing `ActiveModel` literal to keep the repo compiling**

In `crates/presenter-persistence/src/repository/bible.rs`, locate the existing `let translation_model = bible_translation::ActiveModel { ... }` block inside `replace_bible_translation_passages` and add `source_digest: Set(None)` as the last field so the literal stays exhaustive:

```rust
        let translation_model = bible_translation::ActiveModel {
            code: Set(translation.code.clone()),
            name: Set(translation.name.clone()),
            language: Set(translation.language.clone()),
            show_in_dashboard: Set(preserve_dashboard),
            source: Set(translation.source.clone()),
            created_at: Set(Utc::now().into()),
            source_digest: Set(None),
        };
```

This is a minimal patch so the crate builds. Task 4 rewrites this whole function and will change the digest handling to preserve it.

- [ ] **Step 3: Verify build**

Run: `cargo check -p presenter-persistence`
Expected: compiles clean.

- [ ] **Step 4: Commit**

```bash
git add crates/presenter-persistence/src/entities.rs crates/presenter-persistence/src/repository/bible.rs
git commit -m "feat(persistence): add source_digest field to bible_translation entity (#243)"
```

---

## Task 3: Repository digest accessors + unit tests

**Files:**
- Modify: `crates/presenter-persistence/src/repository/bible.rs:50` (end of first `impl Repository` block)
- Test: `crates/presenter-persistence/src/repository/bible.rs` (end-of-file `#[cfg(test)] mod tests`)

- [ ] **Step 1: Write the failing test for `set`/`get` round-trip**

Append a new test module at the end of `crates/presenter-persistence/src/repository/bible.rs`. Use `Repository::connect_in_memory()` to match the existing `presentation_tests` module pattern.

```rust
#[cfg(test)]
mod digest_tests {
    use super::*;
    use crate::repository::Repository;
    use presenter_core::bible::BibleIngestionBatch;
    use presenter_core::BibleTranslation;

    async fn fresh_repo() -> Repository {
        Repository::connect_in_memory()
            .await
            .expect("in-memory repo")
    }

    async fn seed_translation(repo: &Repository, code: &str) {
        let translation = BibleTranslation::new(code, code, "xx").with_source("test");
        let batch = BibleIngestionBatch::new(translation, Vec::new())
            .expect("empty batch is valid");
        repo.replace_bible_translation_passages(&batch)
            .await
            .expect("seed translation");
    }

    #[tokio::test]
    async fn set_and_get_source_digest_round_trip() {
        let repo = fresh_repo().await;
        seed_translation(&repo, "eng-test").await;

        assert_eq!(
            repo.get_bible_source_digest("eng-test").await.unwrap(),
            None,
            "fresh translation has no digest",
        );

        repo.set_bible_source_digest("eng-test", "abc123")
            .await
            .expect("set digest");

        assert_eq!(
            repo.get_bible_source_digest("eng-test").await.unwrap(),
            Some("abc123".to_string()),
        );
    }

    #[tokio::test]
    async fn get_digest_for_unknown_code_is_none() {
        let repo = fresh_repo().await;
        assert_eq!(
            repo.get_bible_source_digest("nope-none").await.unwrap(),
            None,
        );
    }
}
```

> **Note on API signatures:** `BibleIngestionBatch::new(translation, passages)` returns `Result<Self, BibleIngestionError>` — the `.expect(...)` calls above handle that. An empty passages vec is valid.

- [ ] **Step 2: Run the tests, verify they fail**

Run: `cargo test -p presenter-persistence -- digest_tests --nocapture`
Expected: FAIL with errors like "no method named `get_bible_source_digest` found" / "no method named `set_bible_source_digest` found".

- [ ] **Step 3: Implement the accessors**

Add to the `impl Repository` block in `bible.rs` — put them right after `replace_bible_translation_passages`:

```rust
    #[instrument(skip_all)]
    pub async fn get_bible_source_digest(&self, code: &str) -> anyhow::Result<Option<String>> {
        let model = bible_translation::Entity::find_by_id(code.to_string())
            .one(&self.db)
            .await?;
        Ok(model.and_then(|m| m.source_digest))
    }

    #[instrument(skip_all)]
    pub async fn set_bible_source_digest(&self, code: &str, digest: &str) -> anyhow::Result<()> {
        let existing = bible_translation::Entity::find_by_id(code.to_string())
            .one(&self.db)
            .await?
            .ok_or_else(|| anyhow!("bible_translation {code} not found"))?;
        let mut active: bible_translation::ActiveModel = existing.into();
        active.source_digest = Set(Some(digest.to_string()));
        active.update(&self.db).await?;
        Ok(())
    }
```

`Set` and `anyhow!` are already in scope at the top of `bible.rs` — no new imports needed.

- [ ] **Step 4: Run the tests, verify pass**

Run: `cargo test -p presenter-persistence -- digest_tests --nocapture`
Expected: both tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/presenter-persistence/src/repository/bible.rs
git commit -m "feat(persistence): add get/set_bible_source_digest (#243)"
```

---

## Task 4: Rewrite `replace_bible_translation_passages` with fast-import path

**Files:**
- Modify: `crates/presenter-persistence/src/repository/bible.rs:52-123`
- Test: `crates/presenter-persistence/src/repository/bible.rs` end-of-file test module

- [ ] **Step 1: Write a failing test that proves FTS search works after the fast path**

Append a new test module at the end of `crates/presenter-persistence/src/repository/bible.rs`:

```rust
#[cfg(test)]
mod fast_import_tests {
    use super::*;
    use crate::repository::Repository;
    use presenter_core::bible::BibleIngestionBatch;
    use presenter_core::{BiblePassage, BibleReference, BibleTranslation};

    fn make_passage(
        translation: &BibleTranslation,
        book: &str,
        chapter: u16,
        verse: u16,
        content: &str,
    ) -> BiblePassage {
        let reference = BibleReference::new(book.to_string(), chapter, verse, verse)
            .expect("valid reference");
        BiblePassage::new(reference, translation.clone(), content.to_string())
    }

    async fn fresh_repo() -> Repository {
        Repository::connect_in_memory().await.expect("in-memory repo")
    }

    #[tokio::test]
    async fn fast_import_preserves_fts_search() {
        let repo = fresh_repo().await;
        let translation = BibleTranslation::new("eng-fast", "Fast Test", "en")
            .with_source("test");
        let passages = vec![
            make_passage(&translation, "John", 3, 16, "For God so loved the world"),
            make_passage(&translation, "Genesis", 1, 1, "In the beginning God created"),
        ];
        let batch = BibleIngestionBatch::new(translation, passages)
            .expect("valid batch");

        repo.replace_bible_translation_passages(&batch)
            .await
            .expect("import");

        let hits = repo
            .search_bible_passages("eng-fast", "beginning", 10)
            .await
            .expect("search");
        assert!(
            hits.iter().any(|p| p.text.contains("In the beginning")),
            "FTS search should find 'beginning' after fast import, got {:?}",
            hits,
        );
    }

    #[tokio::test]
    async fn fast_import_idempotent_row_counts() {
        let repo = fresh_repo().await;
        let translation = BibleTranslation::new("eng-idem", "Idem Test", "en")
            .with_source("test");
        let passages = vec![
            make_passage(&translation, "John", 1, 1, "In the beginning was the Word"),
            make_passage(&translation, "John", 1, 2, "He was with God in the beginning"),
        ];
        let batch = BibleIngestionBatch::new(translation, passages)
            .expect("valid batch");

        repo.replace_bible_translation_passages(&batch).await.unwrap();
        let first_hits = repo
            .search_bible_passages("eng-idem", "beginning", 10)
            .await
            .unwrap();

        // Second import with identical data
        repo.replace_bible_translation_passages(&batch).await.unwrap();
        let second_hits = repo
            .search_bible_passages("eng-idem", "beginning", 10)
            .await
            .unwrap();

        assert_eq!(
            first_hits.len(),
            second_hits.len(),
            "re-import should produce identical hit count (not accumulating FTS rows)",
        );
        assert_eq!(first_hits.len(), 2);
    }
}
```

- [ ] **Step 2: Run the new tests to ensure they currently FAIL for the right reason**

Run: `cargo test -p presenter-persistence -- fast_import_tests --nocapture`
Expected behaviour depends on current state:
- If tests PASS already: that means the existing per-row trigger path works and our rewrite is purely a speedup. Accept that as baseline. Proceed to Step 3 anyway — the rewrite must still satisfy these assertions.
- If tests FAIL (e.g., `BibleIngestionBatch::new` signature mismatch, `BiblePassage::new` arity different): fix the test helpers to match the actual API, commit the test fixes, and continue.

- [ ] **Step 3: Rewrite `replace_bible_translation_passages`**

Replace the entire function body (`crates/presenter-persistence/src/repository/bible.rs` lines ~52-123) with:

```rust
    pub async fn replace_bible_translation_passages(
        &self,
        batch: &BibleIngestionBatch,
    ) -> anyhow::Result<()> {
        use sea_orm::Statement;
        const SQLITE: DatabaseBackend = DatabaseBackend::Sqlite;

        let (translation, passages) = batch.clone().into_parts();
        let existing = bible_translation::Entity::find_by_id(translation.code.clone())
            .one(&self.db)
            .await?;
        let preserve_dashboard = existing
            .as_ref()
            .map(|model| model.show_in_dashboard)
            .unwrap_or(translation.show_in_dashboard);
        let preserve_digest = existing
            .as_ref()
            .and_then(|model| model.source_digest.clone());

        let txn = self.db.begin().await?;

        // 1. Clear this translation's FTS rows (other translations untouched)
        txn.execute(Statement::from_sql_and_values(
            SQLITE,
            "DELETE FROM bible_passage_fts WHERE translation_code = ?",
            [translation.code.clone().into()],
        ))
        .await?;

        // 2. Drop the FTS triggers inside the transaction. SQLite supports DDL
        //    in transactions; if we roll back, the triggers are restored.
        for trig in [
            "bible_passage_fts_insert",
            "bible_passage_fts_delete",
            "bible_passage_fts_update",
        ] {
            txn.execute(Statement::from_string(
                SQLITE,
                format!("DROP TRIGGER IF EXISTS {trig}"),
            ))
            .await?;
        }

        // 3. Delete old translation row (cascades to old passages via FK)
        bible_translation::Entity::delete_by_id(translation.code.clone())
            .exec(&txn)
            .await?;

        // 4. Insert fresh translation row (preserving show_in_dashboard and existing digest)
        let translation_model = bible_translation::ActiveModel {
            code: Set(translation.code.clone()),
            name: Set(translation.name.clone()),
            language: Set(translation.language.clone()),
            show_in_dashboard: Set(preserve_dashboard),
            source: Set(translation.source.clone()),
            created_at: Set(Utc::now().into()),
            source_digest: Set(preserve_digest),
        };
        bible_translation::Entity::insert(translation_model)
            .exec(&txn)
            .await?;

        // 5. Batch-insert passages (no trigger overhead because triggers are dropped)
        let mut chunk = Vec::with_capacity(BIBLE_INSERT_CHUNK);
        for passage in passages {
            let reference = &passage.reference;
            let (code, number) = match &reference.book_code {
                Some(c) => (c.clone(), reference.book_number.unwrap_or(0) as i32),
                None => match canonical_book_by_name(&reference.book) {
                    Some(meta) => (meta.code.to_string(), meta.number as i32),
                    None => (reference.book.clone(), 0),
                },
            };
            let model = bible_passage::ActiveModel {
                id: Set(Uuid::new_v4().to_string()),
                translation_code: Set(translation.code.clone()),
                book: Set(reference.book.clone()),
                book_code: Set(code),
                book_number: Set(number),
                chapter: Set(reference.chapter as i32),
                verse_start: Set(reference.verse_start as i32),
                verse_end: Set(reference.verse_end as i32),
                content: Set(passage.text.clone()),
                created_at: Set(Utc::now().into()),
            };

            chunk.push(model);
            if chunk.len() == BIBLE_INSERT_CHUNK {
                let to_insert = std::mem::take(&mut chunk);
                bible_passage::Entity::insert_many(to_insert)
                    .exec(&txn)
                    .await?;
            }
        }

        if !chunk.is_empty() {
            bible_passage::Entity::insert_many(chunk)
                .exec(&txn)
                .await?;
        }

        // 6. Bulk populate FTS from the freshly-inserted passages
        txn.execute(Statement::from_sql_and_values(
            SQLITE,
            "INSERT INTO bible_passage_fts(passage_id, translation_code, book, content) \
             SELECT id, translation_code, book, content FROM bible_passages \
             WHERE translation_code = ?",
            [translation.code.clone().into()],
        ))
        .await?;

        // 7. Recreate the FTS triggers with the same bodies as the original migration
        txn.execute(Statement::from_string(
            SQLITE,
            "CREATE TRIGGER bible_passage_fts_insert \
             AFTER INSERT ON bible_passages BEGIN \
                INSERT INTO bible_passage_fts(passage_id, translation_code, book, content) \
                VALUES (new.id, new.translation_code, new.book, new.content); \
             END".to_string(),
        ))
        .await?;
        txn.execute(Statement::from_string(
            SQLITE,
            "CREATE TRIGGER bible_passage_fts_delete \
             AFTER DELETE ON bible_passages BEGIN \
                DELETE FROM bible_passage_fts WHERE passage_id = old.id; \
             END".to_string(),
        ))
        .await?;
        txn.execute(Statement::from_string(
            SQLITE,
            "CREATE TRIGGER bible_passage_fts_update \
             AFTER UPDATE ON bible_passages BEGIN \
                DELETE FROM bible_passage_fts WHERE passage_id = old.id; \
                INSERT INTO bible_passage_fts(passage_id, translation_code, book, content) \
                VALUES (new.id, new.translation_code, new.book, new.content); \
             END".to_string(),
        ))
        .await?;

        txn.commit().await?;
        Ok(())
    }
```

Key notes:
- Uses `txn.execute(Statement::from_string(...))` for DDL / raw SQL, mirroring the pattern in existing migrations.
- `Uuid` is already imported at the top of the file as `use uuid::Uuid`.
- `preserve_digest` means re-importing identical data via this function alone does not nuke the digest. The ingest binary is responsible for explicitly setting the digest after a successful import.

- [ ] **Step 4: Run the tests again**

Run: `cargo test -p presenter-persistence -- fast_import_tests digest_tests --nocapture`
Expected: all four tests PASS (the digest round-trip tests from Task 3 plus the two fast-import tests from this task).

- [ ] **Step 5: Run the entire bible repository test suite to catch regressions**

Run: `cargo test -p presenter-persistence -- bible --nocapture`
Expected: all existing tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/presenter-persistence/src/repository/bible.rs
git commit -m "perf(persistence): drop FTS triggers during bulk bible import (#243)

Move bible import to a fast path that drops the three FTS5 triggers
inside the transaction, batch-inserts passages without per-row trigger
overhead, bulk-populates bible_passage_fts via one INSERT ... SELECT,
and recreates the triggers before commit. Transaction rollback restores
the triggers automatically."
```

---

## Task 5: `scrape_with_bytes` on `BibleScraper`

**Files:**
- Modify: `crates/presenter-bible/src/lib.rs:185-231`

- [ ] **Step 1: Add `scrape_with_bytes` and refactor `scrape`**

Replace the `BibleScraper` impl block at lines ~189-231 with:

```rust
impl<P: BibleContentProvider> BibleScraper<P> {
    pub fn new(provider: P) -> Self {
        Self { provider }
    }

    /// Fetch bytes for `spec` (URL or LocalFile) and return the parsed batch.
    pub async fn scrape(
        &self,
        spec: &BibleTranslationSpec,
    ) -> Result<(BibleIngestionBatch, BibleImportSummary)> {
        let bytes = self.fetch_spec_bytes(spec).await?;
        self.scrape_with_bytes(spec, &bytes)
    }

    /// Parse bytes that the caller has already fetched. Used by the ingest
    /// binary so it can hash the bytes once and pass them in.
    pub fn scrape_with_bytes(
        &self,
        spec: &BibleTranslationSpec,
        bytes: &[u8],
    ) -> Result<(BibleIngestionBatch, BibleImportSummary)> {
        parsers::build_ingestion_batch(bytes, spec)
    }

    async fn fetch_spec_bytes(&self, spec: &BibleTranslationSpec) -> Result<Vec<u8>> {
        match &spec.source {
            BibleSource::Url { url } => self
                .provider
                .fetch_bytes(url)
                .await
                .with_context(|| format!("failed to fetch bible archive from {}", url)),
            BibleSource::LocalFile { env_var, .. } => {
                let path = std::env::var(env_var).with_context(|| {
                    format!(
                        "environment variable {} must be set to the bible archive for {}",
                        env_var, spec.translation.code
                    )
                })?;
                std::fs::read(&path).with_context(|| {
                    format!(
                        "failed to read bible archive for {} from {}",
                        spec.translation.code, path
                    )
                })
            }
        }
    }
}
```

- [ ] **Step 2: Verify existing tests still pass**

Run: `cargo test -p presenter-bible`
Expected: all tests pass unchanged (they call `scrape(spec)`, which still works).

- [ ] **Step 3: Commit**

```bash
git add crates/presenter-bible/src/lib.rs
git commit -m "refactor(bible): add scrape_with_bytes to BibleScraper (#243)"
```

---

## Task 6: `read_local_file_bytes` helper + `ingest_with_bytes` on the service

**Files:**
- Modify: `crates/presenter-bible/src/lib.rs` (add a free function near the spec helpers)
- Modify: `crates/presenter-importer/src/bible.rs`

- [ ] **Step 1: Add `read_local_file_bytes` free function in `presenter-bible`**

At the top-level of `crates/presenter-bible/src/lib.rs` (near `default_translation_specs`), add:

```rust
/// Read the bytes of a `LocalFile` spec's archive without going through a
/// `BibleContentProvider`. Returns `Err` for `Url` specs — callers that need
/// URL support should use `BibleScraper::scrape` or fetch bytes themselves.
pub fn read_local_file_bytes(spec: &BibleTranslationSpec) -> anyhow::Result<Vec<u8>> {
    use anyhow::{anyhow, Context};
    match &spec.source {
        BibleSource::LocalFile { env_var, .. } => {
            let path = std::env::var(env_var).with_context(|| {
                format!(
                    "environment variable {} must be set to the bible archive for {}",
                    env_var, spec.translation.code
                )
            })?;
            std::fs::read(&path).with_context(|| {
                format!(
                    "failed to read bible archive for {} from {}",
                    spec.translation.code, path
                )
            })
        }
        BibleSource::Url { .. } => Err(anyhow!(
            "read_local_file_bytes called on URL spec {}",
            spec.translation.code
        )),
    }
}
```

- [ ] **Step 2: Add `ingest_with_bytes` on `BibleIngestionService`**

Replace the `ingest` method in `crates/presenter-importer/src/bible.rs` (inside `impl<'a, P> BibleIngestionService<'a, P>`) with both methods:

```rust
    pub async fn ingest(&self, spec: &BibleTranslationSpec) -> Result<BibleImportSummary> {
        let (batch, summary) = self.scraper.scrape(spec).await?;
        self.repository
            .replace_bible_translation_passages(&batch)
            .await?;
        Ok(summary)
    }

    pub async fn ingest_with_bytes(
        &self,
        spec: &BibleTranslationSpec,
        bytes: &[u8],
    ) -> Result<BibleImportSummary> {
        let (batch, summary) = self.scraper.scrape_with_bytes(spec, bytes)?;
        self.repository
            .replace_bible_translation_passages(&batch)
            .await?;
        Ok(summary)
    }
```

- [ ] **Step 3: Build to verify**

Run: `cargo check -p presenter-importer -p presenter-bible`
Expected: compiles clean.

- [ ] **Step 4: Commit**

```bash
git add crates/presenter-bible/src/lib.rs crates/presenter-importer/src/bible.rs
git commit -m "feat(importer): add ingest_with_bytes and read_local_file_bytes helper (#243)"
```

---

## Task 7: Digest module + unit tests in `presenter-importer`

**Files:**
- Create: `crates/presenter-importer/src/bible_digest.rs`
- Modify: `crates/presenter-importer/src/lib.rs` (add `mod bible_digest; pub use bible_digest::*;`)
- Modify: `crates/presenter-importer/Cargo.toml` (add `sha2`)

- [ ] **Step 1: Add `sha2` dependency**

In `crates/presenter-importer/Cargo.toml`, under `[dependencies]`, add:

```toml
sha2 = "0.10"
```

- [ ] **Step 2: Create the digest module**

Create `crates/presenter-importer/src/bible_digest.rs`:

```rust
//! Source-archive digest for bible imports.
//!
//! The digest is `SHA-256(file_bytes || parser_version_le_bytes)`. Bumping
//! `PARSER_VERSION` forces re-imports even when the source archive bytes
//! are unchanged — use this when the parser logic changes in a way that
//! would produce different passages from the same archive.

use sha2::{Digest, Sha256};

pub const PARSER_VERSION: u32 = 1;

pub fn compute_source_digest(file_bytes: &[u8], parser_version: u32) -> String {
    let mut h = Sha256::new();
    h.update(file_bytes);
    h.update(parser_version.to_le_bytes());
    format!("{:x}", h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_bytes_same_version_same_digest() {
        let bytes = b"hello world";
        assert_eq!(
            compute_source_digest(bytes, 1),
            compute_source_digest(bytes, 1),
        );
    }

    #[test]
    fn different_version_different_digest() {
        let bytes = b"hello world";
        assert_ne!(
            compute_source_digest(bytes, 1),
            compute_source_digest(bytes, 2),
            "parser version bump must change the digest",
        );
    }

    #[test]
    fn different_bytes_different_digest() {
        assert_ne!(
            compute_source_digest(b"a", 1),
            compute_source_digest(b"b", 1),
        );
    }

    #[test]
    fn digest_is_64_hex_chars() {
        let digest = compute_source_digest(b"x", 1);
        assert_eq!(digest.len(), 64);
        assert!(digest.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
```

- [ ] **Step 3: Register the module**

In `crates/presenter-importer/src/lib.rs`, add at the top (with other `mod`/`pub use` lines):

```rust
mod bible_digest;
pub use bible_digest::{compute_source_digest, PARSER_VERSION};
```

- [ ] **Step 4: Run the digest tests**

Run: `cargo test -p presenter-importer -- bible_digest --nocapture`
Expected: all four tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/presenter-importer/src/bible_digest.rs crates/presenter-importer/src/lib.rs crates/presenter-importer/Cargo.toml
git commit -m "feat(importer): add SHA-256 source digest with parser version (#243)"
```

---

## Task 8: Integration test — skip on digest match, import on mismatch

**Files:**
- Modify: `crates/presenter-importer/src/bible.rs` (append to existing test module)

- [ ] **Step 1: Write the failing integration tests**

Append these tests to the existing `#[cfg(test)] mod tests` block in `crates/presenter-importer/src/bible.rs`. The block already has `SinglePayloadProvider`, `sample_spec`, `make_usfm_archive`, `Repository`, `BibleTranslation`, `BibleIngestionService`, `BibleSource`, `BibleSourceFormat`, and `BibleTranslationSpec` in scope. The existing `sample_spec()` uses translation code `"en-test"`, which we reuse.

```rust
    #[tokio::test]
    async fn digest_match_skips_import() {
        use crate::{compute_source_digest, PARSER_VERSION};
        use presenter_core::bible::BibleIngestionBatch;

        let repo = Repository::connect_in_memory().await.unwrap();

        // Seed an empty translation row so set_bible_source_digest can update it.
        let seed_translation = BibleTranslation::new("en-test", "Test", "en");
        let seed_batch = BibleIngestionBatch::new(seed_translation, Vec::new())
            .expect("empty batch is valid");
        repo.replace_bible_translation_passages(&seed_batch)
            .await
            .unwrap();

        // Stamp the exact digest the archive would produce.
        let archive = make_usfm_archive();
        let expected_digest = compute_source_digest(&archive, PARSER_VERSION);
        repo.set_bible_source_digest("en-test", &expected_digest)
            .await
            .unwrap();

        // Simulate the skip branch of the ingest binary.
        let stored = repo.get_bible_source_digest("en-test").await.unwrap();
        assert_eq!(
            stored.as_deref(),
            Some(expected_digest.as_str()),
            "stored digest should match candidate",
        );

        // No ingest_with_bytes call was made — passages must still be empty.
        let hits = repo
            .search_bible_passages("en-test", "god", 10)
            .await
            .unwrap();
        assert!(
            hits.is_empty(),
            "passages should remain untouched when digest matches",
        );
    }

    #[tokio::test]
    async fn digest_mismatch_triggers_import() {
        use crate::{compute_source_digest, PARSER_VERSION};
        use presenter_core::bible::BibleIngestionBatch;

        let repo = Repository::connect_in_memory().await.unwrap();

        // Seed with STALE digest
        let seed_batch = BibleIngestionBatch::new(
            BibleTranslation::new("en-test", "Test", "en"),
            Vec::new(),
        )
        .expect("empty batch is valid");
        repo.replace_bible_translation_passages(&seed_batch)
            .await
            .unwrap();
        repo.set_bible_source_digest("en-test", "stale-digest-deadbeef")
            .await
            .unwrap();

        // Ingest a real archive — should run because digest mismatches.
        let archive = make_usfm_archive();
        let provider = SinglePayloadProvider {
            payload: archive.clone(),
            url: "memory".to_string(),
        };
        let service = BibleIngestionService::new(&repo, provider);

        let _ = service
            .ingest_with_bytes(&sample_spec(), &archive)
            .await
            .unwrap();
        let fresh_digest = compute_source_digest(&archive, PARSER_VERSION);
        repo.set_bible_source_digest("en-test", &fresh_digest)
            .await
            .unwrap();

        // After import, FTS search should find at least one passage.
        let hits = repo
            .search_bible_passages("en-test", "god", 10)
            .await
            .unwrap();
        assert!(
            !hits.is_empty(),
            "import should have populated passages from the archive",
        );
        assert_eq!(
            repo.get_bible_source_digest("en-test").await.unwrap(),
            Some(fresh_digest),
        );
    }
```

- [ ] **Step 2: Run the tests**

Run: `cargo test -p presenter-importer -- digest_match_skips_import digest_mismatch_triggers_import --nocapture`
Expected: both tests PASS. If they fail, the failure points directly at whichever assertion is wrong — fix the setup (not the assertion).

- [ ] **Step 3: Commit**

```bash
git add crates/presenter-importer/src/bible.rs
git commit -m "test(importer): integration tests for digest skip and mismatch (#243)"
```

---

## Task 9: Wire digest check into the `ingest_bibles` binary

**Files:**
- Modify: `crates/presenter-importer/src/bin/ingest_bibles.rs`

- [ ] **Step 1: Rewrite the binary**

Replace the entire file with:

```rust
use anyhow::Context;
use presenter_bible::{default_translation_specs, read_local_file_bytes};
use presenter_importer::bible::BibleIngestionService;
use presenter_importer::{compute_source_digest, PARSER_VERSION};
use presenter_persistence::{DatabaseSettings, Repository};
use std::env;

const DEFAULT_DB_URL: &str = "sqlite://presenter_dev.db";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let db_url = env::var("PRESENTER_DB_URL").unwrap_or_else(|_| DEFAULT_DB_URL.to_string());
    let repository = Repository::connect(&DatabaseSettings::new(&db_url))
        .await
        .with_context(|| format!("failed to connect to database at {db_url}"))?;
    let ingestion = BibleIngestionService::with_http(&repository)?;

    // Only specs whose source is actually available (URL always, LocalFile when env var set)
    let all_specs = default_translation_specs();
    let available_specs: Vec<_> = all_specs
        .into_iter()
        .filter(|spec| {
            if spec.source.is_available() {
                true
            } else {
                println!(
                    "Skipping {} (local file not configured)",
                    spec.translation.code
                );
                false
            }
        })
        .collect();

    let mut imported = 0usize;
    let mut skipped = 0usize;

    for spec in available_specs {
        let code = spec.translation.code.clone();

        // Read the source bytes ourselves so we can hash once.
        let bytes = match read_local_file_bytes(&spec) {
            Ok(b) => b,
            Err(err) => {
                // URL-based specs (none today in production) — fall back to normal ingest,
                // which fetches via the scraper. No digest optimization for those.
                println!(
                    "read_local_file_bytes failed for {code} ({err}); falling back to network ingest",
                );
                let summary = ingestion
                    .ingest(&spec)
                    .await
                    .with_context(|| format!("bible ingestion failed for {code}"))?;
                println!(
                    "Imported {count} passages for translation {code}",
                    count = summary.passage_count,
                    code = summary.translation_code,
                );
                imported += 1;
                continue;
            }
        };

        let candidate_digest = compute_source_digest(&bytes, PARSER_VERSION);
        let stored_digest = repository
            .get_bible_source_digest(&code)
            .await
            .with_context(|| format!("failed to query stored digest for {code}"))?;

        if stored_digest.as_ref() == Some(&candidate_digest) {
            println!(
                "Skipping {code} (up-to-date, digest {short})",
                short = &candidate_digest[..12],
            );
            skipped += 1;
            continue;
        }

        let summary = ingestion
            .ingest_with_bytes(&spec, &bytes)
            .await
            .with_context(|| format!("bible ingestion failed for {code}"))?;
        repository
            .set_bible_source_digest(&code, &candidate_digest)
            .await
            .with_context(|| format!("failed to persist digest for {code}"))?;
        println!(
            "Imported {count} passages for translation {code} (digest {short})",
            count = summary.passage_count,
            code = summary.translation_code,
            short = &candidate_digest[..12],
        );
        imported += 1;
    }

    println!(
        "Bible import done: {imported} imported, {skipped} skipped (up-to-date)."
    );

    Ok(())
}
```

- [ ] **Step 2: Build the binary**

Run: `cargo build -p presenter-importer --bin ingest_bibles`
Expected: compiles clean.

- [ ] **Step 3: Run a real smoke test against the dev DB**

Only run this if `/opt/presenter-dev/presenter.db` is writeable and has bibles imported already. Stop the dev server first to avoid DB lock:

```bash
sudo systemctl stop presenter-dev
PRESENTER_DB_URL="sqlite:///opt/presenter-dev/presenter.db" \
  PRESENTER_BIBLE_KJV="/opt/presenter-dev/bibles/kjv.usfm.zip" \
  PRESENTER_BIBLE_SEB="/opt/presenter-dev/bibles/seb.bbl.mybible.zip" \
  PRESENTER_BIBLE_ROHACEK="/opt/presenter-dev/bibles/rohacek.bbl.mybible.zip" \
  PRESENTER_BIBLE_SEVP="/opt/presenter-dev/bibles/sevp.obohu.mybible.zip" \
  PRESENTER_BIBLE_MILOST="/opt/presenter-dev/bibles/milost.bbl.mybible.zip" \
  time ./target/debug/ingest_bibles
sudo systemctl start presenter-dev
```

Expected: first run imports everything in <2 min (fast path). Second run prints "Skipping ... (up-to-date)" for every translation in <5 s.

> If the dev DB doesn't have the bible env vars wired into systemd or a local profile, skip this step and rely on CI to prove it.

- [ ] **Step 4: Commit**

```bash
git add crates/presenter-importer/src/bin/ingest_bibles.rs
git commit -m "feat(importer): skip bible import when source digest unchanged (#243)"
```

---

## Task 10: Version bump, fmt, clippy, push, monitor CI

- [ ] **Step 1: Bump workspace version**

In `Cargo.toml`, bump `[workspace.package].version` — e.g., `0.4.20` → `0.4.21`.

Run `cargo check -p presenter-server` so `Cargo.lock` picks up the new versions.

- [ ] **Step 2: Local checks**

```bash
cargo fmt --all
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
```

Fix any findings.

- [ ] **Step 3: Commit version bump**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore: bump version for bible import speedup (#243)"
```

- [ ] **Step 4: Push**

```bash
git push origin dev
```

- [ ] **Step 5: Monitor CI**

```bash
gh run list --branch dev --limit 3
```

Watch the Pipeline run. The `Deploy to Dev` step runs `Import Bible translations`. On this run the digest column is new, so bibles will re-import (fast path) and store digests. Expected wall time for that step: **<2 min** (down from ~40 min). On the *next* dev push after this, the same step should log "Skipping ... (up-to-date)" for every translation in seconds.

If any job fails, `gh run view <id> --log-failed`, fix, commit, push again.

---

## Verification Summary

| Check | How to verify |
|-------|---------------|
| Migration applied | `sqlite3 /opt/presenter-dev/presenter.db "PRAGMA table_info(bible_translations)"` lists `source_digest` |
| Digest written | `sqlite3 /opt/presenter-dev/presenter.db "SELECT code, substr(source_digest,1,12) FROM bible_translations"` shows non-NULL values after first import |
| FTS search works | Dev operator bible search returns results (existing E2E tests cover this) |
| Fast import under 2 min | `time ./target/debug/ingest_bibles` after clearing digest — watch wall time |
| Skip under 5 s | Run the binary twice in a row — second run prints "Skipping" for each code |
| CI deploy faster | Compare `Import Bible translations` step duration in the next two dev pipeline runs |
| No regression | `cargo test -p presenter-persistence -p presenter-importer -p presenter-bible` all pass |


