//! The host worker's two entry points: the periodic tick and the push
//! dispatch. `run_host_worker` (driver.rs) only routes commands and ticks
//! here, so both are callable directly from tests.

use super::driver::HostDriver;
use super::{BibleUpdate, ResolumeConnectionSnapshot, StageUpdate, TimerFrame};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::debug;

/// One push queued for a host worker.
#[derive(Debug, Clone)]
pub(super) enum Push {
    Stage(StageUpdate),
    Bible(BibleUpdate),
    Timer(TimerFrame),
}

impl HostDriver {
    /// The periodic worker tick.
    pub(super) async fn tick(&mut self, status: &Arc<RwLock<ResolumeConnectionSnapshot>>) {
        if self.in_backoff() {
            // #484/#563d: a down host is in its backoff window — skip this
            // tick instead of re-attempting (and re-logging), but say for how
            // much longer so ops reading logs mid-incident can see the driver
            // is still trying, not stuck.
            debug!(
                host = %self.config.host,
                next_retry_in_secs = self.next_retry_in_secs(),
                "resolume host in backoff; skipping mapping refresh"
            );
        } else if let Err(err) = self.refresh_mapping().await {
            self.record_error(err, status).await;
        } else {
            self.mark_connected(status).await;
        }
    }

    /// Apply one queued push; a failure feeds `record_error` (#484 backoff).
    pub(super) async fn dispatch_push(
        &mut self,
        push: Push,
        status: &Arc<RwLock<ResolumeConnectionSnapshot>>,
    ) {
        let result = match push {
            Push::Stage(update) => self.handle_stage(update, status).await,
            Push::Bible(update) => self.handle_bible(update, status).await,
            Push::Timer(frame) => self.handle_timer(frame, status).await,
        };
        if let Err(err) = result {
            self.record_error(err, status).await;
        }
    }
}
