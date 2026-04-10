#[cfg(test)]
use chrono::TimeDelta;
use chrono::{DateTime, Duration, Local, NaiveTime, Utc};
use serde::{Deserialize, Serialize};
use std::cmp::max;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimerState {
    Idle,
    Running,
    Paused,
    Completed,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TimerError {
    #[error("cannot start countdown without a future target time")]
    TargetInPast,
    #[error("invalid timer command: {0}")]
    InvalidCommand(&'static str),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CountdownTimer {
    pub target: DateTime<Utc>,
    pub state: TimerState,
}

impl CountdownTimer {
    /// Creates a new countdown timer with the given target time.
    ///
    /// # Errors
    /// Returns `TimerError::TargetInPast` if the target is not in the future.
    ///
    /// Note: For deterministic testing, prefer `new_with_now()` to avoid clock-dependent behavior.
    pub fn new(target: DateTime<Utc>) -> Result<Self, TimerError> {
        Self::new_with_now(target, Utc::now())
    }

    /// Creates a new countdown timer with explicit current time for deterministic testing.
    ///
    /// # Errors
    /// Returns `TimerError::TargetInPast` if the target is not after `now`.
    pub fn new_with_now(target: DateTime<Utc>, now: DateTime<Utc>) -> Result<Self, TimerError> {
        if target <= now {
            return Err(TimerError::TargetInPast);
        }
        Ok(Self {
            target,
            state: TimerState::Idle,
        })
    }

    pub fn start(&mut self) {
        self.state = TimerState::Running;
    }

    pub fn pause(&mut self) {
        if self.state == TimerState::Running {
            self.state = TimerState::Paused;
        }
    }

    pub fn reset(&mut self) {
        self.state = TimerState::Idle;
    }

    /// Sets a new target time for the countdown.
    ///
    /// # Errors
    /// Returns `TimerError::TargetInPast` if the target is not in the future.
    ///
    /// Note: For deterministic testing, prefer `set_target_with_now()`.
    pub fn set_target(&mut self, target: DateTime<Utc>) -> Result<(), TimerError> {
        self.set_target_with_now(target, Utc::now())
    }

    /// Sets a new target time with explicit current time for deterministic testing.
    ///
    /// # Errors
    /// Returns `TimerError::TargetInPast` if the target is not after `now`.
    pub fn set_target_with_now(
        &mut self,
        target: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<(), TimerError> {
        if target <= now {
            return Err(TimerError::TargetInPast);
        }
        self.target = target;
        self.state = TimerState::Idle;
        Ok(())
    }

    pub fn remaining(&self, now: DateTime<Utc>) -> Duration {
        let delta = self.target - now;
        if delta <= Duration::zero() {
            Duration::zero()
        } else {
            delta
        }
    }

    pub fn update_state(&mut self, now: DateTime<Utc>) {
        if self.remaining(now) == Duration::zero() {
            self.state = TimerState::Completed;
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PreachTimer {
    pub state: TimerState,
    started_at: Option<DateTime<Utc>>,
    accumulated: Duration,
    limit: Option<Duration>,
}

impl Default for PreachTimer {
    fn default() -> Self {
        Self::new()
    }
}

impl PreachTimer {
    pub fn new() -> Self {
        Self {
            state: TimerState::Idle,
            started_at: None,
            accumulated: Duration::zero(),
            limit: None,
        }
    }

    pub fn start(&mut self, now: DateTime<Utc>) {
        if self.state == TimerState::Running {
            return;
        }
        self.started_at = Some(now);
        self.state = TimerState::Running;
    }

    pub fn pause(&mut self, now: DateTime<Utc>) {
        if self.state != TimerState::Running {
            return;
        }
        if let Some(started_at) = self.started_at.take() {
            self.accumulated += now - started_at;
        }
        self.state = TimerState::Paused;
    }

    pub fn reset(&mut self) {
        self.state = TimerState::Idle;
        self.started_at = None;
        self.accumulated = Duration::zero();
    }

    pub fn elapsed(&self, now: DateTime<Utc>) -> Duration {
        match (self.state, self.started_at) {
            (TimerState::Running, Some(started_at)) => self.accumulated + (now - started_at),
            _ => self.accumulated,
        }
    }

    pub fn started_at(&self) -> Option<DateTime<Utc>> {
        self.started_at
    }

    pub fn accumulated_duration(&self) -> Duration {
        self.accumulated
    }

    pub fn set_limit(&mut self, seconds: u64) {
        self.limit = Some(Duration::seconds(seconds as i64));
    }

    pub fn clear_limit(&mut self) {
        self.limit = None;
    }

    pub fn limit_seconds(&self) -> Option<u64> {
        self.limit.map(|d| d.num_seconds().max(0) as u64)
    }

    pub fn from_parts(
        state: TimerState,
        started_at: Option<DateTime<Utc>>,
        accumulated: Duration,
        limit: Option<Duration>,
    ) -> Self {
        Self {
            state,
            started_at,
            accumulated,
            limit,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "command")]
pub enum TimerCommand {
    SetCountdownTarget { target: DateTime<Utc> },
    SetCountdownTargetLocal { hours: u32, minutes: u32 },
    AdjustCountdownTarget { offset_minutes: i32 },
    StartCountdown,
    PauseCountdown,
    ResetCountdown,
    StartPreach,
    PausePreach,
    ResetPreach,
    SetPreachLimit { seconds: u64 },
    ClearPreachLimit,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TimersState {
    pub countdown: CountdownTimer,
    pub preach: PreachTimer,
}

impl TimersState {
    pub fn new(countdown: CountdownTimer, preach: PreachTimer) -> Self {
        Self { countdown, preach }
    }

    pub fn default(now: DateTime<Utc>) -> Self {
        let countdown_target = now + Duration::minutes(15);
        Self {
            countdown: CountdownTimer {
                target: countdown_target,
                state: TimerState::Idle,
            },
            preach: PreachTimer::new(),
        }
    }

    pub fn apply_command(
        &mut self,
        command: &TimerCommand,
        now: DateTime<Utc>,
    ) -> Result<(), TimerError> {
        match command {
            TimerCommand::SetCountdownTarget { target } => {
                self.countdown.set_target_with_now(*target, now)?;
                // Auto-start: setting a countdown target always activates it,
                // regardless of source (operator UI, Companion, etc).
                self.countdown.start();
            }
            TimerCommand::SetCountdownTargetLocal { hours, minutes } => {
                let time = NaiveTime::from_hms_opt(*hours, *minutes, 0)
                    .ok_or(TimerError::InvalidCommand("invalid hours/minutes"))?;
                let local_now = Local::now();
                let today = local_now.date_naive();
                let candidate = today
                    .and_time(time)
                    .and_local_timezone(Local)
                    .single()
                    .ok_or(TimerError::InvalidCommand("ambiguous local time"))?;
                let target_local = if candidate <= local_now {
                    candidate + Duration::days(1)
                } else {
                    candidate
                };
                let target_utc = target_local.with_timezone(&Utc);
                self.countdown.set_target_with_now(target_utc, now)?;
                // Auto-start: setting a wall-clock target always activates the
                // countdown so the operator never needs a separate Start step.
                self.countdown.start();
            }
            TimerCommand::AdjustCountdownTarget { offset_minutes } => {
                let new_target =
                    self.countdown.target + Duration::minutes(i64::from(*offset_minutes));
                self.countdown.set_target_with_now(new_target, now)?;
                // Auto-start: ±5 min adjustments always leave the countdown
                // running, even if it was previously idle/paused.
                self.countdown.start();
            }
            TimerCommand::StartCountdown => {
                if self.countdown.target <= now {
                    self.countdown.target = now + Duration::minutes(15);
                }
                self.countdown.start();
            }
            TimerCommand::PauseCountdown => {
                self.countdown.pause();
            }
            TimerCommand::ResetCountdown => {
                self.countdown.reset();
            }
            TimerCommand::StartPreach => {
                self.preach.start(now);
            }
            TimerCommand::PausePreach => {
                self.preach.pause(now);
            }
            TimerCommand::ResetPreach => {
                self.preach.reset();
            }
            TimerCommand::SetPreachLimit { seconds } => {
                self.preach.set_limit(*seconds);
            }
            TimerCommand::ClearPreachLimit => {
                self.preach.clear_limit();
            }
        }
        Ok(())
    }

    pub fn overview(&self, now: DateTime<Utc>) -> TimersOverview {
        let target_local = self
            .countdown
            .target
            .with_timezone(&Local)
            .format("%H:%M:%S")
            .to_string();
        self.overview_with_local_format(now, &target_local)
    }

    fn overview_with_local_format(&self, now: DateTime<Utc>, target_local: &str) -> TimersOverview {
        let countdown_remaining = self.countdown.remaining(now).num_seconds();
        let remaining_seconds = max(countdown_remaining, 0);
        let elapsed_seconds = self.preach.elapsed(now).num_seconds();
        TimersOverview {
            countdown_to_start: CountdownTimerSnapshot {
                state: self.countdown.state,
                target: self.countdown.target,
                target_local: target_local.to_string(),
                seconds_remaining: remaining_seconds,
            },
            preach_timer: PreachTimerSnapshot {
                state: self.preach.state,
                seconds_elapsed: elapsed_seconds,
                limit_seconds: self.preach.limit_seconds(),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CountdownTimerSnapshot {
    pub state: TimerState,
    pub target: DateTime<Utc>,
    pub target_local: String,
    pub seconds_remaining: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PreachTimerSnapshot {
    pub state: TimerState,
    pub seconds_elapsed: i64,
    pub limit_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TimersOverview {
    pub countdown_to_start: CountdownTimerSnapshot,
    pub preach_timer: PreachTimerSnapshot,
}

impl TimersOverview {
    /// Produces a demo snapshot that can be used before real timers are implemented.
    pub fn demo(now: DateTime<Utc>) -> Self {
        let countdown_target = now + Duration::minutes(15);
        let target_local = countdown_target
            .with_timezone(&Local)
            .format("%H:%M:%S")
            .to_string();
        Self {
            countdown_to_start: CountdownTimerSnapshot {
                state: TimerState::Running,
                target: countdown_target,
                target_local,
                seconds_remaining: (countdown_target - now).num_seconds(),
            },
            preach_timer: PreachTimerSnapshot {
                state: TimerState::Paused,
                seconds_elapsed: 0,
                limit_seconds: None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Timelike;

    #[test]
    fn countdown_cannot_target_past() {
        let now = Utc::now();
        let past = now - TimeDelta::try_seconds(10).unwrap();
        let err = CountdownTimer::new_with_now(past, now).unwrap_err();
        assert_eq!(err, TimerError::TargetInPast);
    }

    #[test]
    fn countdown_cannot_target_exact_now() {
        let now = Utc::now();
        let err = CountdownTimer::new_with_now(now, now).unwrap_err();
        assert_eq!(err, TimerError::TargetInPast);
    }

    #[test]
    fn countdown_reports_remaining_and_completion() {
        let now = Utc::now();
        let target = now + TimeDelta::try_minutes(5).unwrap();
        let mut timer = CountdownTimer::new_with_now(target, now).unwrap();
        timer.start();
        let remaining = timer.remaining(now + TimeDelta::try_minutes(1).unwrap());
        assert_eq!(remaining.num_seconds(), 4 * 60);
        timer.update_state(target + TimeDelta::try_seconds(1).unwrap());
        assert_eq!(timer.state, TimerState::Completed);
    }

    #[test]
    fn set_target_with_now_is_deterministic() {
        let now = Utc::now();
        let initial_target = now + TimeDelta::try_minutes(5).unwrap();
        let mut timer = CountdownTimer::new_with_now(initial_target, now).unwrap();
        timer.start();

        let new_target = now + TimeDelta::try_minutes(10).unwrap();
        timer.set_target_with_now(new_target, now).unwrap();
        assert_eq!(timer.target, new_target);
        assert_eq!(timer.state, TimerState::Idle);
    }

    #[test]
    fn preach_timer_tracks_elapsed() {
        let now = Utc::now();
        let mut timer = PreachTimer::new();
        timer.start(now);
        let later = now + TimeDelta::try_minutes(2).unwrap();
        timer.pause(later);
        assert_eq!(timer.state, TimerState::Paused);
        assert_eq!(timer.elapsed(later).num_minutes(), 2);
        timer.reset();
        assert_eq!(timer.elapsed(later), Duration::zero());
    }

    #[test]
    fn timers_overview_demo_contains_future_target() {
        let now = Utc::now();
        let overview = TimersOverview::demo(now);
        assert!(overview.countdown_to_start.target > now);
        assert!(overview.countdown_to_start.seconds_remaining > 0);
        assert_eq!(overview.preach_timer.state, TimerState::Paused);
    }

    #[test]
    fn countdown_remaining_returns_zero_at_exact_target() {
        let now = Utc::now();
        let target = now + TimeDelta::try_minutes(5).unwrap();
        let timer = CountdownTimer::new_with_now(target, now).unwrap();
        assert_eq!(timer.remaining(target), Duration::zero());
    }

    #[test]
    fn countdown_remaining_returns_zero_past_target() {
        let now = Utc::now();
        let target = now + TimeDelta::try_minutes(5).unwrap();
        let timer = CountdownTimer::new_with_now(target, now).unwrap();
        let past_target = target + TimeDelta::try_seconds(10).unwrap();
        assert_eq!(timer.remaining(past_target), Duration::zero());
    }

    #[test]
    fn preach_timer_elapsed_accumulates_across_pause_resume() {
        let now = Utc::now();
        let mut timer = PreachTimer::new();
        timer.start(now);
        timer.pause(now + TimeDelta::try_seconds(30).unwrap());
        assert_eq!(timer.elapsed(now).num_seconds(), 30);
        timer.start(now + TimeDelta::try_seconds(60).unwrap());
        let elapsed = timer.elapsed(now + TimeDelta::try_seconds(70).unwrap());
        assert_eq!(elapsed.num_seconds(), 40);
    }

    #[test]
    fn preach_timer_reset_clears_accumulated() {
        let now = Utc::now();
        let mut timer = PreachTimer::new();
        timer.start(now);
        timer.pause(now + TimeDelta::try_seconds(30).unwrap());
        timer.reset();
        assert_eq!(timer.elapsed(now).num_seconds(), 0);
        assert_eq!(timer.state, TimerState::Idle);
    }

    #[test]
    fn preach_timer_set_limit() {
        let mut timer = PreachTimer::new();
        timer.set_limit(2700);
        assert_eq!(timer.limit_seconds(), Some(2700));
    }

    #[test]
    fn preach_timer_clear_limit() {
        let mut timer = PreachTimer::new();
        timer.set_limit(2700);
        timer.clear_limit();
        assert_eq!(timer.limit_seconds(), None);
    }

    #[test]
    fn preach_timer_limit_in_snapshot() {
        let now = Utc::now();
        let mut state = TimersState::default(now);
        state.preach.set_limit(1800);
        let overview = state.overview(now);
        assert_eq!(overview.preach_timer.limit_seconds, Some(1800));
    }

    #[test]
    fn preach_timer_snapshot_no_limit() {
        let now = Utc::now();
        let state = TimersState::default(now);
        let overview = state.overview(now);
        assert_eq!(overview.preach_timer.limit_seconds, None);
    }

    #[test]
    fn set_preach_limit_command() {
        let now = Utc::now();
        let mut state = TimersState::default(now);
        state
            .apply_command(&TimerCommand::SetPreachLimit { seconds: 3600 }, now)
            .unwrap();
        assert_eq!(state.preach.limit_seconds(), Some(3600));
    }

    #[test]
    fn clear_preach_limit_command() {
        let now = Utc::now();
        let mut state = TimersState::default(now);
        state
            .apply_command(&TimerCommand::SetPreachLimit { seconds: 3600 }, now)
            .unwrap();
        state
            .apply_command(&TimerCommand::ClearPreachLimit, now)
            .unwrap();
        assert_eq!(state.preach.limit_seconds(), None);
    }

    #[test]
    fn set_countdown_target_local_converts_to_utc_and_auto_starts() {
        let now = Utc::now();
        let mut state = TimersState::default(now);
        let future_hour = (chrono::Local::now().hour() + 2) % 24;
        state
            .apply_command(
                &TimerCommand::SetCountdownTargetLocal {
                    hours: future_hour,
                    minutes: 0,
                },
                now,
            )
            .unwrap();
        assert!(state.countdown.target > now);
        // Auto-start: setting a wall-clock target always activates the countdown.
        assert_eq!(state.countdown.state, TimerState::Running);
    }

    #[test]
    fn set_countdown_target_local_past_time_rolls_to_tomorrow() {
        let now = Utc::now();
        let mut state = TimersState::default(now);
        let local_now = chrono::Local::now();
        let past_hour = if local_now.hour() == 0 {
            23
        } else {
            local_now.hour() - 1
        };
        state
            .apply_command(
                &TimerCommand::SetCountdownTargetLocal {
                    hours: past_hour,
                    minutes: 0,
                },
                now,
            )
            .unwrap();
        let remaining = state.countdown.remaining(now);
        assert!(
            remaining.num_hours() >= 22,
            "expected tomorrow, got {} hours remaining",
            remaining.num_hours()
        );
    }

    #[test]
    fn set_countdown_target_local_keeps_running_when_already_running() {
        // After auto-start, setting a new target while already running stays running.
        let now = Utc::now();
        let mut state = TimersState::default(now);
        state.countdown.start();
        assert_eq!(state.countdown.state, TimerState::Running);
        let future_hour = (chrono::Local::now().hour() + 2) % 24;
        state
            .apply_command(
                &TimerCommand::SetCountdownTargetLocal {
                    hours: future_hour,
                    minutes: 0,
                },
                now,
            )
            .unwrap();
        assert_eq!(state.countdown.state, TimerState::Running);
    }

    #[test]
    fn set_countdown_target_local_rejects_invalid_time() {
        let now = Utc::now();
        let mut state = TimersState::default(now);
        let result = state.apply_command(
            &TimerCommand::SetCountdownTargetLocal {
                hours: 25,
                minutes: 0,
            },
            now,
        );
        assert!(result.is_err());
    }

    #[test]
    fn adjust_countdown_target_adds_minutes() {
        let now = Utc::now();
        let mut state = TimersState::default(now);
        let original_target = state.countdown.target;
        state
            .apply_command(
                &TimerCommand::AdjustCountdownTarget { offset_minutes: 5 },
                now,
            )
            .unwrap();
        let diff = state.countdown.target - original_target;
        assert_eq!(diff.num_minutes(), 5);
    }

    #[test]
    fn adjust_countdown_target_subtracts_minutes() {
        let now = Utc::now();
        let target = now + Duration::minutes(30);
        let mut state = TimersState::new(
            CountdownTimer::new_with_now(target, now).unwrap(),
            PreachTimer::new(),
        );
        state
            .apply_command(
                &TimerCommand::AdjustCountdownTarget { offset_minutes: -5 },
                now,
            )
            .unwrap();
        let diff = target - state.countdown.target;
        assert_eq!(diff.num_minutes(), 5);
    }

    #[test]
    fn adjust_countdown_target_rejects_result_in_past() {
        let now = Utc::now();
        let target = now + Duration::minutes(3);
        let mut state = TimersState::new(
            CountdownTimer::new_with_now(target, now).unwrap(),
            PreachTimer::new(),
        );
        let result = state.apply_command(
            &TimerCommand::AdjustCountdownTarget { offset_minutes: -5 },
            now,
        );
        assert!(result.is_err());
    }

    #[test]
    fn adjust_countdown_target_auto_starts_from_idle() {
        let now = Utc::now();
        let mut state = TimersState::default(now);
        // Confirm precondition: default state is idle.
        assert_eq!(state.countdown.state, TimerState::Idle);
        state
            .apply_command(
                &TimerCommand::AdjustCountdownTarget { offset_minutes: 5 },
                now,
            )
            .unwrap();
        // ±5 min should auto-start the countdown.
        assert_eq!(state.countdown.state, TimerState::Running);
    }

    #[test]
    fn adjust_countdown_target_keeps_running_when_already_running() {
        let now = Utc::now();
        let mut state = TimersState::default(now);
        state.countdown.start();
        state
            .apply_command(
                &TimerCommand::AdjustCountdownTarget { offset_minutes: 5 },
                now,
            )
            .unwrap();
        assert_eq!(state.countdown.state, TimerState::Running);
    }

    #[test]
    fn start_countdown_auto_sets_target_when_expired() {
        let now = Utc::now();
        let past_target = now - TimeDelta::try_minutes(10).unwrap();
        let mut state = TimersState::new(
            CountdownTimer {
                target: past_target,
                state: TimerState::Idle,
            },
            PreachTimer::new(),
        );
        state
            .apply_command(&TimerCommand::StartCountdown, now)
            .unwrap();
        assert_eq!(state.countdown.state, TimerState::Running);
        assert!(state.countdown.target > now);
        let remaining = state.countdown.remaining(now);
        assert!(
            remaining.num_minutes() >= 14 && remaining.num_minutes() <= 15,
            "expected ~15 min, got {} min",
            remaining.num_minutes()
        );
    }
}
