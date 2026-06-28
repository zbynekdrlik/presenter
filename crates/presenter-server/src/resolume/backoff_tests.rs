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

use super::driver::HostDriver;
use super::{ResolumeConnectionSnapshot, StageUpdate, CONNECT_TIMEOUT};
use chrono::Utc;
use presenter_core::{ResolumeHost, ResolumeHostId};
use reqwest::Client;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
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
