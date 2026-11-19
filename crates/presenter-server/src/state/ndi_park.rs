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
//!   ([`AppState::wake_parked_ndi_source`]) through the normal activation path
//!   (`activate_video_source_locked`: start, "connected", sibling reap) before
//!   the POST adds its consumer.
//!
//! Both run under `activation_lock` (the ticker already holds it; the wake
//! takes it), so neither can interleave with an operator activate, switch,
//! deactivate or delete (`ndi-manager-locking.md`, #745a/#747).

use presenter_persistence::SettingsAuditSource;

use super::integrations::NdiActivation;
use super::AppState;

impl AppState {
    /// Park the active source when no stage has watched it for
    /// [`presenter_ndi::manager::PARK_AFTER_IDLE`]. The caller holds `activation_lock`
    /// (the reconnect ticker); the manager logs the park.
    pub(super) async fn park_if_no_viewer(&self, source_id: &str) {
        if let Some(manager) = &self.ndi_manager {
            manager
                .park_if_idle(source_id, presenter_ndi::manager::PARK_AFTER_IDLE)
                .await;
        }
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
        match self
            .activate_video_source_locked(source.id, SettingsAuditSource::StartupDefault, "system")
            .await
        {
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
