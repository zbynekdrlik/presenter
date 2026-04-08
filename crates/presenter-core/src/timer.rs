#[cfg(test)]
use chrono::TimeDelta;
use chrono::{DateTime, Duration, Utc};
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
                let previous_state = self.countdown.state;
                self.countdown.set_target(*target)?;
                match previous_state {
                    TimerState::Running => self.countdown.start(),
                    TimerState::Paused => self.countdown.state = TimerState::Paused,
                    _ => {}
                }
            }
            TimerCommand::StartCountdown => {
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
        let countdown_remaining = self.countdown.remaining(now).num_seconds();
        let remaining_seconds = max(countdown_remaining, 0);
        let elapsed_seconds = self.preach.elapsed(now).num_seconds();
        TimersOverview {
            countdown_to_start: CountdownTimerSnapshot {
                state: self.countdown.state,
                target: self.countdown.target,
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
        Self {
            countdown_to_start: CountdownTimerSnapshot {
                state: TimerState::Running,
                target: countdown_target,
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
}
