//! Stale-while-revalidate cache for the `/healthz` AI verdict (#760 rework).
//!
//! `/healthz` is polled by EVERY open operator tab (the header version poll),
//! by the stage NDI reload guard, by `version_label` on mount, and by the
//! deploy gates. If each hit performed a live `/models` probe, the readiness
//! probe's latency and external-request rate would scale with the number of
//! open tabs and depend on the AI backend's response time — unacceptable for a
//! readiness endpoint. This cache serves a verdict with a short TTL and
//! refreshes in the BACKGROUND, so `/healthz` never blocks on the AI backend
//! and probes it at most once per TTL regardless of how many tabs poll.
//!
//! Scoped PER `AppState` (an `Arc<AiHealthCache>` field), never a module-level
//! `static` — the test suite builds many `AppState`s in one process and a
//! global would cross-contaminate tests.

use serde_json::{json, Value};
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long a cached AI verdict is served before a background refresh is
/// triggered. 30s is short enough that an owner-facing watchdog polling
/// `/healthz` learns of a dead backend within tens of seconds, and long enough
/// that tab fan-out cannot turn into a `/models` request storm.
pub(crate) const AI_HEALTH_TTL: Duration = Duration::from_secs(30);

struct Cached {
    verdict: Value,
    at: Instant,
}

/// Per-`AppState` SWR cache of the `/healthz` `ai` verdict.
pub(crate) struct AiHealthCache {
    ttl: Duration,
    cached: Mutex<Option<Cached>>,
    /// At most one refresh (background OR the cold inline probe) in flight.
    refreshing: AtomicBool,
}

impl AiHealthCache {
    pub(crate) fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            cached: Mutex::new(None),
            refreshing: AtomicBool::new(false),
        }
    }

    /// Recover the guard on a poisoned lock rather than panicking (this repo
    /// bans `unwrap()`/`expect()`/`panic!` in production code) — same
    /// fail-forward posture as `ai::refresh`'s poisoned-lock handling.
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Cached>> {
        self.cached.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Cold-start placeholder: a probe is in flight and no value exists yet.
/// `connected:false` is honest — we genuinely do not know the AI is up.
fn warming() -> Value {
    json!({ "connected": false, "error": "AI status not yet available", "model": "" })
}

/// The very first (cold) probe itself failed to produce a verdict.
fn cold_failure() -> Value {
    json!({ "connected": false, "error": "AI status check failed", "model": "" })
}

/// Stale-while-revalidate read of the AI verdict. `produce` computes a fresh
/// verdict, returning `None` on a computation failure that must NOT clobber a
/// previously-good value.
///
/// - **fresh** cache → returns it with ZERO awaits (no network on the hot path)
/// - **stale** cache → returns the stale value immediately and refreshes in the
///   background (at most one in-flight refresh, guarded by `refreshing`), so a
///   slow/failing backend never delays or hangs `/healthz`
/// - **empty** cache → one bounded inline probe, guarded so concurrent cold
///   hits don't each probe (the loser gets a `warming` placeholder)
pub(crate) async fn get_ai_health<P, Fut>(cache: &Arc<AiHealthCache>, produce: P) -> Value
where
    P: Fn() -> Fut + Send + Sync + Clone + 'static,
    Fut: Future<Output = Option<Value>> + Send + 'static,
{
    {
        let guard = cache.lock();
        if let Some(c) = guard.as_ref() {
            let fresh = c.at.elapsed() < cache.ttl;
            let value = c.verdict.clone();
            drop(guard);
            if fresh {
                return value;
            }
            // Stale: serve the stale value NOW, refresh in the background.
            spawn_refresh_if_idle(cache, produce);
            return value;
        }
    }
    // Cold cache: a single bounded inline probe. Guard it so concurrent cold
    // hits don't each spawn a probe — the loser returns a warming placeholder
    // (the next poll, once the winner has stored a value, gets the real one).
    if cache.refreshing.swap(true, Ordering::AcqRel) {
        return warming();
    }
    let verdict = match produce().await {
        Some(v) => {
            let mut guard = cache.lock();
            *guard = Some(Cached {
                verdict: v.clone(),
                at: Instant::now(),
            });
            v
        }
        None => cold_failure(),
    };
    cache.refreshing.store(false, Ordering::Release);
    verdict
}

/// Spawn a single background refresh if none is already in flight. On success
/// the new verdict replaces the cached one; on failure (`None`) the previous
/// value is kept — a transient failure must not clobber a known-good verdict.
fn spawn_refresh_if_idle<P, Fut>(cache: &Arc<AiHealthCache>, produce: P)
where
    P: Fn() -> Fut + Send + Sync + Clone + 'static,
    Fut: Future<Output = Option<Value>> + Send + 'static,
{
    if cache.refreshing.swap(true, Ordering::AcqRel) {
        return; // a refresh is already in flight — never spawn N probes
    }
    let cache = cache.clone();
    tokio::spawn(async move {
        let result = produce().await;
        if let Some(v) = result {
            let mut guard = cache.lock();
            *guard = Some(Cached {
                verdict: v,
                at: Instant::now(),
            });
        }
        cache.refreshing.store(false, Ordering::Release);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[tokio::test]
    async fn within_ttl_probes_once() {
        let cache = Arc::new(AiHealthCache::new(Duration::from_secs(60)));
        let calls = Arc::new(AtomicUsize::new(0));
        let producer = {
            let calls = calls.clone();
            move || {
                let calls = calls.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Some(json!({"connected": true, "error": null, "model": "m"}))
                }
            }
        };
        let v1 = get_ai_health(&cache, producer.clone()).await;
        let v2 = get_ai_health(&cache, producer.clone()).await;
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "a second /healthz call within the TTL must serve the cache, not re-probe"
        );
        assert_eq!(v1, v2);
    }

    #[tokio::test]
    async fn after_ttl_reprobes() {
        let cache = Arc::new(AiHealthCache::new(Duration::from_millis(20)));
        let calls = Arc::new(AtomicUsize::new(0));
        let producer = {
            let calls = calls.clone();
            move || {
                let calls = calls.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Some(json!({"connected": true, "error": null, "model": "m"}))
                }
            }
        };
        let _ = get_ai_health(&cache, producer.clone()).await; // cold probe
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        tokio::time::sleep(Duration::from_millis(40)).await; // exceed TTL
        let _ = get_ai_health(&cache, producer.clone()).await; // stale -> refresh
        tokio::time::sleep(Duration::from_millis(40)).await; // let the refresh run
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "a probe must happen after the TTL elapses"
        );
    }

    #[tokio::test]
    async fn refresh_failure_keeps_previous_value() {
        let cache = Arc::new(AiHealthCache::new(Duration::from_millis(20)));
        let seeded = get_ai_health(&cache, || async {
            Some(json!({"connected": true, "error": null, "model": "good"}))
        })
        .await;
        assert_eq!(seeded["model"], json!("good"));
        tokio::time::sleep(Duration::from_millis(40)).await; // stale
                                                             // A failing refresh (None): the stale serve must return the previous
                                                             // good value immediately (never hang), and the failure must not
                                                             // clobber it.
        let served = get_ai_health(&cache, || async { None::<Value> }).await;
        assert_eq!(
            served["model"],
            json!("good"),
            "a stale serve returns the previous value, never a hang"
        );
        tokio::time::sleep(Duration::from_millis(40)).await; // let the failing refresh finish
        let after = get_ai_health(&cache, || async { None::<Value> }).await;
        assert_eq!(
            after["model"],
            json!("good"),
            "a failed refresh must keep the previous value"
        );
    }
}
