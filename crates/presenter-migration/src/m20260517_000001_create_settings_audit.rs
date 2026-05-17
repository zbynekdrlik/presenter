use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(Iden)]
pub enum SettingsAudit {
    Table,
    Id,
    SettingTable,
    SettingId,
    Source,
    Actor,
    BeforeJson,
    AfterJson,
    ChangedAt,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(SettingsAudit::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(SettingsAudit::Id)
                            .text()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(SettingsAudit::SettingTable)
                            .text()
                            .not_null(),
                    )
                    .col(ColumnDef::new(SettingsAudit::SettingId).text().not_null())
                    .col(ColumnDef::new(SettingsAudit::Source).text().not_null())
                    .col(
                        ColumnDef::new(SettingsAudit::Actor)
                            .text()
                            .not_null()
                            .default("unknown"),
                    )
                    .col(ColumnDef::new(SettingsAudit::BeforeJson).text().null())
                    .col(ColumnDef::new(SettingsAudit::AfterJson).text().not_null())
                    .col(
                        ColumnDef::new(SettingsAudit::ChangedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_settings_audit_table_id_time")
                    .table(SettingsAudit::Table)
                    .col(SettingsAudit::SettingTable)
                    .col(SettingsAudit::SettingId)
                    .col(SettingsAudit::ChangedAt)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_settings_audit_source_time")
                    .table(SettingsAudit::Table)
                    .col(SettingsAudit::Source)
                    .col(SettingsAudit::ChangedAt)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(SettingsAudit::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await
    }
}
