use sea_orm_migration::prelude::*;

/// #703 (stream-graphics epic #718, PR-1): the data spine for the
/// Resolume-replacement stream-graphics subsystem. Four tables per ADR
/// `0009-stream-graphics.md` §3, created `IF NOT EXISTS` (incremental, prod
/// data) with the default output row seeded:
///
/// - `stream_outputs`  — N nameable outputs (OBS browser sources). `slug`
///   UNIQUE; `active_scene_id` is a plain nullable INTEGER with NO foreign key
///   (the outputs↔scenes reference is circular — the repository clears it on
///   scene delete). Seeded with one row `slug='stream'`.
/// - `stream_scenes`   — base/overlay scenes, `output_id` FK ON DELETE CASCADE.
/// - `stream_elements` — image/countdown/lyrics/verse elements, `scene_id` FK
///   ON DELETE CASCADE, style config in the `props` JSON column.
/// - `stream_assets`   — sha256-addressed uploaded images (referenced only via
///   `props.asset_id`, no FK — matches the ADR's disk-hash asset model).
///
/// Mirrors the `if_not_exists` + `DEFAULT CURRENT_TIMESTAMP` idiom of
/// `m20260625_000001_add_video_sources.rs`, the idempotent `INSERT OR IGNORE`
/// seed of `m20260420_000001_create_group_colors.rs`, and the FK builder style
/// of `m20250927_000001_create_core_tables.rs`.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum StreamOutputs {
    Table,
    Id,
    Slug,
    Name,
    DefaultTransitionMs,
    ActiveSceneId,
    ConfigRevision,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum StreamScenes {
    Table,
    Id,
    OutputId,
    Name,
    Kind,
    Position,
    IsActive,
    TransitionMs,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum StreamElements {
    Table,
    Id,
    SceneId,
    Kind,
    ZOrder,
    Props,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum StreamAssets {
    Table,
    Id,
    Sha256,
    OriginalFilename,
    Mime,
    SizeBytes,
    Width,
    Height,
    CreatedAt,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // 1) stream_outputs -------------------------------------------------
        manager
            .create_table(
                Table::create()
                    .table(StreamOutputs::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(StreamOutputs::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(StreamOutputs::Slug).text().not_null())
                    .col(ColumnDef::new(StreamOutputs::Name).text().not_null())
                    .col(
                        ColumnDef::new(StreamOutputs::DefaultTransitionMs)
                            .integer()
                            .not_null()
                            .default(400),
                    )
                    // Deliberately NO foreign key: outputs↔scenes is circular.
                    .col(
                        ColumnDef::new(StreamOutputs::ActiveSceneId)
                            .integer()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(StreamOutputs::ConfigRevision)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(StreamOutputs::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .extra("DEFAULT CURRENT_TIMESTAMP"),
                    )
                    .col(
                        ColumnDef::new(StreamOutputs::UpdatedAt)
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
                    .name("idx_stream_outputs_slug_unique")
                    .table(StreamOutputs::Table)
                    .col(StreamOutputs::Slug)
                    .unique()
                    .to_owned(),
            )
            .await?;

        // 2) stream_scenes --------------------------------------------------
        manager
            .create_table(
                Table::create()
                    .table(StreamScenes::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(StreamScenes::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(StreamScenes::OutputId).integer().not_null())
                    .col(ColumnDef::new(StreamScenes::Name).text().not_null())
                    .col(ColumnDef::new(StreamScenes::Kind).text().not_null())
                    .col(ColumnDef::new(StreamScenes::Position).integer().not_null())
                    .col(
                        ColumnDef::new(StreamScenes::IsActive)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(ColumnDef::new(StreamScenes::TransitionMs).integer().null())
                    .col(
                        ColumnDef::new(StreamScenes::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .extra("DEFAULT CURRENT_TIMESTAMP"),
                    )
                    .col(
                        ColumnDef::new(StreamScenes::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .extra("DEFAULT CURRENT_TIMESTAMP"),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_stream_scenes_output")
                            .from(StreamScenes::Table, StreamScenes::OutputId)
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
                    .name("idx_stream_scenes_output")
                    .table(StreamScenes::Table)
                    .col(StreamScenes::OutputId)
                    .to_owned(),
            )
            .await?;

        // 3) stream_elements ------------------------------------------------
        manager
            .create_table(
                Table::create()
                    .table(StreamElements::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(StreamElements::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(StreamElements::SceneId).integer().not_null())
                    .col(ColumnDef::new(StreamElements::Kind).text().not_null())
                    .col(ColumnDef::new(StreamElements::ZOrder).integer().not_null())
                    .col(ColumnDef::new(StreamElements::Props).text().not_null())
                    .col(
                        ColumnDef::new(StreamElements::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .extra("DEFAULT CURRENT_TIMESTAMP"),
                    )
                    .col(
                        ColumnDef::new(StreamElements::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .extra("DEFAULT CURRENT_TIMESTAMP"),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_stream_elements_scene")
                            .from(StreamElements::Table, StreamElements::SceneId)
                            .to(StreamScenes::Table, StreamScenes::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_stream_elements_scene")
                    .table(StreamElements::Table)
                    .col(StreamElements::SceneId)
                    .to_owned(),
            )
            .await?;

        // 4) stream_assets --------------------------------------------------
        manager
            .create_table(
                Table::create()
                    .table(StreamAssets::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(StreamAssets::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(StreamAssets::Sha256).text().not_null())
                    .col(
                        ColumnDef::new(StreamAssets::OriginalFilename)
                            .text()
                            .not_null(),
                    )
                    .col(ColumnDef::new(StreamAssets::Mime).text().not_null())
                    .col(ColumnDef::new(StreamAssets::SizeBytes).integer().not_null())
                    .col(ColumnDef::new(StreamAssets::Width).integer().null())
                    .col(ColumnDef::new(StreamAssets::Height).integer().null())
                    .col(
                        ColumnDef::new(StreamAssets::CreatedAt)
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
                    .name("idx_stream_assets_sha256_unique")
                    .table(StreamAssets::Table)
                    .col(StreamAssets::Sha256)
                    .unique()
                    .to_owned(),
            )
            .await?;

        // 5) Seed the default output. Idempotent (slug is UNIQUE + OR IGNORE),
        //    explicit RFC3339 timestamps so the row is entity-readable.
        manager
            .get_connection()
            .execute(sea_orm::Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "INSERT OR IGNORE INTO stream_outputs (slug, name, created_at, updated_at) \
                 VALUES ('stream', 'Stream', '2026-08-20T00:00:00+00:00', '2026-08-20T00:00:00+00:00')",
            ))
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop children before parents. These tables are brand-new in this
        // migration, so a rollback simply removes them (no prod stream data
        // exists yet to protect).
        manager
            .drop_table(
                Table::drop()
                    .table(StreamElements::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(StreamScenes::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(StreamAssets::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(StreamOutputs::Table)
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

    /// Fresh DB: this migration must create all four stream tables, make them
    /// queryable, and seed exactly one default output `stream`.
    #[tokio::test]
    async fn up_creates_all_four_tables_and_seeds_default_output() {
        let db = Database::connect("sqlite::memory:").await.expect("connect");

        // RED: none of the tables exist yet.
        for table in [
            "stream_outputs",
            "stream_scenes",
            "stream_elements",
            "stream_assets",
        ] {
            let before = db
                .execute(Statement::from_string(
                    DbBackend::Sqlite,
                    format!("SELECT 1 FROM {table}"),
                ))
                .await;
            assert!(
                before.is_err(),
                "precondition: {table} must not exist before the migration",
            );
        }

        let manager = SchemaManager::new(&db);
        Migration.up(&manager).await.expect("migration up");

        // GREEN: every table is queryable with its full column set.
        let checks = [
            "SELECT id, slug, name, default_transition_ms, active_scene_id, \
             config_revision, created_at, updated_at FROM stream_outputs",
            "SELECT id, output_id, name, kind, position, is_active, transition_ms, \
             created_at, updated_at FROM stream_scenes",
            "SELECT id, scene_id, kind, z_order, props, created_at, updated_at \
             FROM stream_elements",
            "SELECT id, sha256, original_filename, mime, size_bytes, width, height, \
             created_at FROM stream_assets",
        ];
        for sql in checks {
            let after = db
                .execute(Statement::from_string(DbBackend::Sqlite, sql.to_string()))
                .await;
            assert!(after.is_ok(), "table must be queryable after up: {after:?}");
        }

        // Seed present: exactly one output, slug 'stream', name 'Stream'.
        let row = db
            .query_one(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT COUNT(*) AS n, MIN(slug) AS slug, MIN(name) AS name \
                 FROM stream_outputs"
                    .to_string(),
            ))
            .await
            .expect("query")
            .expect("row");
        let n: i64 = row.try_get_by("n").expect("count");
        let slug: String = row.try_get_by("slug").expect("slug");
        let name: String = row.try_get_by("name").expect("name");
        assert_eq!(n, 1, "exactly one seeded output");
        assert_eq!(slug, "stream", "seeded slug");
        assert_eq!(name, "Stream", "seeded name");
    }

    /// Re-running on a DB that already has the tables is a no-op: the seed is
    /// not duplicated and a pre-existing child row survives (simulates the
    /// existing-DB / incremental direction).
    #[tokio::test]
    async fn up_is_idempotent_and_preserves_rows() {
        let db = Database::connect("sqlite::memory:").await.expect("connect");
        let manager = SchemaManager::new(&db);

        Migration.up(&manager).await.expect("first up");

        // The seed created output id=1; hang a scene off it.
        db.execute(Statement::from_string(
            DbBackend::Sqlite,
            "INSERT INTO stream_scenes \
             (output_id, name, kind, position, is_active, created_at, updated_at) \
             VALUES (1, 'Base', 'base', 0, 0, \
             '2026-08-20T00:00:00+00:00', '2026-08-20T00:00:00+00:00')"
                .to_string(),
        ))
        .await
        .expect("insert scene");

        Migration
            .up(&manager)
            .await
            .expect("second up must be a no-op");

        let outputs: i64 = db
            .query_one(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT COUNT(*) AS n FROM stream_outputs".to_string(),
            ))
            .await
            .expect("query")
            .expect("row")
            .try_get_by("n")
            .expect("count");
        assert_eq!(outputs, 1, "seed must not be duplicated on re-run");

        let scenes: i64 = db
            .query_one(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT COUNT(*) AS n FROM stream_scenes".to_string(),
            ))
            .await
            .expect("query")
            .expect("row")
            .try_get_by("n")
            .expect("count");
        assert_eq!(scenes, 1, "re-run must preserve existing rows");
    }
}
