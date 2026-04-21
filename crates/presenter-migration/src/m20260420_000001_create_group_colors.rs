use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Legacy color mappings for vocal group labels imported from ProPresenter.
/// INSERT OR IGNORE ensures re-running the migration is safe and does not
/// overwrite any colors the operator may have customised after import.
const SEED_ROWS: &[(&str, &str)] = &[
    ("Vsetci", "#E08A3C"),
    ("Zeny", "#C73E9E"),
    ("Muzi", "#2E2E8F"),
    ("Zeny/Muzi", "#4A6CE0"),
    ("Peta", "#8E44C4"),
    ("Stevo", "#D62828"),
    ("Miro", "#3CB371"),
    ("Zuzka", "#E8631C"),
    ("Patrika", "#D4621E"),
    ("Tina", "#E89B7A"),
    ("Miro, Peta, Zuzka", "#C8304A"),
    ("Stevo, Peta", "#8B1A1A"),
    ("Stevo, Peta, Zuzka", "#D81BC0"),
    ("Stevo, Zuzka", "#2B9D9D"),
    ("Peta, Miro", "#6A2C9E"),
    ("Miro, Stevo, Zuzka", "#5A7A7A"),
    ("Stevo, zeny", "#E8A020"),
    ("Tina, Zuzka", "#B83CA4"),
    ("Miro, Zuzka", "#8A9A3A"),
    ("Peta // Vsetci", "#1A1A1A"),
    ("Miro, Tina", "#A05A2C"),
    ("Vsetci // Zuzka", "#A61E1E"),
    ("Muzi, Peta", "#2E8B57"),
    ("Muzi, Zuzka", "#C83030"),
    ("Vsetci okrem zuzky", "#D47A2C"),
    ("Stevo, Miro, Tina", "#1E2B8F"),
    ("Vsetci okrem Peti", "#9E8AC4"),
    ("Stevo, Tina", "#4A7AD4"),
    ("Miro, Tina, Zuzka", "#8B1E3F"),
    ("Miro // vsetci", "#3CA4E0"),
    ("Muzi // Zeny", "#B8A82C"),
    ("Vsetci // Peta", "#2B5AA6"),
    ("Peta, Zuzka", "#3A5AD4"),
    ("Patrika, Miro", "#8B1E3F"),
    ("Vsetci // Patrika, Miro", "#A89A2C"),
    ("Peta, Miro // vsetci", "#1A1A1A"),
    ("Patrika, Miro, Zuzka", "#C41E5A"),
    ("Miro, Tina, Patrika", "#7A2CA6"),
    ("Patrika, Stevo", "#2B5A6A"),
    ("Pomaly", "#1A1A1A"),
    ("Rychlejsie", "#3A3A3A"),
    ("Rychlo", "#9A9A9A"),
    ("Bridge 2", "#A0521E"),
    ("Bridge 1", "#E8831C"),
    ("Chorus 4", "#8B1E2C"),
    ("Chorus 3", "#9A9A9A"),
    ("Inter Chorus", "#1E5A8B"),
    ("Chorus 2", "#C43CB4"),
    ("PreChorus", "#5A1E9E"),
    ("Intro", "#F0E020"),
    ("Chorus 1", "#E02020"),
    ("Verse 1", "#2040E0"),
    ("Verse 2", "#6FE020"),
    ("Verse 3", "#D41E6A"),
    ("Verse 4", "#7A2CA6"),
    ("1. sloha", "#2CA64A"),
    ("2. sloha", "#2B6AA6"),
    ("Peta, Stevo, Zuzka // vsetci", "#1A1A1A"),
    ("Postchorus", "#3CB371"),
    ("Miro, Zuzka // vsetci", "#8B1E3F"),
    ("Patrika, Tina, Stevo", "#D62828"),
    ("Vsetci // Stevo, Peta", "#2B7AC4"),
    ("Patrika, muzi", "#D41EA6"),
    ("Tina, Patrika", "#3CB371"),
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // Create the table if it does not exist yet.
        db.execute(sea_orm::Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            "CREATE TABLE IF NOT EXISTS group_colors (\
                name  TEXT NOT NULL PRIMARY KEY,\
                color TEXT NOT NULL\
            )",
        ))
        .await?;

        // Seed legacy mappings; existing rows are left untouched.
        for (name, color) in SEED_ROWS {
            db.execute(sea_orm::Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Sqlite,
                "INSERT OR IGNORE INTO group_colors (name, color) VALUES (?, ?)",
                [(*name).into(), (*color).into()],
            ))
            .await?;
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute(sea_orm::Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "DROP TABLE IF EXISTS group_colors",
            ))
            .await?;
        Ok(())
    }
}
