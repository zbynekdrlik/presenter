//! #779: lower-third nameplates ("menovky") for stream graphics. One additive,
//! idempotent table `stream_nameplates` — the list of PERSON plates (name +
//! role) shown as animated lower thirds. The SONG plate is virtual (resolved
//! from the live stage snapshot), so it is never a row here.
//!
//! `output_id` FK → `stream_outputs` ON DELETE CASCADE + index (a plate belongs
//! to exactly one output; deleting the output removes its plates), matching the
//! `stream_scenes`/`stream_elements` shape. NO sync columns — stream tables are
//! per-instance runtime data, not part of PP↔SNV sync (arch decision #3, same as
//! `stream_assets`/`stream_fonts`).
//!
//! Additive + idempotent per the repo DB policy — `stream_*` already holds prod
//! data, so this is a NEW incremental migration, never an edit of an applied one.
//! Mirrors `m20260820_000001`'s `if_not_exists` + `DEFAULT CURRENT_TIMESTAMP` +
//! FK-cascade idiom.
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum StreamNameplates {
    Table,
    Id,
    OutputId,
    PrimaryText,
    SecondaryText,
    Position,
    CreatedAt,
    UpdatedAt,
}

/// Only the identifiers needed to reference the parent output table in the FK.
#[derive(DeriveIden)]
enum StreamOutputs {
    Table,
    Id,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(StreamNameplates::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(StreamNameplates::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(StreamNameplates::OutputId)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(StreamNameplates::PrimaryText)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(StreamNameplates::SecondaryText)
                            .text()
                            .not_null()
                            .default(""),
                    )
                    .col(
                        ColumnDef::new(StreamNameplates::Position)
                            .integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(StreamNameplates::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .extra("DEFAULT CURRENT_TIMESTAMP"),
                    )
                    .col(
                        ColumnDef::new(StreamNameplates::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .extra("DEFAULT CURRENT_TIMESTAMP"),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_stream_nameplates_output")
                            .from(StreamNameplates::Table, StreamNameplates::OutputId)
                            .to(StreamOutputs::Table, StreamOutputs::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_stream_nameplates_output")
                    .table(StreamNameplates::Table)
                    .col(StreamNameplates::OutputId)
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
                    .table(StreamNameplates::Table)
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

    /// Fresh DB: the migration creates a queryable `stream_nameplates` table with
    /// its full column set (RED before up, GREEN after).
    #[tokio::test]
    async fn up_creates_stream_nameplates_table() {
        let db = Database::connect("sqlite::memory:").await.expect("connect");

        let before = db
            .execute(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT 1 FROM stream_nameplates".to_string(),
            ))
            .await;
        assert!(
            before.is_err(),
            "precondition: stream_nameplates must not exist before the migration"
        );

        let manager = SchemaManager::new(&db);
        Migration.up(&manager).await.expect("migration up");

        let after = db
            .execute(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT id, output_id, primary_text, secondary_text, position, created_at, \
                 updated_at FROM stream_nameplates"
                    .to_string(),
            ))
            .await;
        assert!(
            after.is_ok(),
            "stream_nameplates queryable with full column set after up: {after:?}"
        );
    }

    /// Re-running on a DB that already has the table is a no-op and preserves
    /// existing rows (idempotent — the incremental / existing-DB direction).
    #[tokio::test]
    async fn up_is_idempotent_and_preserves_rows() {
        let db = Database::connect("sqlite::memory:").await.expect("connect");
        let manager = SchemaManager::new(&db);
        // The insert below references `stream_outputs` (FK, enforced by SQLite on
        // this connection) — create the parent tables first; that migration also
        // seeds the default output with id 1.
        crate::m20260820_000001_create_stream_tables::Migration
            .up(&manager)
            .await
            .expect("stream tables up");
        Migration.up(&manager).await.expect("first up");

        db.execute(Statement::from_string(
            DbBackend::Sqlite,
            "INSERT INTO stream_nameplates \
             (output_id, primary_text, secondary_text, position, created_at, updated_at) \
             VALUES (1, 'Ján Novák', 'pastor', 0, \
             '2026-09-18T00:00:00+00:00', '2026-09-18T00:00:00+00:00')"
                .to_string(),
        ))
        .await
        .expect("insert nameplate");

        Migration
            .up(&manager)
            .await
            .expect("second up must be a no-op");

        let n: i64 = db
            .query_one(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT COUNT(*) AS n FROM stream_nameplates".to_string(),
            ))
            .await
            .expect("query")
            .expect("row")
            .try_get_by("n")
            .expect("count");
        assert_eq!(n, 1, "re-run preserves existing rows");
    }
}
