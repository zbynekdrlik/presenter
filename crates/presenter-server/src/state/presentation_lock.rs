//! Per-presentation async lock registry (#558 V2).
//!
//! All snapshot-replace slide-edit ops (paste / reorder / insert / duplicate
//! / delete) share one shape: read a presentation snapshot, mutate it in
//! memory, then wholesale-replace the persisted slide list and refresh the
//! cache. A concurrent sync apply of the SAME presentation writes directly
//! to the database and is invisible to that in-memory snapshot — without a
//! shared lock, a sync apply landing between the snapshot read and the
//! edit's write is silently overwritten by the edit's now-stale write, and
//! the edit's own `touch` then LWW-propagates that loss back to the peer as
//! if the (destroyed) local state were the newer copy.
//!
//! This registry gives both sides (every snapshot-replace edit op in
//! `slides/edit_ops.rs`, and the sync-apply call site in `sync.rs`) ONE
//! mutex per presentation id to hold across their entire
//! read-snapshot + write + cache-refresh sequence, so the two can never
//! interleave — whichever side acquires the lock second always observes
//! the first side's fully-committed result.

use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};

use presenter_core::PresentationId;
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

/// Registry of per-presentation async mutexes, keyed by [`PresentationId`].
/// Entries are created lazily on first use and never evicted — an idle
/// `Arc<Mutex<()>>` is a few dozen bytes, and a church's library tops out at
/// a few thousand songs, so unbounded growth here is not a real concern
/// (no eviction machinery for a non-problem, per the project's MVP
/// philosophy).
#[derive(Clone, Default)]
pub(crate) struct PresentationLockRegistry {
    locks: Arc<StdMutex<HashMap<PresentationId, Arc<AsyncMutex<()>>>>>,
}

impl PresentationLockRegistry {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Acquire (creating if needed) the async lock for `id`. Hold the
    /// returned guard for the ENTIRE read-snapshot + write + cache-refresh
    /// sequence — that is the whole point: a sync apply and a
    /// snapshot-replace edit op on the SAME presentation must never
    /// interleave.
    pub(crate) async fn lock(&self, id: PresentationId) -> OwnedMutexGuard<()> {
        let mutex = {
            let mut map = match self.locks.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            map.entry(id)
                .or_insert_with(|| Arc::new(AsyncMutex::new(())))
                .clone()
        };
        mutex.lock_owned().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn locks_on_different_presentations_never_block_each_other() {
        let registry = PresentationLockRegistry::new();
        let a = PresentationId::new();
        let b = PresentationId::new();

        let guard_a = registry.lock(a).await;
        let guard_b = tokio::time::timeout(Duration::from_millis(200), registry.lock(b))
            .await
            .expect("locking a DIFFERENT presentation must never block on `a`'s guard");
        drop(guard_a);
        drop(guard_b);
    }

    #[tokio::test]
    async fn the_same_presentation_serializes_concurrent_lockers() {
        let registry = PresentationLockRegistry::new();
        let id = PresentationId::new();

        let guard = registry.lock(id).await;
        let fut = registry.lock(id);
        tokio::pin!(fut);
        let timed_out = tokio::time::timeout(Duration::from_millis(100), &mut fut).await;
        assert!(
            timed_out.is_err(),
            "a second locker on the SAME presentation id must block while the first holds it"
        );

        drop(guard);
        tokio::time::timeout(Duration::from_millis(500), fut)
            .await
            .expect("the second locker must proceed once the first guard is released");
    }
}
