pub use sea_orm_migration::prelude::*;

mod m20250927_000001_create_core_tables;
mod m20260408_000001_add_preach_limit;
mod m20260410_000001_separate_bible;
mod m20260412_000001_bible_fts;

pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20250927_000001_create_core_tables::Migration),
            Box::new(m20260408_000001_add_preach_limit::Migration),
            Box::new(m20260410_000001_separate_bible::Migration),
            Box::new(m20260412_000001_bible_fts::Migration),
        ]
    }
}
