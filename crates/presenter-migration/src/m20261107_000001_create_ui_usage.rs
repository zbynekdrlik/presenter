//! #913: `ui_usage`, how often each control of the GUI is pressed, so what gets
//! a place on a small screen is decided from use. One row per day, page, shell
//! and `data-role`, a counter only: no text, no person, no address. Site-local
//! (no sync columns). Additive + idempotent per the repo DB policy.
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum UiUsage {
    Table,
    Day,
    Page,
    Shell,
    Role,
    Count,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(UiUsage::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(UiUsage::Day).text().not_null())
                    .col(ColumnDef::new(UiUsage::Page).text().not_null())
                    .col(ColumnDef::new(UiUsage::Shell).text().not_null())
                    .col(ColumnDef::new(UiUsage::Role).text().not_null())
                    .col(ColumnDef::new(UiUsage::Count).big_integer().not_null())
                    .primary_key(
                        Index::create()
                            .col(UiUsage::Day)
                            .col(UiUsage::Page)
                            .col(UiUsage::Shell)
                            .col(UiUsage::Role),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(UiUsage::Table).if_exists().to_owned())
            .await
    }
}
