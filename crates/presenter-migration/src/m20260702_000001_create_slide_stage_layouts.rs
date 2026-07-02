use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// #515: per-slide stage-layout markers. When the operator triggers a slide
/// that carries a marker, the server switches the stage display to that
/// layout (exactly like `POST /stage/layout`). One row per marked slide;
/// slides without a row leave the layout untouched.
///
/// `slide_id` is the primary key (slide UUIDs are globally unique);
/// `presentation_id` is stored so all markers of a presentation can be listed
/// for the operator UI and cleaned up when the presentation is deleted.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // Idempotent: CREATE TABLE IF NOT EXISTS makes re-running safe.
        db.execute(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "CREATE TABLE IF NOT EXISTS slide_stage_layouts (\
                slide_id        TEXT NOT NULL PRIMARY KEY,\
                presentation_id TEXT NOT NULL,\
                layout_code     TEXT NOT NULL\
            )",
        ))
        .await?;

        // Listing markers for the operator UI filters by presentation.
        db.execute(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "CREATE INDEX IF NOT EXISTS idx_slide_stage_layouts_presentation \
             ON slide_stage_layouts (presentation_id)",
        ))
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute(sea_orm::Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "DROP TABLE IF EXISTS slide_stage_layouts",
            ))
            .await?;
        Ok(())
    }
}
