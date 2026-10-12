//! #848 third review, finding 3: direct unit tests of what a mapped
//! `#bible-clear` cover's 404 does to the mapping (`cover_404_refetch`, every
//! branch) and of its #563h WARN limiter (`should_warn_cover_404`, key
//! `cover-404:<id>`), on a paused clock like `backoff_tests.rs`
//! `should_warn_missing_clip_rate_limits_repeats_of_the_same_clip`. The worker
//! tests cannot see which branch a 404 took, only the refetches it caused.

use super::bible_clear_cover::CoverRefetch;
use super::bible_clear_cover_tests::COVER_OPACITY;
use super::driver::{HostDriver, MISSING_CLIP_WARN_INTERVAL};
use chrono::Utc;
use presenter_core::{ResolumeHost, ResolumeHostId};
use reqwest::Client;
use std::time::Duration;
use tokio::time::Instant;

/// A second mapped cover's opacity parameter.
const SECOND: i64 = COVER_OPACITY + 1;

/// A driver that never sends a request in these tests.
fn idle_driver() -> HostDriver {
    let now = Utc::now();
    let config = ResolumeHost::new(
        ResolumeHostId::new(),
        "Mock".into(),
        "127.0.0.1".to_string(),
        65503,
        true,
        now,
        now,
    );
    HostDriver::new(Client::new(), config)
}

/// The cached mapping is now one fetched at this instant.
async fn fetch_mapping(driver: &mut HostDriver) -> Option<Instant> {
    tokio::time::advance(Duration::from_secs(1)).await;
    driver.last_mapping_refresh = Some(Instant::now());
    driver.last_mapping_refresh
}

#[tokio::test(start_paused = true)]
async fn a_mapped_cover_404_refetches_once_per_id_and_per_mapping() {
    let mut driver = idle_driver();
    assert_eq!(
        driver.cover_404_refetch(COVER_OPACITY, None),
        CoverRefetch::MappingChanged,
        "a fade scheduled without a mapping"
    );
    let first = fetch_mapping(&mut driver).await;
    assert_eq!(
        driver.cover_404_refetch(COVER_OPACITY, first),
        CoverRefetch::Refetch
    );

    driver.refetch_for_cover_404(COVER_OPACITY, first);
    assert!(
        driver.last_mapping_refresh.is_none(),
        "the mapping is invalidated"
    );
    assert_eq!(
        driver.cover_404_refetch(SECOND, first),
        CoverRefetch::MappingChanged,
        "a refetch drops the mapping, so another cover of that fade refetches nothing"
    );
    // The guard behind that: a mapping refetched for one cover (here cached
    // again at the same instant) is never refetched for another.
    driver.last_mapping_refresh = first;
    assert_eq!(
        driver.cover_404_refetch(SECOND, first),
        CoverRefetch::AlreadyForMapping
    );
    assert_eq!(
        driver.cover_404_refetch(COVER_OPACITY, first),
        CoverRefetch::AlreadyForId
    );

    let second = fetch_mapping(&mut driver).await;
    assert_eq!(
        driver.cover_404_refetch(COVER_OPACITY, second),
        CoverRefetch::AlreadyForId,
        "the refetched mapping lists the broken id again"
    );
    assert_eq!(
        driver.cover_404_refetch(SECOND, second),
        CoverRefetch::Refetch,
        "another broken id refetches once"
    );
    driver.refetch_for_cover_404(SECOND, second);

    let third = fetch_mapping(&mut driver).await;
    for param in [COVER_OPACITY, SECOND] {
        assert_eq!(
            driver.cover_404_refetch(param, third),
            CoverRefetch::AlreadyForId,
            "two broken ids never take turns refetching (param {param})"
        );
    }
    driver.stale_refetch_paused_until = Some(Instant::now() + Duration::from_secs(60));
    assert_eq!(
        driver.cover_404_refetch(SECOND + 1, third),
        CoverRefetch::Paused,
        "#808 paused stale-id refetches"
    );
}

/// #848 sixth review, item 1: a cover refetch is pending only while no
/// mapping is cached. After the refetch for `first`, a NEWER mapping cached
/// ends it; there the refetched mapping is still `first`, so only the
/// `last_mapping_refresh.is_none()` check says so (the worker test
/// `a_mapped_cover_404_after_a_newer_mapping_was_fetched_refetches_nothing`
/// refetched nothing for a cover, so it passes without that check). Never
/// pending for a fade scheduled without a mapping, nor with a mapping no
/// cover refetched (an older one while the refetch is pending, a newer one
/// after it).
#[tokio::test(start_paused = true)]
async fn a_cached_newer_mapping_ends_a_pending_cover_refetch() {
    let mut driver = idle_driver();
    assert!(
        !driver.cover_refetch_pending(None),
        "no mapping and no refetch yet"
    );
    let older = fetch_mapping(&mut driver).await;
    let first = fetch_mapping(&mut driver).await;
    assert!(
        !driver.cover_refetch_pending(first),
        "no cover refetched the cached mapping"
    );

    driver.refetch_for_cover_404(COVER_OPACITY, first);
    assert!(
        driver.cover_refetch_pending(first),
        "the mapping was dropped for a cover's 404 and none is cached"
    );
    assert!(
        !driver.cover_refetch_pending(None),
        "a fade scheduled without a mapping"
    );
    assert!(
        !driver.cover_refetch_pending(older),
        "a fade scheduled with an older mapping: no cover refetched that one"
    );

    let second = fetch_mapping(&mut driver).await;
    assert!(
        !driver.cover_refetch_pending(first),
        "a newer mapping is cached, so the refetch for the first is done"
    );
    assert!(
        !driver.cover_refetch_pending(second),
        "no cover refetched the newer mapping"
    );
}

#[tokio::test(start_paused = true)]
async fn a_mapped_cover_404_warns_once_per_interval_per_id() {
    let mut driver = idle_driver();

    assert!(
        driver.should_warn_cover_404(COVER_OPACITY),
        "the first 404 WARNs"
    );
    assert!(
        !driver.should_warn_cover_404(COVER_OPACITY),
        "a repeat right away is DEBUG"
    );
    assert!(
        driver.should_warn_cover_404(SECOND),
        "another cover has its own key"
    );

    tokio::time::advance(MISSING_CLIP_WARN_INTERVAL - Duration::from_secs(1)).await;
    assert!(
        !driver.should_warn_cover_404(COVER_OPACITY),
        "still inside the interval"
    );

    tokio::time::advance(Duration::from_secs(2)).await;
    assert!(
        driver.should_warn_cover_404(COVER_OPACITY),
        "past the interval it WARNs again"
    );
}
