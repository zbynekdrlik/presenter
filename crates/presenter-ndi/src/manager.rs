//! NdiManager — owns discovery + per-source GStreamer pipelines.
//!
//! Pre-WebRTC the module hosted a custom JPEG receiver/encoder. After the
//! WebRTC migration it manages one `NdiPipeline` per active NDI source and
//! exposes a WHEP signaller bridge for the HTTP shim.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use gstreamer::glib;
use gstreamer::prelude::*;
use tokio::sync::Mutex;

use crate::discovery::{self, FinderShutdown, SourceList};
use crate::ndi_sdk::NdiLib;
use crate::pipeline::{NdiPipeline, PipelineState};

/// Status callback retained for backwards compatibility with the old MJPEG
/// status-reporting path. The WebRTC manager currently invokes it on
/// pipeline state transitions so the live-event hub keeps emitting
/// `NdiConnectionStatus` events.
pub type StatusCallback = Arc<dyn Fn(String) + Send + Sync>;

/// Sentinel error message returned by `whep_signaller_call` when the requested
/// source has no active pipeline. The WHEP HTTP shim string-matches on this
/// to translate the error into a 404. Exposed as a `pub const` so the shim
/// imports the same literal — preventing silent 503-instead-of-404 drift if
/// the message is ever rewritten.
pub const SOURCE_NOT_ACTIVE_ERR: &str = "source not active";

/// One operation in the WHEP signaller protocol.
pub enum WhepOp {
    /// SDP offer (or session-scoped re-offer).
    Post { id: Option<String>, body: Vec<u8> },
    /// ICE trickle update.
    Patch {
        id: String,
        body: Vec<u8>,
        headers: Vec<(String, String)>,
    },
    /// Explicit session teardown.
    Delete { id: String },
}

/// Result returned by `whepserversink`'s signaller, flattened into plain
/// Rust types. Header names and values are extracted from the gstreamer
/// `Structure` inside the manager so consumers (e.g. the axum WHEP router)
/// don't need to depend on gstreamer.
pub struct WhepReply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
}

struct ActiveSource {
    pipeline: NdiPipeline,
}

/// Outcome of `check_active_entry` — drives `start_pipeline`'s control flow.
#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
enum StateCheckOutcome {
    /// Active entry exists and the pipeline is healthy (Streaming or
    /// Starting). Caller should treat the request as a no-op.
    Idempotent,
    /// No entry, OR the existing entry's pipeline is dead (Stopped or
    /// Errored). In the dead case the entry has already been removed.
    /// Caller should proceed to build a fresh pipeline.
    Rebuild,
}

/// Pure state-check for the active-source HashMap. Extracted from
/// `start_pipeline` so the regression test for the "dead pipeline left
/// in HashMap" bug can run without libndi/GPU/gst-plugins — see
/// `start_pipeline_state_check_tests` below.
///
/// Idempotency check must inspect the LIVE pipeline state, not just
/// HashMap presence. A pipeline that transitioned to `Stopped` (NDI
/// broadcaster EOS) or `Errored` (ndisrc fault) keeps its HashMap entry
/// alive — without this state check, both the manual re-activate path
/// and the 30s auto-reconnect loop (state/mod.rs) early-return `Ok` and
/// leave the dead pipeline sitting in the slot. The next WHEP POST then
/// sees `PipelineState::Stopped` and 503s the client, with no recovery
/// path short of an operator-driven `deactivate + activate` cycle.
/// Treat Streaming/Starting as a true idempotent no-op; treat
/// Stopped/Errored as dead and rebuild from scratch.
async fn check_active_entry(
    active: &mut HashMap<String, ActiveSource>,
    source_id: &str,
) -> StateCheckOutcome {
    if let Some(existing) = active.get(source_id) {
        match existing.pipeline.state() {
            PipelineState::Streaming | PipelineState::Starting => {
                return StateCheckOutcome::Idempotent;
            }
            PipelineState::Stopped | PipelineState::Errored(_) => {
                if let Some(mut dead) = active.remove(source_id) {
                    dead.pipeline.stop().await;
                }
            }
        }
    }
    StateCheckOutcome::Rebuild
}

