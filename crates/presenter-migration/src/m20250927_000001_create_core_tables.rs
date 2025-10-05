use sea_orm_migration::prelude::*;
use sea_orm_migration::sea_query::{Query, SimpleExpr, Value};

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
                        ColumnDef::new(Libraries::SearchName)
                            .string()
                            .not_null()
                            .default(""),
                    )
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
                    .table(LibraryFavorites::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(LibraryFavorites::LibraryId)
                            .string_len(36)
                            .not_null()
                            .primary_key(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_library_favorites_library")
                            .from(LibraryFavorites::Table, LibraryFavorites::LibraryId)
                            .to(Libraries::Table, Libraries::Id)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
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
                        ColumnDef::new(Presentations::SearchName)
                            .string()
                            .not_null()
                            .default(""),
                    )
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
                    .name("idx_presentations_library_lookup")
                    .table(Presentations::Table)
                    .col(Presentations::LibraryId)
                    .col(Presentations::Name)
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
                    .col(
                        ColumnDef::new(Slides::MainTextSearch)
                            .text()
                            .not_null()
                            .default(""),
                    )
                    .col(ColumnDef::new(Slides::TranslationText).text().not_null())
                    .col(
                        ColumnDef::new(Slides::TranslationTextSearch)
                            .text()
                            .not_null()
                            .default(""),
                    )
                    .col(ColumnDef::new(Slides::StageText).text().not_null())
                    .col(
                        ColumnDef::new(Slides::StageTextSearch)
                            .text()
                            .not_null()
                            .default(""),
                    )
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
                    .table(PlaylistFavorites::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(PlaylistFavorites::PlaylistId)
                            .string_len(36)
                            .not_null()
                            .primary_key(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_playlist_favorites_playlist")
                            .from(PlaylistFavorites::Table, PlaylistFavorites::PlaylistId)
                            .to(Playlists::Table, Playlists::Id)
                            .on_delete(ForeignKeyAction::Cascade)
                            .on_update(ForeignKeyAction::Cascade),
                    )
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
                        ColumnDef::new(PlaylistEntries::EntryType)
                            .string_len(24)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(PlaylistEntries::PresentationId)
                            .string_len(36)
                            .null(),
                    )
                    .col(
                        ColumnDef::new(PlaylistEntries::Position)
                            .integer()
                            .not_null(),
                    )
                    .col(ColumnDef::new(PlaylistEntries::MidiNote).integer().null())
                    .col(ColumnDef::new(PlaylistEntries::Label).string().null())
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
                    .table(ResolumeHosts::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(ResolumeHosts::Id)
                            .string_len(36)
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(ResolumeHosts::Label).string().not_null())
                    .col(ColumnDef::new(ResolumeHosts::Host).string().not_null())
                    .col(
                        ColumnDef::new(ResolumeHosts::Port)
                            .integer()
                            .not_null()
                            .default(8090),
                    )
                    .col(
                        ColumnDef::new(ResolumeHosts::IsEnabled)
                            .boolean()
                            .not_null()
                            .default(true),
                    )
                    .col(
                        ColumnDef::new(ResolumeHosts::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .extra("DEFAULT CURRENT_TIMESTAMP"),
                    )
                    .col(
                        ColumnDef::new(ResolumeHosts::UpdatedAt)
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
                    .name("idx_resolume_hosts_label_unique")
                    .table(ResolumeHosts::Table)
                    .col(ResolumeHosts::Label)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(OscSettings::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(OscSettings::Id)
                            .string_len(36)
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(OscSettings::Enabled)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(
                        ColumnDef::new(OscSettings::ListenPort)
                            .integer()
                            .not_null()
                            .default(9000),
                    )
                    .col(
                        ColumnDef::new(OscSettings::AddressPattern)
                            .string()
                            .not_null()
                            .default("/note"),
                    )
                    .col(
                        ColumnDef::new(OscSettings::VelocityMode)
                            .string_len(32)
                            .not_null()
                            .default("zero_based"),
                    )
                    .col(
                        ColumnDef::new(OscSettings::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .extra("DEFAULT CURRENT_TIMESTAMP"),
                    )
                    .col(
                        ColumnDef::new(OscSettings::UpdatedAt)
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
                    .table(AbleSetSettings::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(AbleSetSettings::Id)
                            .string_len(36)
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(AbleSetSettings::Enabled)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(
                        ColumnDef::new(AbleSetSettings::Host)
                            .string()
                            .not_null()
                            .default("fohabl.lan"),
                    )
                    .col(
                        ColumnDef::new(AbleSetSettings::OscPort)
                            .integer()
                            .not_null()
                            .default(39051),
                    )
                    .col(
                        ColumnDef::new(AbleSetSettings::HttpPort)
                            .integer()
                            .not_null()
                            .default(80),
                    )
                    .col(
                        ColumnDef::new(AbleSetSettings::LibraryName)
                            .string()
                            .not_null()
                            .default("NEW LEVEL"),
                    )
                    .col(
                        ColumnDef::new(AbleSetSettings::SongPrefixLength)
                            .integer()
                            .not_null()
                            .default(3),
                    )
                    .col(
                        ColumnDef::new(AbleSetSettings::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .extra("DEFAULT CURRENT_TIMESTAMP"),
                    )
                    .col(
                        ColumnDef::new(AbleSetSettings::UpdatedAt)
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
                    .table(Timers::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Timers::Id)
                            .string_len(36)
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(Timers::CountdownTarget)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Timers::CountdownState)
                            .string_len(32)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Timers::PreachState)
                            .string_len(32)
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Timers::PreachStartedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(Timers::PreachAccumulatedSeconds)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(
                        ColumnDef::new(Timers::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .extra("DEFAULT CURRENT_TIMESTAMP"),
                    )
                    .col(
                        ColumnDef::new(Timers::UpdatedAt)
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
                    .table(StageState::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(StageState::Id)
                            .string_len(36)
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(StageState::PresentationId)
                            .string_len(36)
                            .null(),
                    )
                    .col(
                        ColumnDef::new(StageState::CurrentSlideId)
                            .string_len(36)
                            .null(),
                    )
                    .col(
                        ColumnDef::new(StageState::NextSlideId)
                            .string_len(36)
                            .null(),
                    )
                    .col(
                        ColumnDef::new(StageState::UpdatedAt)
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

        manager
            .create_table(
                Table::create()
                    .table(AppSettings::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(AppSettings::Key)
                            .string_len(100)
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(AppSettings::Value).string().not_null())
                    .col(
                        ColumnDef::new(AppSettings::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .extra("DEFAULT CURRENT_TIMESTAMP"),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .exec_stmt(
                Query::insert()
                    .into_table(AppSettings::Table)
                    .columns([AppSettings::Key, AppSettings::Value])
                    .values_panic([
                        SimpleExpr::Value(Value::from("feature.companion.enabled")),
                        SimpleExpr::Value(Value::from("0")),
                    ])
                    .to_owned(),
            )
            .await?;

        manager
            .exec_stmt(
                Query::insert()
                    .into_table(AppSettings::Table)
                    .columns([AppSettings::Key, AppSettings::Value])
                    .values_panic([
                        SimpleExpr::Value(Value::from("feature.companion.port")),
                        SimpleExpr::Value(Value::from("18175")),
                    ])
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    #[allow(elided_lifetimes_in_paths)]
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(AppSettings::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(BiblePassages::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(BibleTranslations::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(StageState::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Timers::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(ResolumeHosts::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(AbleSetSettings::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(OscSettings::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(PlaylistFavorites::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(PlaylistEntries::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Playlists::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(LibraryFavorites::Table).to_owned())
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
    SearchName,
    CreatedAt,
}

#[derive(DeriveIden)]
enum LibraryFavorites {
    Table,
    LibraryId,
}

#[derive(DeriveIden)]
enum Presentations {
    Table,
    Id,
    LibraryId,
    Name,
    SearchName,
    CreatedAt,
}

#[derive(DeriveIden)]
enum Slides {
    Table,
    Id,
    PresentationId,
    Position,
    MainText,
    MainTextSearch,
    TranslationText,
    TranslationTextSearch,
    StageText,
    StageTextSearch,
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
enum PlaylistFavorites {
    Table,
    PlaylistId,
}

#[derive(DeriveIden)]
enum PlaylistEntries {
    Table,
    Id,
    PlaylistId,
    EntryType,
    PresentationId,
    Position,
    MidiNote,
    Label,
}

#[derive(DeriveIden)]
enum ResolumeHosts {
    Table,
    Id,
    Label,
    Host,
    Port,
    IsEnabled,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum OscSettings {
    Table,
    Id,
    Enabled,
    ListenPort,
    AddressPattern,
    VelocityMode,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum AbleSetSettings {
    Table,
    Id,
    Enabled,
    Host,
    OscPort,
    HttpPort,
    LibraryName,
    SongPrefixLength,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Timers {
    Table,
    Id,
    CountdownTarget,
    CountdownState,
    PreachState,
    PreachStartedAt,
    PreachAccumulatedSeconds,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum StageState {
    Table,
    Id,
    PresentationId,
    CurrentSlideId,
    NextSlideId,
    UpdatedAt,
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

#[derive(DeriveIden)]
enum AppSettings {
    Table,
    Key,
    Value,
    UpdatedAt,
}
