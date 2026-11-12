//! #940 (owner 2026-11-12: "kázne staršie ako mesiac by sa mali do archívu
//! presúvať samy"): a sermon whose service date is more than a month past
//! leaves the active list on its own — the #874 archive (`archived_at`), never
//! a delete, so the Prepared tab's archive can bring it back. Once at startup,
//! then once a day; the date is a parameter, so tests choose "today".
//!
//! A sermon is archived automatically ONCE, when its date crosses the cutoff:
//! the cutoff of the last complete pass is kept (`sermons.autoArchivedThrough`,
//! a site-local app setting) and the next pass takes only the dates from there
//! on. So a sermon the preacher restored stays restored.

use chrono::{Days, NaiveDate};
use tokio::time::{interval, Duration, MissedTickBehavior};
use tracing::{info, warn};

use super::super::AppState;
use super::local_today;

/// "More than a month": a sermon older than this many days is archived
/// (31 days → archived, 30 → kept).
pub(crate) const ARCHIVE_AFTER_DAYS: u64 = 30;

/// The cutoff of the last complete pass: dates before it were handled.
pub(crate) const ARCHIVED_THROUGH_KEY: &str = "sermons.autoArchivedThrough";

const DATE_FORMAT: &str = "%Y-%m-%d";

/// How often the task looks whether the local date moved on.
const DAY_CHECK: Duration = Duration::from_secs(3600);

/// The first service date that stays active on `today`.
pub(crate) fn archive_cutoff(today: NaiveDate) -> NaiveDate {
    today
        .checked_sub_days(Days::new(ARCHIVE_AFTER_DAYS))
        .unwrap_or(NaiveDate::MIN)
}

impl AppState {
    /// The stored cutoff of the last complete pass; an unreadable value counts
    /// as none (every older sermon is looked at again, once).
    async fn archived_through(&self) -> anyhow::Result<Option<NaiveDate>> {
        let Some(raw) = self
            .repository
            .get_app_setting(ARCHIVED_THROUGH_KEY)
            .await?
        else {
            return Ok(None);
        };
        match NaiveDate::parse_from_str(&raw, DATE_FORMAT) {
            Ok(date) => Ok(Some(date)),
            Err(err) => {
                warn!(
                    target: "presenter::bible::sermon",
                    value = raw.as_str(),
                    %err,
                    "unreadable sermon archive mark, starting from the beginning"
                );
                Ok(None)
            }
        }
    }

    /// Archive every active sermon dated before [`archive_cutoff`]`(today)`
    /// that no earlier pass handled. Answers how many were archived; one that
    /// fails is logged, the rest still go, and the mark stays where it was so
    /// the next pass tries it again.
    pub(crate) async fn archive_old_sermons(&self, today: NaiveDate) -> anyhow::Result<usize> {
        let cutoff = archive_cutoff(today);
        let from = self.archived_through().await?;
        if from.is_some_and(|from| from >= cutoff) {
            return Ok(0);
        }
        let old = self.repository.active_sermons_dated(from, cutoff).await?;
        let mut archived = 0;
        let mut failed = false;
        for (id, service_date) in old {
            match self.archive_bible_presentation(id).await {
                Ok(()) => {
                    archived += 1;
                    info!(
                        target: "presenter::bible::sermon",
                        sermon_id = %id,
                        %service_date,
                        %today,
                        "sermon archived automatically (older than a month)"
                    );
                }
                Err(err) => {
                    failed = true;
                    warn!(
                        target: "presenter::bible::sermon",
                        sermon_id = %id,
                        %service_date,
                        ?err,
                        "automatic sermon archive failed"
                    );
                }
            }
        }
        if !failed {
            let mark = cutoff.format(DATE_FORMAT).to_string();
            self.repository
                .set_app_setting(ARCHIVED_THROUGH_KEY, &mark)
                .await?;
        }
        Ok(archived)
    }

    async fn archive_old_sermons_logged(&self, today: NaiveDate) {
        if let Err(err) = self.archive_old_sermons(today).await {
            warn!(target: "presenter::bible::sermon", %today, ?err, "sermon archive pass failed");
        }
    }

    /// A startup pass, then one pass whenever the server's local date changed
    /// (checked hourly).
    pub(in crate::state) fn spawn_sermon_archive_task(&self) {
        let state = self.clone();
        tokio::spawn(async move {
            let mut last = local_today();
            state.archive_old_sermons_logged(last).await;
            let mut ticker = interval(DAY_CHECK);
            ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
            ticker.tick().await;
            loop {
                ticker.tick().await;
                let today = local_today();
                if today != last {
                    last = today;
                    state.archive_old_sermons_logged(today).await;
                }
            }
        });
    }
}
