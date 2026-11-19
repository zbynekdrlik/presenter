//! Park an NDI source no stage screen watches (#979).
//!
//! A source active in the DB used to keep its NDI receiver at full bandwidth
//! all day, also with no stage viewer. On PP the sender (resolume-pp) carries
//! NDI and Dante on one network adapter, and the full-bandwidth NDI bursts
//! delayed its Dante packets (songplayer#334, dstack#425).
//!
//! Now a STREAMING source whose pipeline has had no consumer for
//! [`PARK_AFTER_IDLE`] is PARKED: its pipeline (and the NDI receiver in it)
//! stops, while the source stays active in the DB. The server's 30 s reconnect
//! ticker drives [`NdiManager::park_if_idle`] and treats a parked source as
//! healthy; a stage WHEP POST for a parked source starts it again through the
//! normal activation path.
//!
//! The book is a std mutex held only for a map read or write, never across an
//! `.await` (the `active` map's rule, `ndi-manager-locking.md`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use super::NdiManager;
use crate::pipeline::PipelineState;

/// How long a streaming source may go without a stage viewer before it is
/// parked. Long enough that a stage TV reconnect (#793) never churns the
/// pipeline; the ticker's 30 s period makes the real delay 60–90 s.
pub const PARK_AFTER_IDLE: Duration = Duration::from_secs(60);

/// One observation of a streaming source's consumers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::manager) enum IdleStep {
    /// Someone watches (or is joining): no idle time is counted.
    Watched,
    /// Nobody watches, not yet for `after`; idle since `since`.
    Idle { since: Instant },
    /// Nobody has watched for `idle_for` (≥ `after`): park the source.
    Park { idle_for: Duration },
}

/// The pure park decision: `consumers` now, the remembered start of the idle
/// time (`None` = not idle at the previous look), the clock and the limit.
pub(in crate::manager) fn idle_step(
    consumers: usize,
    idle_since: Option<Instant>,
    now: Instant,
    after: Duration,
) -> IdleStep {
    if consumers > 0 {
        return IdleStep::Watched;
    }
    let since = idle_since.unwrap_or(now);
    let idle_for = now.saturating_duration_since(since);
    if idle_for >= after {
        IdleStep::Park { idle_for }
    } else {
        IdleStep::Idle { since }
    }
}

#[derive(Default)]
struct ParkState {
    /// Streaming sources seen with no consumer, and since when.
    idle_since: HashMap<String, Instant>,
    /// Parked sources, and since when.
    parked: HashMap<String, Instant>,
}

/// Idle times and parked sources, per source id.
#[derive(Default)]
pub(in crate::manager) struct ParkBook {
    state: Mutex<ParkState>,
}

impl ParkBook {
    fn lock(&self) -> MutexGuard<'_, ParkState> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Record one look at `source_id`'s consumers and return the decision.
    pub(in crate::manager) fn observe(
        &self,
        source_id: &str,
        consumers: usize,
        now: Instant,
        after: Duration,
    ) -> IdleStep {
        let mut state = self.lock();
        let step = idle_step(
            consumers,
            state.idle_since.get(source_id).copied(),
            now,
            after,
        );
        match step {
            IdleStep::Watched | IdleStep::Park { .. } => {
                state.idle_since.remove(source_id);
            }
            IdleStep::Idle { since } => {
                state.idle_since.insert(source_id.to_string(), since);
            }
        }
        step
    }

    pub(in crate::manager) fn mark_parked(&self, source_id: &str, now: Instant) {
        let mut state = self.lock();
        state.idle_since.remove(source_id);
        state.parked.insert(source_id.to_string(), now);
    }

    pub(in crate::manager) fn parked_since(&self, source_id: &str) -> Option<Instant> {
        self.lock().parked.get(source_id).copied()
    }

    /// The source left the park or the manager (started, stopped): forget it.
    pub(in crate::manager) fn forget(&self, source_id: &str) {
        let mut state = self.lock();
        state.idle_since.remove(source_id);
        state.parked.remove(source_id);
    }

    /// Forget only the idle time (the source is not streaming right now).
    pub(in crate::manager) fn forget_idle(&self, source_id: &str) {
        self.lock().idle_since.remove(source_id);
    }

    /// `stop_all`.
    pub(in crate::manager) fn clear(&self) {
        let mut state = self.lock();
        state.idle_since.clear();
        state.parked.clear();
    }

    /// `stop_other_pipelines(keep_id)`: every other source left the manager.
    pub(in crate::manager) fn retain_only(&self, keep_id: &str) {
        let mut state = self.lock();
        state.idle_since.retain(|id, _| id == keep_id);
        state.parked.retain(|id, _| id == keep_id);
    }

    pub(in crate::manager) fn parked(&self) -> Vec<(String, Instant)> {
        let mut parked: Vec<_> = self
            .lock()
            .parked
            .iter()
            .map(|(id, since)| (id.clone(), *since))
            .collect();
        parked.sort();
        parked
    }
}

impl NdiManager {
    /// Is `source_id` parked (active in the DB, no pipeline, #979)?
    pub fn is_parked(&self, source_id: &str) -> bool {
        self.park_book.parked_since(source_id).is_some()
    }

