use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    #[allow(elided_lifetimes_in_paths)]
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Libraries::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Libraries::Id)
                            .string_len(36)
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Libraries::Name).string().not_null())
                    .col(
                        ColumnDef::new(Libraries::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .extra("DEFAULT CURRENT_TIMESTAMP"),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_libraries_name_unique")
                    .table(Libraries::Table)
                    .col(Libraries::Name)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(Presentations::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Presentations::Id)
                            .string_len(36)
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(Presentations::LibraryId)
                            .string_len(36)
                            .not_null(),
                    )
                    .col(ColumnDef::new(Presentations::Name).string().not_null())
                    .col(
                        ColumnDef::new(Presentations::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .extra("DEFAULT CURRENT_TIMESTAMP"),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_presentations_library")
                            .from(Presentations::Table, Presentations::LibraryId)
                            .to(Libraries::Table, Libraries::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_presentations_library_name")
                    .table(Presentations::Table)
                    .col(Presentations::LibraryId)
                    .col(Presentations::Name)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(Slides::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Slides::Id)
                            .string_len(36)
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(Slides::PresentationId)
                            .string_len(36)
                            .not_null(),
                    )
                    .col(ColumnDef::new(Slides::Position).integer().not_null())
                    .col(ColumnDef::new(Slides::MainText).text().not_null())
                    .col(ColumnDef::new(Slides::TranslationText).text().not_null())
                    .col(ColumnDef::new(Slides::StageText).text().not_null())
                    .col(ColumnDef::new(Slides::GroupName).string().null())
                    .col(
                        ColumnDef::new(Slides::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .extra("DEFAULT CURRENT_TIMESTAMP"),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_slides_presentation")
                            .from(Slides::Table, Slides::PresentationId)
                            .to(Presentations::Table, Presentations::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_slides_presentation_position")
                    .table(Slides::Table)
                    .col(Slides::PresentationId)
                    .col(Slides::Position)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(Playlists::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Playlists::Id)
                            .string_len(36)
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Playlists::Name).string().not_null())
                    .col(
                        ColumnDef::new(Playlists::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .extra("DEFAULT CURRENT_TIMESTAMP"),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_playlists_name_unique")
                    .table(Playlists::Table)
                    .col(Playlists::Name)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(PlaylistEntries::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(PlaylistEntries::Id)
                            .string_len(36)
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(PlaylistEntries::PlaylistId)
                            .string_len(36)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PlaylistEntries::PresentationId)
                            .string_len(36)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PlaylistEntries::Position)
                            .integer()
                            .not_null(),
                    )
                    .col(ColumnDef::new(PlaylistEntries::MidiNote).integer().null())
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_playlist_entries_playlist")
                            .from(PlaylistEntries::Table, PlaylistEntries::PlaylistId)
                            .to(Playlists::Table, Playlists::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_playlist_entries_presentation")
                            .from(PlaylistEntries::Table, PlaylistEntries::PresentationId)
                            .to(Presentations::Table, Presentations::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_playlist_entries_playlist_position")
                    .table(PlaylistEntries::Table)
                    .col(PlaylistEntries::PlaylistId)
                    .col(PlaylistEntries::Position)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_playlist_entries_midi_note")
                    .table(PlaylistEntries::Table)
                    .col(PlaylistEntries::PlaylistId)
                    .col(PlaylistEntries::MidiNote)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(BibleTranslations::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(BibleTranslations::Code)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(BibleTranslations::Name).string().not_null())
                    .col(
                        ColumnDef::new(BibleTranslations::Language)
                            .string()
                            .not_null(),
                    )
                    .col(ColumnDef::new(BibleTranslations::Source).string().null())
                    .col(
                        ColumnDef::new(BibleTranslations::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .extra("DEFAULT CURRENT_TIMESTAMP"),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(BiblePassages::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(BiblePassages::Id)
                            .string_len(36)
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(BiblePassages::TranslationCode)
                            .string()
                            .not_null(),
                    )
                    .col(ColumnDef::new(BiblePassages::Book).string().not_null())
                    .col(ColumnDef::new(BiblePassages::Chapter).integer().not_null())
                    .col(
                        ColumnDef::new(BiblePassages::VerseStart)
                            .integer()
                            .not_null(),
                    )
                    .col(ColumnDef::new(BiblePassages::VerseEnd).integer().not_null())
                    .col(ColumnDef::new(BiblePassages::Content).text().not_null())
                    .col(
                        ColumnDef::new(BiblePassages::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .extra("DEFAULT CURRENT_TIMESTAMP"),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_bible_passages_translation")
                            .from(BiblePassages::Table, BiblePassages::TranslationCode)
                            .to(BibleTranslations::Table, BibleTranslations::Code)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_bible_passages_reference")
                    .table(BiblePassages::Table)
                    .col(BiblePassages::TranslationCode)
                    .col(BiblePassages::Book)
                    .col(BiblePassages::Chapter)
                    .col(BiblePassages::VerseStart)
                    .col(BiblePassages::VerseEnd)
                    .unique()
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    #[allow(elided_lifetimes_in_paths)]
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(BiblePassages::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(BibleTranslations::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(PlaylistEntries::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Playlists::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Slides::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Presentations::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Libraries::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum Libraries {
    Table,
    Id,
    Name,
    CreatedAt,
}

#[derive(DeriveIden)]
enum Presentations {
    Table,
    Id,
    LibraryId,
    Name,
    CreatedAt,
}

#[derive(DeriveIden)]
enum Slides {
    Table,
    Id,
    PresentationId,
    Position,
    MainText,
    TranslationText,
    StageText,
    GroupName,
    CreatedAt,
}

#[derive(DeriveIden)]
enum Playlists {
    Table,
    Id,
    Name,
    CreatedAt,
}

#[derive(DeriveIden)]
enum PlaylistEntries {
    Table,
    Id,
    PlaylistId,
    PresentationId,
    Position,
    MidiNote,
}

#[derive(DeriveIden)]
enum BibleTranslations {
    Table,
    Code,
    Name,
    Language,
    Source,
    CreatedAt,
}

#[derive(DeriveIden)]
enum BiblePassages {
    Table,
    Id,
    TranslationCode,
    Book,
    Chapter,
    VerseStart,
    VerseEnd,
    Content,
    CreatedAt,
}
