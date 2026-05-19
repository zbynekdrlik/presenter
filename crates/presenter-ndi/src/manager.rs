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

/// One operation in the WHEP signaller protocol.
pub enum WhepOp {
    /// SDP offer (or session-scoped re-offer).
    Post {
        id: Option<String>,
        body: Vec<u8>,
    },
    /// ICE trickle update.
    Patch {
        id: String,
        body: Vec<u8>,
        headers: Vec<(String, String)>,
    },
    /// Explicit session teardown.
    Delete { id: String },
}

/// Result returned by `whepserversink`'s signaller as a `gst::Structure`,
/// flattened into idiomatic Rust.
pub struct WhepReply {
    pub status: u16,
    pub headers: Option<gstreamer::Structure>,
    pub body: Option<Vec<u8>>,
}

struct ActiveSource {
    pipeline: NdiPipeline,
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

    pub fn discover_sources(
        &self,
        _timeout_ms: u32,
    ) -> Result<Vec<discovery::NdiSourceInfo>> {
        Ok(self.source_list.read())
    }

    /// Start a pipeline for the given source.
    ///
    /// `source_id` = UUID from the `video_sources` DB row (used as the WHEP URL key).
    /// `ndi_name` = NDI broadcaster name (e.g. "STREAM-SNV (stream)").
    pub async fn start_pipeline(&self, source_id: &str, ndi_name: &str) -> Result<()> {
        let mut active = self.active.lock().await;
        if active.contains_key(source_id) {
            return Ok(()); // Idempotent.
        }

        let whep_url = format!("/ndi/whep/{}", source_id);
        let mut pipeline = NdiPipeline::build(ndi_name, whep_url)?;
        pipeline.start().await?;
        active.insert(
            source_id.to_string(),
            ActiveSource { pipeline },
        );
        Ok(())
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
    pub async fn whep_signaller_call(
        &self,
        source_id: &str,
        op: WhepOp,
    ) -> Result<WhepReply> {
        let sink = {
            let active = self.active.lock().await;
            let src = active
                .get(source_id)
                .ok_or_else(|| anyhow!("source not active"))?;
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
                    signaller.emit_by_name::<()>(
                        "patch",
                        &[&id, &bytes, &sb.build(), &promise],
                    );
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
        let headers = reply.get::<gstreamer::Structure>("headers").ok();
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