pub struct NdiManager {
    _sdk: Arc<NdiLib>,
    source_list: SourceList,
    _finder_shutdown: FinderShutdown,
    /// Map source_id (UUID string) → ActiveSource pipeline.
    active: Mutex<HashMap<String, ActiveSource>>,
}

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
    /// the ndisrc has connected to the broadcaster, ndisrcdemux has negotiated
    /// video/audio caps, and webrtcsink's input pads are ready. Without this
    /// wait, an early WHEP POST hits webrtcsink before input caps are set
    /// and panics on `in_caps.unwrap()` (gst-plugin-webrtc imp.rs:3548).
    ///
    /// A 7-second timeout caps the wait — long enough for ndisrc to find the
    /// source on a healthy LAN, short enough that a missing/dead broadcaster
    /// reports back quickly to the operator.
    pub async fn start_pipeline(&self, source_id: &str, ndi_name: &str) -> Result<()> {
        let mut active = self.active.lock().await;
        if let StateCheckOutcome::Idempotent = check_active_entry(&mut active, source_id).await {
            return Ok(());
        }

        let whep_url = format!("/ndi/whep/{}", source_id);
        let mut pipeline = NdiPipeline::build(ndi_name, whep_url)?;
        pipeline.start().await?;

        // Wait for webrtcsink's video sink-pad to have negotiated caps. Two
        // states aren't enough on their own:
        //  - `pipeline.state == Playing` fires almost immediately on a live
        //    source (NoPreroll) — before ndisrc has actually sent any frame.
        //  - The `streams` HashMap inside webrtcsink only gets `in_caps` set
        //    when the input pad receives its first CAPS event, which is when
        //    ndisrcdemux has identified video/audio formats from real NDI data.
        //
        // Polling `sink_element.sink_pad("video_0").current_caps()` is the
        // most reliable signal that caps are set. Without this, an early WHEP
        // POST hits webrtcsink while `in_caps == None` and panics at
        // gst-plugin-webrtc imp.rs:3548 (`in_caps.unwrap()`).
        //
        // 8-second budget: ndisrc takes ~2-5s on a healthy LAN to find a
        // broadcast + receive first frame. Beyond 8s the source likely doesn't
        // exist and we'd rather fail fast than hang the operator UI.
        let sink = pipeline
            .sink_element()
            .ok_or_else(|| anyhow!("pipeline has no sink element"))?;
        let video_pad = sink
            .static_pad("video_0")
            .ok_or_else(|| anyhow!("whepserversink has no video_0 sink pad"))?;
        let mut watcher = pipeline.state_watcher();
        let caps_ready = tokio::time::timeout(std::time::Duration::from_secs(8), async {
            loop {
                // Bail out if the pipeline errored (e.g. NDI source not found
                // and ndisrc emitted ERROR on the bus).
                if let crate::pipeline::PipelineState::Errored(ref e) = *watcher.borrow_and_update()
                {
                    return Err(anyhow!("pipeline errored: {e}"));
                }
                if video_pad.current_caps().is_some() {
                    return Ok(());
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        })
        .await;

        match caps_ready {
            Ok(Ok(())) => {
                active.insert(source_id.to_string(), ActiveSource { pipeline });
                Ok(())
            }
            Ok(Err(e)) => {
                pipeline.stop().await;
                Err(e)
            }
            Err(_) => {
                pipeline.stop().await;
                Err(anyhow!(
                    "NDI source '{ndi_name}' did not deliver any frame within 8s; \
                     ndisrc could not connect or the broadcaster is silent"
                ))
            }
        }
    }

    /// Stop one pipeline.
    pub async fn stop_pipeline(&self, source_id: &str) {
        let mut active = self.active.lock().await;
        if let Some(mut src) = active.remove(source_id) {
            src.pipeline.stop().await;
        }
    }

    /// Stop ALL pipelines.
    pub async fn stop_all(&self) {
        let mut active = self.active.lock().await;
        for (_, mut src) in active.drain() {
            src.pipeline.stop().await;
        }
    }

    /// Is the given source's pipeline currently active?
    pub async fn is_active(&self, source_id: &str) -> bool {
        self.active.lock().await.contains_key(source_id)
    }

    /// Forward a WHEP HTTP exchange into the source's `whepserversink`
    /// signaller via `emit_by_name`. The signaller's Promise resolves with
    /// `{status: u32, headers: gst::Structure, body: glib::Bytes}`.
    pub async fn whep_signaller_call(&self, source_id: &str, op: WhepOp) -> Result<WhepReply> {
        let sink = {
            let active = self.active.lock().await;
            let src = active
                .get(source_id)
                .ok_or_else(|| anyhow!(SOURCE_NOT_ACTIVE_ERR))?;
            match src.pipeline.state() {
                PipelineState::Streaming | PipelineState::Starting => {}
                PipelineState::Stopped => return Err(anyhow!("pipeline stopped")),
                PipelineState::Errored(e) => return Err(anyhow!("pipeline errored: {e}")),
            }
            src.pipeline
                .sink_element()
                .ok_or_else(|| anyhow!("pipeline has no sink element"))?
        };

        // Do all signaller work (non-Send glib::Object) in a synchronous block
        // before any .await points so the async fn stays Send.
        let fut = {
            let signaller = sink
                .dynamic_cast_ref::<gstreamer::ChildProxy>()
                .ok_or_else(|| anyhow!("sink is not a ChildProxy"))?
                .child_by_name("signaller")
                .ok_or_else(|| anyhow!("no signaller child on whepserversink"))?;

            let (promise, fut) = gstreamer::Promise::new_future();
            match op {
                WhepOp::Post { id, body } => {
                    let bytes = glib::Bytes::from_owned(body);
                    signaller.emit_by_name::<()>("post", &[&id, &bytes, &promise]);
                }
                WhepOp::Patch { id, body, headers } => {
                    let bytes = glib::Bytes::from_owned(body);
                    let mut sb = gstreamer::Structure::builder("whep-signaller/headers");
                    for (k, v) in &headers {
                        sb = sb.field(k.as_str(), v);
                    }
                    signaller.emit_by_name::<()>("patch", &[&id, &bytes, &sb.build(), &promise]);
                }
                WhepOp::Delete { id } => {
                    signaller.emit_by_name::<()>("delete", &[&id, &promise]);
                }
            }
            // `signaller` (non-Send glib::Object) is dropped here before `fut.await`
            fut
        };

        let reply = fut
            .await
            .map_err(|e| anyhow!("whep signaller promise error: {:?}", e))
            .context("whep signaller promise dropped")?
            .ok_or_else(|| anyhow!("whep signaller returned no payload"))?;
        let status = reply
            .get::<u32>("status")
            .map_err(|e| anyhow!("missing status field: {e}"))? as u16;
        let headers = match reply.get::<gstreamer::Structure>("headers") {
            Ok(s) => s
                .iter()
                .filter_map(|(name, value)| {
                    value.get::<String>().ok().map(|v| (name.to_string(), v))
                })
                .collect(),
            Err(_) => Vec::new(),
        };
        let body = reply
            .get::<glib::Bytes>("body")
            .ok()
            .map(|b| b.as_ref().to_vec());
        Ok(WhepReply {
            status,
            headers,
            body,
        })
    }
}

#[cfg(test)]
mod start_pipeline_state_check_tests {
    use super::*;

    /// Empty HashMap → `Rebuild` outcome. Trivial case.
    #[tokio::test]
    async fn empty_map_requests_rebuild() {
        let mut active = HashMap::new();
        let outcome = check_active_entry(&mut active, "any-id").await;
        assert_eq!(outcome, StateCheckOutcome::Rebuild);
        assert!(active.is_empty());
    }

    /// REGRESSION TEST for the production bug surfaced 2026-05-20: an entry
    /// in the HashMap whose pipeline transitioned to `Stopped` (NDI
    /// broadcaster EOS) must be REMOVED so the caller rebuilds. The buggy
    /// version of start_pipeline early-returned Ok on HashMap presence
    /// alone, leaving the dead entry alive forever — WHEP POSTs then
    /// 503'd with "pipeline stopped" and recovery required a manual
    /// deactivate+activate cycle.
    ///
    /// Runs on every CI host (no libndi, no GPU, no gst-plugins required) —
    /// uses `NdiPipeline::stopped_for_test()` which constructs a
    /// pipeline-shaped value in `Stopped` state without invoking real
    /// GStreamer element building.
    #[tokio::test]
    async fn dead_stopped_entry_is_removed_and_rebuild_requested() {
        let mut active: HashMap<String, ActiveSource> = HashMap::new();
        let dead = crate::pipeline::NdiPipeline::stopped_for_test();
        assert_eq!(
            dead.state(),
            PipelineState::Stopped,
            "precondition: stopped_for_test must yield a Stopped pipeline",
        );
        active.insert("test-id".to_string(), ActiveSource { pipeline: dead });
        assert!(active.contains_key("test-id"));

        let outcome = check_active_entry(&mut active, "test-id").await;

        assert_eq!(
            outcome,
            StateCheckOutcome::Rebuild,
            "REGRESSION: dead Stopped entry must trigger Rebuild, not Idempotent",
        );
        assert!(
            !active.contains_key("test-id"),
            "REGRESSION: dead Stopped entry must be removed from the active map",
        );
    }

    /// `Streaming` entry → true idempotent no-op. Confirms the healthy
    /// path is preserved (we don't accidentally remove live pipelines).
    #[tokio::test]
    async fn streaming_entry_is_left_alone_idempotent() {
        let mut active: HashMap<String, ActiveSource> = HashMap::new();
        let mut p = crate::pipeline::NdiPipeline::stopped_for_test();
        p.set_state_for_test(PipelineState::Streaming);
        assert_eq!(p.state(), PipelineState::Streaming);
        active.insert("test-id".to_string(), ActiveSource { pipeline: p });

        let outcome = check_active_entry(&mut active, "test-id").await;

        assert_eq!(outcome, StateCheckOutcome::Idempotent);
        assert!(
            active.contains_key("test-id"),
            "Streaming entry must NOT be removed — that's the idempotent path",
        );
    }

    /// `Errored` entry → same outcome as Stopped: remove + rebuild. Catches
    /// regressions that only handle the Stopped variant.
    #[tokio::test]
    async fn errored_entry_is_removed_and_rebuild_requested() {
        let mut active: HashMap<String, ActiveSource> = HashMap::new();
        let mut p = crate::pipeline::NdiPipeline::stopped_for_test();
        p.set_state_for_test(PipelineState::Errored("ndisrc fault".to_string()));
        active.insert("test-id".to_string(), ActiveSource { pipeline: p });

        let outcome = check_active_entry(&mut active, "test-id").await;

        assert_eq!(outcome, StateCheckOutcome::Rebuild);
        assert!(!active.contains_key("test-id"));
    }
}
