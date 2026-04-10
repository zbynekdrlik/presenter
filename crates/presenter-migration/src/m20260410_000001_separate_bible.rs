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
    db: &impl sea_orm::ConnectionTrait,
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

#[cfg(test)]
mod tests {
    use sea_orm::{ConnectionTrait, Database, DatabaseConnection, Statement};

    use crate::{Migrator, MigratorTrait};

    #[tokio::test]
    async fn migration_runs_on_fresh_db() {
        let db: DatabaseConnection = Database::connect("sqlite::memory:").await.unwrap();
        Migrator::up(&db, None).await.unwrap();

        // Verify bible_presentations table exists
        let result = db
            .query_one(Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "SELECT name FROM sqlite_master WHERE type='table' AND name='bible_presentations'",
            ))
            .await
            .unwrap();
        assert!(result.is_some(), "bible_presentations table should exist");

        // Verify bible_slides table exists
        let result = db
            .query_one(Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "SELECT name FROM sqlite_master WHERE type='table' AND name='bible_slides'",
            ))
            .await
            .unwrap();
        assert!(result.is_some(), "bible_slides table should exist");

        // Verify bible_* columns are gone from slides
        let count_row = db
            .query_one(Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "SELECT COUNT(*) as cnt FROM pragma_table_info('slides') WHERE name LIKE 'bible_%'",
            ))
            .await
            .unwrap()
            .unwrap();
        let cnt: i32 = count_row.try_get("", "cnt").unwrap();
        assert_eq!(cnt, 0, "bible_* columns should be dropped from slides");

        // Verify category column is gone from libraries
        let cat_row = db
            .query_one(Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "SELECT COUNT(*) as cnt FROM pragma_table_info('libraries') WHERE name='category'",
            ))
            .await
            .unwrap()
            .unwrap();
        let cat_cnt: i32 = cat_row.try_get("", "cnt").unwrap();
        assert_eq!(
            cat_cnt, 0,
            "category column should be dropped from libraries"
        );

        // Verify metadata_json is gone from slides
        let meta_row = db
            .query_one(Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "SELECT COUNT(*) as cnt FROM pragma_table_info('slides') WHERE name='metadata_json'",
            ))
            .await
            .unwrap()
            .unwrap();
        let meta_cnt: i32 = meta_row.try_get("", "cnt").unwrap();
        assert_eq!(
            meta_cnt, 0,
            "metadata_json column should be dropped from slides"
        );
    }

    #[tokio::test]
    async fn migration_is_idempotent() {
        let db: DatabaseConnection = Database::connect("sqlite::memory:").await.unwrap();
        // Run migrations twice — second run should succeed (tables already exist)
        Migrator::up(&db, None).await.unwrap();
        Migrator::up(&db, None).await.unwrap();
    }
}
