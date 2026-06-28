use axum::{
    extract::{Query, State},
    Json,
};
use serde::Deserialize;
use tracing::instrument;

use super::super::AppError;
use crate::state::AppState;
use presenter_persistence::ResolumePushAuditEntry;

/// #489: query for `GET /integrations/resolume-push-audit`. Mirrors the
/// settings-audit endpoint (`/integrations/audit`). All filters are optional;
/// `outcome` is a prefix (`error` lists every failed push, `ok` the successes).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResolumePushAuditQuery {
    pub host: Option<String>,
    pub outcome: Option<String>,
    pub since: Option<chrono::DateTime<chrono::Utc>>,
    pub limit: Option<u64>,
}

/// #489: expose the `resolume_push_audit` rows over HTTP so post-event latency +
/// failure analysis is an API call (prod has no sqlite3 — project memory: "use
/// API to query data"). Newest rows first, capped at 1000 per request.
#[instrument(skip_all)]
pub(crate) async fn list_resolume_push_audit(
    State(state): State<AppState>,
    Query(q): Query<ResolumePushAuditQuery>,
) -> Result<Json<Vec<ResolumePushAuditEntry>>, AppError> {
    let limit = q.limit.unwrap_or(100).min(1000);
    let rows = state
        .repository()
        .list_resolume_push_audit(q.host.as_deref(), q.outcome.as_deref(), q.since, limit)
        .await?;
    Ok(Json(rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(host: &str, outcome: &str) -> ResolumePushAuditEntry {
        ResolumePushAuditEntry {
            correlation_id: Some("cid-1".to_string()),
            host: host.to_string(),
            t_queue_wait_ms: 1.0,
            t_ensure_mapping_ms: 0.0,
            t_total_ms: 20.0,
            refetched: false,
            outcome: outcome.to_string(),
            created_at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn endpoint_round_trips_rows_with_host_and_outcome_filters() {
        let state = crate::state::AppState::in_memory().await.unwrap();
        state
            .repository()
            .record_resolume_push_audit(&entry("cg-resolume.lan", "ok"))
            .await
            .unwrap();
        state
            .repository()
            .record_resolume_push_audit(&entry(
                "cg-resolume.lan",
                "error: composition request failed with status 500",
            ))
            .await
            .unwrap();
        state
            .repository()
            .record_resolume_push_audit(&entry("other.lan", "ok"))
            .await
            .unwrap();

        // No filter: all three rows round-trip through the endpoint.
        let Json(all) = list_resolume_push_audit(
            State(state.clone()),
            Query(ResolumePushAuditQuery {
                host: None,
                outcome: None,
                since: None,
                limit: None,
            }),
        )
        .await
        .expect("list all");
        assert_eq!(all.len(), 3);

        // Outcome prefix filter returns only the failed push.
        let Json(errors) = list_resolume_push_audit(
            State(state.clone()),
            Query(ResolumePushAuditQuery {
                host: None,
                outcome: Some("error".to_string()),
                since: None,
                limit: None,
            }),
        )
        .await
        .expect("list errors");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].outcome.starts_with("error: "));

        // Host filter scopes to one host.
        let Json(cg) = list_resolume_push_audit(
            State(state.clone()),
            Query(ResolumePushAuditQuery {
                host: Some("cg-resolume.lan".to_string()),
                outcome: None,
                since: None,
                limit: None,
            }),
        )
        .await
        .expect("list host");
        assert_eq!(cg.len(), 2);

        // Limit is honoured.
        let Json(limited) = list_resolume_push_audit(
            State(state),
            Query(ResolumePushAuditQuery {
                host: None,
                outcome: None,
                since: None,
                limit: Some(1),
            }),
        )
        .await
        .expect("list limited");
        assert_eq!(limited.len(), 1);
    }
}
