use sea_orm_migration::prelude::*;

/// #483: append-only audit of every Resolume per-slide push, so post-event
/// latency analysis is a SQL query instead of grepping journald (which rotates
/// away). Mirrors the `settings_audit` pattern. One row per stage push per
/// host: correlation_id (joins the click-path log), the three key timings,
/// whether the mapping was re-fetched inline, and the outcome.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(Iden)]
pub enum ResolumePushAudit {
    Table,
    Id,
    CorrelationId,
    Host,
    TQueueWaitMs,
    TEnsureMappingMs,
    TTotalMs,
    Refetched,
    Outcome,
    CreatedAt,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(ResolumePushAudit::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(ResolumePushAudit::Id)
                            .text()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(ResolumePushAudit::CorrelationId)
                            .text()
                            .null(),
                    )
                    .col(ColumnDef::new(ResolumePushAudit::Host).text().not_null())
                    .col(
                        ColumnDef::new(ResolumePushAudit::TQueueWaitMs)
                            .double()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ResolumePushAudit::TEnsureMappingMs)
                            .double()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ResolumePushAudit::TTotalMs)
                            .double()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ResolumePushAudit::Refetched)
                            .boolean()
                            .not_null(),
                    )
                    .col(ColumnDef::new(ResolumePushAudit::Outcome).text().not_null())
                    .col(
                        ColumnDef::new(ResolumePushAudit::CreatedAt)
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
                    .name("idx_resolume_push_audit_host_time")
                    .table(ResolumePushAudit::Table)
                    .col(ResolumePushAudit::Host)
                    .col(ResolumePushAudit::CreatedAt)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .if_not_exists()
                    .name("idx_resolume_push_audit_correlation")
                    .table(ResolumePushAudit::Table)
                    .col(ResolumePushAudit::CorrelationId)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(ResolumePushAudit::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await
    }
}
