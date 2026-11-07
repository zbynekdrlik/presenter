//! `POST|GET /ui/usage` (#913): the GUI's press counters. The page sends a batch
//! by `sendBeacon`, so the body arrives as `text/plain`; the rows are checked
//! here, one bad row never costs the others. Read `GET /ui/usage?days=90` before
//! deciding what gets a place on a small screen (`.claude/rules/ui-usage.md`).

use super::AppError;
use crate::state::AppState;
use axum::{
    body::Bytes,
    extract::{Query, State},
    http::StatusCode,
    routing::post,
    Json, Router,
};
use chrono::{Duration, Local};
use presenter_persistence::UiUsageCount;
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::{debug, instrument};

const MAX_ROWS: usize = 200;
const MAX_NAME: usize = 80;
const MAX_COUNT: i64 = 10_000;
const KEPT_DAYS: i64 = 400;
const DEFAULT_DAYS: i64 = 30;

pub fn router() -> Router<AppState> {
    Router::new().route("/ui/usage", post(record_usage).get(read_usage))
}

#[derive(Debug, Deserialize)]
struct UsageRow {
    page: String,
    shell: String,
    role: String,
    count: i64,
}

fn is_name(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= MAX_NAME
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '/' | '.'))
}

/// A control's name and a sane count, never free text.
fn accepted(row: UsageRow) -> Option<UiUsageCount> {
    let fits = is_name(&row.page)
        && matches!(row.shell.as_str(), "phone" | "desktop")
        && is_name(&row.role)
        && (1..=MAX_COUNT).contains(&row.count);
    fits.then_some(UiUsageCount {
        page: row.page,
        shell: row.shell,
        role: row.role,
        count: row.count,
    })
}

fn date_days_ago(days: i64) -> String {
    (Local::now().date_naive() - Duration::days(days))
        .format("%Y-%m-%d")
        .to_string()
}

#[instrument(skip_all)]
async fn record_usage(State(state): State<AppState>, body: Bytes) -> Result<StatusCode, AppError> {
    let Ok(rows) = serde_json::from_slice::<Vec<UsageRow>>(&body) else {
        debug!(bytes = body.len(), "ui usage: unreadable batch");
        return Ok(StatusCode::BAD_REQUEST);
    };
    let rows: Vec<UiUsageCount> = rows
        .into_iter()
        .take(MAX_ROWS)
        .filter_map(accepted)
        .collect();
    debug!(rows = rows.len(), "ui usage: batch");
    if !rows.is_empty() {
        state
            .repository()
            .add_ui_usage(&date_days_ago(0), &date_days_ago(KEPT_DAYS), &rows)
            .await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
struct UsageQuery {
    days: Option<i64>,
}

#[instrument(skip_all)]
async fn read_usage(
    State(state): State<AppState>,
    Query(query): Query<UsageQuery>,
) -> Result<Json<Value>, AppError> {
    let days = query.days.unwrap_or(DEFAULT_DAYS).clamp(1, KEPT_DAYS);
    let rows = state
        .repository()
        .ui_usage_since(&date_days_ago(days))
        .await?;
    let rows: Vec<Value> = rows
        .into_iter()
        .map(|row| json!({"page": row.page, "shell": row.shell, "role": row.role, "count": row.count}))
        .collect();
    Ok(Json(json!({"days": days, "rows": rows})))
}

#[cfg(test)]
mod tests {
    use crate::router::build_router;
    use crate::state::AppState;
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use serde_json::{json, Value};
    use tower::ServiceExt;

    async fn post(app: &axum::Router, body: &str) -> StatusCode {
        let request = Request::builder()
            .method(Method::POST)
            .uri("/ui/usage")
            .header("content-type", "text/plain;charset=UTF-8")
            .body(Body::from(body.to_string()))
            .unwrap();
        app.clone().oneshot(request).await.unwrap().status()
    }

    async fn read(app: &axum::Router) -> Value {
        let request = Request::builder()
            .uri("/ui/usage?days=7")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn row(role: &str, count: i64) -> Value {
        json!({"page": "/ui/study", "shell": "phone", "role": role, "count": count})
    }

    #[tokio::test]
    async fn a_batch_as_a_beacon_sends_it_is_counted_and_read_back() {
        let app = build_router(AppState::in_memory().await.unwrap());
        let batch = json!([row("study-fav", 3), row("study-sermons", 1)]).to_string();
        assert_eq!(post(&app, &batch).await, StatusCode::NO_CONTENT);
        assert_eq!(post(&app, &batch).await, StatusCode::NO_CONTENT);
        let read = read(&app).await;
        assert_eq!(read["days"], 7);
        assert_eq!(
            read["rows"],
            json!([row("study-fav", 6), row("study-sermons", 2)])
        );
    }

    #[tokio::test]
    async fn a_bad_row_costs_only_itself_and_free_text_is_never_kept() {
        let app = build_router(AppState::in_memory().await.unwrap());
        let batch = json!([
            row("study-fav", 1),
            row("Ján 3,16 text verša", 1),
            row("", 1),
            row("study-zero", 0),
            row("study-huge", 10_001),
            {"page": "/ui/study", "shell": "tv", "role": "study-shell", "count": 1},
        ])
        .to_string();
        assert_eq!(post(&app, &batch).await, StatusCode::NO_CONTENT);
        assert_eq!(read(&app).await["rows"], json!([row("study-fav", 1)]));
    }

    #[tokio::test]
    async fn an_unreadable_batch_is_refused_and_adds_nothing() {
        let app = build_router(AppState::in_memory().await.unwrap());
        assert_eq!(post(&app, "not json").await, StatusCode::BAD_REQUEST);
        assert_eq!(read(&app).await["rows"], json!([]));
    }
}
