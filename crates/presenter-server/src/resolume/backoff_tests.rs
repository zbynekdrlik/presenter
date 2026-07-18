//! #484 regression: a persistently-failing enabled Resolume host must NOT log
//! an ERROR on every failed attempt, and must BACK OFF its retries instead of
//! re-attempting on every push + every 10 s mapping tick.
//!
//! The incident: a down-but-enabled host (`resolume-pp`) logged one ERROR per
//! failure — 163,943 identical lines over ~3 days — and re-fetched the whole
//! `/composition` on every attempt, drowning the audit logs.
//!
//! Kept in its own file (self-contained helpers) so it is independent of the
//! larger `tests.rs` fixtures and does not trip the fn-length cap there.

use super::driver::{backoff_interval, should_log_error, HostDriver};
use super::{ResolumeConnectionSnapshot, ResolumeErrorKind, StageUpdate, CONNECT_TIMEOUT};
use chrono::Utc;
use presenter_core::{ResolumeHost, ResolumeHostId};
use reqwest::Client;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A minimal `tracing::Subscriber` that counts ERROR-level events. Installed as
/// the scoped thread-local default so a test can assert how many ERROR lines
/// `record_error` actually emitted.
struct ErrorCounter {
    errors: Arc<AtomicUsize>,
}

impl tracing::Subscriber for ErrorCounter {
    fn enabled(&self, _metadata: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _span: &tracing::span::Id, _values: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _span: &tracing::span::Id, _follows: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        if *event.metadata().level() == tracing::Level::ERROR {
            self.errors.fetch_add(1, Ordering::SeqCst);
        }
    }
    fn enter(&self, _span: &tracing::span::Id) {}
    fn exit(&self, _span: &tracing::span::Id) {}
}

/// An enabled host pointing at `host:port`, with a fresh (disabled) status.
fn driver_for(host: &str, port: u16) -> (HostDriver, Arc<RwLock<ResolumeConnectionSnapshot>>) {
    let now = Utc::now();
    let config = ResolumeHost::new(
        ResolumeHostId::new(),
        "Mock".into(),
        host.to_string(),
        port,
        true,
        now,
        now,
    );
    let client = Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .build()
        .expect("client");
    (
        HostDriver::new(client, config),
        Arc::new(RwLock::new(ResolumeConnectionSnapshot::disabled())),
    )
}

fn count_requests(requests: &[wiremock::Request], method_name: &str, path_name: &str) -> usize {
    requests
        .iter()
        .filter(|req| req.method.as_str() == method_name && req.url.path() == path_name)
        .count()
}

fn stage_main(text: &str) -> StageUpdate {
    StageUpdate {
        current_main: Some(text.to_string()),
        current_translation: None,
        song_name: None,
        band_name: None,
        enqueued_at: None,
        correlation_id: None,
    }
}

/// #484 (log dedup): driving N consecutive failures must NOT emit N ERROR log
/// lines. `record_error` must dedup to a bounded (O(log N)) number of lines —
/// log on the transition into `Error` and then only at widening milestones.
///
/// RED before the fix: `record_error` logged unconditionally → 64 ERROR lines.
/// GREEN after: logs only on the transition + power-of-two milestones → 7.
#[tokio::test]
async fn down_host_does_not_log_an_error_on_every_failure() {
    let errors = Arc::new(AtomicUsize::new(0));
    let _guard = tracing::subscriber::set_default(ErrorCounter {
        errors: errors.clone(),
    });

    let (mut driver, status) = driver_for("127.0.0.1", 65500);

    const FAILURES: usize = 64;
    for _ in 0..FAILURES {
        driver
            .record_error(anyhow::anyhow!("host down"), &status)
            .await;
    }

    let logged = errors.load(Ordering::SeqCst);
    assert!(
        logged < FAILURES,
        "ERROR log must be deduped, not one per failure (got {logged} for {FAILURES} failures)"
    );
    assert!(
        logged <= 10,
        "a down host's ERROR log must be bounded (O(log N)); got {logged}"
    );
}

