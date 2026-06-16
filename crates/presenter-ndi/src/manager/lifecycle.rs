//! `NdiManager` lifecycle + control surface: construction, source discovery,
//! starting and stopping pipelines, and active-map membership queries. Split
//! out of the manager god-file (#357).

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use tokio::sync::Mutex;

use crate::discovery;
use crate::ndi_sdk::NdiLib;
use crate::pipeline::NdiPipeline;

use super::{check_active_entry, ActiveSource, NdiManager, StateCheckOutcome};

impl NdiManager {
    pub fn try_new() -> Option<Self> {
        let sdk = Arc::new(NdiLib::load().ok()?);
        let (source_list, finder_shutdown) = discovery::spawn_persistent_finder(Arc::clone(&sdk));
        Some(Self {
            _sdk: sdk,
            source_list,
            _finder_shutdown: finder_shutdown,
            active: Mutex::new(HashMap::new()),
        })
    }

    pub fn is_available(&self) -> bool {
        true
    }

    pub fn discover_sources(&self, _timeout_ms: u32) -> Result<Vec<discovery::NdiSourceInfo>> {
        Ok(self.source_list.read())
    }

    /// Start a pipeline for the given source.
    ///
    /// `source_id` = UUID from the `video_sources` DB row (used as the WHEP URL key).
    /// `ndi_name` = NDI broadcaster name (e.g. "STREAM-SNV (stream)").
    ///
    /// Returns only AFTER the pipeline has transitioned to `Streaming` — i.e.
    /// the GStreamer bus has emitted `StateChanged → Playing` for the pipeline
    /// element. For the shared-encoder topology (#336), this means ndisrc is
    /// alive and ndisrcdemux has begun delivering frames; the encoder + tee
    /// will start producing H264 buffers shortly after. Downstream webrtcbin
    /// consumers attach lazily via `add_consumer`; they do not require encoder
    /// caps at attach time (SDP exchange happens independently).
    ///
    /// An 8-second timeout caps the wait — long enough for ndisrc to find the
    /// source on a healthy LAN, short enough that a missing/dead broadcaster
    /// reports back quickly to the operator.
    pub async fn start_pipeline(
        self: &std::sync::Arc<Self>,
        source_id: &str,
        ndi_name: &str,
    ) -> Result<()> {
        let mut active = self.active.lock().await;

        // Operator-reactivation path: if the existing entry is dead, snapshot
        // its supervisor handle BEFORE `check_active_entry` removes the entry,
        // so we can abort the prior supervisor below. Without this, a
        // cool-off-bound supervisor that's mid-5-min-sleep keeps running and
        // ends up double-watching the new pipeline alongside the fresh
        // supervisor we spawn below (deep-review 🔵 #3, 2026-05-24 PR #340).
        // Safe to `.take()` here because we hold the lock: state observed by
        // `check_active_entry` below cannot change between these two reads.
        let prior_supervisor: Option<tokio::task::JoinHandle<()>> = active
            .get_mut(source_id)
            .filter(|entry| {
                matches!(
                    entry.pipeline.state(),
                    crate::pipeline::PipelineState::Stopped
                        | crate::pipeline::PipelineState::Errored(_)
                )
            })
            .and_then(|entry| entry.supervisor.take());

        if let StateCheckOutcome::Idempotent = check_active_entry(&mut active, source_id).await {
            // Pipeline turned out healthy — the dead-state filter above didn't
            // match, so prior_supervisor is None. If somehow it leaked, drop
            // the handle (does NOT cancel the task in tokio; the supervisor
            // is still owned by its `ActiveSource.supervisor` slot if we
            // didn't `.take()`).
            debug_assert!(prior_supervisor.is_none());
            return Ok(());
        }
        // The entry was dead → check_active_entry removed it. Abort the prior
        // supervisor (if any) so it doesn't double-watch the new pipeline we
        // build below.
        if let Some(handle) = prior_supervisor {
            handle.abort();
        }

        let whep_url = format!("/ndi/whep/{}", source_id);
        let pipeline = NdiPipeline::build(ndi_name, whep_url)?;
        pipeline.start().await?;

        // Wait for the pipeline to reach Streaming state. The bus-watch task
        // (started by pipeline.start()) sets state to Streaming once the
        // GStreamer pipeline element posts StateChanged → Playing.
        //
        // The new shared-encoder topology (ndisrc → demux → videoconvert →
        // encoder → rtph264pay → tee) has no whepserversink, so polling
        // `sink_element.static_pad("video_0").current_caps()` is no longer
        // applicable. Watching for PipelineState::Streaming is the correct
        // signal: the bus-watch only promotes to Streaming after PLAYING,
        // which requires ndisrcdemux to have negotiated caps with its upstream
        // ndisrc — equivalent timing to the old caps-wait.
        //
        // 8-second budget: ndisrc takes ~2-5s on a healthy LAN to find a
        // broadcast + receive first frame. Beyond 8s the source likely doesn't
        // exist and we'd rather fail fast than hang the operator UI.
        let mut watcher = pipeline.state_watcher();
        let streaming_ready = tokio::time::timeout(std::time::Duration::from_secs(8), async {
            loop {
                let state = watcher.borrow_and_update().clone();
                match state {
                    crate::pipeline::PipelineState::Errored(ref e) => {
                        return Err(anyhow!("pipeline errored: {e}"));
                    }
                    crate::pipeline::PipelineState::Streaming => return Ok(()),
                    _ => {}
                }
                if watcher.changed().await.is_err() {
                    return Err(anyhow!("state watcher closed unexpectedly"));
                }
            }
        })
        .await;

        match streaming_ready {
            Ok(Ok(())) => {
                // pipeline.state_watcher() and self.spawn_supervisor must
                // run before pipeline is wrapped into Arc and moved into
                // ActiveSource on the active.insert line below.
                let watcher = pipeline.state_watcher();
                let supervisor =
                    self.spawn_supervisor(source_id.to_string(), ndi_name.to_string(), watcher);
                active.insert(
                    source_id.to_string(),
                    ActiveSource {
                        pipeline: std::sync::Arc::new(pipeline),
                        supervisor: Some(supervisor),
                    },
                );
                Ok(())
            }
            Ok(Err(e)) => {
                pipeline.stop().await;
                Err(e)
            }
            Err(_) => {
                pipeline.stop().await;
                Err(anyhow!(
                    "NDI source '{ndi_name}' did not reach Streaming within 8s; \
                     ndisrc could not connect or the broadcaster is silent"
                ))
            }
        }
    }

