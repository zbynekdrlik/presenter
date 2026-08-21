//! Stream-graphics Companion command family (issue #711, epic #718 §7).
//!
//! The Bitfocus plugin (#712) sends `{ type: "command", command: "stream_*",
//! payload }` over `/companion/ws`; `protocol::parse_command` delegates any
//! `stream_`-prefixed command here. Scenes are addressed BY NAME (matched
//! case-insensitively, unique per output), `output` is an optional slug that
//! defaults to `"stream"` (the seeded default output). The wire contract is
//! FIXED by arch #718 §7 and the built plugin (`lib/stream.js`):
//!
//! | command | payload |
//! |---|---|
//! | `stream_scene_set` | `{ scene, output? }` — exclusive base activate |
//! | `stream_scene_clear` | `{ output? }` — base → none (transparent) |
//! | `stream_overlay_on`/`_off`/`_toggle` | `{ scene, output? }` |
//! | `stream_clear` | `{ output? }` — base clear + ALL overlays off |
//!
//! Execution goes through the [`AppState`] show-state methods (never the
//! repository directly) so every activation publishes [`LiveEvent::StreamState`]
//! (`state/stream.rs`). Kind validation (base-as-overlay / overlay-as-base),
//! unknown scene and unknown output all surface as the non-fatal
//! `{type:"error"}` reply the plugin logs — never a panic (arch §7).
//!
//! Variables (`stream_scene` / `stream_overlays`) are resolved from scene ids to
//! NAMES via `repository().load_output_def`; because that read is async while
//! `variables::apply_live_event` is sync, the resolution lives here and is driven
//! from the companion live-loop (`mod.rs`) — see [`resolve_stream_variables`].

use super::protocol::CompanionCommand;
use super::variables::VariableBuilder;
use crate::live::LiveEvent;
use crate::state::AppState;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use tracing::warn;

/// Output slug used when a command omits `output` (matches the plugin default
/// and the migration-seeded default output).
const DEFAULT_OUTPUT: &str = "stream";

/// Placeholder shown in `stream_scene` / `stream_overlays` when there is no
/// active base scene / no active overlays.
const PLACEHOLDER: &str = "-";

