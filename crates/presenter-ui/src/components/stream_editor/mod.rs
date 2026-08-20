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

pub mod editor_panel;
pub mod editor_scenes;
pub mod element_form;
pub mod props_access;
pub mod text_style_form;

use leptos::prelude::*;
use presenter_core::{
    SceneKind, StreamElementDef, StreamElementProps, StreamOutputDef, StreamSceneDef,
    StreamShowState,
};
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
    /// The scene currently opened for element authoring (#714). `None` = the
    /// element panel is closed.
    pub selected_scene: RwSignal<Option<i64>>,
    /// The element currently opened in the property form (#714). `None` = no
    /// element selected (list only).
    pub selected_element: RwSignal<Option<i64>>,
    /// Inline validation error for the property form — the server's 422 message
    /// (#714). Empty = no error.
    pub prop_error: RwSignal<String>,
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
            self.reload_def().await;
        });
    }

    /// Awaitable core of [`Self::refresh`] — refetch the def + reseed `active`.
    /// Used where a follow-up action must SEQUENCE after the def is present
    /// (e.g. `add_element` selecting the freshly-created element).
    async fn reload_def(self) {
        match crate::api::get_json::<StreamOutputDef>(&def_path()).await {
            Ok(def) => {
                self.active.set(show_state_from_def(&def));
                self.def.set(Some(def));
            }
            Err(e) => self.show_toast(&format!("Načítanie zlyhalo: {e}"), "error"),
        }
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
                    if self.selected_scene.get_untracked() == Some(id) {
                        self.close_panel();
                    }
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

    // ---- Element authoring (#714) -----------------------------------------

    /// Open a scene for element authoring; clears any element selection + error.
    pub fn select_scene(self, scene_id: i64) {
        self.selected_scene.set(Some(scene_id));
        self.selected_element.set(None);
        self.prop_error.set(String::new());
    }

    /// Close the element panel (no scene selected).
    pub fn close_panel(self) {
        self.selected_scene.set(None);
        self.selected_element.set(None);
        self.prop_error.set(String::new());
    }

    /// Open an element in the property form; clears any prior inline error.
    pub fn select_element(self, element_id: i64) {
        self.selected_element.set(Some(element_id));
        self.prop_error.set(String::new());
    }

    /// Create an element on a scene with the given default props, then refetch
    /// the def INLINE (so the new element is present) and select it.
    pub fn add_element(self, scene_id: i64, props: StreamElementProps) {
        leptos::task::spawn_local(async move {
            match crate::api::post_json_detail::<StreamElementProps, StreamElementDef>(
                &elements_path(scene_id),
                &props,
            )
            .await
            {
                Ok(created) => {
                    self.reload_def().await;
                    self.prop_error.set(String::new());
                    self.selected_element.set(Some(created.id));
                    self.show_toast("Prvok pridaný.", "success");
                }
                Err(e) => self.show_toast(&format!("Pridanie prvku zlyhalo: {e}"), "error"),
            }
        });
    }

    /// Delete an element (native confirm first), then refetch the def.
    pub fn delete_element(self, element_id: i64) {
        let confirmed = crate::utils::window::window()
            .confirm_with_message("Zmazať tento prvok?")
            .unwrap_or(false);
        if !confirmed {
            return;
        }
        leptos::task::spawn_local(async move {
            match crate::api::delete(&element_path(element_id)).await {
                Ok(()) => {
                    if self.selected_element.get_untracked() == Some(element_id) {
                        self.selected_element.set(None);
                    }
                    self.reload_def().await;
                    self.show_toast("Prvok zmazaný.", "success");
                }
                Err(e) => self.show_toast(&format!("Zmazanie prvku zlyhalo: {e}"), "error"),
            }
        });
    }

    /// Move an element one step in z-order within its scene. The server reorder
    /// endpoint wants the FULL element-id set for the scene, so read the def,
    /// list the scene's elements in order, swap the one pair, send them all.
    pub fn move_element(self, element_id: i64, up: bool) {
        let Some(def) = self.def.get_untracked() else {
            return;
        };
        let Some(scene) = def
            .scenes
            .iter()
            .find(|s| s.elements.iter().any(|e| e.id == element_id))
        else {
            return;
        };
        let scene_id = scene.id;
        let mut ids: Vec<i64> = scene.elements.iter().map(|e| e.id).collect();
        let Some(pos) = ids.iter().position(|x| *x == element_id) else {
            return;
        };
        let target = if up {
            match pos.checked_sub(1) {
                Some(t) => t,
                None => return,
            }
        } else if pos + 1 < ids.len() {
            pos + 1
        } else {
            return;
        };
        ids.swap(pos, target);
        leptos::task::spawn_local(async move {
            let req = ReorderReq { ids };
            match crate::api::put_no_content(&elements_order_path(scene_id), &req).await {
                Ok(()) => self.refresh(),
                Err(e) => self.show_toast(&format!("Zmena poradia prvku zlyhala: {e}"), "error"),
            }
        });
    }

    /// PATCH the selected element's props (explicit Save). On a 422 the server
    /// message is rendered inline (`prop_error`) and the def is left unchanged;
    /// on success the def is refetched.
    pub fn save_props(self, element_id: i64, props: StreamElementProps) {
        leptos::task::spawn_local(async move {
            match crate::api::patch_json_detail::<StreamElementProps, StreamElementDef>(
                &element_path(element_id),
                &props,
            )
            .await
            {
                Ok(_) => {
                    self.prop_error.set(String::new());
                    self.refresh();
                    self.show_toast("Uložené.", "success");
                }
                Err(e) => self.prop_error.set(format!("Neplatné hodnoty: {e}")),
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

fn elements_path(scene_id: i64) -> String {
    format!("/stream/api/scenes/{scene_id}/elements")
}

fn elements_order_path(scene_id: i64) -> String {
    format!("/stream/api/scenes/{scene_id}/elements/order")
}

fn element_path(id: i64) -> String {
    format!("/stream/api/elements/{id}")
}
