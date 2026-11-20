//! #952: bookmark folders of `/ui/study` (owner, 2026-11-20: folders, not
//! tags). `study_bookmark_folders` holds the reader's own folders ("Viera",
//! "Modlitba"), one list per `owner_email` (NULL = the shared list of the LAN,
//! as `study_bookmarks`); `study_bookmarks.folder_id` names the folder a
//! bookmark sits in, NULL = the default folder "Kde som skončil". So every
//! bookmark already stored stays exactly where it is.
//!
//! Incremental and idempotent: `IF NOT EXISTS` for the table and its index, a
//! `pragma_table_info` guard for the column (SQLite refuses to add a column
//! twice). The m20261108 migration is never edited: SNV has bookmarks. No
//! foreign key: a folder's delete removes its bookmarks in the same
//! transaction (the repository), as the bookmarks table never had keys.
use sea_orm::{ConnectionTrait, Statement};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const CREATE_FOLDERS: &str = r#"CREATE TABLE IF NOT EXISTS "study_bookmark_folders" (
    "id" integer NOT NULL PRIMARY KEY AUTOINCREMENT,
    "owner_email" text NULL,
    "name" text NOT NULL,
    "position" integer NOT NULL,
    "created_at" text NOT NULL,
    "updated_at" text NOT NULL
)"#;

const CREATE_FOLDER_INDEX: &str = r#"CREATE INDEX IF NOT EXISTS
    "idx_study_bookmark_folders_owner" ON "study_bookmark_folders" ("owner_email", "position")"#;

const ADD_FOLDER_COLUMN: &str =
    r#"ALTER TABLE "study_bookmarks" ADD COLUMN "folder_id" integer NULL"#;

const CREATE_FOLDER_ID_INDEX: &str = r#"CREATE INDEX IF NOT EXISTS
    "idx_study_bookmarks_folder" ON "study_bookmarks" ("folder_id")"#;

async fn has_folder_column<C: ConnectionTrait>(db: &C) -> Result<bool, DbErr> {
    let row = db
        .query_one(Statement::from_string(
            db.get_database_backend(),
            "SELECT COUNT(*) AS cnt FROM pragma_table_info('study_bookmarks') \
             WHERE name = 'folder_id'",
        ))
        .await?;
    Ok(match row {
        Some(row) => row.try_get::<i32>("", "cnt")? > 0,
        None => false,
    })
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        for sql in [CREATE_FOLDERS, CREATE_FOLDER_INDEX] {
            db.execute_unprepared(sql).await?;
        }
        if !has_folder_column(db).await? {
            db.execute_unprepared(ADD_FOLDER_COLUMN).await?;
        }
        db.execute_unprepared(CREATE_FOLDER_ID_INDEX).await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // The folders table is new here; the column stays (SQLite before 3.35
        // has no DROP COLUMN, and a NULL there is the default folder anyway).
        manager
            .get_connection()
            .execute_unprepared(r#"DROP TABLE IF EXISTS "study_bookmark_folders""#)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{Database, DbBackend};

    /// `study_bookmarks` as m20261108 left it, with one SNV-like row.
    async fn db_before() -> sea_orm::DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.expect("connect");
        for sql in [
            r#"CREATE TABLE "study_bookmarks" ("id" integer NOT NULL PRIMARY KEY AUTOINCREMENT,
               "owner_email" text NULL, "name" text NOT NULL, "translation_code" text NOT NULL,
               "book_code" text NOT NULL, "book_name" text NOT NULL, "chapter" integer NOT NULL,
               "verse" integer NOT NULL, "position" integer NOT NULL,
               "created_at" text NOT NULL, "updated_at" text NOT NULL)"#,
            "INSERT INTO study_bookmarks (owner_email, name, translation_code, book_code, \
             book_name, chapter, verse, position, created_at, updated_at) \
             VALUES ('p@example.org', 'Hebrejom 11:1', 'slk-seb', 'HEB', 'Hebrejom', 11, 1, 0, \
             't', 't')",
        ] {
            db.execute(Statement::from_string(DbBackend::Sqlite, sql.to_string()))
                .await
                .expect("seed");
        }
        db
    }

    #[tokio::test]
    async fn existing_bookmarks_land_in_the_default_folder_and_a_rerun_is_a_no_op() {
        let db = db_before().await;
        let manager = SchemaManager::new(&db);
        Migration.up(&manager).await.expect("up");
        Migration
            .up(&manager)
            .await
            .expect("a second up is a no-op");
        let row = db
            .query_one(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT name, folder_id FROM study_bookmarks".to_string(),
            ))
            .await
            .expect("query")
            .expect("the row survived");
        let name: String = row.try_get_by("name").expect("name");
        let folder: Option<i64> = row.try_get_by("folder_id").expect("folder_id");
        assert_eq!(name, "Hebrejom 11:1");
        assert_eq!(folder, None, "NULL = the default folder");
        let folders = db
            .query_one(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT COUNT(*) AS cnt FROM study_bookmark_folders".to_string(),
            ))
            .await
            .expect("the folders table exists")
            .expect("count");
        assert_eq!(folders.try_get::<i32>("", "cnt").expect("cnt"), 0);
    }
}