/// A parsed stream-graphics command (arch #718 §7). `scene` is a NAME, `output`
/// a resolved slug (never empty — defaulted to [`DEFAULT_OUTPUT`]).
pub(super) enum StreamCommand {
    SceneSet { output: String, scene: String },
    SceneClear { output: String },
    OverlayOn { output: String, scene: String },
    OverlayOff { output: String, scene: String },
    OverlayToggle { output: String, scene: String },
    Clear { output: String },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SceneOutputPayload {
    scene: String,
    #[serde(default)]
    output: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OutputPayload {
    #[serde(default)]
    output: Option<String>,
}

/// Resolve the optional `output` slug: trim, and fall back to the default
/// output when omitted or blank.
fn resolve_output(output: Option<String>) -> String {
    match output {
        Some(slug) if !slug.trim().is_empty() => slug.trim().to_string(),
        _ => DEFAULT_OUTPUT.to_string(),
    }
}

/// Parse a `{scene, output?}` payload into `(output, scene)`. A missing/blank
/// scene name is a client error (`Err` → non-fatal error reply).
fn scene_output_fields(name: &str, payload: Value) -> Result<(String, String), String> {
    let data: SceneOutputPayload =
        serde_json::from_value(payload).map_err(|err| format!("invalid {name} payload: {err}"))?;
    let scene = data.scene.trim().to_string();
    if scene.is_empty() {
        return Err(format!("{name}: scene name is required"));
    }
    Ok((resolve_output(data.output), scene))
}

/// Parse an `{output?}` payload into the resolved output slug.
fn output_field(name: &str, payload: Value) -> Result<String, String> {
    let data: OutputPayload =
        serde_json::from_value(payload).map_err(|err| format!("invalid {name} payload: {err}"))?;
    Ok(resolve_output(data.output))
}

/// Parse any `stream_`-prefixed command name + payload. Delegated to from
/// `protocol::parse_command`. Unknown `stream_*` names and malformed payloads
/// return `Err(message)`, which the caller renders as a non-fatal error reply.
pub(super) fn parse_stream_command(name: &str, payload: Value) -> Result<CompanionCommand, String> {
    let command = match name {
        "stream_scene_set" => {
            let (output, scene) = scene_output_fields(name, payload)?;
            StreamCommand::SceneSet { output, scene }
        }
        "stream_scene_clear" => StreamCommand::SceneClear {
            output: output_field(name, payload)?,
        },
        "stream_overlay_on" => {
            let (output, scene) = scene_output_fields(name, payload)?;
            StreamCommand::OverlayOn { output, scene }
        }
        "stream_overlay_off" => {
            let (output, scene) = scene_output_fields(name, payload)?;
            StreamCommand::OverlayOff { output, scene }
        }
        "stream_overlay_toggle" => {
            let (output, scene) = scene_output_fields(name, payload)?;
            StreamCommand::OverlayToggle { output, scene }
        }
        "stream_clear" => StreamCommand::Clear {
            output: output_field(name, payload)?,
        },
        other => return Err(format!("unknown command: {other}")),
    };
    Ok(CompanionCommand::Stream(command))
}

fn activation_error(err: anyhow::Error) -> String {
    format!("stream command failed: {err}")
}

/// Execute a parsed stream command against [`AppState`]. Resolves scene NAME →
/// id where needed, then calls the show-state methods (which validate kind and
/// publish [`LiveEvent::StreamState`]). Returns `Ok(())` on success (→ `Ack`) or
/// `Err(message)` on any refusal (→ non-fatal error reply).
pub(super) async fn execute_stream_command(
    state: &AppState,
    command: StreamCommand,
) -> Result<(), String> {
    match command {
        StreamCommand::SceneSet { output, scene } => {
            let scene_id = resolve_scene_id(state, &output, &scene).await?;
            state
                .stream_activate_scene(&output, Some(scene_id))
                .await
                .map_err(activation_error)?;
        }
        StreamCommand::SceneClear { output } => {
            state
                .stream_activate_scene(&output, None)
                .await
                .map_err(activation_error)?;
        }
        StreamCommand::OverlayOn { output, scene } => {
            let scene_id = resolve_scene_id(state, &output, &scene).await?;
            state
                .stream_set_overlay(&output, scene_id, true)
                .await
                .map_err(activation_error)?;
        }
        StreamCommand::OverlayOff { output, scene } => {
            let scene_id = resolve_scene_id(state, &output, &scene).await?;
            state
                .stream_set_overlay(&output, scene_id, false)
                .await
                .map_err(activation_error)?;
        }
        StreamCommand::OverlayToggle { output, scene } => {
            let (scene_id, is_active) = resolve_scene(state, &output, &scene).await?;
            state
                .stream_set_overlay(&output, scene_id, !is_active)
                .await
                .map_err(activation_error)?;
        }
        StreamCommand::Clear { output } => {
            state
                .stream_clear(&output)
                .await
                .map_err(activation_error)?;
        }
    }
    Ok(())
}

/// Resolve a scene NAME to `(id, is_active)` within an output, case-insensitively
/// (scene names are unique per output). An unknown output surfaces as an error
/// (from `load_output_def`); an unknown name is a distinct error. The caller's
/// state method then validates the scene KIND.
async fn resolve_scene(state: &AppState, output: &str, scene: &str) -> Result<(i64, bool), String> {
    let def = state
        .repository()
        .load_output_def(output)
        .await
        .map_err(|err| format!("unknown stream output {output:?}: {err}"))?;
    let target = scene.to_lowercase();
    def.scenes
        .iter()
        .find(|candidate| candidate.name.to_lowercase() == target)
        .map(|candidate| (candidate.id, candidate.is_active))
        .ok_or_else(|| format!("unknown stream scene {scene:?} in output {output:?}"))
}

async fn resolve_scene_id(state: &AppState, output: &str, scene: &str) -> Result<i64, String> {
    Ok(resolve_scene(state, output, scene).await?.0)
}

/// The resolved Companion stream variables: the active base scene NAME and the
/// comma-joined active overlay NAMES (`-` when unset / empty).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StreamVariables {
    pub(super) scene: String,
    pub(super) overlays: String,
}

impl StreamVariables {
    fn cleared() -> Self {
        Self {
            scene: PLACEHOLDER.to_string(),
            overlays: PLACEHOLDER.to_string(),
        }
    }
}

/// Resolve a `StreamState` activation (scene ids) into display NAMES for the
/// `stream_scene` / `stream_overlays` variables. Async (repository def read), so
/// it runs from the companion live-loop rather than the sync `apply_live_event`.
/// A failed def read (e.g. the output was deleted) degrades to the cleared
/// placeholders rather than failing the loop.
pub(super) async fn resolve_stream_variables(
    state: &AppState,
    output: &str,
    active_scene_id: Option<i64>,
    active_overlay_ids: &[i64],
) -> StreamVariables {
    let def = match state.repository().load_output_def(output).await {
        Ok(def) => def,
        Err(error) => {
            warn!(%error, output, "failed to resolve stream companion variable names");
            return StreamVariables::cleared();
        }
    };
    let name_by_id: HashMap<i64, &str> = def
        .scenes
        .iter()
        .map(|scene| (scene.id, scene.name.as_str()))
        .collect();
    let scene = active_scene_id
        .and_then(|id| name_by_id.get(&id).copied())
        .map(str::to_string)
        .unwrap_or_else(|| PLACEHOLDER.to_string());
    let overlay_names: Vec<&str> = active_overlay_ids
        .iter()
        .filter_map(|id| name_by_id.get(id).copied())
        .collect();
    let overlays = if overlay_names.is_empty() {
        PLACEHOLDER.to_string()
    } else {
        overlay_names.join(", ")
    };
    StreamVariables { scene, overlays }
}

/// Write the `stream_scene` / `stream_overlays` variables (delegated from
/// `variables::to_variables` to keep `variables.rs` growth minimal). Absent
/// state renders both placeholders.
pub(super) fn write_stream_variables(
    builder: &mut VariableBuilder,
    stream: Option<&StreamVariables>,
) {
    match stream {
        Some(vars) => {
            builder.set("stream_scene", vars.scene.clone());
            builder.set("stream_overlays", vars.overlays.clone());
        }
        None => {
            builder.set("stream_scene", PLACEHOLDER.to_string());
            builder.set("stream_overlays", PLACEHOLDER.to_string());
        }
    }
}

/// Route a `StreamState` live event into the companion variable state, resolving
/// scene ids → names. Returns whether the variables changed (i.e. a fresh send
/// is warranted). Non-`StreamState` events are not this function's concern.
pub(super) async fn apply_stream_state_event(
    state: &AppState,
    variables: &mut super::variables::CompanionVariableState,
    event: &LiveEvent,
) -> bool {
    if let LiveEvent::StreamState {
        output,
        active_scene_id,
        active_overlay_ids,
        ..
    } = event
    {
        let vars =
            resolve_stream_variables(state, output, *active_scene_id, active_overlay_ids).await;
        variables.apply_stream_state(vars)
    } else {
        false
    }
}
