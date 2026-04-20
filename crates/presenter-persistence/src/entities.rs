pub mod library {
    use super::presentation;
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "libraries")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub name: String,
        pub search_name: String,
        pub created_at: DateTimeWithTimeZone,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {
        #[sea_orm(has_many = "presentation::Entity")]
        Presentations,
    }

    impl Related<presentation::Entity> for Entity {
        fn to() -> RelationDef {
            Relation::Presentations.def()
        }
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod library_favorite {
    use super::library;
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "library_favorites")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub library_id: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {
        #[sea_orm(
            belongs_to = "library::Entity",
            from = "Column::LibraryId",
            to = "library::Column::Id",
            on_update = "Cascade",
            on_delete = "Cascade"
        )]
        Library,
    }

    impl Related<library::Entity> for Entity {
        fn to() -> RelationDef {
            Relation::Library.def()
        }
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod presentation {
    use super::{library, slide};
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "presentations")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub library_id: String,
        pub name: String,
        pub search_name: String,
        pub created_at: DateTimeWithTimeZone,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {
        #[sea_orm(
            belongs_to = "library::Entity",
            from = "Column::LibraryId",
            to = "library::Column::Id",
            on_update = "Cascade",
            on_delete = "Cascade"
        )]
        Library,
        #[sea_orm(has_many = "slide::Entity")]
        Slides,
    }

    impl Related<library::Entity> for Entity {
        fn to() -> RelationDef {
            Relation::Library.def()
        }
    }

    impl Related<slide::Entity> for Entity {
        fn to() -> RelationDef {
            Relation::Slides.def()
        }
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod slide {
    use super::presentation;
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "slides")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub presentation_id: String,
        pub position: i32,
        // Worship columns
        pub worship_main: String,
        pub worship_main_search: String,
        pub worship_translate: String,
        pub worship_translate_search: String,
        pub worship_stage: String,
        pub worship_stage_search: String,
        pub worship_group: Option<String>,
        pub created_at: DateTimeWithTimeZone,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {
        #[sea_orm(
            belongs_to = "presentation::Entity",
            from = "Column::PresentationId",
            to = "presentation::Column::Id",
            on_update = "Cascade",
            on_delete = "Cascade"
        )]
        Presentation,
    }

    impl Related<presentation::Entity> for Entity {
        fn to() -> RelationDef {
            Relation::Presentation.def()
        }
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod playlist {
    use super::playlist_entry;
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "playlists")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: String,
        pub name: String,
        pub created_at: DateTimeWithTimeZone,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {
        #[sea_orm(has_many = "playlist_entry::Entity")]
        Entries,
    }

    impl Related<playlist_entry::Entity> for Entity {
        fn to() -> RelationDef {
            Relation::Entries.def()
        }
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod playlist_favorite {
    use super::playlist;
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "playlist_favorites")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub playlist_id: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {
        #[sea_orm(
            belongs_to = "playlist::Entity",
            from = "Column::PlaylistId",
            to = "playlist::Column::Id",
            on_update = "Cascade",
            on_delete = "Cascade"
        )]
        Playlist,
    }

    impl Related<playlist::Entity> for Entity {
        fn to() -> RelationDef {
            Relation::Playlist.def()
        }
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod resolume_host {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "resolume_hosts")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: String,
        pub label: String,
        pub host: String,
        pub port: i32,
        pub is_enabled: bool,
        pub created_at: DateTimeWithTimeZone,
        pub updated_at: DateTimeWithTimeZone,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod android_stage_display {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "android_stage_displays")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: String,
        pub label: String,
        pub host: String,
        pub port: i32,
        pub launch_component: String,
        pub is_enabled: bool,
        pub created_at: DateTimeWithTimeZone,
        pub updated_at: DateTimeWithTimeZone,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod osc_settings {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "osc_settings")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: String,
        pub enabled: bool,
        pub listen_port: i32,
        pub address_pattern: String,
        pub velocity_mode: String,
        pub created_at: DateTimeWithTimeZone,
        pub updated_at: DateTimeWithTimeZone,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod playlist_entry {
    use super::{playlist, presentation};
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "playlist_entries")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: String,
        pub playlist_id: String,
        pub entry_type: String,
        pub presentation_id: Option<String>,
        pub position: i32,
        pub midi_note: Option<i32>,
        pub label: Option<String>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {
        #[sea_orm(
            belongs_to = "playlist::Entity",
            from = "Column::PlaylistId",
            to = "playlist::Column::Id",
            on_update = "Cascade",
            on_delete = "Cascade"
        )]
        Playlist,
        #[sea_orm(
            belongs_to = "presentation::Entity",
            from = "Column::PresentationId",
            to = "presentation::Column::Id",
            on_update = "Cascade",
            on_delete = "Cascade"
        )]
        Presentation,
    }

    impl Related<playlist::Entity> for Entity {
        fn to() -> RelationDef {
            Relation::Playlist.def()
        }
    }

    impl Related<presentation::Entity> for Entity {
        fn to() -> RelationDef {
            Relation::Presentation.def()
        }
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod bible_translation {
    use super::bible_passage;
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "bible_translations")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub code: String,
        pub name: String,
        pub language: String,
        pub show_in_dashboard: bool,
        pub source: Option<String>,
        pub created_at: DateTimeWithTimeZone,
        pub source_digest: Option<String>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {
        #[sea_orm(has_many = "bible_passage::Entity")]
        Passages,
    }

    impl Related<bible_passage::Entity> for Entity {
        fn to() -> RelationDef {
            Relation::Passages.def()
        }
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod bible_passage {
    use super::bible_translation;
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "bible_passages")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: String,
        pub translation_code: String,
        pub book: String,
        pub book_code: String,
        pub book_number: i32,
        pub chapter: i32,
        pub verse_start: i32,
        pub verse_end: i32,
        pub content: String,
        pub created_at: DateTimeWithTimeZone,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {
        #[sea_orm(
            belongs_to = "bible_translation::Entity",
            from = "Column::TranslationCode",
            to = "bible_translation::Column::Code",
            on_update = "Cascade",
            on_delete = "Cascade"
        )]
        Translation,
    }

    impl Related<bible_translation::Entity> for Entity {
        fn to() -> RelationDef {
            Relation::Translation.def()
        }
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod bible_presentation {
    use super::bible_slide;
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "bible_presentations")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub name: String,
        pub created_at: DateTimeWithTimeZone,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {
        #[sea_orm(has_many = "bible_slide::Entity")]
        Slides,
    }

    impl Related<bible_slide::Entity> for Entity {
        fn to() -> RelationDef {
            Relation::Slides.def()
        }
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod bible_slide {
    use super::bible_presentation;
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "bible_slides")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub presentation_id: String,
        pub slide_order: i32,
        pub main_text: String,
        pub main_search: String,
        pub main_reference: String,
        pub secondary_text: String,
        pub secondary_search: String,
        pub secondary_reference: String,
        pub metadata_json: Option<String>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {
        #[sea_orm(
            belongs_to = "bible_presentation::Entity",
            from = "Column::PresentationId",
            to = "bible_presentation::Column::Id",
            on_update = "Cascade",
            on_delete = "Cascade"
        )]
        Presentation,
    }

    impl Related<bible_presentation::Entity> for Entity {
        fn to() -> RelationDef {
            Relation::Presentation.def()
        }
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub use app_settings::Entity as AppSettingsEntity;
pub use bible_passage::Entity as BiblePassageEntity;
pub use bible_presentation::Entity as BiblePresentationEntity;
pub use bible_slide::Entity as BibleSlideEntity;
pub use bible_translation::Entity as BibleTranslationEntity;
pub use library::Entity as LibraryEntity;
pub use playlist::Entity as PlaylistEntity;
pub use playlist_entry::Entity as PlaylistEntryEntity;
pub use presentation::Entity as PresentationEntity;
pub use slide::Entity as SlideEntity;

pub mod app_settings {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "app_settings")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub key: String,
        pub value: String,
        pub updated_at: DateTimeWithTimeZone,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod timers {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "timers")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: String,
        pub countdown_target: DateTimeWithTimeZone,
        pub countdown_state: String,
        pub preach_state: String,
        pub preach_started_at: Option<DateTimeWithTimeZone>,
        pub preach_accumulated_seconds: i64,
        pub preach_limit_seconds: Option<i64>,
        pub created_at: DateTimeWithTimeZone,
        pub updated_at: DateTimeWithTimeZone,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod stage_state {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "stage_state")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: String,
        pub presentation_id: Option<String>,
        pub current_slide_id: Option<String>,
        pub next_slide_id: Option<String>,
        pub playlist_id: Option<String>,
        pub updated_at: DateTimeWithTimeZone,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod ableset_settings {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "ableset_settings")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub enabled: bool,
        pub host: String,
        pub osc_port: i32,
        pub http_port: i32,
        pub library_name: String,
        pub song_prefix_length: i32,
        pub created_at: DateTimeWithTimeZone,
        pub updated_at: DateTimeWithTimeZone,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod video_source {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "video_sources")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub label: String,
        pub ndi_name: String,
        pub is_active: bool,
        pub created_at: DateTimeWithTimeZone,
        pub updated_at: DateTimeWithTimeZone,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod group_color {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "group_colors")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub name: String,
        pub color: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
