use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

const SEED_ROWS: &[(&str, &str)] = &[
    ("Stage SD1", "sd1l.lan"),
    ("Stage SD2", "sd2l.lan"),
    ("Stage SD3", "sd3l.lan"),
    ("Stage SD4", "sd4l.lan"),
];

const DEFAULT_LAUNCH_COMPONENT: &str = "com.fullykiosk.videokiosk/de.ozerov.fully.MainActivity";

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // Only seed if the table is empty. If an operator has added any
        // rows (even after deleting and re-adding), leave them alone.
        let row = db
            .query_one(sea_orm::Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "SELECT COUNT(*) as cnt FROM android_stage_displays",
            ))
            .await?;

        let count = row
            .map(|r| r.try_get::<i32>("", "cnt").unwrap_or(0))
            .unwrap_or(0);

        if count > 0 {
            return Ok(());
        }

        for (label, host) in SEED_ROWS {
            let id = uuid::Uuid::new_v4().to_string();
            db.execute(sea_orm::Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Sqlite,
                "INSERT INTO android_stage_displays \
                 (id, label, host, port, launch_component, is_enabled, created_at, updated_at) \
                 VALUES (?, ?, ?, 5555, ?, 1, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)",
                [
                    id.into(),
                    (*label).into(),
                    (*host).into(),
                    DEFAULT_LAUNCH_COMPONENT.into(),
                ],
            ))
            .await?;
        }

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // No-op. Do not delete operator data on rollback.
        Ok(())
    }
}
