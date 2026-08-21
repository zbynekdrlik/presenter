//! REST client for the stream-graphics output definition (#709, epic #718).
//!
//! The output page cold-loads the full definition of an output — its scenes
//! (ordered), each scene's elements (ordered by `z_order`), the active base
//! scene id, active-overlay flags, and `config_revision` — then keeps it fresh
//! via `/live/ws` (see `ws/stream.rs`). The write/CRUD side of `/stream/api/*`
//! (#707) is driven by the operator editor (#713+), not by this output page.
//!
//! Endpoint + payload per epic #718 architecture §5/§8 and issue #707:
//!   `GET /stream/api/outputs/{slug}/def` -> `StreamOutputDef` (JSON).

use super::{get_json, ApiError};
use presenter_core::StreamOutputDef;

/// Fetch the full definition + active state of one output by slug.
///
/// `slug` is caller-controlled but is a path SEGMENT; it must already be a valid
/// output slug (`^[a-z0-9-]{1,64}$`) — the pathname branch that constructs it
/// only forwards a single non-empty segment.
pub async fn get_output_def(slug: &str) -> Result<StreamOutputDef, ApiError> {
    get_json(&format!("/stream/api/outputs/{slug}/def")).await
}
