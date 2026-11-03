//! #874: the Prepared tab's own order and an archive instead of deleting.
//! `position` orders the active list (smallest first); `archived_at` set =
//! archived. The backfill keeps today's alphabetical order, so the list looks
//! the same right after the deploy.
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const ADD_POSITION: &str =
    r#"ALTER TABLE "bible_presentations" ADD COLUMN "position" INTEGER NOT NULL DEFAULT 0"#;
const ADD_ARCHIVED_AT: &str =
    r#"ALTER TABLE "bible_presentations" ADD COLUMN "archived_at" TEXT NULL"#;
const BACKFILL_POSITION: &str = r#"UPDATE "bible_presentations" SET "position" = (
    SELECT COUNT(*) FROM "bible_presentations" AS b
    WHERE b."name" < "bible_presentations"."name"
       OR (b."name" = "bible_presentations"."name" AND b."id" < "bible_presentations"."id")
)"#;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(ADD_POSITION).await?;
        db.execute_unprepared(ADD_ARCHIVED_AT).await?;
        db.execute_unprepared(BACKFILL_POSITION).await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared(r#"ALTER TABLE "bible_presentations" DROP COLUMN "archived_at""#)
            .await?;
        db.execute_unprepared(r#"ALTER TABLE "bible_presentations" DROP COLUMN "position""#)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::{Migrator, MigratorTrait};
    use sea_orm::{ConnectionTrait, Database, DatabaseConnection, Statement};
    use sea_orm_migration::MigrationTrait;

    fn sql(text: &str) -> Statement {
        Statement::from_string(sea_orm::DatabaseBackend::Sqlite, text.to_string())
    }

    #[tokio::test]
    async fn the_backfill_keeps_the_alphabetical_order_and_nothing_is_archived() {
        let db: DatabaseConnection = Database::connect("sqlite::memory:").await.unwrap();
        let before_this = u32::try_from(Migrator::migrations().len() - 1).unwrap();
        Migrator::up(&db, Some(before_this)).await.unwrap();
        for (id, name) in [
            ("c", "Kázeň 2"),
            ("a", "Ján 3"),
            ("b", "Kázeň 1"),
            ("d", "Ján 3"),
        ] {
            db.execute(sql(&format!(
                "INSERT INTO bible_presentations (id, name, created_at) VALUES ('{id}', '{name}', '2026-11-01T00:00:00+00:00')"
            )))
            .await
            .unwrap();
        }
        super::Migration
            .up(&sea_orm_migration::SchemaManager::new(&db))
            .await
            .unwrap();

        let rows = db
            .query_all(sql(
                "SELECT id, position, archived_at IS NULL AS active FROM bible_presentations ORDER BY position",
            ))
            .await
            .unwrap();
        let got: Vec<(String, i32, bool)> = rows
            .iter()
            .map(|r| {
                (
                    r.try_get("", "id").unwrap(),
                    r.try_get("", "position").unwrap(),
                    r.try_get("", "active").unwrap(),
                )
            })
            .collect();
        assert_eq!(
            got,
            vec![
                ("a".to_string(), 0, true),
                ("d".to_string(), 1, true),
                ("b".to_string(), 2, true),
                ("c".to_string(), 3, true),
            ]
        );
    }
}
