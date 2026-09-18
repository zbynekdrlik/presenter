//! #778: uploaded web fonts for stream graphics. One additive, idempotent
//! table `stream_fonts` — one row per FACE (family + weight + italic), the
//! metadata for a sha256-addressed font file stored on disk under
//! `<stream-assets>/fonts/<sha256>.<ext>` (bytes never in git; the repo is
//! public). Referenced only by `props.<style>.font_family` (a NAME match, no
//! FK), so — like `stream_assets` — no foreign key and no sync columns
//! (per-instance runtime data; the import script targets each instance).
//!
//! Created `IF NOT EXISTS` with a `sha256` UNIQUE index (content-addressed
//! dedup). Additive + idempotent per the repo DB policy — `stream_*` already
//! holds prod data, so this is a NEW incremental migration, never an edit of
//! the applied `m20260820_000001_create_stream_tables`. Mirrors that migration's
//! `if_not_exists` + `DEFAULT CURRENT_TIMESTAMP` idiom and the `stream_assets`
//! table shape.
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum StreamFonts {
    Table,
    Id,
    Sha256,
    OriginalFilename,
    Family,
    Weight,
    Italic,
    Format,
    SizeBytes,
    CreatedAt,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(StreamFonts::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(StreamFonts::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(StreamFonts::Sha256).text().not_null())
                    .col(
                        ColumnDef::new(StreamFonts::OriginalFilename)
                            .text()
                            .not_null(),
                    )
                    .col(ColumnDef::new(StreamFonts::Family).text().not_null())
                    .col(ColumnDef::new(StreamFonts::Weight).integer().not_null())
                    .col(
                        ColumnDef::new(StreamFonts::Italic)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(ColumnDef::new(StreamFonts::Format).text().not_null())
                    .col(ColumnDef::new(StreamFonts::SizeBytes).integer().not_null())
                    .col(
                        ColumnDef::new(StreamFonts::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .extra("DEFAULT CURRENT_TIMESTAMP"),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_stream_fonts_sha256_unique")
                    .table(StreamFonts::Table)
                    .col(StreamFonts::Sha256)
                    .unique()
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Brand-new table in this migration; a rollback simply removes it.
        manager
            .drop_table(
                Table::drop()
                    .table(StreamFonts::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};

    /// Fresh DB: the migration creates a queryable `stream_fonts` table with its
    /// full column set (RED before up, GREEN after).
    #[tokio::test]
    async fn up_creates_stream_fonts_table() {
        let db = Database::connect("sqlite::memory:").await.expect("connect");

        let before = db
            .execute(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT 1 FROM stream_fonts".to_string(),
            ))
            .await;
        assert!(
            before.is_err(),
            "precondition: stream_fonts must not exist before the migration"
        );

        let manager = SchemaManager::new(&db);
        Migration.up(&manager).await.expect("migration up");

        let after = db
            .execute(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT id, sha256, original_filename, family, weight, italic, format, \
                 size_bytes, created_at FROM stream_fonts"
                    .to_string(),
            ))
            .await;
        assert!(
            after.is_ok(),
            "stream_fonts queryable with full column set after up: {after:?}"
        );
    }

    /// The sha256 UNIQUE index rejects a duplicate hash.
    #[tokio::test]
    async fn sha256_is_unique() {
        let db = Database::connect("sqlite::memory:").await.expect("connect");
        let manager = SchemaManager::new(&db);
        Migration.up(&manager).await.expect("up");

        let insert = |sha: &str| {
            format!(
                "INSERT INTO stream_fonts \
                 (sha256, original_filename, family, weight, italic, format, size_bytes, created_at) \
                 VALUES ('{sha}', 'f.ttf', 'Fam', 400, 0, 'ttf', 1024, \
                 '2026-09-18T00:00:00+00:00')"
            )
        };
        db.execute(Statement::from_string(DbBackend::Sqlite, insert("aaa")))
            .await
            .expect("first insert");
        let dup = db
            .execute(Statement::from_string(DbBackend::Sqlite, insert("aaa")))
            .await;
        assert!(
            dup.is_err(),
            "duplicate sha256 rejected by the unique index"
        );
    }

    /// Re-running on a DB that already has the table is a no-op and preserves
    /// existing rows (idempotent — the incremental / existing-DB direction).
    #[tokio::test]
    async fn up_is_idempotent_and_preserves_rows() {
        let db = Database::connect("sqlite::memory:").await.expect("connect");
        let manager = SchemaManager::new(&db);
        Migration.up(&manager).await.expect("first up");

        db.execute(Statement::from_string(
            DbBackend::Sqlite,
            "INSERT INTO stream_fonts \
             (sha256, original_filename, family, weight, italic, format, size_bytes, created_at) \
             VALUES ('deadbeef', 'brand.ttf', 'Brand', 700, 1, 'ttf', 4096, \
             '2026-09-18T00:00:00+00:00')"
                .to_string(),
        ))
        .await
        .expect("insert font");

        Migration
            .up(&manager)
            .await
            .expect("second up must be a no-op");

        let n: i64 = db
            .query_one(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT COUNT(*) AS n FROM stream_fonts".to_string(),
            ))
            .await
            .expect("query")
            .expect("row")
            .try_get_by("n")
            .expect("count");
        assert_eq!(n, 1, "re-run preserves existing rows");
    }
}
