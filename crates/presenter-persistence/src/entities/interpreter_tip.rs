//! #852 šepkár tip row (`interpreter_tips`, migration
//! `m20261014_000001_create_interpreter_tips`). No relations and no sync
//! columns: site-local runtime data, never synced between SNV and PP.
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "interpreter_tips")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i32,
    /// `YYYY-MM-DD`, the server's local date at creation.
    pub service_date: String,
    pub source_term: Option<String>,
    pub target_term: Option<String>,
    pub note: Option<String>,
    pub sender_name: String,
    /// Audit + rate limit only; never sent to a client.
    pub sender_ip: String,
    /// Audit only, ≤ 200 chars.
    pub user_agent: Option<String>,
    pub created_at: DateTimeWithTimeZone,
    pub dismissed_at: Option<DateTimeWithTimeZone>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
