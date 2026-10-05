//! #808: when the host worker talks to Arena, and when it pulls the whole
//! composition.
//!
//! `GET /api/v1/composition` is 16.3 MB on SNV's Arena. Every fetch held
//! win-resolume's CPU 0 in an NDIS DPC for 4–11 ms, which stalled the LED wall,
//! SongPlayer's VBAN/NDI output and cg OBS. The worker used to re-fetch it every
//! 10 s. Now:
//!
//! - the 10 s [`HostDriver::tick`] is a liveness probe, `GET /api/v1/product`
//!   (~64 B), whenever a mapping is cached;
//! - the composition is fetched only when the mapping is needed:
//!   - there is no mapping: cold start, config change, port drift, or the
//!     #563b threshold invalidated it during an outage (the recovery refetch);
//!   - a push got a 404 for a mapped id ([`StaleIdError`]): invalidate,
//!     refetch, retry that push once ([`HostDriver::dispatch_push`]);
//!   - the operator asked for it ([`HostDriver::manual_refresh`], the
//!     settings page's "Refresh mapping" button);
//!   - the mapping looks incomplete (Arena still loading), a push needs a lane
//!     it lacks, or the operator switched decks: `provisional_mapping.rs`.

use super::clip_map::{sorted_text_param_ids, ClipMapping};
use super::driver::{FetchReason, HostCommand, HostDriver};
use super::port_drift::is_resolume_product_body;
use super::{BibleUpdate, ResolumeConnectionSnapshot, ResolumeRegistry, StageUpdate, TimerFrame};
use anyhow::{anyhow, Context};
use presenter_core::ResolumeHostId;
use reqwest::StatusCode;
use serde::Serialize;
use std::{fmt, sync::Arc, time::Duration};
use tokio::{
    sync::{mpsc, oneshot, RwLock},
    time::Instant,
};
use tracing::{debug, warn};

type Status = Arc<RwLock<ResolumeConnectionSnapshot>>;

/// The probe body is tiny, but a false negative opens a backoff window in
/// which pushes are skipped, so it gets more headroom than `ACTION_TIMEOUT`.
const LIVENESS_TIMEOUT: Duration = Duration::from_secs(5);
/// After a FRESHLY fetched mapping still got a 404, stale-id refetches pause
/// this long. Those 404s then count as ordinary failures (#563b, #484).
const STALE_REFETCH_COOLDOWN: Duration = Duration::from_secs(60);
/// How long the API waits for a host worker to run a requested refresh: the
/// pushes queued ahead of it plus one `COMPOSITION_TIMEOUT` fetch.
const MANUAL_REFRESH_REPLY_TIMEOUT: Duration = Duration::from_secs(60);

/// One push queued for a host worker.
#[derive(Debug, Clone)]
pub(super) enum Push {
    Stage(StageUpdate),
    Bible(BibleUpdate),
    Timer(TimerFrame),
}

#[derive(Debug, Clone, Copy)]
enum StaleTarget {
    TextParameter,
    Clip,
}

/// Resolume answered 404 for a parameter or clip id taken from the cached
/// mapping: the clip was deleted, or another composition was loaded. Typed so
/// [`HostDriver::dispatch_push`] can tell it from a transient failure.
#[derive(Debug)]
pub(super) struct StaleIdError {
    target: StaleTarget,
    id: i64,
}

impl StaleIdError {
    pub(super) fn text_parameter(id: i64) -> anyhow::Error {
        anyhow::Error::new(Self {
            target: StaleTarget::TextParameter,
            id,
        })
    }

    pub(super) fn clip(id: i64) -> anyhow::Error {
        anyhow::Error::new(Self {
            target: StaleTarget::Clip,
            id,
        })
    }
}

impl fmt::Display for StaleIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let what = match self.target {
            StaleTarget::TextParameter => "text parameter",
            StaleTarget::Clip => "clip",
        };
        write!(
            f,
            "Resolume has no {what} {} (404 Not Found); the composition mapping is stale",
            self.id
        )
    }
}

impl std::error::Error for StaleIdError {}

pub(super) fn is_stale_id_error(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| cause.is::<StaleIdError>())
}

/// The reply to an operator-requested mapping refresh, returned as JSON by
/// `POST /integrations/resolume/hosts/{id}/refresh-mapping`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MappingRefreshResult {
    pub(crate) success: bool,
    /// Expected clip names the refreshed composition lacks (e.g. `"#timer"`).
    pub(crate) missing_clips: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
}

impl MappingRefreshResult {
    fn refreshed(missing_clips: Vec<String>) -> Self {
        Self {
            success: true,
            missing_clips,
            error: None,
        }
    }

    fn failed(error: impl Into<String>) -> Self {
        Self {
            success: false,
            missing_clips: Vec::new(),
            error: Some(error.into()),
        }
    }
}

