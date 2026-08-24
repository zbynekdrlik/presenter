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
        // #578 library sync (mirrors the #555 presentation sync columns):
        pub updated_at: DateTimeWithTimeZone,
        pub sync_id: String,
        pub deleted_at: Option<DateTimeWithTimeZone>,
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
        // #555 song sync:
        pub updated_at: DateTimeWithTimeZone,
        pub sync_id: String,
        pub deleted_at: Option<DateTimeWithTimeZone>,
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
        /// #564: runtime-discovered port (auto-recovery from Arena port
        /// drift). `NULL` means "dial `port`".
        pub active_port: Option<i32>,
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
        // #496: index of the triggered playlist entry, disambiguating which
        // occurrence of a repeated song is active. Nullable; added by
        // m20260629_000001_add_stage_active_entry_index.
        pub active_entry_index: Option<i32>,
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

pub mod settings_audit {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "settings_audit")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub setting_table: String,
        pub setting_id: String,
        pub source: String,
        pub actor: String,
        pub before_json: Option<String>,
        pub after_json: String,
        pub changed_at: DateTimeWithTimeZone,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod resolume_push_audit {
    use sea_orm::entity::prelude::*;

    // No `Eq` derive: the timing columns are `f64`, which is not `Eq`.
    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "resolume_push_audit")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub correlation_id: Option<String>,
        pub host: String,
        pub t_queue_wait_ms: f64,
        pub t_ensure_mapping_ms: f64,
        pub t_total_ms: f64,
        pub refetched: bool,
        pub outcome: String,
        pub created_at: DateTimeWithTimeZone,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod slide_stage_layout {
    use sea_orm::entity::prelude::*;

    /// #515: per-slide stage-layout marker. Triggering a slide that carries a
    /// marker switches the stage display to `layout_code`.
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "slide_stage_layouts")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub slide_id: String,
        pub presentation_id: String,
        pub layout_code: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

// ---------------------------------------------------------------------------
// #703 stream-graphics (epic #718): the four tables created by
// `m20260820_000001_create_stream_tables.rs`. Schema-only foundation — the
// repository CRUD, state manager and routes land in later epic issues.
// ---------------------------------------------------------------------------

pub mod stream_output {
    use sea_orm::entity::prelude::*;

