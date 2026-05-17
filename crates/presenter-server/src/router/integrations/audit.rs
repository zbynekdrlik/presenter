use axum::{
    extract::{Query, State},
    Json,
};
use serde::Deserialize;
use tracing::instrument;

use super::super::AppError;
use crate::state::AppState;
use presenter_persistence::SettingsAuditEntry;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AuditQuery {
    pub table: Option<String>,
    pub setting_id: Option<String>,
    pub since: Option<chrono::DateTime<chrono::Utc>>,
    pub limit: Option<u64>,
}

#[instrument(skip_all)]
pub(crate) async fn list_settings_audit(
    State(state): State<AppState>,
    Query(q): Query<AuditQuery>,
) -> Result<Json<Vec<SettingsAuditEntry>>, AppError> {
    let limit = q.limit.unwrap_or(100).min(1000);
    let rows = state
        .repository()
        .list_settings_audit(q.table.as_deref(), q.setting_id.as_deref(), q.since, limit)
        .await?;
    Ok(Json(rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn list_settings_audit_returns_no_http_setter_rows_on_fresh_state() {
        let state = crate::state::AppState::in_memory().await.unwrap();
        let res = list_settings_audit(
            State(state),
            Query(AuditQuery {
                table: None,
                setting_id: None,
                since: None,
                limit: None,
            }),
        )
        .await;
        let Ok(Json(rows)) = res else {
            panic!("expected Ok");
        };
        // Fresh in-memory state contains StartupDefault rows; assert no HttpSetter rows.
        assert!(rows
            .iter()
            .all(|r| r.source != presenter_persistence::SettingsAuditSource::HttpSetter));
    }
}
