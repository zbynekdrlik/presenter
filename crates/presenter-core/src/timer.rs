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
    pub fn new(target: DateTime<Utc>) -> Result<Self, TimerError> {
        if target <= Utc::now() {
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

    pub fn set_target(&mut self, target: DateTime<Utc>) -> Result<(), TimerError> {
        if target <= Utc::now() {
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

    pub fn from_parts(
        state: TimerState,
        started_at: Option<DateTime<Utc>>,
        accumulated: Duration,
    ) -> Self {
        Self {
            state,
            started_at,
            accumulated,
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
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn countdown_cannot_target_past() {
        let past = Utc::now() - TimeDelta::try_seconds(10).unwrap();
        let err = CountdownTimer::new(past).unwrap_err();
        assert_eq!(err, TimerError::TargetInPast);
    }

    #[test]
    fn countdown_reports_remaining_and_completion() {
        let target = Utc::now() + TimeDelta::try_minutes(5).unwrap();
        let mut timer = CountdownTimer::new(target).unwrap();
        timer.start();
        let remaining = timer.remaining(Utc::now() + TimeDelta::try_minutes(1).unwrap());
        assert!(remaining.num_seconds() <= 4 * 60 + 1);
        timer.update_state(target + TimeDelta::try_seconds(1).unwrap());
        assert_eq!(timer.state, TimerState::Completed);
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
}