    /// A nameable stream output (one OBS browser source). `slug` is UNIQUE;
    /// `active_scene_id` is a plain nullable INTEGER with NO foreign key (the
    /// outputs↔scenes reference is circular — the repository clears it on scene
    /// delete). Migration seeds one default row `slug='stream'`.
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "stream_outputs")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i32,
        // UNIQUE enforced by the migration's `idx_stream_outputs_slug_unique`.
        pub slug: String,
        pub name: String,
        pub default_transition_ms: i32,
        // #752: LAYER-level scene-transition durations. NULL = inherit
        // `default_transition_ms`; 0 = cut (#716). `base_*` covers all base
        // scene switches, `overlay_*` all overlay on/off toggles.
        pub base_transition_ms: Option<i32>,
        pub overlay_transition_ms: Option<i32>,
        pub active_scene_id: Option<i32>,
        pub config_revision: i32,
        pub created_at: DateTimeWithTimeZone,
        pub updated_at: DateTimeWithTimeZone,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {
        #[sea_orm(has_many = "super::stream_scene::Entity")]
        Scenes,
    }

    impl Related<super::stream_scene::Entity> for Entity {
        fn to() -> RelationDef {
            Relation::Scenes.def()
        }
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod stream_scene {
    use sea_orm::entity::prelude::*;

    /// A base or overlay scene within an output. `output_id` FK ON DELETE
    /// CASCADE. `is_active` (0/1) marks active overlays; the base scene uses
    /// `stream_outputs.active_scene_id`. `transition_ms` overrides the output
    /// default when set.
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "stream_scenes")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i32,
        pub output_id: i32,
        pub name: String,
        pub kind: String,
        pub position: i32,
        pub is_active: i32,
        pub transition_ms: Option<i32>,
        pub created_at: DateTimeWithTimeZone,
        pub updated_at: DateTimeWithTimeZone,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {
        #[sea_orm(
            belongs_to = "super::stream_output::Entity",
            from = "Column::OutputId",
            to = "super::stream_output::Column::Id",
            on_delete = "Cascade"
        )]
        Output,
        #[sea_orm(has_many = "super::stream_element::Entity")]
        Elements,
    }

    impl Related<super::stream_output::Entity> for Entity {
        fn to() -> RelationDef {
            Relation::Output.def()
        }
    }

    impl Related<super::stream_element::Entity> for Entity {
        fn to() -> RelationDef {
            Relation::Elements.def()
        }
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod stream_element {
    use sea_orm::entity::prelude::*;

    /// An element within a scene (image/countdown/lyrics/verse). `scene_id` FK
    /// ON DELETE CASCADE. All per-element style lives in `props` (JSON, later
    /// validated against `StreamElementProps` — #704).
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "stream_elements")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i32,
        pub scene_id: i32,
        pub kind: String,
        pub z_order: i32,
        pub props: String,
        pub created_at: DateTimeWithTimeZone,
        pub updated_at: DateTimeWithTimeZone,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {
        #[sea_orm(
            belongs_to = "super::stream_scene::Entity",
            from = "Column::SceneId",
            to = "super::stream_scene::Column::Id",
            on_delete = "Cascade"
        )]
        Scene,
    }

    impl Related<super::stream_scene::Entity> for Entity {
        fn to() -> RelationDef {
            Relation::Scene.def()
        }
    }

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod stream_asset {
    use sea_orm::entity::prelude::*;

    /// A sha256-addressed uploaded image. Referenced only via `props.asset_id`
    /// (no foreign key — matches the on-disk hash-named asset model), so no
    /// relations.
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "stream_assets")]
    pub struct Model {
        #[sea_orm(primary_key)]
        pub id: i32,
        // UNIQUE enforced by the migration's `idx_stream_assets_sha256_unique`.
        pub sha256: String,
        pub original_filename: String,
        pub mime: String,
        pub size_bytes: i32,
        pub width: Option<i32>,
        pub height: Option<i32>,
        pub created_at: DateTimeWithTimeZone,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

#[cfg(test)]
mod stream_entities_roundtrip_tests {
    //! #703: each stream entity must compile and round-trip (insert + select)
    //! against the real schema produced by the full migrator, and the migration
    //! must have seeded the default `stream` output that the entity can read.
    use super::{stream_asset, stream_element, stream_output, stream_scene};
    use presenter_migration::{Migrator, MigratorTrait};
    use sea_orm::{
        ActiveModelTrait, ColumnTrait, ConnectOptions, Database, DatabaseConnection, EntityTrait,
        NotSet, QueryFilter, Set,
    };

    async fn migrated_db() -> DatabaseConnection {
        // Single connection so the whole test shares one in-memory DB.
        let mut opts = ConnectOptions::new("sqlite::memory:");
        opts.max_connections(1).min_connections(1);
        let db = Database::connect(opts).await.expect("connect");
        Migrator::up(&db, None).await.expect("migrate");
        db
    }

    fn ts() -> sea_orm::prelude::DateTimeWithTimeZone {
        chrono::DateTime::parse_from_rfc3339("2026-08-20T12:00:00+00:00").expect("rfc3339")
    }

    #[tokio::test]
    async fn each_stream_entity_inserts_and_selects_back() {
        let db = migrated_db().await;

        let output = stream_output::ActiveModel {
            id: NotSet,
            slug: Set("event".to_string()),
            name: Set("Event".to_string()),
            default_transition_ms: Set(500),
            base_transition_ms: Set(Some(0)),
            overlay_transition_ms: Set(Some(800)),
            active_scene_id: Set(None),
            config_revision: Set(0),
            created_at: Set(ts()),
            updated_at: Set(ts()),
        }
        .insert(&db)
        .await
        .expect("insert output");

        let scene = stream_scene::ActiveModel {
            id: NotSet,
            output_id: Set(output.id),
            name: Set("Base".to_string()),
            kind: Set("base".to_string()),
            position: Set(0),
            is_active: Set(0),
            transition_ms: Set(None),
            created_at: Set(ts()),
            updated_at: Set(ts()),
        }
        .insert(&db)
        .await
        .expect("insert scene");

        let element = stream_element::ActiveModel {
            id: NotSet,
            scene_id: Set(scene.id),
            kind: Set("image".to_string()),
            z_order: Set(0),
            props: Set(r#"{"kind":"image"}"#.to_string()),
            created_at: Set(ts()),
            updated_at: Set(ts()),
        }
        .insert(&db)
        .await
        .expect("insert element");

        let asset = stream_asset::ActiveModel {
            id: NotSet,
            sha256: Set("abc123".to_string()),
            original_filename: Set("logo.png".to_string()),
            mime: Set("image/png".to_string()),
            size_bytes: Set(2048),
            width: Set(Some(1920)),
            height: Set(Some(1080)),
            created_at: Set(ts()),
        }
        .insert(&db)
        .await
        .expect("insert asset");

        // Round-trip: select each back and assert the values persisted.
        let got_output = stream_output::Entity::find_by_id(output.id)
            .one(&db)
            .await
            .expect("query output")
            .expect("output row");
        assert_eq!(got_output.slug, "event");
        assert_eq!(got_output.default_transition_ms, 500);
        assert_eq!(
            got_output.base_transition_ms,
            Some(0),
            "base kind transition (0 = cut)"
        );
        assert_eq!(
            got_output.overlay_transition_ms,
            Some(800),
            "overlay kind transition"
        );
        assert_eq!(got_output.active_scene_id, None);

        let got_scene = stream_scene::Entity::find_by_id(scene.id)
            .one(&db)
            .await
            .expect("query scene")
            .expect("scene row");
        assert_eq!(got_scene.output_id, output.id);
        assert_eq!(got_scene.kind, "base");

        let got_element = stream_element::Entity::find_by_id(element.id)
            .one(&db)
            .await
            .expect("query element")
            .expect("element row");
        assert_eq!(got_element.scene_id, scene.id);
        assert_eq!(got_element.props, r#"{"kind":"image"}"#);

        let got_asset = stream_asset::Entity::find_by_id(asset.id)
            .one(&db)
            .await
            .expect("query asset")
            .expect("asset row");
        assert_eq!(got_asset.sha256, "abc123");
        assert_eq!(got_asset.width, Some(1920));

        // The migration-seeded default output is readable through the entity
        // (proves the seed row's timestamp format is entity-parseable).
        let seeded = stream_output::Entity::find()
            .filter(stream_output::Column::Slug.eq("stream"))
            .one(&db)
            .await
            .expect("query seed");
        let seeded = seeded.expect("seeded default output present");
        assert_eq!(seeded.name, "Stream");
        assert_eq!(seeded.default_transition_ms, 400);
        // The kind-transition columns are NULL for the pre-#752 seeded row
        // (they inherit `default_transition_ms` at resolution time).
        assert_eq!(seeded.base_transition_ms, None);
        assert_eq!(seeded.overlay_transition_ms, None);
    }
}
