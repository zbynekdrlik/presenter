//! Page-level keep-alive for an Android stage display (#990).
//!
//! The #419 keep-alive treats "our stage app is foreground" as proof of life.
//! It is not: SNV SD2 (2026-11-21) had `StageActivity` in front for 11 hours
//! while its page sat dark — the wasm download was reset at boot and nothing
//! retried. A page that works opens a `/live/ws` stage connection, and the
//! #793 registry (`StageConnections`) records the real peer IP of each one.
//! So: no live stage connection from the display's address for
//! [`PAGE_SILENCE_LIMIT`] → the tick re-fires the VIEW intent (a fresh page via
//! singleTask `onNewIntent` → `loadUrl`). Relaunches of one display are
//! [`PAGE_RELAUNCH_MIN_GAP`] apart, doubling while they stay fruitless up to
//! [`PAGE_RELAUNCH_MAX_GAP`] (the #484 back-off), so neither a server-side
//! outage (every display silent at once) nor a display whose page connects
//! from an address its `host` does not resolve to becomes a relaunch or log
//! storm.
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

/// Gap after the first page relaunch of a streak; doubles per fruitless one.
pub(super) const PAGE_RELAUNCH_MIN_GAP: Duration = Duration::from_secs(5 * 60);

/// The longest gap between two page relaunches of one display.
pub(super) const PAGE_RELAUNCH_MAX_GAP: Duration = Duration::from_secs(60 * 60);

/// How long one host lookup may hold the device worker.
const HOST_LOOKUP_TIMEOUT: Duration = Duration::from_secs(2);

/// The wait after the `relaunches`-th fruitless relaunch (1-based):
/// 5, 10, 20, 40, then 60 min.
pub(super) fn relaunch_gap(relaunches: u32) -> Duration {
    let shift = relaunches.saturating_sub(1).min(16);
    PAGE_RELAUNCH_MIN_GAP
        .saturating_mul(1u32 << shift)
        .min(PAGE_RELAUNCH_MAX_GAP)
}

/// Per-display page liveness memory, owned by `run_device_worker`.
#[derive(Debug)]
pub(super) struct PageLiveness {
    /// The last tick that saw a live stage connection, or the last launch
    /// (a fresh page gets the full limit to connect), or the worker's start.
    last_live: Instant,
    /// The last page relaunch attempt (the rate limit counts attempts).
    last_relaunch: Option<Instant>,
    /// Page relaunches since the page was last seen live.
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
    /// the display's host did not resolve) — an unknown reading changes
    /// nothing. Returns how many relaunches it took when a live reading ends a
    /// streak of them (for the one recovery line).
    pub(super) fn observe(&mut self, live: Option<bool>, now: Instant) -> Option<u32> {
        if live != Some(true) {
            return None;
        }
        self.last_live = now;
        let ended = std::mem::take(&mut self.fruitless_relaunches);
        (ended > 0).then_some(ended)
    }

    /// Whether the page has been silent for [`PAGE_SILENCE_LIMIT`] and the
    /// last relaunch is at least [`relaunch_gap`] ago.
    pub(super) fn relaunch_due(&self, now: Instant) -> bool {
        let silent = now.saturating_duration_since(self.last_live) >= PAGE_SILENCE_LIMIT;
        let gap_ok = self.last_relaunch.is_none_or(|at| {
            now.saturating_duration_since(at) >= relaunch_gap(self.fruitless_relaunches)
        });
        silent && gap_ok
    }

    /// A launch (any path) just fired: the fresh page gets the full silence
    /// limit to connect.
    pub(super) fn note_launch(&mut self, now: Instant) {
        self.last_live = now;
    }

    /// A page relaunch FIRED (the VIEW intent reached the TV). Only fired ones
    /// count: a TV that is off fails the cycle before any intent, and a
    /// forced cycle costs nothing more than the plain tick that runs anyway.
    /// Returns the streak.
    pub(super) fn note_relaunch(&mut self, now: Instant) -> u32 {
        self.last_relaunch = Some(now);
        self.fruitless_relaunches = self.fruitless_relaunches.saturating_add(1);
        self.fruitless_relaunches
    }

