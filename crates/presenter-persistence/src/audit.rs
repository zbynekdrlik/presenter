use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingsAuditSource {
    HttpSetter,
    CompanionSetter,
    StartupDefault,
    SchemaMigration,
}

impl SettingsAuditSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HttpSetter => "http_setter",
            Self::CompanionSetter => "companion_setter",
            Self::StartupDefault => "startup_default",
            Self::SchemaMigration => "schema_migration",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsAuditEntry {
    pub id: String,
    pub setting_table: String,
    pub setting_id: String,
    pub source: SettingsAuditSource,
    pub actor: String,
    pub before_json: Option<serde_json::Value>,
    pub after_json: serde_json::Value,
    pub changed_at: chrono::DateTime<chrono::Utc>,
}
