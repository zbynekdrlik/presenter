pub mod library {
    use super::presentation;
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "libraries")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: String,
        pub name: String,
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

pub mod presentation {
    use super::{library, slide};
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "presentations")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: String,
        pub library_id: String,
        pub name: String,
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
        #[sea_orm(primary_key)]
        pub id: String,
        pub presentation_id: String,
        pub position: i32,
        pub main_text: String,
        pub translation_text: String,
        pub stage_text: String,
        pub group_name: Option<String>,
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

pub mod playlist_entry {
    use super::{playlist, presentation};
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "playlist_entries")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: String,
        pub playlist_id: String,
        pub presentation_id: String,
        pub position: i32,
        pub midi_note: Option<i32>,
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
        pub source: Option<String>,
        pub created_at: DateTimeWithTimeZone,
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

pub use bible_passage::Entity as BiblePassageEntity;
pub use bible_translation::Entity as BibleTranslationEntity;
pub use library::Entity as LibraryEntity;
pub use playlist::Entity as PlaylistEntity;
pub use playlist_entry::Entity as PlaylistEntryEntity;
pub use presentation::Entity as PresentationEntity;
pub use slide::Entity as SlideEntity;
