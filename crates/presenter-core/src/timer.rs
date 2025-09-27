#[cfg(test)]
use chrono::TimeDelta;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
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
}
