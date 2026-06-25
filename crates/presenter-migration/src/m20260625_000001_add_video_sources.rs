use sea_orm_migration::prelude::*;

/// #468: `video_sources` was added to the INITIAL migration
/// (`m20250927_000001`) on 2026-04-04 rather than as an incremental one. Any DB
/// created before that (e.g. PP's 0.4.71 install) therefore never got the table
/// and 500s on the video-sources endpoint. This incremental migration creates it
/// `IF NOT EXISTS`, so it is a no-op on DBs that already have it (prod/dev, and
/// PP after its manual patch) and reconciles any older DB.
///
/// The column set is identical to the initial migration's `video_sources` table.
#[derive(DeriveMigrationName)]
pub struct Migration;

/// Local column identifiers, mirroring the initial migration's `VideoSources`
/// enum so this incremental `create_table` produces the exact same schema.
#[derive(DeriveIden)]
enum VideoSources {
    Table,
    Id,
    Label,
    NdiName,
    IsActive,
    CreatedAt,
    UpdatedAt,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(VideoSources::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(VideoSources::Id)
                            .string_len(36)
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(VideoSources::Label).string().not_null())
                    .col(ColumnDef::new(VideoSources::NdiName).string().not_null())
                    .col(
                        ColumnDef::new(VideoSources::IsActive)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(
                        ColumnDef::new(VideoSources::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .extra("DEFAULT CURRENT_TIMESTAMP"),
                    )
                    .col(
                        ColumnDef::new(VideoSources::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .extra("DEFAULT CURRENT_TIMESTAMP"),
                    )
                    .to_owned(),
            )
            .await?;
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // No-op: dropping video_sources would destroy user-configured NDI
        // sources. Reverting the reconciliation is never desirable.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};

    /// A bare DB that LACKS `video_sources` (simulating an install predating the
    /// table) must have it created by this migration, after which it is
    /// queryable. RED before the migration (the SELECT errors), GREEN after.
    #[tokio::test]
    async fn creates_video_sources_on_a_db_that_lacks_it() {
        let db = Database::connect("sqlite::memory:").await.expect("connect");

        // RED: the table does not exist yet — a query must fail.
        let before = db
            .execute(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT id FROM video_sources".to_string(),
            ))
            .await;
        assert!(
            before.is_err(),
            "precondition: video_sources must not exist before the migration",
        );

        // Apply the migration.
        let manager = SchemaManager::new(&db);
        Migration.up(&manager).await.expect("migration up");

        // GREEN: the table now exists and is queryable.
        let after = db
            .execute(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT id, label, ndi_name, is_active, created_at, updated_at \
                 FROM video_sources"
                    .to_string(),
            ))
            .await;
        assert!(
            after.is_ok(),
            "video_sources must be queryable after the migration: {after:?}",
        );
    }

    /// Running on a DB that ALREADY has the table is a no-op (IF NOT EXISTS), so
    /// it is safe on prod/dev/PP and on re-run.
    #[tokio::test]
    async fn is_idempotent_when_table_already_exists() {
        let db = Database::connect("sqlite::memory:").await.expect("connect");
        let manager = SchemaManager::new(&db);

        Migration.up(&manager).await.expect("first up");
        // Insert a row so we can prove the second run does not drop/recreate it.
        db.execute(Statement::from_string(
            DbBackend::Sqlite,
            "INSERT INTO video_sources (id, label, ndi_name, is_active, created_at, updated_at) \
             VALUES ('s1', 'Cam 1', 'NDI Cam 1', 0, '2026-06-25T00:00:00Z', '2026-06-25T00:00:00Z')"
                .to_string(),
        ))
        .await
        .expect("insert");

        Migration
            .up(&manager)
            .await
            .expect("second up must be a no-op");

        let row = db
            .query_one(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT COUNT(*) AS n FROM video_sources".to_string(),
            ))
            .await
            .expect("query")
            .expect("row");
        let n: i64 = row.try_get_by("n").expect("count");
        assert_eq!(
            n, 1,
            "re-run must preserve existing rows (no drop/recreate)"
        );
    }
}
