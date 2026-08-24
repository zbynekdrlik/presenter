//! Long-running background task spawning for [`AppState`] — heartbeats,
//! periodic trash pruning, timer ticking, WAL checkpointing, and the NDI
//! auto-reconnect loop. Extracted from `state/mod.rs` (#486-style split) to
//! keep that file under the repo's per-file size cap; pure code motion, no
//! behavior change.

use chrono::Utc;
use presenter_persistence::PRUNE_HORIZON;
use tokio::time::{interval, Duration as TokioDuration, MissedTickBehavior};
use tracing::warn;
use uuid::Uuid;

use super::{should_auto_restore_ndi, AppState};
use crate::live::LiveEvent;

impl AppState {
    pub(super) fn spawn_heartbeat_tasks(&self) {
        let hub = self.live_hub.clone();
        let connections = self.stage_connections.clone();
        let config = self.heartbeat_config;
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(config.interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                let now = Utc::now();
                let heartbeat_id = Uuid::new_v4();
                connections.note_heartbeat_sent(heartbeat_id, now).await;
                hub.publish(LiveEvent::Heartbeat {
                    id: heartbeat_id,
                    timestamp: now,
                });
                let grace = config.grace_duration();
                let disconnect = config.disconnect_duration();
                let changed = connections.apply_timeouts(now, grace, disconnect).await;
                if !changed.is_empty() {
                    tracing::debug!(count = changed.len(), "stage connection statuses updated");
                    for snapshot in changed {
                        hub.publish(LiveEvent::StageConnection { snapshot });
                    }
                }
            }
        });
    }

    /// Periodically hard-delete trash older than PRUNE_HORIZON — songs
    /// (#555) then, on the SAME horizon, soft-deleted libraries (#578). Low
    /// frequency (every 6h); months of use must not grow the tables unbounded.
    /// #558 round-3 T8: PRUNE_HORIZON is the SAME constant `sync_apply.rs` uses
    /// to distinguish a fresh unknown tombstone from an already-pruned one —
    /// one shared source so the two can never drift apart. Extracted from
    /// `spawn_background_tasks` to keep that fn under the 120-line gate (#578).
    fn spawn_trash_prune_task(&self) {
        let prune_state = self.clone();
        tokio::spawn(async move {
            let mut ticker = interval(TokioDuration::from_secs(6 * 3600));
            ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                match prune_state
                    .repository
                    .prune_deleted_presentations(PRUNE_HORIZON)
                    .await
                {
                    // #558 round-4 U7: log the horizon's actual VALUE (days),
                    // not the Rust constant's name — a log reader can't look it up.
                    Ok(n) if n > 0 => tracing::info!(
                        pruned = n,
                        horizon_days = PRUNE_HORIZON.num_days(),
                        "pruned trashed songs older than the prune horizon"
                    ),
                    Ok(_) => {}
                    Err(err) => warn!(?err, "trash prune failed"),
                }
                // #578: soft-deleted LIBRARIES age out on the SAME horizon.
                // Runs AFTER the presentation prune so a still-live song in a
                // to-be-pruned library is already tombstoned; the library
                // hard-delete's FK cascade then clears whatever remains.
                match prune_state
                    .repository
                    .prune_deleted_libraries(PRUNE_HORIZON)
                    .await
                {
                    Ok(n) if n > 0 => tracing::info!(
                        pruned = n,
                        horizon_days = PRUNE_HORIZON.num_days(),
                        "pruned trashed libraries older than the prune horizon"
                    ),
                    Ok(_) => {}
                    Err(err) => warn!(?err, "library trash prune failed"),
                }
            }
        });
    }

    pub(super) fn spawn_background_tasks(&self) {
        let timers_state = self.clone();
        tokio::spawn(async move {
            if let Err(err) = timers_state.tick_timers().await {
                warn!(?err, "timer tick failed");
            }
            let mut ticker = interval(TokioDuration::from_secs(1));
            ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                if let Err(err) = timers_state.tick_timers().await {
                    warn!(?err, "timer tick failed");
                }
            }
        });

        let wal_state = self.clone();
        tokio::spawn(async move {
            let mut ticker = interval(TokioDuration::from_secs(300));
            ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                if let Err(err) = wal_state.repository.wal_checkpoint().await {
                    warn!(?err, "periodic WAL checkpoint failed");
                }
            }
        });

        self.spawn_trash_prune_task();

        // NDI auto-reconnect: if a source is marked active in DB but no stream
        // is running, retry activation every 30 seconds. Handles the case where
        // the NDI source comes online after server startup.
        //
        // #333 item 6 (deep-review): the per-tick encoder gate. The one-shot
        // startup auto-restore is gated by should_auto_restore_ndi() above,
        // but this 30s loop was NOT gated and would re-trigger the wedge
        // state every 30 seconds on an encoder-missing host. We probe
        // hw_h264_encoder() PER TICK (a cheap instantiate-and-discard
        // loadability probe — #443; GStreamer element construction only
        // allocates the GObject, hardware opens at the READY transition) so a
        // host whose plugin registry self-heals can resume reconnect without a
        // process restart.
        if self.ndi_manager().is_some() {
            let ndi_state = self.clone();
            tokio::spawn(async move {
                let mut ticker = interval(TokioDuration::from_secs(30));
                ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
                loop {
                    ticker.tick().await;
                    let manager_loaded = ndi_state.ndi_manager().is_some();
                    let encoder_available =
                        manager_loaded && presenter_ndi::hw_h264_encoder().is_some();
                    if should_auto_restore_ndi(manager_loaded, encoder_available) {
                        // #747: the read (active source) + activate is done atomically
                        // under `activation_lock` inside `reconnect_active_video_source`,
                        // so an operator deactivate cannot land in a read→activate gap
                        // and get its source revived by this ticker.
                        match ndi_state
                            .reconnect_active_video_source(
                                presenter_persistence::SettingsAuditSource::StartupDefault,
                                "system",
                            )
                            .await
                        {
                            Ok(Some(source)) => {
                                tracing::info!(
                                    ndi_name = %source.ndi_name,
                                    "NDI auto-reconnect: source restored"
                                );
                            }
                            Ok(None) => {}
                            Err(err) => {
                                tracing::debug!(
                                    ?err,
                                    "NDI auto-reconnect: source not yet available or DB query failed"
                                );
                            }
                        }
                    }
                }
            });
        }
    }
}
