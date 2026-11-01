//! #851 P2 sermon entities (spec §7.1), in their own file because
//! `entities.rs` sits at 927 production lines (cap 1000). Re-exported from
//! `entities` as `bible_sermon` / `bible_sermon_item`. Every string primary key
//! is `auto_increment = false` and is written with `Entity::insert(..).exec`
//! (database skill #658: `ActiveModel::insert` cannot read a String key back).

pub mod bible_sermon {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "bible_sermons")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub presentation_id: String,
        /// `YYYY-MM-DD`, the server-local date of the service.
        pub service_date: String,
        pub default_translation_code: String,
        pub secondary_translation_code: Option<String>,
        pub author_email: Option<String>,
        pub revision: i64,
        pub created_at: DateTimeWithTimeZone,
        pub updated_at: DateTimeWithTimeZone,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod bible_sermon_item {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "bible_sermon_items")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub presentation_id: String,
        pub item_order: i32,
        /// No foreign key on purpose (spec §5.7).
        pub translation_code: String,
        pub book_code: String,
        pub book_number: i32,
        pub book_name: String,
        pub chapter: i32,
        pub verse_start: i32,
        pub verse_end: i32,
        pub source_json: String,
        pub edits_json: String,
        pub slide_count: i32,
        pub created_at: DateTimeWithTimeZone,
        pub updated_at: DateTimeWithTimeZone,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
