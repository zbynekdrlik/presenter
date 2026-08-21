//! Stream-graphics operator editor (`/ui/stream`) — component tree + shared
//! state (epic #718, ADR 0009; PR-6 #713).
//!
//! v1 skeleton per the epic architecture: base scenes as clickable COLUMNS
//! (exclusive activation), an OVERLAY row (independent on/off toggles), plus
//! add / remove / rename / reorder. Element CRUD + property panel is #714;
//! preview iframe + assets are #715 — NOT here.
//!
//! Talks to the `/stream/api/*` REST surface (#707) via the generic
//! `crate::api::*_json` helpers and reflects live state through the generic
//! `/live/ws` client (`crate::ws::use_live_websocket`, wired in the page). The
//! request DTOs below are the write-side payloads (camelCase, matching the
//! server's `router/stream.rs`); the response types come from `presenter-core`.

pub mod editor_scenes;

use leptos::prelude::*;
use presenter_core::{SceneKind, StreamOutputDef, StreamSceneDef, StreamShowState};
use serde::Serialize;

/// The single default output authored by this v1 editor (seeded by the
/// migration + `AppState::in_memory()`; N nameable outputs are a later PR).
pub const DEFAULT_OUTPUT_SLUG: &str = "stream";

/// Toast auto-hide delay, matching the settings page's 4200 ms.
const TOAST_HIDE_MS: u32 = 4_200;