/// #484 (backoff): a host in its post-error backoff window must STOP
/// re-attempting on every push. On current code each push re-fetched the whole
/// `/composition` (after `record_error` invalidated the cache), so a down host
/// re-attempted on every line.
///
/// RED before the fix: every push re-fetched → 3 GET /composition. GREEN after:
/// the first push fails and opens the backoff window, the next pushes are
/// skipped → exactly 1 GET /composition.
#[tokio::test]
async fn down_host_skips_push_attempts_while_in_backoff() {
    let server = MockServer::start().await;
    // The host is "down": /composition always errors.
    Mock::given(method("GET"))
        .and(path("/api/v1/composition"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let addr = server.address();
    let (mut driver, status) = driver_for(&addr.ip().to_string(), addr.port());

    // Three pushes back-to-back (well within the first backoff window),
    // mirroring the worker: on error, record_error.
    for line in ["a", "b", "c"] {
        if let Err(err) = driver.handle_stage(stage_main(line), &status).await {
            driver.record_error(err, &status).await;
        }
    }

    let requests = server.received_requests().await.expect("requests");
    assert_eq!(
        count_requests(&requests, "GET", "/api/v1/composition"),
        1,
        "a down host must attempt once then back off, not re-fetch on every push"
    );
}

/// #484 (backoff schedule): retry spacing must GROW with consecutive failures
/// and cap at ~1 attempt per minute, so a persistently-down host stops
/// hammering. Pure, so the schedule is pinned without sleeping.
#[test]
fn backoff_interval_grows_with_failures_and_caps_at_one_per_minute() {
    assert_eq!(
        backoff_interval(0),
        Duration::ZERO,
        "no backoff when healthy"
    );
    assert_eq!(backoff_interval(1), Duration::from_secs(1));
    assert_eq!(backoff_interval(2), Duration::from_secs(2));
    assert_eq!(backoff_interval(3), Duration::from_secs(4));
    assert_eq!(backoff_interval(4), Duration::from_secs(8));

    // Spacing strictly grows over the early failures (the regression: it used
    // to retry on every push + every 10 s tick, i.e. constant spacing).
    assert!(backoff_interval(1) < backoff_interval(2));
    assert!(backoff_interval(2) < backoff_interval(3));
    assert!(backoff_interval(3) < backoff_interval(4));

    // Caps at ~1/min for a persistently-down host.
    assert_eq!(backoff_interval(7), Duration::from_secs(60));
    assert_eq!(backoff_interval(1000), Duration::from_secs(60));
    assert!(backoff_interval(u32::MAX) <= Duration::from_secs(60));
}

/// #484 (log dedup schedule): the ERROR log fires on the transition into Error
/// and then only at power-of-two milestones, so a long down-streak is
/// logarithmic, not one line per attempt.
#[test]
fn should_log_error_logs_on_transition_then_at_widening_milestones() {
    // The transition (first failure) and power-of-two milestones log.
    for n in [1, 2, 4, 8, 16, 32, 64, 1024] {
        assert!(
            should_log_error(n),
            "failure #{n} is a milestone and must log"
        );
    }
    // Everything in between is suppressed.
    for n in [0, 3, 5, 6, 7, 9, 100, 1000] {
        assert!(!should_log_error(n), "failure #{n} must be suppressed");
    }
    // Over a long streak the logged count is logarithmic in the failure count.
    let logged = (1..=1024).filter(|n| should_log_error(*n)).count();
    assert_eq!(
        logged, 11,
        "1..=1024 → milestones 1,2,4,…,1024 = 11 ERROR lines, not 1024"
    );
}

/// #484 (recovery): an error opens the backoff window; it clears once the
/// interval elapses (so the host can retry) and a successful op clears it
/// outright (so the host is never permanently stuck). Uses paused tokio time —
/// no wall-clock sleeping.
#[tokio::test(start_paused = true)]
async fn backoff_window_opens_on_error_and_clears_after_the_interval() {
    let (mut driver, status) = driver_for("127.0.0.1", 65501);
    assert!(!driver.in_backoff(), "no backoff before any error");

    // First failure → ~1 s window.
    driver.record_error(anyhow::anyhow!("down"), &status).await;
    assert!(driver.in_backoff(), "in backoff immediately after an error");

    tokio::time::advance(Duration::from_millis(500)).await;
    assert!(
        driver.in_backoff(),
        "still backing off before the interval elapses"
    );

    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(
        !driver.in_backoff(),
        "backoff window clears after the interval, allowing a retry"
    );

    // A successful op clears the window outright.
    driver
        .record_error(anyhow::anyhow!("down again"), &status)
        .await;
    assert!(driver.in_backoff());
    driver.mark_connected(&status).await;
    assert!(!driver.in_backoff(), "recovery clears the backoff window");
}

/// #563b: a single failure (or two) must NOT invalidate the cached
/// composition mapping — only the 3rd CONSECUTIVE failure crosses
/// `CACHE_INVALIDATION_THRESHOLD`. Pre-fix, `record_error` invalidated the
/// cache unconditionally, forcing a full (potentially multi-MB) refetch on
/// every transient blip — the #563 incident's self-aggravating flap loop.
#[tokio::test]
async fn record_error_keeps_the_cached_mapping_through_transient_failures_then_invalidates_at_the_threshold(
) {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/composition"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "name": "Composition", "layers": [], "columns": [],
        })))
        .mount(&server)
        .await;
    let addr = server.address();
    let (mut driver, status) = driver_for(&addr.ip().to_string(), addr.port());

    driver.refresh_mapping().await.expect("initial fetch");
    assert!(driver.mapping.is_some(), "precondition: mapping cached");

    driver
        .record_error(anyhow::anyhow!("blip 1"), &status)
        .await;
    assert!(
        driver.mapping.is_some(),
        "1st failure must NOT invalidate the cached mapping"
    );

    driver
        .record_error(anyhow::anyhow!("blip 2"), &status)
        .await;
    assert!(
        driver.mapping.is_some(),
        "2nd consecutive failure must still serve the stale-but-good mapping"
    );

    driver
        .record_error(anyhow::anyhow!("blip 3"), &status)
        .await;
    assert!(
        driver.mapping.is_none(),
        "the 3rd consecutive failure crosses the threshold and invalidates the cache"
    );
}

