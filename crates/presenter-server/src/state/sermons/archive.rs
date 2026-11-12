//! #940 (owner 2026-11-12: "kázne staršie ako mesiac by sa mali do archívu
//! presúvať samy"): a sermon whose service date is more than a month past
//! leaves the active list on its own — the #874 archive (`archived_at`), never
//! a delete, so the Prepared tab's archive can bring it back. Once at startup,
//! then once a day; the date is a parameter, so tests choose "today".

use chrono::{Days, NaiveDate};
use tokio::time::{interval, Duration, MissedTickBehavior};
use tracing::{info, warn};

use super::super::AppState;
use super::local_today;

/// "More than a month": a sermon older than this many days is archived
/// (31 days → archived, 30 → kept).
pub(crate) const ARCHIVE_AFTER_DAYS: u64 = 30;

/// How often the task looks whether the local date moved on.
const DAY_CHECK: Duration = Duration::from_secs(3600);

/// The first service date that stays active on `today`.
pub(crate) fn archive_cutoff(today: NaiveDate) -> NaiveDate {
    today
        .checked_sub_days(Days::new(ARCHIVE_AFTER_DAYS))
        .unwrap_or(NaiveDate::MIN)
}

impl AppState {
    /// Archive every active sermon dated before [`archive_cutoff`]`(today)`.
    /// Answers how many were archived; one that fails is logged and the rest
    /// still go (the next pass tries it again).
    pub(crate) async fn archive_old_sermons(&self, today: NaiveDate) -> anyhow::Result<usize> {
        let cutoff = archive_cutoff(today);
        let old = self.repository.active_sermons_before(cutoff).await?;
        let mut archived = 0;
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
                Err(err) => warn!(
                    target: "presenter::bible::sermon",
                    sermon_id = %id,
                    %service_date,
                    ?err,
                    "automatic sermon archive failed"
                ),
            }
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
