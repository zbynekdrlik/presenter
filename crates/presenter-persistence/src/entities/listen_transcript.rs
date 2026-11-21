//! #852 C3-persist: one closed sentence of the guest's transcript
//! (`listen_transcript`, migration `m20261121_000001_create_listen_transcript`).
//! No relations and no sync columns: site-local runtime data, kept 7 days.
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "listen_transcript")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    pub mic_id: String,
    /// The `/listen/ws` sentence id (increasing across restarts).
    pub sentence_id: i64,
    pub text: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub closed_at: DateTimeWithTimeZone,
    pub language: Option<String>,
    /// `YYYY-MM-DD`, the server's local date when the row was written.
    pub service_date: String,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
