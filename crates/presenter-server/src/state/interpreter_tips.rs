//! Šepkár tips (#852, spec §9): the state glue between the router, the
//! repository and the live hub.
//!
//! - `create` = validate (core) → duplicate check (2 min, any sender) → rate
//!   limit → insert → publish. A 422 or a duplicate never uses a rate slot.
//!   The duplicate check, the rate limit and the insert run one create at a
//!   time (`create_lock`), so two quick taps or two šepkárs sending the same
//!   word at once cannot both pass the check before either has stored a row.
//! - Every write publishes `LiveEvent::InterpreterTips` with the FULL active
//!   list (newest first, capped at `TIP_LIVE_LIST_CAP`). Publishes are
//!   serialized, so two concurrent writes can never end on the older list.
//! - One background task (`spawn_interpreter_tips_task`, started from
//!   `spawn_background_tasks`, so validate mode never runs it) deletes tips
//!   older than 30 days once per service date and re-publishes when midnight
//!   or the 4 h window changed the active list.
//! - "Now" and "today" come from an injectable `TipClock`.

mod rate_limit;

use super::AppState;
use crate::live::LiveEvent;
use chrono::{DateTime, Local, NaiveDate, Utc};
use presenter_core::interpreter_tips::{
    find_recent_duplicate, is_active, service_date_string, validate_tip, NewTipRequest, TipDto,
    TipValidationError, TipsList, ValidTip, TIP_LIVE_LIST_CAP, TIP_RETENTION_DAYS,
};
use presenter_persistence::NewInterpreterTip;
use rate_limit::{RateScope, RateVerdict, TipRateLimiter};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use tokio::time::{interval, Duration, Instant, MissedTickBehavior};

/// Where "now" and "today" come from. Production: the system clock and the
/// server's LOCAL calendar date (`chrono::Local`); tests inject a manual one.
pub(crate) trait TipClock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
    /// The service date of an instant: the server's local date.
    fn service_date(&self, at: DateTime<Utc>) -> NaiveDate;
}

struct SystemTipClock;

impl TipClock for SystemTipClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }

    fn service_date(&self, at: DateTime<Utc>) -> NaiveDate {
        at.with_timezone(&Local).date_naive()
    }
}

/// Which tips `GET /interpreter/tips` answers with.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum TipScope {
    /// The interpreter's screen: the active tips.
    #[default]
    Active,
    /// The šepkár page: every tip of today, hidden ones included.
    Today,
}

/// Who sent a request: audit + rate-limit key, never shown to anyone.
#[derive(Debug, Clone)]
pub(crate) struct TipOrigin {
    pub(crate) client_ip: String,
    pub(crate) user_agent: Option<String>,
}

/// A create's outcome: the new tip, or the earlier one it repeats.
#[derive(Debug, Clone)]
pub(crate) struct CreatedTip {
    pub(crate) tip: TipDto,
    pub(crate) duplicate: bool,
}

/// A refusal decided in this layer, not the repository, so it has its own
/// type; `router/interpreter_tips.rs` maps it to 422 / 429 + Retry-After
/// (`repository-error-pattern.md`: "a DOMAIN error gets its OWN small enum").
#[derive(Debug, thiserror::Error)]
pub(crate) enum TipRefusal {
    #[error("{0}")]
    Invalid(TipValidationError),
    #[error("Pomaly — najviac 5 tipov za minútu. Skús znova o {retry_after_secs} s.")]
    SenderRateLimited { retry_after_secs: u64 },
    #[error("Tlmočník dostal veľa tipov naraz. Skús znova o {retry_after_secs} s.")]
    OverallRateLimited { retry_after_secs: u64 },
}

impl TipRefusal {
    /// Seconds for `Retry-After`; `None` for a validation refusal.
    pub(crate) fn retry_after_secs(&self) -> Option<u64> {
        match self {
            Self::Invalid(_) => None,
            Self::SenderRateLimited { retry_after_secs }
            | Self::OverallRateLimited { retry_after_secs } => Some(*retry_after_secs),
        }
    }
}