    /// Stop one pipeline.
    pub async fn stop_pipeline(&self, source_id: &str) {
        let mut active = self.active.lock().await;
        if let Some(mut src) = active.remove(source_id) {
            if let Some(handle) = src.supervisor.take() {
                handle.abort();
            }
            src.pipeline.stop().await;
        }
    }

    /// Stop every active pipeline EXCEPT the one for `keep_id`.
    ///
    /// #370: called from the activate-switch path. Switching the active video
    /// source (deactivate A → activate B) used to start B's pipeline while
    /// leaving A's pipeline + its `nvh264enc` encoder streaming forever — the
    /// DB flipped A's row to `is_active=false` but the manager was never told.
    /// Two source pipelines (= two NVENC encoders) then accumulated after every
    /// switch. Reaping the orphaned siblings here keeps exactly ONE source
    /// pipeline running per the single-active-source invariant.
    pub async fn stop_other_pipelines(&self, keep_id: &str) {
        let mut active = self.active.lock().await;
        super::retain_only_active(&mut active, keep_id).await;
    }

    /// Stop ALL pipelines.
    pub async fn stop_all(&self) {
        let mut active = self.active.lock().await;
        for (_, src) in active.drain() {
            if let Some(handle) = src.supervisor {
                handle.abort();
            }
            src.pipeline.stop().await;
        }
    }

    /// Is the given source's pipeline currently active?
    pub async fn is_active(&self, source_id: &str) -> bool {
        self.active.lock().await.contains_key(source_id)
    }
}
