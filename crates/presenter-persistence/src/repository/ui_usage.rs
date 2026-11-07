//! How often each control of the GUI is pressed (#913): plain counters, read
//! before deciding what gets a place on a small screen (`.claude/rules/ui-usage.md`).

use super::Repository;
use anyhow::Context;
use sea_orm::{ConnectionTrait, FromQueryResult, Statement, TransactionTrait};
use tracing::instrument;

/// One control's presses: on a day when written, summed when read.
#[derive(Debug, Clone, PartialEq, Eq, FromQueryResult)]
pub struct UiUsageCount {
    pub page: String,
    pub shell: String,
    pub role: String,
    pub count: i64,
}

impl Repository {
    /// Adds `rows` to `day` and forgets the days before `oldest_kept`. Dates are
    /// `YYYY-MM-DD`, so they compare as text.
    #[instrument(skip_all)]
    pub async fn add_ui_usage(
        &self,
        day: &str,
        oldest_kept: &str,
        rows: &[UiUsageCount],
    ) -> anyhow::Result<()> {
        let backend = self.db.get_database_backend();
        let txn = self.db.begin().await.context("ui usage: begin")?;
        for row in rows {
            txn.execute(Statement::from_sql_and_values(
                backend,
                "INSERT INTO ui_usage (day, page, shell, role, count) VALUES (?, ?, ?, ?, ?) \
                 ON CONFLICT (day, page, shell, role) DO UPDATE SET count = count + excluded.count",
                [
                    day.into(),
                    row.page.clone().into(),
                    row.shell.clone().into(),
                    row.role.clone().into(),
                    row.count.into(),
                ],
            ))
            .await
            .context("ui usage: add")?;
        }
        txn.execute(Statement::from_sql_and_values(
            backend,
            "DELETE FROM ui_usage WHERE day < ?",
            [oldest_kept.into()],
        ))
        .await
        .context("ui usage: retention")?;
        txn.commit().await.context("ui usage: commit")?;
        Ok(())
    }

    /// Presses since `since`, per page, shell and control, most used first.
    #[instrument(skip_all)]
    pub async fn ui_usage_since(&self, since: &str) -> anyhow::Result<Vec<UiUsageCount>> {
        let backend = self.db.get_database_backend();
        UiUsageCount::find_by_statement(Statement::from_sql_and_values(
            backend,
            "SELECT page, shell, role, CAST(SUM(count) AS INTEGER) AS count FROM ui_usage \
             WHERE day >= ? GROUP BY page, shell, role ORDER BY count DESC, page, shell, role",
            [since.into()],
        ))
        .all(&self.db)
        .await
        .context("ui usage: read")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(role: &str, count: i64) -> UiUsageCount {
        UiUsageCount {
            page: "/ui/study".to_string(),
            shell: "phone".to_string(),
            role: role.to_string(),
            count,
        }
    }

    async fn repo() -> Repository {
        Repository::connect_in_memory().await.unwrap()
    }

    #[tokio::test]
    async fn presses_add_up_per_control_most_used_first() {
        let repo = repo().await;
        let oldest = "2026-01-01";
        repo.add_ui_usage("2026-11-06", oldest, &[press("a", 2), press("b", 5)])
            .await
            .unwrap();
        repo.add_ui_usage("2026-11-07", oldest, &[press("a", 4)])
            .await
            .unwrap();
        repo.add_ui_usage("2026-11-07", oldest, &[press("a", 1)])
            .await
            .unwrap();
        assert_eq!(
            repo.ui_usage_since("2026-11-01").await.unwrap(),
            vec![press("a", 7), press("b", 5)]
        );
        assert_eq!(
            repo.ui_usage_since("2026-11-07").await.unwrap(),
            vec![press("a", 5)]
        );
    }

    #[tokio::test]
    async fn the_days_before_the_oldest_kept_are_forgotten() {
        let repo = repo().await;
        repo.add_ui_usage("2025-01-01", "2020-01-01", &[press("old", 3)])
            .await
            .unwrap();
        repo.add_ui_usage("2026-11-07", "2026-01-01", &[press("new", 1)])
            .await
            .unwrap();
        assert_eq!(
            repo.ui_usage_since("2000-01-01").await.unwrap(),
            vec![press("new", 1)]
        );
    }
}
