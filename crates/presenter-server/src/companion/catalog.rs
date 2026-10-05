//! Companion module catalog (#814): the CHOICES the Bitfocus module builds its
//! dropdowns from — the operator-selectable stage layouts and, per stream
//! output, its base + overlay scene names.
//!
//! Same push model as the #779 nameplate list: an outgoing `catalog` message
//! sent on connect (and lag recovery) and re-sent whenever a stream CONFIG
//! write changes its content (`LiveEvent::StreamConfigChanged` — output
//! create/rename, scene create/rename/delete/reorder; an element-only edit
//! re-resolves to the SAME catalog and sends nothing). Deleting an OUTPUT
//! publishes no live event, so it drops out at the next config change or
//! reconnect (stored values keep working via `allowCustom`). Layouts are compiled in
//! (`StageDisplayLayout::operator_selectable`), so they only change with a
//! deploy, which restarts the server and reconnects the module.
//!
//! Resolving the stream part needs async repository reads, so — like
//! `StreamState` — it runs in the companion live-loop (`protocol.rs`
//! `handle_live_event`), never the sync `variables::apply_live_event`.

use super::variables::CompanionVariableState;
use crate::state::AppState;
use presenter_core::{SceneKind, StageDisplayLayout};
use serde::Serialize;
use tracing::{info, warn};

/// One stage layout choice: the code the `stage.layout` command sends + its
/// display name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct CatalogLayout {
    pub(super) code: String,
    pub(super) name: String,
}

/// One scene of a stream output, addressed BY NAME by the `stream_*` commands.
/// `kind` serialises as `"base"` / `"overlay"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct CatalogScene {
    pub(super) name: String,
    pub(super) kind: SceneKind,
}

/// One stream output (its slug is the `output` the `stream_*` commands send)
/// with its scenes in the editor's order (base before overlay, then position).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct CatalogOutput {
    pub(super) slug: String,
    pub(super) name: String,
    pub(super) scenes: Vec<CatalogScene>,
}

/// The full catalog snapshot held per Companion session (in
/// `CompanionVariableState`) so a re-send happens only when content changed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub(super) struct CompanionCatalog {
    pub(super) layouts: Vec<CatalogLayout>,
    pub(super) stream: Vec<CatalogOutput>,
}

/// The layouts the `stage.layout` command ACCEPTS — the operator-selectable
/// set (`built_in()` minus the internal `camera-crew`, which
/// `validate_operator_selectable` refuses). Same set as `GET /stage-displays`.
pub(super) fn catalog_layouts() -> Vec<CatalogLayout> {
    StageDisplayLayout::operator_selectable()
        .into_iter()
        .map(|layout| CatalogLayout {
            code: layout.code,
            name: layout.name,
        })
        .collect()
}

/// Every stream output with its scenes. A failed output LIST is an error (the
/// caller decides how to degrade); a single output whose def fails to load
/// (e.g. deleted between the list and the read) is skipped with a warning.
async fn resolve_stream_catalog(state: &AppState) -> anyhow::Result<Vec<CatalogOutput>> {
    let outputs = state.repository().list_stream_outputs().await?;
    let mut catalog = Vec::with_capacity(outputs.len());
    for output in outputs {
        match state.repository().load_output_def(&output.slug).await {
            Ok(def) => catalog.push(CatalogOutput {
                slug: def.slug,
                name: def.name,
                scenes: def
                    .scenes
                    .into_iter()
                    .map(|scene| CatalogScene {
                        name: scene.name,
                        kind: scene.kind,
                    })
                    .collect(),
            }),
            Err(error) => warn!(
                %error,
                output = %output.slug,
                "failed to load stream output def for the companion catalog — skipping it"
            ),
        }
    }
    Ok(catalog)
}

/// Resolve the full catalog (layouts + stream outputs/scenes).
pub(super) async fn resolve_catalog(state: &AppState) -> anyhow::Result<CompanionCatalog> {
    Ok(CompanionCatalog {
        layouts: catalog_layouts(),
        stream: resolve_stream_catalog(state).await?,
    })
}

/// The connect-time catalog. A failed stream read degrades to layouts-only
/// (the module keeps its free-typed values working via `allowCustom`) rather
/// than failing the connect.
pub(super) async fn initial_catalog(state: &AppState) -> CompanionCatalog {
    match resolve_catalog(state).await {
        Ok(catalog) => catalog,
        Err(error) => {
            warn!(%error, "failed to resolve stream outputs for the companion catalog");
            CompanionCatalog {
                layouts: catalog_layouts(),
                stream: Vec::new(),
            }
        }
    }
}

/// Re-resolve the catalog after a stream config change and store it. Returns
/// whether its CONTENT changed (→ the caller re-sends it). A failed read keeps
/// the previous catalog (returns `false`) so a transient error never wipes the
/// module's dropdowns.
pub(super) async fn refresh_catalog(
    state: &AppState,
    variables: &mut CompanionVariableState,
) -> bool {
    match resolve_catalog(state).await {
        Ok(catalog) => {
            let changed = variables.apply_catalog(catalog);
            if changed {
                let catalog = variables.catalog();
                info!(
                    layouts = catalog.layouts.len(),
                    outputs = catalog.stream.len(),
                    "companion catalog changed after a stream config write; re-sending"
                );
            }
            changed
        }
        Err(error) => {
            warn!(%error, "failed to refresh the companion catalog; keeping the previous one");
            false
        }
    }
}