/// #563d: every host error surfaces `nextRetryInSecs` — both on the
/// `HostDriver` itself (the backoff-skip debug log reads it) and in the
/// shared status snapshot (the `/integrations/resolume/status` poll reads
/// it) — so ops/UI can see the driver is still retrying, not stuck. Uses
/// paused TOKIO time for the driver's own `Instant`-based clock; the
/// snapshot's `next_retry_at` is a real (chrono) wall-clock timestamp, so it
/// is only asserted immediately after the write, before any virtual-time
/// advance.
#[tokio::test(start_paused = true)]
async fn record_error_surfaces_next_retry_in_secs_on_the_driver_and_in_the_snapshot() {
    let (mut driver, status) = driver_for("127.0.0.1", 65502);

    driver.record_error(anyhow::anyhow!("down"), &status).await;
    assert_eq!(
        driver.next_retry_in_secs(),
        Some(1),
        "1st failure → ~1s backoff (see backoff_interval)"
    );

    let snap = status.read().await.clone();
    let secs = snap
        .next_retry_in_secs(Utc::now())
        .expect("a backoff window must be open in the snapshot too");
    assert!(
        (0..=1).contains(&secs),
        "expected ~1s remaining, got {secs}"
    );

    tokio::time::advance(Duration::from_secs(2)).await;
    assert_eq!(
        driver.next_retry_in_secs(),
        Some(0),
        "past the retry time, seconds-remaining clamps at 0, never negative"
    );

    driver.mark_connected(&status).await;
    assert_eq!(driver.next_retry_in_secs(), None, "recovery clears it");
    let snap_after = status.read().await.clone();
    assert_eq!(
        snap_after.next_retry_at, None,
        "recovery clears the snapshot too"
    );
}

/// #563c: a REAL connect-refused error (nothing listens on the port) must
/// classify as `ConnectRefused` and render as a single-line full chain in
/// `last_error` — not the opaque top-level "failed to fetch composition"
/// `.to_string()` gave, which dropped exactly the detail the incident
/// diagnosis needed.
#[tokio::test]
async fn record_error_populates_last_error_kind_and_a_single_line_full_chain() {
    let (mut driver, status) = driver_for("127.0.0.1", 65504);
    let err = driver
        .refresh_mapping()
        .await
        .expect_err("nothing listens on this port");
    driver.record_error(err, &status).await;

    let snap = status.read().await.clone();
    assert_eq!(
        snap.last_error_kind,
        Some(ResolumeErrorKind::ConnectRefused)
    );
    let rendered = snap.last_error.expect("last_error must be set");
    assert!(
        !rendered.contains('\n'),
        "the rendered chain must stay on one line: {rendered}"
    );
    assert!(
        rendered.to_ascii_lowercase().contains("refus"),
        "must include the underlying cause, not just the top-level context: {rendered}"
    );
}

/// #563h: the per-push "mapping missing #x clip" WARN must be rate-limited
/// per clip name — un-throttled it floods at one line per call (507/hour
/// observed in the field for a single #timer clip re-checked every second).
#[tokio::test(start_paused = true)]
async fn should_warn_missing_clip_rate_limits_repeats_of_the_same_clip() {
    let (mut driver, _status) = driver_for("127.0.0.1", 65503);

    assert!(
        driver.should_warn_missing_clip("timer"),
        "the first warning for a clip always fires"
    );
    assert!(
        !driver.should_warn_missing_clip("timer"),
        "an immediate repeat for the SAME clip must be suppressed"
    );
    assert!(
        driver.should_warn_missing_clip("song-name"),
        "a DIFFERENT clip is tracked independently of #timer's rate limit"
    );

    tokio::time::advance(Duration::from_secs(299)).await;
    assert!(
        !driver.should_warn_missing_clip("timer"),
        "still within MISSING_CLIP_WARN_INTERVAL"
    );

    tokio::time::advance(Duration::from_secs(2)).await;
    assert!(
        driver.should_warn_missing_clip("timer"),
        "past the interval, the warning fires again"
    );
}
