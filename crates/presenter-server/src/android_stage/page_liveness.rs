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
//!
//! The relaunch goes through the existing forced-launch path of
//! `connect_and_launch` (the #419 foreground gate is bypassed exactly like the
//! launch-now endpoint), so a display whose app is NOT foreground relaunches
//! as before either way.

use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use presenter_core::AndroidStageDisplay;
use tokio::sync::RwLock;
use tracing::{debug, info};

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
    /// The last tick that saw a live stage connection, or the last launch
    /// (a fresh page gets the full limit to connect), or the worker's start.
    last_live: Instant,
    /// The last page relaunch attempt (the rate limit counts attempts).
    last_relaunch: Option<Instant>,
    /// Page relaunches since the page was last seen live (log field).
    fruitless_relaunches: u32,
}

impl PageLiveness {
    pub(super) fn new(now: Instant) -> Self {
        Self {
            last_live: now,
            last_relaunch: None,
            fruitless_relaunches: 0,
        }
    }

    /// Record this tick's reading: `Some(true)` = a live stage connection from
    /// the display, `Some(false)` = none, `None` = unknown (no registry, or
    /// the display's host did not resolve) — an unknown reading changes nothing.
    pub(super) fn observe(&mut self, live: Option<bool>, now: Instant) {
        if live == Some(true) {
            self.last_live = now;
            self.fruitless_relaunches = 0;
        }
    }

    /// Whether the page has been silent for [`PAGE_SILENCE_LIMIT`] and the
    /// last relaunch is at least [`PAGE_RELAUNCH_MIN_GAP`] ago.
    pub(super) fn relaunch_due(&self, now: Instant) -> bool {
        let silent = now.saturating_duration_since(self.last_live) >= PAGE_SILENCE_LIMIT;
        let gap_ok = self
            .last_relaunch
            .is_none_or(|at| now.saturating_duration_since(at) >= PAGE_RELAUNCH_MIN_GAP);
        silent && gap_ok
    }

    /// A launch (any path) just fired: the fresh page gets the full silence
    /// limit to connect.
    pub(super) fn note_launch(&mut self, now: Instant) {
        self.last_live = now;
    }

    /// A page relaunch was attempted (the rate limit counts attempts, so an
    /// unreachable TV is not retried every tick either).
    pub(super) fn note_relaunch_attempt(&mut self, now: Instant) {
        self.last_relaunch = Some(now);
        self.fruitless_relaunches = self.fruitless_relaunches.saturating_add(1);
    }

    fn silent_for(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.last_live)
    }
}

/// The display's addresses: its configured `host` (an IP literal, or a name
/// like `sd2l.lan`) resolved by the system resolver — the same one `adb
/// connect <host>:<port>` uses. `None` when it does not resolve.
async fn display_addresses(host: &str, port: u16) -> Option<Vec<IpAddr>> {
    match tokio::net::lookup_host((host, port)).await {
        Ok(addrs) => {
            let ips: Vec<IpAddr> = addrs.map(|addr| addr.ip()).collect();
            (!ips.is_empty()).then_some(ips)
        }
        Err(err) => {
            debug!(
                host,
                ?err,
                "android stage keep-alive: display host does not resolve (#990)"
            );
            None
        }
    }
}

/// Whether the display's page has a live stage connection right now; `None`
/// when that cannot be told (no registry attached, the host does not resolve).
async fn page_is_live(
    connections: Option<&StageConnections>,
    config: &AndroidStageDisplay,
) -> Option<bool> {
    let connections = connections?;
    let addrs = display_addresses(&config.host, config.port).await?;
    Some(connections.has_live_connection_from(&addrs).await)
}

/// One periodic keep-alive tick: read the page's liveness, then run the
/// #419 cycle — forced (the VIEW intent re-fired) when the page is dead.
/// Returns whether a launch fired.
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
    let live = page_is_live(connections, config).await;
    page.observe(live, now);
    let relaunch_page = live == Some(false) && page.relaunch_due(now);
    let silent_for = page.silent_for(now);
    if relaunch_page {
        page.note_relaunch_attempt(now);
    }
    let result = super::connect_and_launch(
        runner,
        stage_url,
        apk_path,
        config,
        status,
        relaunch_page,
        link,
    )
    .await;
    if matches!(result, Ok(true)) {
        page.note_launch(now);
        if relaunch_page {
            info!(
                display = %config.label,
                host = %config.host,
                silent_s = silent_for.as_secs(),
                relaunches = page.fruitless_relaunches,
                "android stage keep-alive: no live stage connection from the display — re-fired the stage page (#990)"
            );
        }
    }
    result
}