    /// How long `source_id` has been parked; `None` when it is not.
    pub fn parked_for(&self, source_id: &str) -> Option<Duration> {
        self.park_book
            .parked_since(source_id)
            .map(|since| since.elapsed())
    }

    /// Every parked source with how long it has been parked (for `/healthz`).
    pub fn parked_sources(&self) -> Vec<(String, Duration)> {
        self.park_book
            .parked()
            .into_iter()
            .map(|(id, since)| (id, since.elapsed()))
            .collect()
    }

    /// Look at `source_id`'s consumers and park it once it has had none for
    /// `after`. Returns `true` when this call parked it.
    ///
    /// Only a STREAMING pipeline is counted (a starting, errored or missing one
    /// belongs to the activation / repair paths). The consumers are counted with
    /// the `active` lock released; the slot is then removed only if it still
    /// holds the same pipeline (`Arc::ptr_eq`), and the pipeline is stopped
    /// outside the lock. A viewer whose POST lands in the gap between the count
    /// and the removal loses its new session; its stage retries and the retry
    /// wakes the parked source.
    pub async fn park_if_idle(&self, source_id: &str, after: Duration) -> bool {
        let pipeline = {
            let active = self.active.lock().await;
            match active.get(source_id) {
                Some(src) if matches!(src.pipeline.state(), PipelineState::Streaming) => {
                    Arc::clone(&src.pipeline)
                }
                _ => {
                    self.park_book.forget_idle(source_id);
                    return false;
                }
            }
        };
        let consumers = pipeline.consumer_count().await;
        let now = Instant::now();
        let IdleStep::Park { idle_for } = self.park_book.observe(source_id, consumers, now, after)
        else {
            return false;
        };
        let removed = {
            let mut active = self.active.lock().await;
            match active.get(source_id) {
                Some(src) if Arc::ptr_eq(&src.pipeline, &pipeline) => active.remove(source_id),
                _ => None,
            }
        };
        let Some(mut src) = removed else {
            return false;
        };
        if let Some(handle) = src.supervisor.take() {
            handle.abort();
        }
        self.park_book.mark_parked(source_id, now);
        tracing::info!(
            source_id,
            idle_secs = idle_for.as_secs(),
            "NDI source parked: no stage viewer, receiver disconnected until a stage opens it (#979)"
        );
        src.pipeline.stop().await;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AFTER: Duration = Duration::from_secs(60);

    #[test]
    fn a_watched_source_is_never_idle() {
        let now = Instant::now();
        assert_eq!(idle_step(1, None, now, AFTER), IdleStep::Watched);
        assert_eq!(
            idle_step(2, Some(now - Duration::from_secs(600)), now, AFTER),
            IdleStep::Watched
        );
    }

    #[test]
    fn the_first_idle_look_starts_the_idle_time_now() {
        let now = Instant::now();
        assert_eq!(
            idle_step(0, None, now, AFTER),
            IdleStep::Idle { since: now }
        );
    }

    #[test]
    fn idle_shorter_than_the_limit_waits_and_at_the_limit_parks() {
        let start = Instant::now();
        let early = start + Duration::from_secs(59);
        assert_eq!(
            idle_step(0, Some(start), early, AFTER),
            IdleStep::Idle { since: start }
        );
        let due = start + AFTER;
        assert_eq!(
            idle_step(0, Some(start), due, AFTER),
            IdleStep::Park { idle_for: AFTER }
        );
    }

    #[test]
    fn a_viewer_in_between_restarts_the_idle_time() {
        let book = ParkBook::default();
        let t0 = Instant::now();
        assert_eq!(
            book.observe("a", 0, t0, AFTER),
            IdleStep::Idle { since: t0 }
        );
        let t1 = t0 + Duration::from_secs(30);
        assert_eq!(book.observe("a", 1, t1, AFTER), IdleStep::Watched);
        let t2 = t0 + Duration::from_secs(70);
        assert_eq!(
            book.observe("a", 0, t2, AFTER),
            IdleStep::Idle { since: t2 }
        );
        let t3 = t2 + AFTER;
        assert_eq!(
            book.observe("a", 0, t3, AFTER),
            IdleStep::Park { idle_for: AFTER }
        );
    }

    #[test]
    fn parked_sources_are_forgotten_by_every_stop() {
        let book = ParkBook::default();
        let now = Instant::now();
        book.mark_parked("a", now);
        book.mark_parked("b", now);
        book.mark_parked("c", now);
        book.retain_only("b");
        assert!(book.parked_since("a").is_none());
        assert_eq!(book.parked_since("b"), Some(now));
        book.forget("b");
        assert!(book.parked_since("b").is_none());
        book.mark_parked("d", now);
        book.clear();
        assert!(book.parked().is_empty());
    }

    #[test]
    fn parking_ends_the_idle_time() {
        let book = ParkBook::default();
        let t0 = Instant::now();
        book.observe("a", 0, t0, AFTER);
        book.mark_parked("a", t0 + AFTER);
        // Unparked later: the idle time starts over, it is not carried.
        book.forget("a");
        let t1 = t0 + Duration::from_secs(600);
        assert_eq!(
            book.observe("a", 0, t1, AFTER),
            IdleStep::Idle { since: t1 }
        );
    }
}
