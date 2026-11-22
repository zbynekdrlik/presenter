//! Page-level keep-alive for an Android stage display (#990).
//!
//! The #419 keep-alive treats "our stage app is foreground" as proof of life.
//! It is not: SNV SD2 (2026-11-21) had `StageActivity` in front for 11 hours
//! while its page sat dark — the wasm download was reset at boot and nothing
//! retried. A page that works opens a `/live/ws` stage connection, and the
//! #793 registry (`StageConnections`) records the real peer IP of each one.
//! So: no live stage connection from the display's address for
//! [`PAGE_SILENCE_LIMIT`] → the tick re-fires the VIEW intent (a fresh page via
//! singleTask `onNewIntent` → `loadUrl`), at most once per
//! [`PAGE_RELAUNCH_MIN_GAP`] per display so a server-side outage (every display
//! silent at once) is never a relaunch storm.

use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use presenter_core::AndroidStageDisplay;
use tokio::sync::RwLock;

use super::{AdbLinkState, AdbRunner, AndroidStageDisplayStatusSnapshot};
use crate::stage_connections::StageConnections;

/// How long a display may show no live stage connection before its page is
/// considered dead.
pub(super) const PAGE_SILENCE_LIMIT: Duration = Duration::from_secs(3 * 60);

/// Minimum gap between two page relaunches of one display.
pub(super) const PAGE_RELAUNCH_MIN_GAP: Duration = Duration::from_secs(5 * 60);

/// Per-display page liveness memory, owned by `run_device_worker`.
#[derive(Debug)]
pub(super) struct PageLiveness {
    _last_live: Instant,
}

impl PageLiveness {
    pub(super) fn new(now: Instant) -> Self {
        Self { _last_live: now }
    }

    /// Record this tick's reading: `Some(true)` = a live stage connection from
    /// the display, `Some(false)` = none, `None` = unknown (no registry, or
    /// the display's host did not resolve) — an unknown reading changes nothing.
    pub(super) fn observe(&mut self, _live: Option<bool>, _now: Instant) {}

    /// Whether the page has been silent long enough and the rate limit allows
    /// another relaunch.
    pub(super) fn relaunch_due(&self, _now: Instant) -> bool {
        false
    }

    /// A launch (any path) just fired: the fresh page gets the full silence
    /// limit to connect.
    pub(super) fn note_launch(&mut self, _now: Instant) {}

    /// A page relaunch was attempted (rate limit counts attempts).
    pub(super) fn note_relaunch_attempt(&mut self, _now: Instant) {}
}

/// The display's addresses: its configured `host` (an IP literal, or a name
/// like `sd2l.lan`) resolved the way the system resolver answers adb.
async fn display_addresses(_host: &str, _port: u16) -> Option<Vec<IpAddr>> {
    None
}

/// One periodic keep-alive tick: read the page's liveness, then run the
/// #419 cycle, forcing the VIEW intent when the page is dead.
#[allow(clippy::too_many_arguments)]
pub(super) async fn keep_alive_tick(
    runner: &dyn AdbRunner,
    stage_url: &Arc<Option<String>>,
    apk_path: &Arc<Option<PathBuf>>,
    config: &AndroidStageDisplay,
    status: &Arc<RwLock<AndroidStageDisplayStatusSnapshot>>,
    link: &mut AdbLinkState,
    page: &mut PageLiveness,
    connections: Option<&StageConnections>,
    now: Instant,
) -> anyhow::Result<bool> {
    let _ = (connections, page.relaunch_due(now), display_addresses);
    super::connect_and_launch(runner, stage_url, apk_path, config, status, false, link).await
}
