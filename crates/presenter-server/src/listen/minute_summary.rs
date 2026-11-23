//! The per-minute mic summary (#933, dstack#261): one INFO line per mic and
//! minute so a service's mic levels can be read afterwards from the journal
//! (`GET /listen/status` only shows the live value). The accumulators live in
//! each mic's counters ([`MinuteMeter`]); the supervisor takes them every
//! minute. A mic that went silent gets ONE line with `packets=0`, then none
//! until packets return (the #484 log-flood rule).

use std::time::Duration;

use presenter_core::audio::level::{is_voiced, SILENCE_DBFS};
use tokio::time::{interval_at, Instant, MissedTickBehavior};
use tracing::info;

use super::{ListenService, MIC_TARGET};

#[cfg(test)]
mod tests;

/// The summary period.
pub(crate) const SUMMARY_EVERY: Duration = Duration::from_secs(60);

/// What one mic did in one minute.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MinuteSummary {
    pub(crate) packets: u64,
    pub(crate) lost: u64,
    pub(crate) late: u64,
    /// Loudest chain batch of the minute (dBFS); `SILENCE_DBFS` without a batch.
    pub(crate) level_dbfs_max: f32,
    /// Mean over the minute's chain batches in the POWER domain
    /// (`10·log10(mean(10^(dBFS/10)))`, so one loud word is not averaged away
    /// by silence); `SILENCE_DBFS` without a batch.
    pub(crate) level_dbfs_avg: f32,
    /// Share of the minute's chain batches that were voiced, 0–100.
    pub(crate) voiced_pct: u8,
}

/// The accumulators of the current minute plus the counter values at its start.
#[derive(Debug, Default)]
pub(crate) struct MinuteMeter {
    base_packets: u64,
    base_lost: u64,
    base_late: u64,
    /// The previous minute had packets: its silence gets its one zero line.
    active: bool,
    batches: u32,
    voiced: u32,
    max_dbfs: Option<f32>,
    power_sum: f64,
}

impl MinuteMeter {
    /// One chain batch's level (a current chain only).
    pub(crate) fn batch(&mut self, level_dbfs: f32) {
        self.batches += 1;
        if is_voiced(level_dbfs) {
            self.voiced += 1;
        }
        self.max_dbfs = Some(self.max_dbfs.map_or(level_dbfs, |max| max.max(level_dbfs)));
        self.power_sum += 10f64.powf(f64::from(level_dbfs) / 10.0);
    }

    /// Close the minute against the mic's running counters and start the next
    /// one; `None` when the mic has nothing to say (silent, and already said so).
    pub(crate) fn take(&mut self, packets: u64, lost: u64, late: u64) -> Option<MinuteSummary> {
        let delta = packets.saturating_sub(self.base_packets);
        let summary = MinuteSummary {
            packets: delta,
            lost: lost.saturating_sub(self.base_lost),
            late: late.saturating_sub(self.base_late),
            level_dbfs_max: self.max_dbfs.unwrap_or(SILENCE_DBFS),
            level_dbfs_avg: self.avg_dbfs(),
            voiced_pct: self.voiced_pct(),
        };
        let say = delta > 0 || self.active;
        *self = Self {
            base_packets: packets,
            base_lost: lost,
            base_late: late,
            active: delta > 0,
            ..Self::default()
        };
        say.then_some(summary)
    }

    fn avg_dbfs(&self) -> f32 {
        if self.batches == 0 {
            return SILENCE_DBFS;
        }
        let mean = self.power_sum / f64::from(self.batches);
        ((10.0 * mean.log10()) as f32).max(SILENCE_DBFS)
    }

    fn voiced_pct(&self) -> u8 {
        if self.batches == 0 {
            return 0;
        }
        let pct = (f64::from(self.voiced) * 100.0 / f64::from(self.batches)).round();
        pct.clamp(0.0, 100.0) as u8
    }
}

/// Take every mic's minute and log one line per mic that has one.
pub(super) fn log_minute(service: &ListenService, now: Instant) {
    let lines = service.with_health(|health| health.take_minute_summaries(now));
    for (mic, state, s) in lines {
        info!(
            target: MIC_TARGET,
            mic = %mic,
            state = ?state,
            packets = s.packets,
            lost = s.lost,
            late = s.late,
            level_dbfs_max = s.level_dbfs_max,
            level_dbfs_avg = s.level_dbfs_avg,
            voiced_pct = s.voiced_pct,
            "mic minute summary"
        );
    }
}

/// The summary clock: one line set every [`SUMMARY_EVERY`], for as long as the
/// service lives (a tokio interval, so tests pause and advance time).
pub(super) async fn supervise(service: ListenService) {
    let mut tick = interval_at(Instant::now() + SUMMARY_EVERY, SUMMARY_EVERY);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        log_minute(&service, Instant::now());
    }
}
