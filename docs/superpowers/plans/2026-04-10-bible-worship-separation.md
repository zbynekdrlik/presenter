# Bible/Worship Separation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fully separate bible content from worship content at the storage, domain, repository, broadcasting, and AI-tools layers, eliminating field overloading and string-based type discrimination.

**Architecture:** Add `bible_presentations` and `bible_slides` tables (no library wrapper since there's only one bible per system). Drop the bible_* columns from the worship `slides` table and the dead `category` column from `libraries`. Drop any existing bible library row outright (user explicitly OK with dropping the 2 production presentations). Add new `BiblePresentation` / `BibleSlide` Rust types with their own newtype IDs. Replace all `eq_ignore_ascii_case("Bible")` lookups with proper repository methods. Delete the broadcasting leak that emits `BibleUpdate` based on the worship `stage` field.

**Tech Stack:** Rust (sea-orm, sea-orm-migration, axum, tokio), Leptos WASM (presenter-ui), Playwright E2E

**Spec:** `docs/superpowers/specs/2026-04-10-bible-worship-separation-design.md`

---

## Context

Issue #231: Bible and worship currently share storage and broadcasting code. Symptoms:

1. **Bible library shows in worship UI list** (#227 — pre-existing since v0.1.2)
2. **Field overloading**: worship `stage` field is reused for bible reference labels, causing worship slides with non-empty stage text to trigger spurious `BibleUpdate` broadcasts to Resolume
3. **String matching** for type identification in 4 code locations (`state/bible.rs:204, 230`, `ai/tools.rs:375, 685`)
4. **Dead `category` column** on `libraries` from an aborted earlier separation attempt

User chose a **single big-bang PR** approach (per spec). Existing bible presentations on production are dropped (user confirmed).

**Key existing code:**
- `crates/presenter-migration/src/m20250927_000001_create_core_tables.rs` — original schema creating slides with 14 columns + dead category column
- `crates/presenter-migration/src/m20260408_000001_add_preach_limit.rs` — example of an idempotent ALTER migration with `pragma_table_info` column-existence check
- `crates/presenter-persistence/src/entities.rs:106-155` — `slide::Model` with 7 worship + 7 bible + metadata_json fields
- `crates/presenter-persistence/src/repository/util.rs:66, 371` — `is_bible = !model.bible_main.is_empty()` content inspection
- `crates/presenter-server/src/state/bible.rs:198-244` — string-based bible library lookups
- `crates/presenter-server/src/state/broadcasting.rs:83-96` — the BibleUpdate-from-stage-field leak
- `crates/presenter-server/src/ai/tools.rs:375, 685` — magic string checks
- `crates/presenter-core/src/bible.rs` — currently empty / not present; will create

---

## File Structure

### New Files
| File | Purpose |
|------|---------|
| `crates/presenter-migration/src/m20260410_000001_separate_bible.rs` | Schema migration: add bible_presentations + bible_slides tables, drop the bible library row, drop bible_* columns from slides, drop category column from libraries |
| `crates/presenter-core/src/bible.rs` | Domain types: `BiblePresentation`, `BibleSlide`, `BiblePresentationId`, `BibleSlideId`, `BiblePresentationSummary` |
| `crates/presenter-persistence/src/repository/bible.rs` | Repository methods for bible presentations and slides |
| `crates/presenter-persistence/src/entities/bible_presentation.rs` (or inline in entities.rs) | sea-orm entity for `bible_presentations` |
| `crates/presenter-persistence/src/entities/bible_slide.rs` (or inline in entities.rs) | sea-orm entity for `bible_slides` |

### Modified Files
| File | Change |
|------|--------|
| `crates/presenter-migration/src/lib.rs` | Register the new migration |
| `crates/presenter-persistence/src/entities.rs` | Add `bible_presentation` and `bible_slide` entity modules; remove the 7 bible_* fields and `metadata_json` from `slide::Model` |
| `crates/presenter-persistence/src/repository/mod.rs` | Re-export new bible methods |
| `crates/presenter-persistence/src/repository/util.rs` | Drop the `is_bible` content inspection at lines 66 and 371; simplify worship slide conversion |
| `crates/presenter-core/src/lib.rs` | Re-export `bible` module types; document the unprefixed-means-worship convention |
| `crates/presenter-server/src/state/bible.rs` | Replace string-based library lookups (lines 198-244) with new repository methods; remove `create_bible_presentation`'s ensure-library branch |
| `crates/presenter-server/src/state/broadcasting.rs` | Delete the BibleUpdate-from-stage-field leak (lines 83-96) |
| `crates/presenter-server/src/router/bible.rs` | Update handlers to use new `BiblePresentation`/`BibleSlide` types |
| `crates/presenter-server/src/ai/tools.rs` | Replace magic-string checks at lines 375 and 685 |
| `tests/e2e/bible-presentation-append.spec.ts` | Verify still passing after changes |
| `tests/e2e/bible-trigger-slide.spec.ts` | Verify still passing after changes |
| New: regression test E2E file or new test in existing file | Worship library list does NOT contain Bible after a bible presentation is created |
| `Cargo.toml` | Version bump 0.4.14 → 0.4.15 |

---

## Task 1: Schema Migration

**Files:**
- Create: `crates/presenter-migration/src/m20260410_000001_separate_bible.rs`
- Modify: `crates/presenter-migration/src/lib.rs`

- [ ] **Step 1: Create the migration file**

Create `crates/presenter-migration/src/m20260410_000001_separate_bible.rs` with this exact content:

```rust
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // 1. Create bible_presentations table (idempotent)
        db.execute(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            r#"CREATE TABLE IF NOT EXISTS "bible_presentations" (
                "id" varchar(36) NOT NULL PRIMARY KEY,
                "name" varchar NOT NULL,
                "created_at" timestamp_with_timezone_text NOT NULL DEFAULT CURRENT_TIMESTAMP
            )"#,
        ))
        .await?;

        // 2. Create bible_slides table (idempotent)
        db.execute(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            r#"CREATE TABLE IF NOT EXISTS "bible_slides" (
                "id" varchar(36) NOT NULL PRIMARY KEY,
                "presentation_id" varchar(36) NOT NULL,
                "slide_order" integer NOT NULL,
                "main_text" text NOT NULL,
                "main_search" text NOT NULL DEFAULT '',
                "main_reference" text NOT NULL,
                "secondary_text" text NOT NULL DEFAULT '',
                "secondary_search" text NOT NULL DEFAULT '',
                "secondary_reference" text NOT NULL DEFAULT '',
                "metadata_json" text,
                FOREIGN KEY ("presentation_id") REFERENCES "bible_presentations"("id") ON DELETE CASCADE
            )"#,
        ))
        .await?;

        // 3. Index on presentation_id for slide lookups (idempotent)
        db.execute(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            r#"CREATE INDEX IF NOT EXISTS "idx_bible_slides_presentation_id"
               ON "bible_slides" ("presentation_id")"#,
        ))
        .await?;

        // 4. Delete any existing bible library row + cascade-delete its
        //    presentations and slides. User explicitly confirmed this is OK.
        //    SQLite cascades through the existing FKs.
        db.execute(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            r#"DELETE FROM "libraries" WHERE LOWER("name") = 'bible'"#,
        ))
        .await?;

        // 5. Drop the dead category column from libraries (guarded).
        if column_exists(db, "libraries", "category").await? {
            db.execute(sea_orm::Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                r#"ALTER TABLE "libraries" DROP COLUMN "category""#,
            ))
            .await?;
        }

        // 6. Drop the bible_* columns and the metadata_json column from slides.
        for col in [
            "bible_main",
            "bible_main_search",
            "bible_main_reference",
            "bible_translation",
            "bible_translation_search",
            "bible_translation_reference",
            "metadata_json",
        ] {
            if column_exists(db, "slides", col).await? {
                db.execute(sea_orm::Statement::from_string(
                    sea_orm::DatabaseBackend::Sqlite,
                    format!(r#"ALTER TABLE "slides" DROP COLUMN "{col}""#),
                ))
                .await?;
            }
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // Re-add the dropped columns to slides (with empty defaults).
        for (col, sql_type) in [
            ("bible_main", "text NOT NULL DEFAULT ''"),
            ("bible_main_search", "text NOT NULL DEFAULT ''"),
            ("bible_main_reference", "text NOT NULL DEFAULT ''"),
            ("bible_translation", "text NOT NULL DEFAULT ''"),
            ("bible_translation_search", "text NOT NULL DEFAULT ''"),
            ("bible_translation_reference", "text NOT NULL DEFAULT ''"),
            ("metadata_json", "text"),
        ] {
            if !column_exists(db, "slides", col).await? {
                db.execute(sea_orm::Statement::from_string(
                    sea_orm::DatabaseBackend::Sqlite,
                    format!(r#"ALTER TABLE "slides" ADD COLUMN "{col}" {sql_type}"#),
                ))
                .await?;
            }
        }

        // Re-add the dead category column.
        if !column_exists(db, "libraries", "category").await? {
            db.execute(sea_orm::Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                r#"ALTER TABLE "libraries" ADD COLUMN "category" varchar(32) NOT NULL DEFAULT 'worship'"#,
            ))
            .await?;
        }

        // Drop the new bible tables.
        db.execute(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            r#"DROP TABLE IF EXISTS "bible_slides""#,
        ))
        .await?;
        db.execute(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            r#"DROP TABLE IF EXISTS "bible_presentations""#,
        ))
        .await?;

        Ok(())
    }
}

async fn column_exists(
    db: &sea_orm::DatabaseConnection,
    table: &str,
    column: &str,
) -> Result<bool, DbErr> {
    let row = db
        .query_one(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            format!(
                "SELECT COUNT(*) as cnt FROM pragma_table_info('{table}') WHERE name='{column}'"
            ),
        ))
        .await?;
    Ok(row
        .map(|r| r.try_get::<i32>("", "cnt").unwrap_or(0) > 0)
        .unwrap_or(false))
}
```

- [ ] **Step 2: Register the migration**

In `crates/presenter-migration/src/lib.rs`, add the new migration to the module list and `migrations()` vec:

```rust
pub use sea_orm_migration::prelude::*;

mod m20250927_000001_create_core_tables;
mod m20260408_000001_add_preach_limit;
mod m20260410_000001_separate_bible;

pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20250927_000001_create_core_tables::Migration),
            Box::new(m20260408_000001_add_preach_limit::Migration),
            Box::new(m20260410_000001_separate_bible::Migration),
        ]
    }
}
```

- [ ] **Step 3: Run cargo check on the migration crate**

```bash
cargo check -p presenter-migration 2>&1 | tail -10
```

Expected: clean build, no errors.

- [ ] **Step 4: Test the migration against a fresh DB**

```bash
rm -f /tmp/migration-test.db
PRESENTER_DB_URL=sqlite:///tmp/migration-test.db cargo run -p presenter-server -- --migrations-only 2>&1 | tail -20
```

If `--migrations-only` doesn't exist, instead:

```bash
rm -f /tmp/migration-test.db
PRESENTER_DB_URL=sqlite:///tmp/migration-test.db timeout 5 cargo run -p presenter-server 2>&1 | grep -i 'migrat\|error' | head -10
```

Then verify the schema:

```bash
sqlite3 /tmp/migration-test.db "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name" | grep bible
```

Expected output includes `bible_presentations` and `bible_slides`. Should NOT include any "category" column on libraries:

```bash
sqlite3 /tmp/migration-test.db ".schema libraries" | grep -c category
```

Expected: `0`

- [ ] **Step 5: Test the migration against a copy of the dev DB**

```bash
cp /opt/presenter-dev/presenter.db /tmp/migration-prod-copy.db
PRESENTER_DB_URL=sqlite:///tmp/migration-prod-copy.db timeout 10 cargo run -p presenter-server 2>&1 | grep -iE 'migrat|error|fatal' | head -15
```

Expected: migration runs, server starts, no errors. Verify the bible library row is gone:

```bash
sqlite3 /tmp/migration-prod-copy.db "SELECT name FROM libraries WHERE LOWER(name) = 'bible'"
```

Expected: empty output.

Verify bible_* columns are gone from slides:

```bash
sqlite3 /tmp/migration-prod-copy.db ".schema slides" | grep -c bible_
```

Expected: `0`

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/presenter-migration/src/m20260410_000001_separate_bible.rs crates/presenter-migration/src/lib.rs
git commit -m "feat(migration): add separate_bible migration for #231

Creates bible_presentations and bible_slides tables. Drops the
dead category column from libraries. Drops the 7 bible_* columns
and metadata_json from slides. Deletes any existing bible library
row (user explicitly confirmed dropping the 2 production bible
presentations).

The migration is idempotent: tables use IF NOT EXISTS, column
drops are guarded by pragma_table_info checks following the
pattern from m20260408_000001_add_preach_limit."
```

---

## Task 2: Bible Domain Types

**Files:**
- Create: `crates/presenter-core/src/bible.rs`
- Modify: `crates/presenter-core/src/lib.rs`

- [ ] **Step 1: Check what already exists in presenter-core for bible**

```bash
ls crates/presenter-core/src/ | grep -i bible
```

If `bible.rs` already exists, read it first to know what to extend. The existing module likely has `BibleSlideOutput`, `BibleSlideMetadata`, `BibleSlideVerseRef` etc. — leave those alone, just add the new types alongside.

- [ ] **Step 2: Add the new types to `crates/presenter-core/src/bible.rs`**

Append (or create) with the following content. Adjust imports if the file already exists:

```rust
use crate::slide::{SlideText, SlideTextError};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Identifier for a `BiblePresentation`. Distinct from `PresentationId` so
/// the type system prevents mixing worship and bible IDs at API boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BiblePresentationId(pub Uuid);

impl BiblePresentationId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn from_uuid(id: Uuid) -> Self {
        Self(id)
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for BiblePresentationId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for BiblePresentationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Identifier for a `BibleSlide`. Distinct from `SlideId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BibleSlideId(pub Uuid);

impl BibleSlideId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn from_uuid(id: Uuid) -> Self {
        Self(id)
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for BibleSlideId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for BibleSlideId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// A user-curated collection of bible slides (e.g., for a sermon series).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BiblePresentation {
    pub id: BiblePresentationId,
    pub name: String,
    pub slides: Vec<BibleSlide>,
    pub created_at: DateTime<Utc>,
}

/// A single bible slide within a `BiblePresentation`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BibleSlide {
    pub id: BibleSlideId,
    pub order: u32,
    pub main: SlideText,
    pub main_reference: String,
    pub secondary: SlideText,
    pub secondary_reference: String,
    pub metadata: Option<BibleSlideMetadata>,
}

/// Lightweight summary of a bible presentation for list views.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BiblePresentationSummary {
    pub id: BiblePresentationId,
    pub name: String,
    pub slide_count: usize,
}

/// Bible-specific metadata stored as JSON in the database.
///
/// This carries reference info that worship slides do not have:
/// translation codes, book/chapter/verse breakdown, etc.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BibleSlideMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translation_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secondary_translation_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub book: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub book_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub book_number: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter: Option<i32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub verses: Vec<BibleSlideVerseRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub main_reference_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translation_reference_label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BibleSlideVerseRef {
    pub start: i32,
    pub end: i32,
}
```

**IMPORTANT:** If `BibleSlideMetadata` and `BibleSlideVerseRef` already exist in this file or in `crates/presenter-core/src/slide.rs`, do NOT redefine them — re-export the existing definitions instead. Use grep to check first:

```bash
grep -rn "struct BibleSlideMetadata\|struct BibleSlideVerseRef" crates/presenter-core/src/
```

If they exist elsewhere, use:

```rust
pub use crate::slide::{BibleSlideMetadata, BibleSlideVerseRef};
```

at the top of `bible.rs` instead of redefining.

- [ ] **Step 3: Re-export from `crates/presenter-core/src/lib.rs`**

Find the existing `pub mod bible;` (or similar) line. If `bible.rs` is a new module, add:

```rust
pub mod bible;
```

And re-export the new types alongside existing exports:

```rust
pub use bible::{
    BiblePresentation, BiblePresentationId, BiblePresentationSummary, BibleSlide,
    BibleSlideId, BibleSlideMetadata, BibleSlideVerseRef,
};
```

**Convention comment:** add this near the top of `lib.rs`:

```rust
// Naming convention:
// - Unprefixed types (Library, Presentation, Slide, SlideContent) mean WORSHIP.
// - Bible-prefixed types (BiblePresentation, BibleSlide) mean BIBLE.
// Bible has no library wrapper — there is exactly one bible per system.
```

- [ ] **Step 4: Add unit tests for the new ID types**

In the same `bible.rs` file, append:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bible_presentation_id_roundtrips_uuid() {
        let original = Uuid::new_v4();
        let id = BiblePresentationId::from_uuid(original);
        assert_eq!(id.as_uuid(), original);
    }

    #[test]
    fn bible_slide_id_roundtrips_uuid() {
        let original = Uuid::new_v4();
        let id = BibleSlideId::from_uuid(original);
        assert_eq!(id.as_uuid(), original);
    }

    #[test]
    fn bible_presentation_id_serializes_as_uuid_string() {
        let id = BiblePresentationId::from_uuid(
            Uuid::parse_str("01234567-89ab-cdef-0123-456789abcdef").unwrap(),
        );
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, r#""01234567-89ab-cdef-0123-456789abcdef""#);
    }

    #[test]
    fn bible_presentation_serialization_uses_camel_case() {
        let pres = BiblePresentation {
            id: BiblePresentationId::from_uuid(Uuid::nil()),
            name: "Test".to_string(),
            slides: vec![],
            created_at: chrono::DateTime::<Utc>::from_timestamp(0, 0).unwrap(),
        };
        let json = serde_json::to_string(&pres).unwrap();
        assert!(json.contains(r#""createdAt""#));
        assert!(!json.contains(r#""created_at""#));
    }
}
```

- [ ] **Step 5: Run the tests**

```bash
cargo test -p presenter-core --lib bible:: 2>&1 | tail -15
```

Expected: 4 new tests pass.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/presenter-core/src/bible.rs crates/presenter-core/src/lib.rs
git commit -m "feat(core): add BiblePresentation and BibleSlide domain types (#231)

Adds BiblePresentation, BibleSlide, BiblePresentationId, BibleSlideId,
and BiblePresentationSummary types to crates/presenter-core/src/bible.rs.
The new IDs are newtype wrappers that are NOT compatible with the
existing PresentationId/SlideId, so the type system prevents mixing
worship and bible IDs at API boundaries.

Documents the convention in lib.rs: unprefixed types mean worship,
Bible-prefixed types mean bible. Bible has no library wrapper since
there's exactly one bible per system.

Includes 4 unit tests covering ID round-trip, JSON serialization,
and camelCase output."
```

---

## Task 3: Bible sea-orm Entities

**Files:**
- Modify: `crates/presenter-persistence/src/entities.rs`

- [ ] **Step 1: Add bible_presentation entity module**

In `crates/presenter-persistence/src/entities.rs`, add this module after the existing `bible_passage` module (around line 406):

```rust
pub mod bible_presentation {
    use super::bible_slide;
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "bible_presentations")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub name: String,
        pub created_at: DateTimeWithTimeZone,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {
        #[sea_orm(has_many = "bible_slide::Entity")]
        Slides,
    }

    impl Related<bible_slide::Entity> for Entity {
        fn to() -> RelationDef {
            Relation::Slides.def()
        }
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod bible_slide {
    use super::bible_presentation;
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "bible_slides")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub presentation_id: String,
        pub slide_order: i32,
        pub main_text: String,
        pub main_search: String,
        pub main_reference: String,
        pub secondary_text: String,
        pub secondary_search: String,
        pub secondary_reference: String,
        pub metadata_json: Option<String>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {
        #[sea_orm(
            belongs_to = "bible_presentation::Entity",
            from = "Column::PresentationId",
            to = "bible_presentation::Column::Id",
            on_update = "Cascade",
            on_delete = "Cascade"
        )]
        Presentation,
    }

    impl Related<bible_presentation::Entity> for Entity {
        fn to() -> RelationDef {
            Relation::Presentation.def()
        }
    }

    impl ActiveModelBehavior for ActiveModel {}
}
```

- [ ] **Step 2: Re-export the new entities**

Find the existing `pub use ... as ...Entity;` block (around line 408-415) and add:

```rust
pub use bible_presentation::Entity as BiblePresentationEntity;
pub use bible_slide::Entity as BibleSlideEntity;
```

- [ ] **Step 3: Remove bible_* fields from `slide::Model`**

Find `pub mod slide` (around line 106). Replace the entire `Model` struct definition (lines 110-134) so that the bible_* and metadata_json fields are removed:

```rust
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "slides")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub presentation_id: String,
        pub position: i32,
        // Worship columns
        pub worship_main: String,
        pub worship_main_search: String,
        pub worship_translate: String,
        pub worship_translate_search: String,
        pub worship_stage: String,
        pub worship_stage_search: String,
        pub worship_group: Option<String>,
        pub created_at: DateTimeWithTimeZone,
    }
```

- [ ] **Step 4: Build to find broken references**

```bash
cargo check -p presenter-persistence 2>&1 | tail -30
```

Expected: errors in `repository/util.rs` (and possibly other files) referencing the now-removed fields. **Do NOT fix these in this task** — they're handled in Task 4. Note the file/line of each error for reference.

- [ ] **Step 5: Commit (allowing repository to be temporarily broken)**

```bash
cargo fmt --all
git add crates/presenter-persistence/src/entities.rs
git commit -m "feat(persistence): add bible_presentation/bible_slide entities, drop bible_* from slide (#231)

Adds sea-orm entity definitions for bible_presentations and
bible_slides tables. Removes the 7 bible_* fields and metadata_json
from slide::Model since those columns are dropped by the new
migration.

This commit temporarily breaks repository/util.rs which still
references the removed fields — fixed in the next task. Done as
two commits so the entity change and the repo cleanup are
reviewable independently."
```

**Note:** It's OK that this commit leaves the workspace not compiling. The next task fixes it. We commit anyway because the changes are independently meaningful.

---

## Task 4: Repository Cleanup + Bible Repository Methods

**Files:**
- Modify: `crates/presenter-persistence/src/repository/util.rs`
- Create: `crates/presenter-persistence/src/repository/bible.rs`
- Modify: `crates/presenter-persistence/src/repository/mod.rs`

- [ ] **Step 1: Read util.rs to find all bible references**

```bash
grep -n "bible_\|is_bible\|metadata_json\|BibleSlide\|MetadataJson" crates/presenter-persistence/src/repository/util.rs
```

Make a list of every line that references bible-specific fields. These all need to be cleaned up.

- [ ] **Step 2: Simplify `to_domain_slide` in util.rs**

Around line 64, the current `to_domain_slide` function checks `is_bible = !model.bible_main.is_empty()` and branches on it. Replace the entire function so it ONLY handles worship slides:

Open `crates/presenter-persistence/src/repository/util.rs`, find the `to_domain_slide` function and rewrite it. The function should:
1. Take a `slide::Model` reference
2. Build a `Slide` from the worship_* fields only
3. NOT check `is_bible` and NOT touch any bible_* fields (which no longer exist)

Concrete pattern (adapt to the actual surrounding code; read the current function before editing):

```rust
pub(crate) fn to_domain_slide(model: &slide::Model) -> anyhow::Result<Slide> {
    let main = SlideText::new(&model.worship_main)
        .map_err(|e| anyhow::anyhow!("invalid worship_main: {e}"))?;
    let translation = SlideText::new(&model.worship_translate)
        .map_err(|e| anyhow::anyhow!("invalid worship_translate: {e}"))?;
    let stage = SlideText::new(&model.worship_stage)
        .map_err(|e| anyhow::anyhow!("invalid worship_stage: {e}"))?;
    let group = model
        .worship_group
        .as_ref()
        .map(|name| SlideGroup::new(name.clone()))
        .transpose()
        .map_err(|e| anyhow::anyhow!("invalid worship_group: {e}"))?;
    let content = SlideContent::new(main, translation, stage, group);
    Ok(Slide {
        id: SlideId::from_uuid(parse_uuid(&model.id)?),
        order: model.position as u32,
        content,
    })
}
```

The exact field initialization may vary based on the current `Slide` struct. **Read the current function before editing** and preserve any non-bible logic (e.g., metadata, timestamps, error handling) that exists.

- [ ] **Step 3: Simplify `build_slide_active_model` in util.rs**

Around line 371, the same file has `build_slide_active_model` which checks `is_bible` based on metadata. Replace it so it only writes worship_* columns. The function should:
1. Take a `Slide` (worship slide)
2. Build a `slide::ActiveModel` with worship_main, worship_main_search, worship_translate, worship_translate_search, worship_stage, worship_stage_search, worship_group set
3. NOT touch any bible_* fields (they don't exist anymore)
4. NOT touch metadata_json (column dropped)

Read the current function first to preserve its non-bible behavior (id generation, search field computation, etc.).

- [ ] **Step 4: Build to verify util.rs compiles**

```bash
cargo check -p presenter-persistence 2>&1 | tail -20
```

Expected: clean build OR errors only in OTHER files (state/bible.rs, etc.) that we'll fix in later tasks.

If util.rs itself still has errors (e.g., unused imports), fix them before proceeding:

```bash
cargo check -p presenter-persistence 2>&1 | grep "src/repository/util.rs"
```

- [ ] **Step 5: Create bible.rs repository file**

Create `crates/presenter-persistence/src/repository/bible.rs`:

```rust
use crate::entities::{bible_presentation, bible_slide};
use crate::repository::Repository;
use anyhow::{Context, Result};
use chrono::Utc;
use presenter_core::{
    BiblePresentation, BiblePresentationId, BiblePresentationSummary, BibleSlide, BibleSlideId,
    BibleSlideMetadata, SlideText,
};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};

impl Repository {
    /// Returns lightweight summaries of all bible presentations, ordered by name.
    pub async fn list_bible_presentation_summaries(
        &self,
    ) -> Result<Vec<BiblePresentationSummary>> {
        let presentations = bible_presentation::Entity::find()
            .order_by_asc(bible_presentation::Column::Name)
            .all(&self.db)
            .await
            .context("listing bible presentations")?;

        let mut summaries = Vec::with_capacity(presentations.len());
        for p in presentations {
            let slide_count = bible_slide::Entity::find()
                .filter(bible_slide::Column::PresentationId.eq(&p.id))
                .count(&self.db)
                .await
                .context("counting slides for bible presentation")?
                as usize;
            summaries.push(BiblePresentationSummary {
                id: BiblePresentationId::from_uuid(parse_uuid(&p.id)?),
                name: p.name,
                slide_count,
            });
        }
        Ok(summaries)
    }

    /// Fetches a single bible presentation with all its slides.
    pub async fn fetch_bible_presentation(
        &self,
        id: BiblePresentationId,
    ) -> Result<Option<BiblePresentation>> {
        let id_str = id.to_string();
        let Some(p) = bible_presentation::Entity::find_by_id(&id_str)
            .one(&self.db)
            .await
            .context("fetching bible presentation")?
        else {
            return Ok(None);
        };

        let slide_models = bible_slide::Entity::find()
            .filter(bible_slide::Column::PresentationId.eq(&id_str))
            .order_by_asc(bible_slide::Column::SlideOrder)
            .all(&self.db)
            .await
            .context("fetching bible slides")?;

        let mut slides = Vec::with_capacity(slide_models.len());
        for m in slide_models {
            slides.push(model_to_bible_slide(m)?);
        }

        Ok(Some(BiblePresentation {
            id,
            name: p.name,
            slides,
            created_at: p.created_at.with_timezone(&Utc),
        }))
    }

    /// Creates a new empty bible presentation.
    pub async fn create_bible_presentation(&self, name: &str) -> Result<BiblePresentation> {
        let id = BiblePresentationId::new();
        let now = Utc::now();
        let model = bible_presentation::ActiveModel {
            id: Set(id.to_string()),
            name: Set(name.to_string()),
            created_at: Set(now.into()),
        };
        model
            .insert(&self.db)
            .await
            .context("creating bible presentation")?;
        Ok(BiblePresentation {
            id,
            name: name.to_string(),
            slides: vec![],
            created_at: now,
        })
    }

    /// Renames an existing bible presentation.
    pub async fn rename_bible_presentation(
        &self,
        id: BiblePresentationId,
        name: &str,
    ) -> Result<()> {
        let id_str = id.to_string();
        let Some(model) = bible_presentation::Entity::find_by_id(&id_str)
            .one(&self.db)
            .await
            .context("fetching bible presentation for rename")?
        else {
            return Err(anyhow::anyhow!("bible presentation not found"));
        };
        let mut active: bible_presentation::ActiveModel = model.into();
        active.name = Set(name.to_string());
        active
            .update(&self.db)
            .await
            .context("renaming bible presentation")?;
        Ok(())
    }

    /// Deletes a bible presentation and all its slides (cascade).
    pub async fn delete_bible_presentation(&self, id: BiblePresentationId) -> Result<()> {
        let id_str = id.to_string();
        bible_presentation::Entity::delete_by_id(&id_str)
            .exec(&self.db)
            .await
            .context("deleting bible presentation")?;
        Ok(())
    }

    /// Replaces all slides on a bible presentation with the provided ones.
    pub async fn replace_bible_presentation_slides(
        &self,
        id: BiblePresentationId,
        slides: &[BibleSlide],
    ) -> Result<()> {
        let id_str = id.to_string();

        // Verify the presentation exists
        let exists = bible_presentation::Entity::find_by_id(&id_str)
            .one(&self.db)
            .await
            .context("checking bible presentation exists")?
            .is_some();
        if !exists {
            return Err(anyhow::anyhow!("bible presentation not found"));
        }

        // Delete existing slides
        bible_slide::Entity::delete_many()
            .filter(bible_slide::Column::PresentationId.eq(&id_str))
            .exec(&self.db)
            .await
            .context("clearing bible slides")?;

        // Insert new slides
        for slide in slides {
            let active = bible_slide_to_active_model(slide, &id_str)?;
            active
                .insert(&self.db)
                .await
                .context("inserting bible slide")?;
        }
        Ok(())
    }

    /// Appends slides to an existing bible presentation. Returns the
    /// updated presentation with all slides included.
    pub async fn append_bible_presentation_slides(
        &self,
        id: BiblePresentationId,
        slides: &[BibleSlide],
    ) -> Result<BiblePresentation> {
        let id_str = id.to_string();

        // Find current max order
        let max_order = bible_slide::Entity::find()
            .filter(bible_slide::Column::PresentationId.eq(&id_str))
            .order_by_desc(bible_slide::Column::SlideOrder)
            .one(&self.db)
            .await
            .context("finding max slide order")?
            .map(|m| m.slide_order)
            .unwrap_or(-1);

        for (i, slide) in slides.iter().enumerate() {
            let mut renumbered = slide.clone();
            renumbered.order = (max_order + 1 + i as i32) as u32;
            let active = bible_slide_to_active_model(&renumbered, &id_str)?;
            active
                .insert(&self.db)
                .await
                .context("appending bible slide")?;
        }

        self.fetch_bible_presentation(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("bible presentation disappeared"))
    }
}

fn parse_uuid(s: &str) -> Result<uuid::Uuid> {
    uuid::Uuid::parse_str(s).context("parsing UUID")
}

fn model_to_bible_slide(model: bible_slide::Model) -> Result<BibleSlide> {
    let main = SlideText::new(&model.main_text)
        .map_err(|e| anyhow::anyhow!("invalid main_text: {e}"))?;
    let secondary = SlideText::new(&model.secondary_text)
        .map_err(|e| anyhow::anyhow!("invalid secondary_text: {e}"))?;
    let metadata: Option<BibleSlideMetadata> = model
        .metadata_json
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .context("parsing bible slide metadata JSON")?;
    Ok(BibleSlide {
        id: BibleSlideId::from_uuid(parse_uuid(&model.id)?),
        order: model.slide_order as u32,
        main,
        main_reference: model.main_reference,
        secondary,
        secondary_reference: model.secondary_reference,
        metadata,
    })
}

fn bible_slide_to_active_model(
    slide: &BibleSlide,
    presentation_id: &str,
) -> Result<bible_slide::ActiveModel> {
    let metadata_json = slide
        .metadata
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .context("serializing bible slide metadata")?;
    Ok(bible_slide::ActiveModel {
        id: Set(slide.id.to_string()),
        presentation_id: Set(presentation_id.to_string()),
        slide_order: Set(slide.order as i32),
        main_text: Set(slide.main.value().to_string()),
        main_search: Set(normalize_search(slide.main.value())),
        main_reference: Set(slide.main_reference.clone()),
        secondary_text: Set(slide.secondary.value().to_string()),
        secondary_search: Set(normalize_search(slide.secondary.value())),
        secondary_reference: Set(slide.secondary_reference.clone()),
        metadata_json: Set(metadata_json),
    })
}

/// Lower-cased text for search indexing. Matches the convention used by
/// the worship slide repository.
fn normalize_search(text: &str) -> String {
    text.to_lowercase()
}
```

**IMPORTANT:** Look at how the existing worship `repository/slide.rs` (or `repository/util.rs`) computes the search field — it may use a more sophisticated normalization (diacritic stripping, etc.). If so, extract that helper into a shared location or call it from here. Don't reinvent.

```bash
grep -rn "search.*normalize\|normalize_search\|search_text\|fold_to_ascii" crates/presenter-persistence/src/repository/
```

If there's an existing helper, reuse it.

- [ ] **Step 6: Register the new module in `repository/mod.rs`**

Add `pub mod bible;` to `crates/presenter-persistence/src/repository/mod.rs` alongside the other repository submodule declarations.

- [ ] **Step 7: Build the persistence crate**

```bash
cargo check -p presenter-persistence 2>&1 | tail -20
```

Expected: clean build.

- [ ] **Step 8: Add unit tests for the bible repository methods**

In `crates/presenter-persistence/src/repository/bible.rs`, append:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository::Repository;
    use sea_orm::Database;

    async fn fresh_repo() -> Repository {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        // Run migrations
        use presenter_migration::{Migrator, MigratorTrait};
        Migrator::up(&db, None).await.unwrap();
        Repository { db }
    }

    #[tokio::test]
    async fn create_and_fetch_bible_presentation() {
        let repo = fresh_repo().await;
        let created = repo.create_bible_presentation("My Sermon").await.unwrap();
        assert_eq!(created.name, "My Sermon");
        assert!(created.slides.is_empty());

        let fetched = repo
            .fetch_bible_presentation(created.id)
            .await
            .unwrap()
            .expect("should exist");
        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.name, "My Sermon");
    }

    #[tokio::test]
    async fn list_bible_presentation_summaries_returns_all() {
        let repo = fresh_repo().await;
        repo.create_bible_presentation("Bravo").await.unwrap();
        repo.create_bible_presentation("Alpha").await.unwrap();
        let list = repo.list_bible_presentation_summaries().await.unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "Alpha");
        assert_eq!(list[1].name, "Bravo");
    }

    #[tokio::test]
    async fn rename_bible_presentation_updates_name() {
        let repo = fresh_repo().await;
        let p = repo.create_bible_presentation("Old").await.unwrap();
        repo.rename_bible_presentation(p.id, "New").await.unwrap();
        let fetched = repo.fetch_bible_presentation(p.id).await.unwrap().unwrap();
        assert_eq!(fetched.name, "New");
    }

    #[tokio::test]
    async fn delete_bible_presentation_removes_it() {
        let repo = fresh_repo().await;
        let p = repo.create_bible_presentation("Doomed").await.unwrap();
        repo.delete_bible_presentation(p.id).await.unwrap();
        assert!(repo.fetch_bible_presentation(p.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn replace_bible_slides_overwrites_existing() {
        let repo = fresh_repo().await;
        let p = repo.create_bible_presentation("Test").await.unwrap();
        let slide = BibleSlide {
            id: BibleSlideId::new(),
            order: 0,
            main: SlideText::new("For God so loved the world").unwrap(),
            main_reference: "John 3:16".to_string(),
            secondary: SlideText::new("").unwrap(),
            secondary_reference: String::new(),
            metadata: None,
        };
        repo.replace_bible_presentation_slides(p.id, &[slide.clone()])
            .await
            .unwrap();
        let fetched = repo.fetch_bible_presentation(p.id).await.unwrap().unwrap();
        assert_eq!(fetched.slides.len(), 1);
        assert_eq!(fetched.slides[0].main_reference, "John 3:16");

        // Replace with empty slides clears them
        repo.replace_bible_presentation_slides(p.id, &[])
            .await
            .unwrap();
        let fetched = repo.fetch_bible_presentation(p.id).await.unwrap().unwrap();
        assert!(fetched.slides.is_empty());
    }

    #[tokio::test]
    async fn append_bible_slides_preserves_order() {
        let repo = fresh_repo().await;
        let p = repo.create_bible_presentation("Test").await.unwrap();
        let slide_a = BibleSlide {
            id: BibleSlideId::new(),
            order: 0,
            main: SlideText::new("First").unwrap(),
            main_reference: "Gen 1:1".to_string(),
            secondary: SlideText::new("").unwrap(),
            secondary_reference: String::new(),
            metadata: None,
        };
        let slide_b = BibleSlide {
            id: BibleSlideId::new(),
            order: 0,
            main: SlideText::new("Second").unwrap(),
            main_reference: "Gen 1:2".to_string(),
            secondary: SlideText::new("").unwrap(),
            secondary_reference: String::new(),
            metadata: None,
        };
        repo.append_bible_presentation_slides(p.id, &[slide_a.clone()])
            .await
            .unwrap();
        let result = repo
            .append_bible_presentation_slides(p.id, &[slide_b.clone()])
            .await
            .unwrap();
        assert_eq!(result.slides.len(), 2);
        assert_eq!(result.slides[0].order, 0);
        assert_eq!(result.slides[1].order, 1);
        assert_eq!(result.slides[0].main_reference, "Gen 1:1");
        assert_eq!(result.slides[1].main_reference, "Gen 1:2");
    }
}
```

**Note:** the test helper `fresh_repo` assumes `Repository` has a public `db` field. If it's private, use whatever constructor pattern the existing repository tests use. Check:

```bash
grep -rn "Repository {" crates/presenter-persistence/src/repository/ | head -5
```

- [ ] **Step 9: Run the new repository tests**

```bash
cargo test -p presenter-persistence repository::bible --lib 2>&1 | tail -20
```

Expected: all 6 tests pass.

- [ ] **Step 10: Run the full persistence test suite to catch regressions**

```bash
cargo test -p presenter-persistence 2>&1 | tail -10
```

Expected: all tests pass (including pre-existing worship repository tests).

- [ ] **Step 11: Commit**

```bash
cargo fmt --all
git add crates/presenter-persistence/src/repository/util.rs crates/presenter-persistence/src/repository/bible.rs crates/presenter-persistence/src/repository/mod.rs
git commit -m "feat(persistence): add bible repository methods, drop is_bible inspection (#231)

- New repository/bible.rs with 7 methods:
  list_bible_presentation_summaries, fetch_bible_presentation,
  create_bible_presentation, rename_bible_presentation,
  delete_bible_presentation, replace_bible_presentation_slides,
  append_bible_presentation_slides

- repository/util.rs: simplified to_domain_slide and
  build_slide_active_model. Both now handle worship-only since the
  bible_* columns are gone. Removed the is_bible content
  inspection.

- 6 new unit tests against an in-memory sqlite database covering
  CRUD + replace + append for bible presentations and slides."
```

---

## Task 5: State Layer — Replace String Lookups in bible.rs

**Files:**
- Modify: `crates/presenter-server/src/state/bible.rs`

- [ ] **Step 1: Read the current bible state file**

```bash
wc -l crates/presenter-server/src/state/bible.rs
```

If it's large, focus on these specific functions (find them by name):
- `list_bible_presentations` (~line 198)
- `bible_presentation_detail` (~line 217)
- `create_bible_presentation` (~line 225)
- `rename_bible_presentation` (~line 246)
- `append_bible_presentation_slides` (~line 254)

- [ ] **Step 2: Replace `list_bible_presentations`**

Find the existing function (it does `repository.fetch_libraries().await?` then filters by name). Replace with:

```rust
pub async fn list_bible_presentations(
    &self,
) -> anyhow::Result<Vec<presenter_core::BiblePresentationSummary>> {
    self.repository.list_bible_presentation_summaries().await
}
```

The return type changes from `Vec<PresentationSummary>` to `Vec<BiblePresentationSummary>`. **This is a breaking change for callers** — note which callers exist:

```bash
grep -rn "list_bible_presentations" crates/presenter-server/src/
```

Update the router handler in `crates/presenter-server/src/router/bible.rs` to accept the new return type. The JSON shape should be similar (id, name, slide_count) so frontend should be unaffected — but verify by reading the existing handler.

- [ ] **Step 3: Replace `bible_presentation_detail`**

```rust
pub async fn bible_presentation_detail(
    &self,
    id: presenter_core::BiblePresentationId,
) -> anyhow::Result<Option<presenter_core::BiblePresentation>> {
    self.repository.fetch_bible_presentation(id).await
}
```

Note the parameter type change from `PresentationId` to `BiblePresentationId`. Update the router handler accordingly.

- [ ] **Step 4: Replace `create_bible_presentation`**

```rust
pub async fn create_bible_presentation(
    &self,
    name: &str,
) -> anyhow::Result<presenter_core::BiblePresentation> {
    let presentation = self.repository.create_bible_presentation(name).await?;
    self.live_hub.publish(LiveEvent::BibleSlidesChanged {
        presentation_id: presentation.id.to_string(),
    });
    Ok(presentation)
}
```

The "ensure library exists" branch (lines 226-235 of the original) is GONE — there's no library wrapper anymore. The function is much simpler.

- [ ] **Step 5: Replace `rename_bible_presentation`**

```rust
pub async fn rename_bible_presentation(
    &self,
    id: presenter_core::BiblePresentationId,
    name: &str,
) -> anyhow::Result<()> {
    self.repository.rename_bible_presentation(id, name).await
}
```

- [ ] **Step 6: Replace `append_bible_presentation_slides`**

The existing function (~line 254) does some empty-slide filtering before appending. Preserve that logic but use the new types:

```rust
pub async fn append_bible_presentation_slides(
    &self,
    id: presenter_core::BiblePresentationId,
    new_slides: Vec<presenter_core::BibleSlide>,
) -> anyhow::Result<presenter_core::BiblePresentation> {
    // Filter out empty placeholder slides (empty main and empty reference)
    let filtered: Vec<_> = new_slides
        .into_iter()
        .filter(|s| !s.main.value().is_empty() || !s.main_reference.is_empty())
        .collect();
    let presentation = self
        .repository
        .append_bible_presentation_slides(id, &filtered)
        .await?;
    self.live_hub.publish(LiveEvent::BibleSlidesChanged {
        presentation_id: presentation.id.to_string(),
    });
    Ok(presentation)
}
```

- [ ] **Step 7: Add a delete method if it doesn't already exist**

```bash
grep -n "delete_bible_presentation\|pub async fn delete" crates/presenter-server/src/state/bible.rs
```

If there's no `delete_bible_presentation` in state, add:

```rust
pub async fn delete_bible_presentation(
    &self,
    id: presenter_core::BiblePresentationId,
) -> anyhow::Result<()> {
    self.repository.delete_bible_presentation(id).await?;
    self.live_hub.publish(LiveEvent::BibleSlidesChanged {
        presentation_id: id.to_string(),
    });
    Ok(())
}
```

If it already exists, update its parameter type from `PresentationId` to `BiblePresentationId`.

- [ ] **Step 8: Build to check for errors**

```bash
cargo check -p presenter-server 2>&1 | grep -A2 'error' | head -40
```

Expected errors will likely be in `router/bible.rs` calling these methods with the wrong types. Note the errors but don't fix them yet — that's Task 6.

- [ ] **Step 9: Commit the state changes (knowing router is broken)**

```bash
cargo fmt --all
git add crates/presenter-server/src/state/bible.rs
git commit -m "refactor(state): replace string-based bible library lookups (#231)

state/bible.rs now uses BiblePresentationId and the new repository
methods directly. The 'find library named Bible' string match is
gone. The 'ensure_library_exists' branch in create_bible_presentation
is gone — there's no library wrapper anymore.

This commit temporarily breaks router/bible.rs which still passes
PresentationId to the renamed methods. Fixed in the next commit."
```

---

## Task 6: Router — Update bible.rs Handlers

**Files:**
- Modify: `crates/presenter-server/src/router/bible.rs`

- [ ] **Step 1: Read the current router file**

```bash
wc -l crates/presenter-server/src/router/bible.rs
grep -n "pub.*async fn\|PresentationId\|SlideId" crates/presenter-server/src/router/bible.rs | head -30
```

Identify every handler that takes `Path<Uuid>` for a presentation ID and constructs `PresentationId::from_uuid`. They all need to use `BiblePresentationId` instead.

Same for slide IDs — change `SlideId::from_uuid` to `BibleSlideId::from_uuid` where it's used in the bible context.

- [ ] **Step 2: Update each handler one at a time**

For each handler, the pattern is:
- Change `PresentationId` → `BiblePresentationId`
- Change `SlideId` → `BibleSlideId` (in bible-specific contexts)
- Change `Vec<Slide>` → `Vec<BibleSlide>` for incoming slide payloads
- Update the request DTO (the JSON body type) if it had bible-specific fields

**IMPORTANT:** Read each handler carefully. The DTOs (request body structs) may have field names like `main_text`, `secondary_text`, `main_reference` that already match `BibleSlide`. If they're `Slide`-shaped instead, you may need to map them to `BibleSlide`.

Example for the `create_bible_presentation` handler:

```rust
#[instrument(skip_all)]
pub(crate) async fn create_bible_presentation(
    State(state): State<AppState>,
    Json(payload): Json<CreatePresentationRequest>,
) -> Result<Json<BiblePresentationResponse>, AppError> {
    let presentation = state.create_bible_presentation(&payload.name).await?;
    Ok(Json(BiblePresentationResponse::from(presentation)))
}
```

You may need to introduce or rename `BiblePresentationResponse` to make the API surface explicit. The wire JSON shape should remain the same as today (so the frontend is unaffected) — verify by checking the existing handler's response type.

- [ ] **Step 3: Build until cargo check passes**

```bash
cargo check -p presenter-server 2>&1 | grep -A2 'error' | head -40
```

Iterate on router/bible.rs handlers until there are zero errors. **Do not edit files outside router/bible.rs in this task** — if you find errors elsewhere, that's a sign Task 5 needs revisiting.

- [ ] **Step 4: Run the full server unit test suite**

```bash
cargo test -p presenter-server --lib 2>&1 | tail -20
```

Expected: all tests pass. Some bible-related tests may fail if their assertions used the old types — fix them by updating to `BiblePresentation` / `BibleSlide`.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/presenter-server/src/router/bible.rs
git commit -m "feat(router): wire bible router to new repository types (#231)

Router handlers in router/bible.rs now use BiblePresentationId,
BibleSlideId, and BiblePresentation/BibleSlide instead of the
generic worship types. Wire JSON shape on the API is unchanged
so the frontend continues to work without modification."
```

---

## Task 7: Delete the Broadcasting Leak

**Files:**
- Modify: `crates/presenter-server/src/state/broadcasting.rs`

- [ ] **Step 1: Find the leak**

Open `crates/presenter-server/src/state/broadcasting.rs` and locate the block at lines 83-96:

```rust
// If the current slide has a Bible reference in the `stage` field,
// also emit a BibleUpdate so Resolume #bible-reference-a/b clips
// show the actual reference instead of the library name.
if let Some(ref slide) = context.resolution.current {
    if !slide.stage.is_empty() {
        let bible_output = BibleSlideOutput {
            main_text: slide.main.clone(),
            main_reference: slide.stage.clone(),
            secondary_text: slide.translation.clone(),
            secondary_reference: String::new(),
            triggered_at: now,
        };
        self.resolume_registry
            .bible_update(BibleUpdate::from_slide_output(Some(bible_output)))
            .await;
    }
}
```

- [ ] **Step 2: Delete the entire block**

Remove the lines exactly. Replace with NOTHING (no comment, no placeholder). The function `broadcast_stage_resolution` should end after the `stage_update` call without any bible-update logic.

- [ ] **Step 3: Verify no other code in broadcasting.rs touches BibleUpdate from worship paths**

```bash
grep -n "BibleUpdate\|bible_update\|BibleSlideOutput" crates/presenter-server/src/state/broadcasting.rs
```

Expected: zero matches after the deletion. If anything else remains, evaluate whether it should also be removed.

- [ ] **Step 4: Build**

```bash
cargo check -p presenter-server 2>&1 | tail -10
```

Expected: clean build. Unused imports may surface — clean them up.

- [ ] **Step 5: Add a regression test**

In `crates/presenter-server/src/state/tests.rs` (or wherever broadcasting tests live), add:

```rust
#[tokio::test]
async fn broadcast_stage_resolution_does_not_emit_bible_update_for_worship_stage_text() {
    // Regression for #231: previously, any worship slide with non-empty
    // `stage` field would trigger a spurious BibleUpdate to Resolume.
    let state = test_app_state_with_mock_resolume().await;

    let resolution = StageResolution {
        presentation_id: Some(PresentationId::new()),
        presentation_name: Some("Test Song".to_string()),
        library_name: Some("Worship".to_string()),
        current_slide_id: None,
        current: Some(StageDisplaySlide {
            main: "Verse one".to_string(),
            translation: "Verz jeden".to_string(),
            stage: "Verse 1".to_string(), // Non-empty stage text — used to trigger leak
            group: Some("Verse".to_string()),
        }),
        next_slide_id: None,
        next: None,
        current_index: Some(1),
        total_slides: Some(5),
        playlist_id: None,
        playlist_name: None,
        playlist_entries: None,
    };

    state.broadcast_stage_resolution(resolution).await.unwrap();

    let resolume_calls = state.mock_resolume_registry.calls().await;
    assert!(
        resolume_calls.iter().all(|c| !matches!(c, ResolumeCall::BibleUpdate(_))),
        "Worship stage broadcast should NOT emit BibleUpdate, got: {resolume_calls:?}"
    );
    assert!(
        resolume_calls.iter().any(|c| matches!(c, ResolumeCall::StageUpdate(_))),
        "Worship stage broadcast should emit StageUpdate"
    );
}
```

**IMPORTANT:** The mock infrastructure (`test_app_state_with_mock_resolume`, `mock_resolume_registry`, `ResolumeCall`) may not exist. Read the existing tests in `state/tests.rs` first to see what mock pattern is used. If the existing test pattern doesn't have a Resolume mock, you need to add one in this task. Check:

```bash
grep -rn "ResolumeRegistry\|mock_resolume\|MockResolume" crates/presenter-server/src/
```

If there's no existing mock, the test can be done at integration level instead — set up a real ResolumeRegistry but inject a fake host that records calls. Or skip this unit test and rely on the E2E test from Task 9.

If you skip the unit test, document the skip in the commit message and ensure Task 9's E2E covers it.

- [ ] **Step 6: Run tests**

```bash
cargo test -p presenter-server --lib 2>&1 | tail -10
```

- [ ] **Step 7: Commit**

```bash
cargo fmt --all
git add crates/presenter-server/src/state/broadcasting.rs crates/presenter-server/src/state/tests.rs
git commit -m "fix(broadcasting): delete BibleUpdate-from-stage-field leak (#231)

The block in broadcast_stage_resolution that emitted a BibleUpdate
whenever a worship slide had non-empty 'stage' text is removed.
Worship slides with stage text now produce only StageUpdate, never
BibleUpdate. Bible broadcasts only happen via the dedicated bible
trigger code path in state/bible.rs.

Adds a regression test asserting that worship stage broadcasts do
not invoke the Resolume bible_update method."
```

---

## Task 8: Replace Magic Strings in ai/tools.rs

**Files:**
- Modify: `crates/presenter-server/src/ai/tools.rs`

- [ ] **Step 1: Read the two magic-string sites**

```bash
grep -n -B3 -A8 'eq_ignore_ascii_case("[Bb]ible")' crates/presenter-server/src/ai/tools.rs
```

Expected: two matches at approximately lines 375 and 685. Read the surrounding 10-15 lines of each to understand the intent.

- [ ] **Step 2: Decide the replacement for each site**

For each match, the question is: WHY is it checking if the library is "bible"? Common reasons:
- "Skip bible libraries when listing worship libraries" → after the migration, bible libraries don't exist in `libraries`, so the check is trivially always false. **Delete the check entirely**.
- "Special-case bible when generating slides" → the AI is creating slides into a library; if the target library is bible, do bible-specific work. **Replace with a parameter**: the caller should pass an explicit `target: SlideTarget::Worship | SlideTarget::Bible` enum.

Read each site and decide which case applies. Report your decision in the commit message.

- [ ] **Step 3: Implement the replacement at line ~375**

Read the function containing line 375. If the check is "is this library bible?", and bible libraries no longer exist:

- Delete the check (it was always going to be false post-migration)
- Delete any branches that depended on the check being true
- Update the function signature if it took a library specifically for bible/worship discrimination

If the check was branching to call bible-specific code, replace with a direct call to the bible repository API:

```rust
let bible_summaries = state.list_bible_presentation_summaries().await?;
// ... use bible_summaries instead of looking through worship libraries
```

- [ ] **Step 4: Implement the replacement at line ~685**

Same approach as step 3 for the second site.

- [ ] **Step 5: Build and test**

```bash
cargo check -p presenter-server 2>&1 | tail -10
cargo test -p presenter-server --lib ai 2>&1 | tail -15
```

Expected: clean build, AI tests pass.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/presenter-server/src/ai/tools.rs
git commit -m "refactor(ai): replace 'Bible' magic-string checks with proper API (#231)

Both sites in ai/tools.rs that did
  lib.name.eq_ignore_ascii_case('bible')
have been replaced. After the migration there are no bible libraries
in the libraries table, so the check is dead code. The bible-specific
branches now call list_bible_presentation_summaries directly.

Removes the last 'magic Bible string' references in the codebase.
Combined with state/bible.rs, all 4 string-match locations identified
in the design spec are gone."
```

---

## Task 9: E2E Regression Tests

**Files:**
- Modify or create: `tests/e2e/bible-presentation-append.spec.ts` (or new file `tests/e2e/bible-worship-isolation.spec.ts`)

- [ ] **Step 1: Add a regression test for #227 (Bible out of worship list)**

Create `tests/e2e/bible-worship-isolation.spec.ts`:

```typescript
import { test, expect } from "@playwright/test";
import {
  deriveTestConfig,
  refreshDevData,
  startTestServer,
  stopServer,
  type ServerHandle,
} from "./support";

let serverHandle: ServerHandle | undefined;
let baseURL: string;

test.describe.configure({ timeout: 180_000 });

test.beforeAll(async ({}, testInfo) => {
  const config = deriveTestConfig(testInfo);
  baseURL = config.baseURL;
  await refreshDevData(config.dbUrl);
  serverHandle = await startTestServer(config.port, config.dbUrl, config.oscPort);
});

test.afterAll(async () => {
  await stopServer(serverHandle);
  serverHandle = undefined;
});

test("creating a bible presentation does not add a Bible library to the worship library list", async ({
  request,
}) => {
  // Create a bible presentation via the bible API
  const createResp = await request.post(
    new URL("/bible/presentations", baseURL).toString(),
    {
      data: { name: "E2E Test Sermon" },
    },
  );
  expect(createResp.ok()).toBe(true);

  // Fetch the worship library summary
  const libsResp = await request.get(
    new URL("/libraries/summary", baseURL).toString(),
  );
  expect(libsResp.ok()).toBe(true);
  const libs = (await libsResp.json()) as Array<{ name: string }>;

  // Assert no library is named "Bible" (case-insensitive)
  const bibleLibs = libs.filter(
    (l) => l.name.toLowerCase() === "bible",
  );
  expect(bibleLibs).toHaveLength(0);
});

test("worship slide with stage text does not trigger bible Resolume update", async ({
  request,
}) => {
  // This test asserts the broadcasting leak is fixed by checking that
  // after triggering a worship slide that has non-empty stage text, the
  // server does NOT broadcast a BibleSlide live event.
  //
  // We collect live events from the websocket. Worship stage broadcasts
  // should produce a Stage event but never a BibleSlide event.
  //
  // (Implementation depends on the test harness — see existing
  // ableton-osc.spec.ts or similar for the WebSocket subscription pattern.)
  //
  // If the WebSocket subscription pattern is too involved, this test can
  // instead assert at the API level: after triggering a worship slide
  // with stage text, /bible/active should still return null/empty.

  const activeBefore = await request.get(
    new URL("/bible/active", baseURL).toString(),
  );
  const beforeBody = activeBefore.ok() ? await activeBefore.json() : null;

  // Set up a worship presentation with a slide that has non-empty stage text.
  // This requires creating a library, presentation, slides, then triggering one.
  // The exact API calls match the existing setup in stage-layout.spec.ts —
  // copy that pattern.

  // ... (setup code copied from stage-layout.spec.ts) ...

  const activeAfter = await request.get(
    new URL("/bible/active", baseURL).toString(),
  );
  const afterBody = activeAfter.ok() ? await activeAfter.json() : null;

  // /bible/active should not have changed — the worship trigger didn't touch bible state
  expect(afterBody).toEqual(beforeBody);
});
```

**IMPORTANT:** The second test (broadcasting leak regression) needs the worship slide setup. Read `tests/e2e/stage-layout.spec.ts` for the pattern of creating a library + presentation + slides via the API and triggering one. Copy that pattern; do not invent.

- [ ] **Step 2: Run the new tests locally**

```bash
npm run test:playwright -- bible-worship-isolation 2>&1 | tail -30
```

Expected: both tests pass against a server with the migration applied (Task 1).

- [ ] **Step 3: Verify existing bible E2E tests still pass**

```bash
npm run test:playwright -- bible-presentation-append bible-trigger-slide 2>&1 | tail -30
```

Expected: existing tests pass on the new schema. If they fail, read the failure carefully — they may need updating for the new BiblePresentationId/BibleSlideId types in API responses.

- [ ] **Step 4: Commit**

```bash
git add tests/e2e/bible-worship-isolation.spec.ts
git commit -m "test(e2e): add regression tests for bible/worship isolation (#231)

Two new E2E tests:

1. Creating a bible presentation does NOT add a Bible library row
   to the worship /libraries/summary list. Regression guard for
   #227 (the symptom of the coupling) — after the migration the
   bible library row no longer exists at all.

2. Triggering a worship slide with non-empty stage text does NOT
   change /bible/active. Regression guard for the broadcasting
   leak fixed in state/broadcasting.rs."
```

---

## Task 10: Version Bump, Format, Push, Monitor CI

- [ ] **Step 1: Bump the workspace version**

In `Cargo.toml`, find:

```toml
[workspace.package]
version = "0.4.14"
```

Change to:

```toml
[workspace.package]
version = "0.4.15"
```

- [ ] **Step 2: Refresh Cargo.lock**

```bash
cargo check -p presenter-server 2>&1 | tail -3
```

This updates `Cargo.lock` with the new version.

- [ ] **Step 3: Run all local quality checks**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings -W clippy::all
cargo test -p presenter-core -p presenter-persistence -p presenter-server 2>&1 | tail -20
./scripts/dev/quality-check.sh --strict --against origin/main 2>&1 | grep -E 'fail|FAIL' | head -5
```

Expected: zero failures across all checks. Fix anything that breaks before pushing.

- [ ] **Step 4: Sync with main**

```bash
git fetch origin
git merge origin/main --no-edit
```

Resolve any conflicts (unlikely but possible).

- [ ] **Step 5: Commit version bump**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore: bump version to 0.4.15"
```

- [ ] **Step 6: Push and monitor CI**

```bash
git push origin dev
gh run list --branch dev --limit 3
```

Watch the run until ALL jobs reach a terminal state. The critical jobs to watch:
- **Test** — unit tests including the new repository tests
- **Build** — full release build
- **Playwright E2E shards** — including the new bible-worship-isolation tests
- **Migration test against prod DB** — `pipeline.yml:778-804` runs the migration on a copy of prod
- **Deploy to Dev** — the dev server should now have the new schema

If any job fails, run `gh run view <run-id> --log-failed`, fix the root cause in ONE commit, push ONCE, and monitor again.

- [ ] **Step 7: Verify dev deployment**

```bash
curl -s http://10.77.8.134:8080/healthz
```

Expected: `{"channel":"dev","status":"ok","version":"0.4.15"}`

```bash
sqlite3 /opt/presenter-dev/presenter.db ".schema bible_presentations"
sqlite3 /opt/presenter-dev/presenter.db ".schema slides" | grep -c bible_
sqlite3 /opt/presenter-dev/presenter.db "SELECT name FROM libraries WHERE LOWER(name) = 'bible'"
```

Expected:
- bible_presentations table exists
- 0 bible_* columns on slides
- Empty result for bible library row

- [ ] **Step 8: Open PR**

```bash
gh pr create --title "fix: fully separate bible from worship (#231)" --body-file - <<'EOF'
## Summary

Fully separates bible content from worship at every layer per the design spec at `docs/superpowers/specs/2026-04-10-bible-worship-separation-design.md`.

### Schema
- New `bible_presentations` and `bible_slides` tables
- Dropped 7 `bible_*` columns + `metadata_json` from `slides`
- Dropped dead `category` column from `libraries`
- Dropped any existing bible library row (user explicitly confirmed this — the 2 production bible presentations are gone)

### Domain
- New `BiblePresentation`, `BibleSlide`, `BiblePresentationId`, `BibleSlideId` types
- Existing `Library`, `Presentation`, `Slide` types now mean WORSHIP only

### Repository
- New `repository/bible.rs` with 7 methods (list/fetch/create/rename/delete/replace/append)
- `repository/util.rs` simplified — no more `is_bible = !bible_main.is_empty()` content inspection

### State / broadcasting
- `state/bible.rs` uses the new repository methods, no string lookups
- The `broadcast_stage_resolution` BibleUpdate-from-stage-field leak is DELETED (worship slides no longer trigger spurious bible Resolume broadcasts)

### AI tools
- Both magic-string `eq_ignore_ascii_case("Bible")` checks replaced with proper API calls

### Tests
- 4 new domain tests (`bible.rs` ID round-trip, JSON serialization)
- 6 new repository tests (`repository/bible.rs` CRUD + replace + append against in-memory sqlite)
- 2 new E2E tests (`bible-worship-isolation.spec.ts`):
  - Creating a bible presentation does not add a Bible library to the worship list
  - Triggering a worship slide with stage text does not change `/bible/active`
- 1 new unit test for the broadcasting leak fix
- All existing bible E2E tests continue to pass on the new schema

## Verification
- [x] Migration runs cleanly against a fresh DB
- [x] Migration runs cleanly against a copy of prod DB (via dev pipeline step)
- [x] Dev deployed v0.4.15, schema verified live
- [x] CI fully green

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
```

- [ ] **Step 9: After PR is mergeable, wait for explicit user merge instruction**

Per project policy, NEVER merge without explicit user instruction. Provide the PR URL and wait.

---

## Verification Checklist

After PR merges to main and deploys to production:

- [ ] `curl http://10.77.9.205/healthz` returns version 0.4.15
- [ ] `curl http://10.77.9.205/libraries/summary | jq '.[] | select(.name | ascii_downcase == "bible")'` returns empty (no bible library in worship list)
- [ ] Open `http://presenter.lan/ui/operator` Worship tab → no Bible row in library list
- [ ] Open `http://presenter.lan/ui/bible` → Bible tab loads, can create a new bible presentation, can edit and trigger it
- [ ] Trigger a worship slide that has non-empty stage text → Resolume bible clips do NOT change
- [ ] Trigger a bible presentation → Resolume bible clips DO change
- [ ] Existing E2E tests in `bible-presentation-append.spec.ts` and `bible-trigger-slide.spec.ts` pass on the new schema

---

## Risks Reminder

- **Old binary against new schema:** If something needs to be rolled back, the old `slide::Model` references columns that no longer exist. **Rollback path:** restore the auto-backup taken in the dev pipeline `Backup database` step before deploy.
- **Production data loss:** The 2 existing bible presentations on production are GONE after this deploys. User explicitly confirmed.
- **Migration crashes mid-run:** All steps are idempotent. Tables use `IF NOT EXISTS`, the `DELETE` is naturally re-runnable, the `ALTER DROP COLUMN` is guarded by `pragma_table_info` checks. Crash + restart picks up where it left off.
