//! Park the NDI receiver while no stage screen watches it, wake it for the
//! first one that opens it (#979, requested by songplayer#334).
//!
//! The manager owns the park itself (`presenter_ndi::manager::park`): a
//! streaming source whose pipeline has had no consumer for
//! [`presenter_ndi::manager::PARK_AFTER_IDLE`] loses its pipeline and stays active in
//! the DB. This file is the server half:
//!
//! - the 30 s reconnect ticker asks the manager to park the active source
//!   ([`AppState::park_if_no_viewer`]) and treats a parked one as healthy
//!   (`ReconnectAction::Heartbeat { parked: true }`, never a repair);
//! - a stage's WHEP POST for a parked source starts it again
//!   ([`AppState::wake_parked_ndi_source`]) through the manager half of the
//!   activation (`start_active_video_source_locked`: start, status, sibling
//!   reap — no DB write, no audit row) before the POST adds its consumer; a
//!   POST that raced a park wakes it and tries once more
//!   ([`AppState::post_whep_waking`]).
//!
//! Both run under `activation_lock` (the ticker already holds it; the wake
//! takes it), so neither can interleave with an operator activate, switch,
//! deactivate or delete (`ndi-manager-locking.md`, #745a/#747).

use presenter_ndi::manager::{NdiSessionError, WhepReply};

use super::integrations::NdiActivation;
use super::AppState;

impl AppState {
    /// Park the active source when no stage has watched it for
    /// [`presenter_ndi::manager::PARK_AFTER_IDLE`]. The caller holds `activation_lock`
    /// (the reconnect ticker); the manager logs the park.
    pub(super) async fn park_if_no_viewer(&self, source_id: &str) -> bool {
        match &self.ndi_manager {
            Some(manager) => {
                manager
                    .park_if_idle(source_id, presenter_ndi::manager::PARK_AFTER_IDLE)
                    .await
            }
            None => false,
        }
    }

    /// A stage's WHEP POST (new consumer) with the #979 wake around it: a parked
    /// source is woken first, and a POST that finds no pipeline because the
    /// source was parked in the meantime (between the wake check and the POST)
    /// wakes it and tries once more instead of answering "not producing".
    pub(crate) async fn post_whep_waking<F, Fut>(
        &self,
        source_id: &str,
        mut post: F,
    ) -> anyhow::Result<WhepReply>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = anyhow::Result<WhepReply>>,
    {
        self.wake_parked_ndi_source(source_id).await;
        let first = post().await;
        let not_active = matches!(
            first
                .as_ref()
                .map_err(|err| err.downcast_ref::<NdiSessionError>()),
            Err(Some(NdiSessionError::SourceNotActive))
        );
        let parked_meanwhile = not_active
            && self
                .ndi_manager
                .as_ref()
                .is_some_and(|manager| manager.is_parked(source_id));
        if !parked_meanwhile {
            return first;
        }
        tracing::info!(
            source_id,
            "WHEP POST: the source was parked meanwhile — waking it and trying once more (#979)"
        );
        self.wake_parked_ndi_source(source_id).await;
        post().await
    }

    /// A stage opened `source_id`: when it is parked, start its pipeline again
    /// before the WHEP POST adds the consumer. A source that is not parked
    /// returns at once (no lock taken). Waits for the start (≤ 8 s, the
    /// activation's streaming wait); a still-silent sender or a failed start
    /// is logged by the activation, and the POST then answers as for any
    /// source that is not producing (204, the stage retries).
    pub(crate) async fn wake_parked_ndi_source(&self, source_id: &str) {
        let Some(manager) = &self.ndi_manager else {
            return;
        };
        if !manager.is_parked(source_id) {
            return;
        }
        let _activation_guard = self.activation_lock.lock().await;
        // Re-read under the lock: a concurrent POST may have woken it already,
        // or the operator switched / deactivated meanwhile (that also ends the
        // park, so the check below is enough).
        let Some(parked_for) = manager
            .parked_sources()
            .into_iter()
            .find_map(|(id, parked_for)| (id == source_id).then_some(parked_for))
        else {
            return;
        };
        let source = match self.repository.get_active_video_source().await {
            Ok(Some(source)) if source.id.to_string() == source_id => source,
            Ok(_) => {
                tracing::warn!(
                    source_id,
                    "NDI source parked but no longer the active source — not waking it (#979)"
                );
                return;
            }
            Err(err) => {
                tracing::warn!(
                    source_id,
                    error = %err,
                    "NDI wake: reading the active video source failed (#979)"
                );
                return;
            }
        };
        tracing::info!(
            source_id,
            ndi_name = %source.ndi_name,
            parked_secs = parked_for.as_secs(),
            "NDI source woken: a stage opened it, starting the receiver (#979)"
        );
        match self.start_active_video_source_locked(source).await {
            Ok((_, NdiActivation::Streaming)) => {}
            Ok((_, NdiActivation::NotStreaming)) => {
                tracing::info!(
                    source_id,
                    "NDI wake: the sender is not producing yet — the stage keeps retrying (#979)"
                );
            }
            Err(err) => {
                tracing::warn!(source_id, error = %err, "NDI wake: starting the receiver failed (#979)");
            }
        }
    }
}