// ---- Client write DTOs (camelCase, mirroring router/stream.rs) -------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SetActiveSceneReq {
    scene_id: Option<i64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SetOverlayReq {
    active: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateSceneReq {
    name: String,
    kind: SceneKind,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RenameSceneReq {
    name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReorderReq {
    ids: Vec<i64>,
}

/// Derive the show-state (active base + active overlays + revision) implied by
/// a freshly-fetched def — used to seed the local `active` signal on load /
/// refetch. Activation events update `active` directly afterwards.
pub fn show_state_from_def(def: &StreamOutputDef) -> StreamShowState {
    StreamShowState {
        active_scene_id: def.active_scene_id,
        active_overlay_ids: def
            .scenes
            .iter()
            .filter(|s| s.kind == SceneKind::Overlay && s.is_active)
            .map(|s| s.id)
            .collect(),
        config_revision: def.config_revision,
    }
}

/// Shared editor state, threaded (by value — every field is a `Copy` signal)
/// into each sub-component. `def` is the full configuration; `active` is the
/// live show-state (base + overlays). The toast triple mirrors the settings
/// page's `ToastHandle`.
#[derive(Clone, Copy)]
pub struct StreamEditorCtx {
    pub def: RwSignal<Option<StreamOutputDef>>,
    pub active: RwSignal<StreamShowState>,
    pub toast_msg: RwSignal<String>,
    pub toast_visible: RwSignal<bool>,
    pub toast_state: RwSignal<String>,
}

impl StreamEditorCtx {
    /// Show a transient toast, auto-hiding after `TOAST_HIDE_MS`.
    pub fn show_toast(self, msg: &str, variant: &str) {
        self.toast_state.set(variant.to_string());
        self.toast_msg.set(msg.to_string());
        self.toast_visible.set(true);
        let visible = self.toast_visible;
        gloo_timers::callback::Timeout::new(TOAST_HIDE_MS, move || visible.set(false)).forget();
    }

    /// Re-fetch the full def and reseed `active` from it. Called on mount, after
    /// every config write, and on a `StreamConfigChanged` with a newer revision.
    pub fn refresh(self) {
        leptos::task::spawn_local(async move {
            match crate::api::get_json::<StreamOutputDef>(&def_path()).await {
                Ok(def) => {
                    self.active.set(show_state_from_def(&def));
                    self.def.set(Some(def));
                }
                Err(e) => self.show_toast(&format!("Načítanie zlyhalo: {e}"), "error"),
            }
        });
    }

    /// Exclusive base activation; `None` clears the base (transparent). The
    /// returned show-state is applied directly (activation does not bump
    /// `config_revision`, so no def refetch is needed).
    pub fn activate_base(self, scene_id: Option<i64>) {
        leptos::task::spawn_local(async move {
            let req = SetActiveSceneReq { scene_id };
            match crate::api::put_json::<_, StreamShowState>(&active_scene_path(), &req).await {
                Ok(state) => self.active.set(state),
                Err(e) => self.show_toast(&format!("Aktivácia zlyhala: {e}"), "error"),
            }
        });
    }

    /// Toggle one overlay on/off (independent of the base + other overlays).
    pub fn toggle_overlay(self, scene_id: i64, active: bool) {
        leptos::task::spawn_local(async move {
            let req = SetOverlayReq { active };
            match crate::api::put_json::<_, StreamShowState>(&overlay_path(scene_id), &req).await {
                Ok(state) => self.active.set(state),
                Err(e) => self.show_toast(&format!("Overlay zlyhal: {e}"), "error"),
            }
        });
    }

    /// Create a base/overlay scene, then refetch the def.
    pub fn add_scene(self, name: String, kind: SceneKind) {
        let trimmed = name.trim().to_string();
        if trimmed.is_empty() {
            self.show_toast("Názov scény nesmie byť prázdny.", "error");
            return;
        }
        leptos::task::spawn_local(async move {
            let req = CreateSceneReq {
                name: trimmed,
                kind,
            };
            match crate::api::post_json::<_, StreamSceneDef>(&scenes_path(), &req).await {
                Ok(_) => {
                    self.refresh();
                    self.show_toast("Scéna pridaná.", "success");
                }
                Err(e) => self.show_toast(&format!("Pridanie zlyhalo: {e}"), "error"),
            }
        });
    }

    /// Delete a scene (native confirm dialog first), then refetch the def.
    pub fn delete_scene(self, id: i64) {
        let confirmed = crate::utils::window::window()
            .confirm_with_message("Zmazať túto scénu?")
            .unwrap_or(false);
        if !confirmed {
            return;
        }
        leptos::task::spawn_local(async move {
            match crate::api::delete(&scene_path(id)).await {
                Ok(()) => {
                    self.refresh();
                    self.show_toast("Scéna zmazaná.", "success");
                }
                Err(e) => self.show_toast(&format!("Zmazanie zlyhalo: {e}"), "error"),
            }
        });
    }

    /// Rename a scene, then refetch the def.
    pub fn rename_scene(self, id: i64, name: String) {
        let trimmed = name.trim().to_string();
        if trimmed.is_empty() {
            self.show_toast("Názov scény nesmie byť prázdny.", "error");
            return;
        }
        leptos::task::spawn_local(async move {
            let req = RenameSceneReq { name: trimmed };
            match crate::api::patch_json::<_, StreamSceneDef>(&scene_path(id), &req).await {
                Ok(_) => {
                    self.refresh();
                    self.show_toast("Premenované.", "success");
                }
                Err(e) => self.show_toast(&format!("Premenovanie zlyhalo: {e}"), "error"),
            }
        });
    }

    /// Move a scene one step within its own kind (base or overlay). The server
    /// reorder endpoint wants the FULL id set for the output and re-assigns
    /// positions per-kind by list order, so we send base ids ++ overlay ids
    /// with the one swap applied.
    pub fn move_scene(self, id: i64, up: bool) {
        let Some(def) = self.def.get_untracked() else {
            return;
        };
        let mut base: Vec<i64> = kind_ids(&def, SceneKind::Base);
        let mut overlay: Vec<i64> = kind_ids(&def, SceneKind::Overlay);
        let list = if base.contains(&id) {
            &mut base
        } else {
            &mut overlay
        };
        let Some(pos) = list.iter().position(|x| *x == id) else {
            return;
        };
        let target = if up {
            match pos.checked_sub(1) {
                Some(t) => t,
                None => return,
            }
        } else if pos + 1 < list.len() {
            pos + 1
        } else {
            return;
        };
        list.swap(pos, target);
        let mut ids = base;
        ids.extend(overlay);
        leptos::task::spawn_local(async move {
            let req = ReorderReq { ids };
            match crate::api::put_no_content(&scenes_order_path(), &req).await {
                Ok(()) => self.refresh(),
                Err(e) => self.show_toast(&format!("Zmena poradia zlyhala: {e}"), "error"),
            }
        });
    }
}

/// Scene ids of one kind, in current (def) order.
fn kind_ids(def: &StreamOutputDef, kind: SceneKind) -> Vec<i64> {
    def.scenes
        .iter()
        .filter(|s| s.kind == kind)
        .map(|s| s.id)
        .collect()
}

fn def_path() -> String {
    format!("/stream/api/outputs/{DEFAULT_OUTPUT_SLUG}/def")
}

fn scenes_path() -> String {
    format!("/stream/api/outputs/{DEFAULT_OUTPUT_SLUG}/scenes")
}

fn scenes_order_path() -> String {
    format!("/stream/api/outputs/{DEFAULT_OUTPUT_SLUG}/scenes/order")
}

fn active_scene_path() -> String {
    format!("/stream/api/outputs/{DEFAULT_OUTPUT_SLUG}/active-scene")
}

fn overlay_path(id: i64) -> String {
    format!("/stream/api/outputs/{DEFAULT_OUTPUT_SLUG}/overlays/{id}")
}

fn scene_path(id: i64) -> String {
    format!("/stream/api/scenes/{id}")
}