impl HostDriver {
    /// The 10 s worker tick: a `/product` liveness probe while a mapping is
    /// cached, then the deck check (a switched deck is refetched now); the
    /// composition only when there is no mapping.
    pub(super) async fn tick(&mut self, status: &Status) {
        if !self.config.is_enabled {
            return;
        }
        if self.in_backoff() {
            // #484/#563d: a down host is in its backoff window — skip this
            // tick instead of re-attempting (and re-logging), but say for how
            // much longer so ops reading logs mid-incident can see the driver
            // is still trying, not stuck.
            debug!(
                host = %self.config.host,
                next_retry_in_secs = self.next_retry_in_secs(),
                "resolume host in backoff; skipping liveness tick"
            );
            return;
        }
        let result = if self.mapping.is_some() {
            match self.probe_liveness().await {
                Ok(()) => self.follow_deck_switch().await,
                Err(err) => Err(err),
            }
        } else {
            // Cold start, config change, port drift, or the #563b threshold
            // invalidated the mapping during an outage. Arena may have
            // restarted or reloaded, so read the composition once.
            self.ensure_mapping().await.map(|_| ())
        };
        match result {
            Ok(()) => self.mark_connected(status).await,
            Err(err) => self.record_error(err, status).await,
        }
    }

    /// `GET /api/v1/product`: Resolume's `ProductInfo`
    /// (`{"name": "Arena" | "Avenue", "major": .., ..}`, REST swagger
    /// `get_product`). Proves the host is up and still Resolume without asking
    /// Arena to serialize its whole composition.
    async fn probe_liveness(&mut self) -> anyhow::Result<()> {
        let endpoint = self.endpoint().await?;
        let url = format!("{}/product", endpoint.base_url);
        let response = self
            .apply_host_header(self.client.get(&url), &endpoint)
            .timeout(LIVENESS_TIMEOUT)
            .send()
            .await
            .with_context(|| format!("liveness probe GET {url} failed"))?;
        let status = response.status();
        if status == StatusCode::NOT_FOUND {
            if self.product_verified {
                // This server identified as Resolume on /product before, so
                // the 404 means it changed (Arena gone, another service on
                // the port).
                return Err(anyhow!(
                    "liveness probe GET {url} answered 404 although this host identified as Resolume before"
                ));
            }
            // An Arena build older than the /product endpoint. This endpoint
            // already served a valid composition and the web server answered,
            // so it is up. A stale mapping still heals through a push's 404.
            debug!(
                host = %self.config.host,
                %url,
                "resolume has no /product endpoint (404); counting the HTTP answer as alive"
            );
            return Ok(());
        }
        if !status.is_success() {
            return Err(anyhow!(
                "liveness probe GET {url} failed with status {status}"
            ));
        }
        let body = response
            .json::<serde_json::Value>()
            .await
            .with_context(|| format!("invalid liveness probe JSON from {url}"))?;
        if !is_resolume_product_body(&body) {
            return Err(anyhow!(
                "liveness probe GET {url} did not identify as Resolume Arena/Avenue"
            ));
        }
        self.product_verified = true;
        debug!(host = %self.config.host, "resolume liveness probe ok");
        Ok(())
    }

    /// Apply one queued push. A 404 for a mapped id means the composition
    /// changed under us: invalidate the mapping and retry the push once — its
    /// `ensure_mapping` refetches (reason `stale-id`). Any other failure, or a
    /// failed retry, feeds `record_error` (#484 backoff, #563b threshold).
    pub(super) async fn dispatch_push(&mut self, push: Push, status: &Status) {
        // Taken before the pre-check, so a mapping it fetched counts as one
        // this push fetched itself (a 404 on it is not a stale id).
        let fetched_before = self.last_mapping_refresh;
        // #808 regression: the deck check and the one lane refetch per deck.
        self.prepare_mapping_for_push(&push).await;
        let mut result = self.apply_push(&push, status).await;
        let stale_detail = match &result {
            Err(err) if is_stale_id_error(err) => Some(format!("{err:#}")),
            _ => None,
        };
        if let Some(detail) = stale_detail {
            if self.last_mapping_refresh != fetched_before {
                // This push already ran on a mapping it fetched itself (cold
                // start, recovery refetch): refetching it again cannot help.
                self.pause_stale_refetch();
            } else if self.begin_stale_refetch(&detail) {
                result = self.apply_push(&push, status).await;
                if matches!(&result, Err(err) if is_stale_id_error(err)) {
                    self.pause_stale_refetch();
                }
            }
        }
        if let Err(err) = result {
            self.record_error(err, status).await;
        }
    }

