//! The transcription meter (#852 C3-7, plan Task 7.4, Decision 11): per mic
//! per local day the audio sent and the connected time (what the cap counts),
//! fed by the session's running totals; the `listen.usage` row is written
//! every 60 s while it changed and at every session end (the supervisor), so
//! the cap survives an auto-deploy. Also per mic, in memory only: the day's
//! `connections` (connection numbers that delivered words or ended — a
//! connect that failed before either is not counted), `reconnects` (outages:
//! entries into `reconnecting`, not each retry) and the latency of the last
//! 50 voiced onsets (p50 / p90).

use std::collections::{BTreeMap, VecDeque};

use chrono::NaiveDate;
use presenter_core::listen::transcription::{MicUsage, UsageRow};

/// The onsets the latency figures are taken over.
pub(crate) const LATENCY_WINDOW: usize = 50;

/// One mic's day besides its usage.
#[derive(Debug, Default, Clone)]
struct MicCounters {
    connections: u32,
    reconnects: u32,
    latencies: VecDeque<u32>,
}

/// The latency figures of one mic.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Latency {
    pub(crate) last: Option<u32>,
    pub(crate) p50: Option<u32>,
    pub(crate) p90: Option<u32>,
}

#[derive(Debug, Clone)]
pub(crate) struct Meter {
    day: NaiveDate,
    usage: BTreeMap<String, MicUsage>,
    counters: BTreeMap<String, MicCounters>,
    /// The usage changed since the row was last written.
    dirty: bool,
}

impl Meter {
    /// Today's meter from the stored row (a row of another day starts at 0).
    pub(crate) fn new(stored: Option<UsageRow>, today: NaiveDate) -> Self {
        let usage = stored
            .filter(|row| row.day == today)
            .map(|row| row.mics)
            .unwrap_or_default();
        Self {
            day: today,
            usage,
            counters: BTreeMap::new(),
            dirty: false,
        }
    }

    /// A new local day starts every mic at zero; `true` when it did.
    pub(crate) fn roll(&mut self, today: NaiveDate) -> bool {
        if self.day == today {
            return false;
        }
        *self = Self::new(None, today);
        self.dirty = true;
        true
    }

    pub(crate) fn usage(&self, mic: &str) -> MicUsage {
        self.usage.get(mic).copied().unwrap_or_default()
    }

    /// Every mic's connected time today: what the ONE daily cap counts
    /// (#852 T1).
    pub(crate) fn connected_total(&self) -> u64 {
        self.usage
            .values()
            .fold(0, |total, usage| total.saturating_add(usage.connected_ms))
    }

    /// Add what a session metered since its last report.
    pub(crate) fn add_usage(&mut self, mic: &str, audio_ms: u64, connected_ms: u64) {
        if audio_ms == 0 && connected_ms == 0 {
            return;
        }
        let usage = self.usage.entry(mic.to_string()).or_default();
        usage.audio_ms = usage.audio_ms.saturating_add(audio_ms);
        usage.connected_ms = usage.connected_ms.saturating_add(connected_ms);
        self.dirty = true;
    }

    pub(crate) fn add_connections(&mut self, mic: &str, connections: u32) {
        let counters = self.counters.entry(mic.to_string()).or_default();
        counters.connections = counters.connections.saturating_add(connections);
    }

    pub(crate) fn add_reconnect(&mut self, mic: &str) {
        let counters = self.counters.entry(mic.to_string()).or_default();
        counters.reconnects = counters.reconnects.saturating_add(1);
    }

    pub(crate) fn add_onset(&mut self, mic: &str, latency_ms: u32) {
        let latencies = &mut self.counters.entry(mic.to_string()).or_default().latencies;
        if latencies.len() == LATENCY_WINDOW {
            let _oldest = latencies.pop_front();
        }
        latencies.push_back(latency_ms);
    }

    /// `(connections, reconnects)` of the day.
    pub(crate) fn counts(&self, mic: &str) -> (u32, u32) {
        self.counters
            .get(mic)
            .map_or((0, 0), |c| (c.connections, c.reconnects))
    }

    pub(crate) fn latency(&self, mic: &str) -> Latency {
        let Some(latencies) = self
            .counters
            .get(mic)
            .map(|c| &c.latencies)
            .filter(|l| !l.is_empty())
        else {
            return Latency::default();
        };
        let mut sorted: Vec<u32> = latencies.iter().copied().collect();
        sorted.sort_unstable();
        Latency {
            last: latencies.back().copied(),
            p50: Some(percentile(&sorted, 50)),
            p90: Some(percentile(&sorted, 90)),
        }
    }

    /// The row to write, when the usage changed since the last one.
    pub(crate) fn take_dirty_row(&mut self) -> Option<UsageRow> {
        if !self.dirty {
            return None;
        }
        self.dirty = false;
        Some(self.row())
    }

    /// A write failed: the row is written again at the next chance.
    pub(crate) fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub(crate) fn row(&self) -> UsageRow {
        UsageRow {
            day: self.day,
            mics: self.usage.clone(),
        }
    }
}

/// The nearest-rank percentile of a sorted, non-empty list.
fn percentile(sorted: &[u32], pct: usize) -> u32 {
    let rank = (pct * sorted.len()).div_ceil(100).max(1);
    sorted[rank.min(sorted.len()) - 1]
}