    /// The #419 gate itself launched the stage (the app was not in front — a
    /// TV that just booted, someone left the stage): a fresh start, so the
    /// back-off of an earlier streak no longer applies. A page that then
    /// fails is relaunched after [`PAGE_SILENCE_LIMIT`], not after an hour.
    pub(super) fn note_device_launch(&mut self, now: Instant) {
        self.last_relaunch = None;
        self.fruitless_relaunches = 0;
        self.note_launch(now);
    }

    fn silent_for(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.last_live)
    }
}

/// The display's addresses: its configured `host` (an IP literal, or a name
/// like `sd2l.lan`) resolved by the system resolver — the same one `adb
/// connect <host>:<port>` uses. `None` when it does not resolve in time.
async fn display_addresses(host: &str, port: u16) -> Option<Vec<IpAddr>> {
    match tokio::time::timeout(HOST_LOOKUP_TIMEOUT, tokio::net::lookup_host((host, port))).await {
        Ok(Ok(addrs)) => {
            let ips: Vec<IpAddr> = addrs.map(|addr| addr.ip()).collect();
            (!ips.is_empty()).then_some(ips)
        }
        Ok(Err(err)) => {
            debug!(
                host,
                ?err,
                "android stage keep-alive: display host does not resolve (#990)"
            );
            None
        }
        Err(_) => {
            debug!(
                host,
                "android stage keep-alive: display host lookup timed out (#990)"
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
    if let Some(relaunches) = page.observe(live, now) {
        info!(
            display = %config.label,
            relaunches,
            "android stage keep-alive: the stage page is connected again (#990)"
        );
    }
    let relaunch_page = live == Some(false) && page.relaunch_due(now);
    let silent_for = page.silent_for(now);
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
        if relaunch_page {
            page.note_launch(now);
            let relaunches = page.note_relaunch(now);
            log_page_relaunch(config, silent_for, relaunches);
        } else {
            page.note_device_launch(now);
        }
    }
    result
}

/// An explicit launch (startup, config change, the launch-now endpoint):
/// always fires, and a fired launch gives the fresh page the full
/// [`PAGE_SILENCE_LIMIT`] to connect.
#[allow(clippy::too_many_arguments)]
pub(super) async fn forced_launch(
    runner: &dyn AdbRunner,
    stage_url: &Arc<Option<String>>,
    apk_path: &Arc<Option<PathBuf>>,
    config: &AndroidStageDisplay,
    status: &Arc<RwLock<AndroidStageDisplayStatusSnapshot>>,
    link: &mut AdbLinkState,
    page: &mut PageLiveness,
    now: Instant,
) -> anyhow::Result<bool> {
    let result =
        super::connect_and_launch(runner, stage_url, apk_path, config, status, true, link).await;
    if matches!(result, Ok(true)) {
        page.note_device_launch(now);
    }
    result
}

/// INFO on the 1st, 2nd, 4th, 8th … relaunch of a streak, DEBUG otherwise
/// (`should_log_adb_streak`, the #484 power-of-two gate).
fn log_page_relaunch(config: &AndroidStageDisplay, silent_for: Duration, relaunches: u32) {
    let next_in_min = relaunch_gap(relaunches).as_secs() / 60;
    if super::should_log_adb_streak(relaunches) {
        info!(
            display = %config.label,
            host = %config.host,
            silent_s = silent_for.as_secs(),
            relaunches,
            next_in_min,
            "android stage keep-alive: no live stage connection from the display — re-fired the stage page (#990)"
        );
    } else {
        debug!(
            display = %config.label,
            host = %config.host,
            silent_s = silent_for.as_secs(),
            relaunches,
            next_in_min,
            "android stage keep-alive: no live stage connection from the display — re-fired the stage page (#990)"
        );
    }
}