    async fn apply_push(&mut self, push: &Push, status: &Status) -> anyhow::Result<()> {
        match push {
            Push::Stage(update) => self.handle_stage(update.clone(), status).await,
            Push::Bible(update) => self.handle_bible(update.clone(), status).await,
            Push::Timer(frame) => self.handle_timer(frame.clone(), status).await,
        }
    }

    /// Invalidate the mapping so the retry refetches it — unless a fresh
    /// mapping already got a 404 within [`STALE_REFETCH_COOLDOWN`]. Returns
    /// whether to retry.
    fn begin_stale_refetch(&mut self, detail: &str) -> bool {
        if self.stale_refetch_paused() {
            debug!(
                host = %self.config.host,
                error = %detail,
                "resolume stale id while stale-id refetches are paused; counting it as a failure"
            );
            return false;
        }
        warn!(
            host = %self.config.host,
            error = %detail,
            "resolume rejected a mapped id (404); refetching the composition and retrying the push once"
        );
        self.invalidate_mapping(FetchReason::StaleId);
        true
    }

    /// True while a fresh mapping's 404 paused stale-id refetches.
    pub(super) fn stale_refetch_paused(&self) -> bool {
        matches!(self.stale_refetch_paused_until, Some(until) if Instant::now() < until)
    }

    /// A FRESH mapping (fetched by this push or by its retry) still got a 404.
    /// Pause stale-id refetches, so a permanently bad id cannot turn every push
    /// into a full composition fetch.
    fn pause_stale_refetch(&mut self) {
        warn!(
            host = %self.config.host,
            pause_secs = STALE_REFETCH_COOLDOWN.as_secs(),
            "resolume still rejects an id on a freshly fetched mapping; pausing stale-id refetches"
        );
        self.stale_refetch_paused_until = Some(Instant::now() + STALE_REFETCH_COOLDOWN);
    }

    /// The operator's "Refresh mapping" (settings page / API). Fetches the
    /// composition now, inside a backoff window too, because the operator asked
    /// for it. It also re-arms stale-id refetches.
    pub(super) async fn manual_refresh(&mut self, status: &Status) -> MappingRefreshResult {
        if !self.config.is_enabled {
            return MappingRefreshResult::failed("the Resolume host is disabled");
        }
        self.stale_refetch_paused_until = None;
        match self.refresh_mapping().await {
            Ok(()) => {
                self.mark_connected(status).await;
                MappingRefreshResult::refreshed(self.missing_clips.clone())
            }
            Err(err) => {
                let detail = format!("{err:#}");
                self.record_error(err, status).await;
                MappingRefreshResult::failed(detail)
            }
        }
    }

    /// #267/#808: a payload deduped against an old param id must be re-sent
    /// when that id changed (another composition was loaded) or when there was
    /// no mapping to compare with. The stale-id, recovery and cold paths drop
    /// the mapping before they fetch, so they re-send the metadata once (same
    /// text, no visible change). A manual refresh keeps the mapping until the
    /// new one is parsed, so an unchanged id keeps its dedup there.
    pub(super) fn reset_dedup_for_changed_ids(&mut self, next: &ClipMapping) {
        let (timer, song, band) = match &self.mapping {
            Some(old) => (
                old.timer_param_ids() != next.timer_param_ids(),
                sorted_text_param_ids(&old.song_name) != sorted_text_param_ids(&next.song_name),
                sorted_text_param_ids(&old.band_name) != sorted_text_param_ids(&next.band_name),
            ),
            None => (true, true, true),
        };
        if timer {
            self.last_timer_payload = None;
        }
        if song {
            self.last_song_name_payload = None;
        }
        if band {
            self.last_band_name_payload = None;
        }
    }
}

impl ResolumeRegistry {
    /// Ask a host's worker to refetch its composition mapping now (#808).
    pub(crate) async fn refresh_mapping(&self, id: ResolumeHostId) -> MappingRefreshResult {
        let (reply_tx, reply_rx) = oneshot::channel();
        let queued = match self.hosts.read().await.get(&id) {
            Some(entry) => entry
                .command_tx
                .try_send(HostCommand::RefreshMapping(reply_tx)),
            None => return MappingRefreshResult::failed("no worker runs for this Resolume host"),
        };
        if let Err(err) = queued {
            let busy = matches!(err, mpsc::error::TrySendError::Full(_));
            warn!(host_id = %id, busy, "resolume mapping refresh not queued");
            return MappingRefreshResult::failed(if busy {
                "the Resolume host worker is busy; try again"
            } else {
                "the Resolume host worker stopped"
            });
        }
        match tokio::time::timeout(MANUAL_REFRESH_REPLY_TIMEOUT, reply_rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => MappingRefreshResult::failed("the Resolume host worker stopped"),
            Err(_) => {
                MappingRefreshResult::failed("timed out waiting for the Resolume host worker")
            }
        }
    }
}