/// The tip state every `AppState` clone shares.
#[derive(Clone)]
pub(crate) struct InterpreterTipsState {
    clock: Arc<RwLock<Arc<dyn TipClock>>>,
    limiter: Arc<Mutex<TipRateLimiter>>,
    /// Held across "duplicate check + rate limit + insert" of one create.
    create_lock: Arc<tokio::sync::Mutex<()>>,
    /// Held across "read the active list + publish it".
    publish_lock: Arc<tokio::sync::Mutex<()>>,
    /// Ids of the last published list. The minute tick reads the database
    /// only while it is non-empty.
    published: Arc<Mutex<Vec<i64>>>,
}

impl InterpreterTipsState {
    pub(crate) fn new() -> Self {
        Self {
            clock: Arc::new(RwLock::new(Arc::new(SystemTipClock))),
            limiter: Arc::new(Mutex::new(TipRateLimiter::new())),
            create_lock: Arc::new(tokio::sync::Mutex::new(())),
            publish_lock: Arc::new(tokio::sync::Mutex::new(())),
            published: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn clock(&self) -> Arc<dyn TipClock> {
        self.clock
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn published_ids(&self) -> Vec<i64> {
        self.published
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn set_published_ids(&self, ids: Vec<i64>) {
        *self
            .published
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = ids;
    }
}

/// What the background task remembers between minutes.
#[derive(Debug, Default)]
struct TipMaintenance {
    /// Service date of the last retention pass.
    last_prune: Option<NaiveDate>,
    /// Consecutive failed boundary checks (#484 power-of-two WARN gate).
    tick_failures: u32,
}

/// Today's tips plus the instant and the date they were judged at.
struct TipsSnapshot {
    now: DateTime<Utc>,
    today: NaiveDate,
    todays: Vec<TipDto>,
}

impl TipsSnapshot {
    fn active(&self) -> Vec<TipDto> {
        self.todays
            .iter()
            .filter(|tip| is_active(tip, self.today, self.now))
            .cloned()
            .collect()
    }
}

fn ids(tips: &[TipDto]) -> Vec<i64> {
    tips.iter().map(|tip| tip.id).collect()
}

fn log_tip(action: &'static str, tip: &TipDto, client_ip: &str, duplicate: bool) {
    tracing::info!(
        target: "presenter::interpreter::tip",
        action,
        id = tip.id,
        sender_name = tip.sender_name.as_str(),
        client_ip,
        chars = tip.chars(),
        duplicate,
        "šepkár tip"
    );
}

impl AppState {
    async fn tips_snapshot(&self) -> anyhow::Result<TipsSnapshot> {
        let clock = self.interpreter_tips.clock();
        let now = clock.now();
        let today = clock.service_date(now);
        let todays = self.repository.list_interpreter_tips_on(today).await?;
        Ok(TipsSnapshot { now, today, todays })
    }

    /// `GET /interpreter/tips`: the active tips or all of today's, plus the
    /// active ids. Not capped (only the live event is).
    pub(crate) async fn interpreter_tips_list(&self, scope: TipScope) -> anyhow::Result<TipsList> {
        let snapshot = self.tips_snapshot().await?;
        let active = snapshot.active();
        let active_ids = ids(&active);
        let service_date = service_date_string(snapshot.today);
        let tips = match scope {
            TipScope::Active => active,
            TipScope::Today => snapshot.todays,
        };
        Ok(TipsList {
            service_date,
            tips,
            active_ids,
        })
    }

    /// `POST /interpreter/tips` (spec §9.4).
    pub(crate) async fn create_interpreter_tip(
        &self,
        request: &NewTipRequest,
        origin: TipOrigin,
    ) -> anyhow::Result<CreatedTip> {
        let valid = validate_tip(request).map_err(TipRefusal::Invalid)?;
        let created = {
            let _one_at_a_time = self.interpreter_tips.create_lock.lock().await;
            self.store_new_tip(valid, &origin).await?
        };
        if !created.duplicate {
            self.publish_interpreter_tips_logged().await;
        }
        Ok(created)
    }

    /// The part of a create that must not interleave with another create:
    /// the duplicate check, the rate limit and the insert.
    async fn store_new_tip(
        &self,
        valid: ValidTip,
        origin: &TipOrigin,
    ) -> anyhow::Result<CreatedTip> {
        let snapshot = self.tips_snapshot().await?;
        if let Some(earlier) = find_recent_duplicate(&snapshot.todays, &valid, snapshot.now) {
            log_tip("create", earlier, &origin.client_ip, true);
            return Ok(CreatedTip {
                tip: earlier.clone(),
                duplicate: true,
            });
        }
        self.check_tip_rate(&origin.client_ip, &valid.sender_name)?;
        let tip = self
            .repository
            .insert_interpreter_tip(NewInterpreterTip {
                service_date: snapshot.today,
                source_term: valid.source_term,
                target_term: valid.target_term,
                note: valid.note,
                sender_name: valid.sender_name,
                sender_ip: origin.client_ip.clone(),
                user_agent: origin.user_agent.clone(),
                created_at: snapshot.now,
            })
            .await?;
        log_tip("create", &tip, &origin.client_ip, false);
        Ok(CreatedTip {
            tip,
            duplicate: false,
        })
    }

    fn check_tip_rate(&self, client_ip: &str, sender_name: &str) -> Result<(), TipRefusal> {
        let verdict = self
            .interpreter_tips
            .limiter
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .check(client_ip, sender_name, Instant::now());
        let RateVerdict::Refused {
            scope,
            retry_after_secs,
            warn,
        } = verdict
        else {
            return Ok(());
        };
        if let Some(suppressed) = warn {
            tracing::warn!(
                target: "presenter::interpreter::tip",
                scope = scope.as_str(),
                sender_name,
                client_ip,
                retry_after_secs,
                suppressed,
                "šepkár tip refused by the rate limit"
            );
        }
        Err(match scope {
            RateScope::Sender => TipRefusal::SenderRateLimited { retry_after_secs },
            RateScope::Overall => TipRefusal::OverallRateLimited { retry_after_secs },
        })
    }

    /// `POST /interpreter/tips/{id}/dismiss` — idempotent.
    pub(crate) async fn dismiss_interpreter_tip(
        &self,
        id: i64,
        client_ip: &str,
    ) -> anyhow::Result<TipDto> {
        let now = self.interpreter_tips.clock().now();
        let tip = self.repository.dismiss_interpreter_tip(id, now).await?;
        log_tip("dismiss", &tip, client_ip, false);
        self.publish_interpreter_tips_logged().await;
        Ok(tip)
    }

    /// `POST /interpreter/tips/{id}/restore` — the 5 s undo.
    pub(crate) async fn restore_interpreter_tip(
        &self,
        id: i64,
        client_ip: &str,
    ) -> anyhow::Result<TipDto> {
        let clock = self.interpreter_tips.clock();
        let today = clock.service_date(clock.now());
        let tip = self.repository.restore_interpreter_tip(id, today).await?;
        log_tip("restore", &tip, client_ip, false);
        self.publish_interpreter_tips_logged().await;
        Ok(tip)
    }

    /// `POST /interpreter/tips/dismiss-all` — hides every active tip.
    pub(crate) async fn dismiss_all_interpreter_tips(
        &self,
        client_ip: &str,
    ) -> anyhow::Result<u64> {
        let snapshot = self.tips_snapshot().await?;
        let active = ids(&snapshot.active());
        let dismissed = self
            .repository
            .dismiss_interpreter_tips(&active, snapshot.now)
            .await?;
        tracing::info!(
            target: "presenter::interpreter::tip",
            action = "dismiss_all",
            dismissed,
            client_ip,
            "šepkár tips hidden"
        );
        self.publish_interpreter_tips_logged().await;
        Ok(dismissed)
    }

    /// Publish the full active list (newest first, capped) and remember its
    /// ids. Serialized: the last publish always reflects the last commit.
    pub(crate) async fn publish_interpreter_tips(&self) -> anyhow::Result<()> {
        let _serialized = self.interpreter_tips.publish_lock.lock().await;
        let snapshot = self.tips_snapshot().await?;
        let mut tips = snapshot.active();
        tips.truncate(TIP_LIVE_LIST_CAP);
        self.interpreter_tips.set_published_ids(ids(&tips));
        self.live_hub.publish(LiveEvent::InterpreterTips {
            service_date: service_date_string(snapshot.today),
            tips,
        });
        Ok(())
    }

    async fn publish_interpreter_tips_logged(&self) {
        if let Err(err) = self.publish_interpreter_tips().await {
            tracing::warn!(
                target: "presenter::interpreter::tip",
                ?err,
                "publishing the šepkár tips failed — clients resync on reconnect"
            );
        }
    }

    /// The minute boundary check: re-publish when midnight or the 4 h window
    /// changed the active list. Reads nothing while the last list was empty.
    pub(crate) async fn interpreter_tips_tick(&self) -> anyhow::Result<()> {
        let published = self.interpreter_tips.published_ids();
        if published.is_empty() {
            return Ok(());
        }
        let snapshot = self.tips_snapshot().await?;
        let mut active = snapshot.active();
        active.truncate(TIP_LIVE_LIST_CAP);
        if ids(&active) != published {
            self.publish_interpreter_tips().await?;
        }
        Ok(())
    }

    /// Delete tips whose service date is more than `TIP_RETENTION_DAYS` ago.
    pub(crate) async fn prune_interpreter_tips(&self) -> anyhow::Result<u64> {
        let clock = self.interpreter_tips.clock();
        let today = clock.service_date(clock.now());
        let Some(cutoff) = today.checked_sub_signed(chrono::Duration::days(TIP_RETENTION_DAYS))
        else {
            return Ok(0);
        };
        let pruned = self
            .repository
            .prune_interpreter_tips_before(cutoff)
            .await?;
        if pruned > 0 {
            tracing::info!(
                target: "presenter::interpreter::tip",
                pruned,
                retention_days = TIP_RETENTION_DAYS,
                "old šepkár tips deleted"
            );
        }
        Ok(pruned)
    }

    /// One minute of the background task: retention once per service date
    /// (one attempt, failed or not — a broken DB must not WARN every minute),
    /// then the boundary check (failures WARN on #484 power-of-two streaks).
    async fn interpreter_tips_maintenance(&self, memory: &mut TipMaintenance) {
        let clock = self.interpreter_tips.clock();
        let today = clock.service_date(clock.now());
        if memory.last_prune != Some(today) {
            memory.last_prune = Some(today);
            if let Err(err) = self.prune_interpreter_tips().await {
                tracing::warn!(
                    target: "presenter::interpreter::tip",
                    ?err,
                    "šepkár tip retention failed — next try tomorrow"
                );
            }
        }
        match self.interpreter_tips_tick().await {
            Ok(()) => {
                if memory.tick_failures > 0 {
                    tracing::info!(
                        target: "presenter::interpreter::tip",
                        failures = memory.tick_failures,
                        "šepkár tip boundary check recovered"
                    );
                }
                memory.tick_failures = 0;
            }
            Err(err) => {
                memory.tick_failures = memory.tick_failures.saturating_add(1);
                if memory.tick_failures.is_power_of_two() {
                    tracing::warn!(
                        target: "presenter::interpreter::tip",
                        ?err,
                        failures = memory.tick_failures,
                        "šepkár tip boundary check failed"
                    );
                }
            }
        }
    }

    /// The minute task (spec §9.5 boundary + §9.9 retention). The first
    /// publish seeds `published`, so a restart mid-service still sends the
    /// boundary event for the tips already on screen.
    pub(super) fn spawn_interpreter_tips_task(&self) {
        let state = self.clone();
        tokio::spawn(async move {
            state.publish_interpreter_tips_logged().await;
            let mut memory = TipMaintenance::default();
            let mut ticker = interval(Duration::from_secs(60));
            ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                state.interpreter_tips_maintenance(&mut memory).await;
            }
        });
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use tokio::sync::broadcast;

    /// A settable clock judging service dates in a fixed zone.
    pub(crate) struct ManualTipClock {
        now: Mutex<DateTime<Utc>>,
        offset: chrono::FixedOffset,
    }

    impl ManualTipClock {
        /// `local` is RFC 3339 with the zone, e.g. `2026-10-18T19:42:00+02:00`.
        pub(crate) fn at(local: &str) -> Arc<Self> {
            let parsed = DateTime::parse_from_rfc3339(local).expect("RFC 3339 time");
            Arc::new(Self {
                now: Mutex::new(parsed.with_timezone(&Utc)),
                offset: *parsed.offset(),
            })
        }

        pub(crate) fn advance(&self, by: chrono::Duration) {
            let mut now = self.now.lock().unwrap_or_else(PoisonError::into_inner);
            *now += by;
        }
    }

    impl TipClock for ManualTipClock {
        fn now(&self) -> DateTime<Utc> {
            *self.now.lock().unwrap_or_else(PoisonError::into_inner)
        }

        fn service_date(&self, at: DateTime<Utc>) -> NaiveDate {
            at.with_timezone(&self.offset).date_naive()
        }
    }

    /// The last `InterpreterTips` event waiting in `rx`, skipping every other
    /// event (heartbeats interleave); `None` if there is none.
    pub(crate) fn last_tips_event(
        rx: &mut broadcast::Receiver<LiveEvent>,
    ) -> Option<(String, Vec<TipDto>)> {
        let mut last = None;
        while let Ok(event) = rx.try_recv() {
            if let LiveEvent::InterpreterTips { service_date, tips } = event {
                last = Some((service_date, tips));
            }
        }
        last
    }

    impl AppState {
        pub(crate) fn set_interpreter_tip_clock(&self, clock: Arc<dyn TipClock>) {
            *self
                .interpreter_tips
                .clock
                .write()
                .unwrap_or_else(PoisonError::into_inner) = clock;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{last_tips_event, ManualTipClock};
    use super::*;
    use crate::tracing_capture::Capture;
    use tokio::sync::broadcast::error::RecvError;

    fn origin() -> TipOrigin {
        TipOrigin {
            client_ip: "203.0.113.9".to_string(),
            user_agent: None,
        }
    }

    fn note(sender: &str, text: &str) -> NewTipRequest {
        NewTipRequest {
            sender_name: sender.to_string(),
            note: Some(text.to_string()),
            ..NewTipRequest::default()
        }
    }

    async fn state_at(local: &str) -> (AppState, Arc<ManualTipClock>) {
        let state = AppState::in_memory().await.expect("state");
        let clock = ManualTipClock::at(local);
        state.set_interpreter_tip_clock(clock.clone());
        (state, clock)
    }

    fn stored(day: NaiveDate, created_at: DateTime<Utc>, text: &str) -> NewInterpreterTip {
        NewInterpreterTip {
            service_date: day,
            source_term: None,
            target_term: None,
            note: Some(text.to_string()),
            sender_name: "Peter".to_string(),
            sender_ip: "192.0.2.1".to_string(),
            user_agent: None,
            created_at,
        }
    }

    fn day(month: u32, date: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, month, date).expect("date")
    }

    #[tokio::test]
    async fn the_four_hour_window_empties_the_list_on_the_next_tick() {
        let (state, clock) = state_at("2026-10-18T17:00:00+02:00").await;
        let mut rx = state.live_hub().subscribe();
        let created = state
            .create_interpreter_tip(&note("Peter", "a"), origin())
            .await
            .expect("create");
        let (_, tips) = last_tips_event(&mut rx).expect("create publishes");
        assert_eq!(ids(&tips), vec![created.tip.id]);

        clock.advance(chrono::Duration::minutes(3 * 60 + 59));
        state.interpreter_tips_tick().await.expect("tick");
        assert!(
            last_tips_event(&mut rx).is_none(),
            "still inside the 4 h window"
        );

        clock.advance(chrono::Duration::minutes(2));
        state.interpreter_tips_tick().await.expect("tick");
        let (date, tips) = last_tips_event(&mut rx).expect("the boundary publishes");
        assert_eq!(date, "2026-10-18");
        assert!(tips.is_empty());

        clock.advance(chrono::Duration::minutes(1));
        state.interpreter_tips_tick().await.expect("tick");
        assert!(
            last_tips_event(&mut rx).is_none(),
            "an empty list is not re-published"
        );
    }

    #[tokio::test]
    async fn midnight_empties_the_list_on_the_next_tick() {
        let (state, clock) = state_at("2026-10-18T23:50:00+02:00").await;
        let mut rx = state.live_hub().subscribe();
        state
            .create_interpreter_tip(&note("Peter", "a"), origin())
            .await
            .expect("create");
        assert_eq!(last_tips_event(&mut rx).expect("create").1.len(), 1);
        clock.advance(chrono::Duration::minutes(15));
        state.interpreter_tips_tick().await.expect("tick");
        let (date, tips) = last_tips_event(&mut rx).expect("midnight publishes");
        assert_eq!(date, "2026-10-19");
        assert!(tips.is_empty());
    }

    #[tokio::test]
    async fn a_restart_mid_service_still_sends_the_boundary_event() {
        let (state, clock) = state_at("2026-10-18T17:00:00+02:00").await;
        let now = state.interpreter_tips.clock().now();
        state
            .repository()
            .insert_interpreter_tip(stored(day(10, 18), now, "before the restart"))
            .await
            .expect("insert");
        // The task's first step after a (re)start seeds the published list.
        state.publish_interpreter_tips().await.expect("seed");
        let mut rx = state.live_hub().subscribe();
        clock.advance(chrono::Duration::hours(4));
        state.interpreter_tips_tick().await.expect("tick");
        assert!(last_tips_event(&mut rx).expect("boundary").1.is_empty());
    }

    #[tokio::test]
    async fn the_task_publishes_the_list_on_the_tips_already_stored_at_start() {
        let (state, _clock) = state_at("2026-10-18T17:00:00+02:00").await;
        let now = state.interpreter_tips.clock().now();
        let tip = state
            .repository()
            .insert_interpreter_tip(stored(day(10, 18), now, "stored before the start"))
            .await
            .expect("insert");
        let mut rx = state.live_hub().subscribe();
        state.spawn_interpreter_tips_task();
        let published = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                match rx.recv().await {
                    Ok(LiveEvent::InterpreterTips { tips, .. }) => break tips,
                    Ok(_) | Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => panic!("the live hub closed"),
                }
            }
        })
        .await
        .expect("the task publishes at start");
        assert_eq!(ids(&published), vec![tip.id]);
    }

    #[tokio::test]
    async fn the_tick_publishes_nothing_while_the_list_is_empty() {
        let (state, clock) = state_at("2026-10-18T17:00:00+02:00").await;
        let mut rx = state.live_hub().subscribe();
        clock.advance(chrono::Duration::hours(8));
        state.interpreter_tips_tick().await.expect("tick");
        assert!(last_tips_event(&mut rx).is_none());
    }

    #[tokio::test]
    async fn retention_runs_at_startup_and_once_per_service_date() {
        let (state, clock) = state_at("2026-10-18T10:00:00+02:00").await;
        let now = state.interpreter_tips.clock().now();
        let repo = state.repository();
        repo.insert_interpreter_tip(stored(day(9, 17), now, "31 days"))
            .await
            .expect("insert");
        repo.insert_interpreter_tip(stored(day(9, 18), now, "30 days"))
            .await
            .expect("insert");
        let mut memory = TipMaintenance::default();
        state.interpreter_tips_maintenance(&mut memory).await;
        assert!(repo
            .list_interpreter_tips_on(day(9, 17))
            .await
            .expect("list")
            .is_empty());
        assert_eq!(
            repo.list_interpreter_tips_on(day(9, 18))
                .await
                .expect("list")
                .len(),
            1
        );

        repo.insert_interpreter_tip(stored(day(9, 16), now, "late arrival"))
            .await
            .expect("insert");
        state.interpreter_tips_maintenance(&mut memory).await;
        assert_eq!(
            repo.list_interpreter_tips_on(day(9, 16))
                .await
                .expect("list")
                .len(),
            1,
            "one retention pass per service date"
        );

        clock.advance(chrono::Duration::days(1));
        state.interpreter_tips_maintenance(&mut memory).await;
        assert!(repo
            .list_interpreter_tips_on(day(9, 16))
            .await
            .expect("list")
            .is_empty());
        assert!(
            repo.list_interpreter_tips_on(day(9, 18))
                .await
                .expect("list")
                .is_empty(),
            "now 31 days old"
        );
    }

    #[tokio::test]
    async fn a_failing_boundary_check_warns_on_power_of_two_streaks_and_logs_the_recovery() {
        let capture = Capture::install();
        let (state, _clock) = state_at("2026-10-18T17:00:00+02:00").await;
        state
            .create_interpreter_tip(&note("Peter", "a"), origin())
            .await
            .expect("create");
        let conn = state.repository().connection();
        sea_orm::ConnectionTrait::execute_unprepared(
            conn,
            "ALTER TABLE interpreter_tips RENAME TO interpreter_tips_off",
        )
        .await
        .expect("take the table away");
        let mut memory = TipMaintenance::default();
        for _ in 0..3 {
            state.interpreter_tips_maintenance(&mut memory).await;
        }
        assert_eq!(memory.tick_failures, 3);
        assert_eq!(
            capture
                .events_with_message("šepkár tip boundary check failed")
                .len(),
            2,
            "WARN on the 1st and 2nd failure, quiet on the 3rd, the next one is the 4th"
        );
        assert_eq!(
            capture
                .events_with_message("šepkár tip retention failed")
                .len(),
            1,
            "one retention attempt per service date, failed or not"
        );

        sea_orm::ConnectionTrait::execute_unprepared(
            conn,
            "ALTER TABLE interpreter_tips_off RENAME TO interpreter_tips",
        )
        .await
        .expect("give the table back");
        state.interpreter_tips_maintenance(&mut memory).await;
        assert_eq!(memory.tick_failures, 0);
        let recovered = capture.events_with_message("šepkár tip boundary check recovered");
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].field("failures"), Some("3"));
    }

