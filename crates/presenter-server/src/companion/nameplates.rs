//! Lower-third nameplate Companion surface (#779, epic #718 §7/§11).
//!
//! Companion needs three things the scene/overlay surface doesn't: a
//! server-populated LIST of plates (so the show/toggle actions get a dropdown
//! and the presets can be generated), per-plate text VARIABLES (so a button can
//! display the plate's name/role), and an ACTIVE marker (so a feedback lights
//! while a plate is on air). The plate list + active plate travel to the plugin
//! as a new outgoing `nameplates` message (list) plus the normal `variables`
//! message (texts). Resolving the list needs an async repository read, so — like
//! `StreamState` — it runs in the companion live-loop (`mod.rs`), not the sync
//! `variables::apply_live_event`.

use super::variables::{CompanionVariableState, VariableBuilder};
use crate::state::AppState;
use serde::Serialize;

/// Output whose nameplates this v1 module tracks (the seeded default output).
pub(super) const DEFAULT_OUTPUT: &str = "stream";

/// Placeholder for an idle `nameplate_active_*` variable.
const PLACEHOLDER: &str = "-";

/// One person plate in the outgoing `nameplates` message — the plugin's dropdown
/// and preset source. `role` may be empty (a plate with only a name).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct NameplatePlate {
    pub(super) id: i64,
    pub(super) name: String,
    pub(super) role: String,
}

/// Resolve an output's person plates into the wire shape. A failed read (e.g. a
/// deleted output) degrades to an empty list rather than failing the loop.
pub(super) async fn resolve_nameplates(state: &AppState, output: &str) -> Vec<NameplatePlate> {
    match state.repository().list_stream_nameplates(output).await {
        Ok(list) => list
            .into_iter()
            .map(|p| NameplatePlate {
                id: p.id,
                name: p.primary_text,
                role: p.secondary_text,
            })
            .collect(),
        Err(error) => {
            tracing::warn!(%error, output, "failed to load nameplates for companion");
            Vec::new()
        }
    }
}

/// Write the nameplate variables into the builder (delegated from
/// `variables::to_variables`): per-plate `nameplate_<id>_name`/`_role`, the song
/// plate's `nameplate_song_name`/`_role` (mirroring the live stage snapshot), and
/// the on-air `nameplate_active_name`/`_role`/`_id` (placeholders when idle).
pub(super) fn write_nameplate_variables(
    builder: &mut VariableBuilder,
    state: &CompanionVariableState,
) {
    for plate in state.nameplate_plates() {
        builder.set(&format!("nameplate_{}_name", plate.id), plate.name.clone());
        builder.set(&format!("nameplate_{}_role", plate.id), plate.role.clone());
    }
    // The song plate's texts mirror the live stage snapshot (current song +
    // library), so they track worship changes without a plate edit.
    let (song_name, song_role) = state.stage_song_texts();
    builder.set("nameplate_song_name", song_name);
    builder.set("nameplate_song_role", song_role);

    match state.nameplate_active() {
        Some(active) => {
            builder.set("nameplate_active_name", active.primary.clone());
            builder.set("nameplate_active_role", active.secondary.clone());
            builder.set(
                "nameplate_active_id",
                active
                    .nameplate_id
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "song".to_string()),
            );
        }
        None => {
            builder.set("nameplate_active_name", String::new());
            builder.set("nameplate_active_role", String::new());
            builder.set("nameplate_active_id", PLACEHOLDER.to_string());
        }
    }
}
