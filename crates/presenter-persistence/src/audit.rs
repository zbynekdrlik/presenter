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

/// #483: one append-only row per Resolume per-slide push, so post-event latency
/// analysis is a SQL query rather than a journald grep. The `id` is assigned by
/// the repository on insert; the producer leaves it empty.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolumePushAuditEntry {
    /// Joins the server-side "stage click timing" line with this push.
    pub correlation_id: Option<String>,
    /// Resolume host the push targeted (e.g. `cg-resolume.lan`).
    pub host: String,
    /// Time the update sat in the per-host worker queue before pickup.
    pub t_queue_wait_ms: f64,
    /// Time spent in `ensure_mapping` (≈0 on a cache hit; the #483 spike when
    /// the old code re-fetched the whole composition inline).
    pub t_ensure_mapping_ms: f64,
    /// Total time handling this push (queue-wait excluded; pickup→done).
    pub t_total_ms: f64,
    /// Whether this push re-fetched the composition inline (should be false in
    /// steady state after #483 — only true on cold cache / error invalidation).
    pub refetched: bool,
    /// `ok` or `error` (with a short reason).
    pub outcome: String,
    /// When the push completed.
    pub created_at: chrono::DateTime<chrono::Utc>,
}