    #[tokio::test]
    async fn the_live_list_is_capped_at_twenty_newest_first_but_get_is_not() {
        let (state, _clock) = state_at("2026-10-18T19:00:00+02:00").await;
        let now = state.interpreter_tips.clock().now();
        for n in 0..25 {
            state
                .repository()
                .insert_interpreter_tip(stored(day(10, 18), now, &format!("tip {n}")))
                .await
                .expect("insert");
        }
        let mut rx = state.live_hub().subscribe();
        state.publish_interpreter_tips().await.expect("publish");
        let (_, tips) = last_tips_event(&mut rx).expect("published");
        assert_eq!(tips.len(), TIP_LIVE_LIST_CAP);
        assert_eq!(tips[0].note.as_deref(), Some("tip 24"));
        assert!(tips.windows(2).all(|pair| pair[0].id > pair[1].id));
        let list = state
            .interpreter_tips_list(TipScope::Active)
            .await
            .expect("list");
        assert_eq!(list.tips.len(), 25);
        assert_eq!(list.active_ids.len(), 25);
    }

    #[tokio::test]
    async fn two_creates_of_the_same_tip_at_once_store_one_row() {
        let (state, _clock) = state_at("2026-10-18T19:00:00+02:00").await;
        let (first, second) = tokio::join!(
            state.create_interpreter_tip(&note("Peter", "same word"), origin()),
            state.create_interpreter_tip(&note("Jana", "Same  word"), origin()),
        );
        let (first, second) = (first.expect("first"), second.expect("second"));
        assert_ne!(
            first.duplicate, second.duplicate,
            "exactly one of the two repeats the other"
        );
        assert_eq!(first.tip.id, second.tip.id);
        let today = state
            .interpreter_tips_list(TipScope::Today)
            .await
            .expect("list");
        assert_eq!(today.tips.len(), 1);
    }
}
