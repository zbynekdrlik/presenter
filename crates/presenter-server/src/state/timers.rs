use chrono::{DateTime, Utc};
use presenter_core::{TimerCommand, TimerState, TimersOverview, TimersState};

use crate::{live::LiveEvent, resolume::TimerFrame};

use super::AppState;

impl AppState {
    pub async fn timers_overview(&self) -> anyhow::Result<TimersOverview> {
        let now = Utc::now();
        let state = self.load_or_init_timers(now).await?;
        Ok(state.overview(now))
    }

    pub async fn execute_timer_command(
        &self,
        command: TimerCommand,
    ) -> anyhow::Result<TimersOverview> {
        let now = Utc::now();
        let mut state = self.load_or_init_timers(now).await?;
        state.apply_command(&command, now)?;
        self.repository.upsert_timers_state(&state).await?;
        let overview = state.overview(now);
        self.live_hub.publish(LiveEvent::Timers {
            overview: overview.clone(),
        });
        self.broadcast_stage_snapshots().await?;
        Ok(overview)
    }

    pub async fn tick_timers(&self) -> anyhow::Result<()> {
        let now = Utc::now();
        let mut state = self.load_or_init_timers(now).await?;
        let mut state_changed = false;
        if state.countdown.state == TimerState::Running {
            let previous = state.countdown.state;
            state.countdown.update_state(now);
            state_changed = state.countdown.state != previous;
        }

        let overview = state.overview(now);

        if state_changed {
            self.repository.upsert_timers_state(&state).await?;
        }

        let formatted = format_countdown_text(overview.countdown_to_start.seconds_remaining);
        self.resolume_registry
            .timer_update(TimerFrame::new(formatted))
            .await;

        self.live_hub.publish(LiveEvent::Timers { overview });

        Ok(())
    }

    pub(super) async fn load_or_init_timers(
        &self,
        now: DateTime<Utc>,
    ) -> anyhow::Result<TimersState> {
        if let Some(state) = self.repository.get_timers_state().await? {
            Ok(state)
        } else {
            let state = TimersState::default(now);
            self.repository.upsert_timers_state(&state).await?;
            self.live_hub.publish(LiveEvent::Timers {
                overview: state.overview(now),
            });
            Ok(state)
        }
    }
}

pub(crate) fn format_countdown_text(seconds_remaining: i64) -> String {
    let total = seconds_remaining.max(0);
    if total < 60 {
        total.to_string()
    } else {
        let minutes = total / 60;
        let seconds = total % 60;
        format!("{minutes:02}:{seconds:02}")
    }
}
