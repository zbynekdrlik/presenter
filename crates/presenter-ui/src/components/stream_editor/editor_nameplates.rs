//! "Menovky" (lower-third nameplate) editor panel (#779).
//!
//! An output-scoped panel (mounted OUTSIDE the `selected_scene` gate, after
//! `EditorScenes`): the person-plate list (add / inline edit / delete / reorder),
//! a virtual "Pieseň" row, per-row Zobraziť / Skryť buttons that drive the REAL
//! output, a Prehrať button that previews the plate animation in the preview
//! iframe WITHOUT broadcasting (#777 channel sibling), an on-air highlight from
//! the live `StreamNameplate` event, and a one-click "Vytvoriť menovkovú vrstvu"
//! that creates a default-styled `lower_third` overlay scene when the output has
//! none. The plate LIST + show-state methods live here to keep `mod.rs` under its
//! size cap; the shared `StreamEditorCtx` holds only the two signals.

use leptos::prelude::*;
use presenter_core::{
    ActiveNameplate, Nameplate, NameplateSource, SceneKind, StreamElementDef, StreamElementProps,
    StreamSceneDef, StreamShowState,
};
use serde::Serialize;

use super::output_paths::{
    nameplates_active_path, nameplates_order_path, nameplates_path, overlay_path, scenes_path,
};
use super::props_access::default_element_props;
use super::{bool_attr, StreamEditorCtx};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NameplateTextReq {
    primary_text: String,
    secondary_text: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SetActiveReq {
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<i64>,
}

/// Plate resource path (id-scoped, so not output-scoped — stays local).
fn nameplate_path(id: i64) -> String {
    format!("/stream/api/nameplates/{id}")
}

impl StreamEditorCtx {
    /// Re-fetch the person-plate list (mount + on a `StreamNameplatesChanged`).
    pub fn reload_nameplates(self) {
        leptos::task::spawn_local(async move {
            let path = nameplates_path(&self.output_slug.get_untracked());
            match crate::api::get_json::<Vec<Nameplate>>(&path).await {
                Ok(list) => self.nameplates.set(list),
                Err(e) => self.show_toast(&format!("Načítanie menoviek zlyhalo: {e}"), "error"),
            }
        });
    }

    /// Cold-load the plate currently on air (mount) so the highlight is correct.
    pub fn reload_active_nameplate(self) {
        leptos::task::spawn_local(async move {
            let path = nameplates_active_path(&self.output_slug.get_untracked());
            if let Ok(active) = crate::api::get_json::<Option<ActiveNameplate>>(&path).await {
                self.active_nameplate.set(active);
            }
        });
    }

    /// Add a person plate, then refetch the list.
    pub fn add_nameplate(self, primary: String, secondary: String) {
        let primary = primary.trim().to_string();
        if primary.is_empty() {
            self.show_toast("Meno nesmie byť prázdne.", "error");
            return;
        }
        let secondary = secondary.trim().to_string();
        leptos::task::spawn_local(async move {
            let req = NameplateTextReq {
                primary_text: primary,
                secondary_text: secondary,
            };
            let path = nameplates_path(&self.output_slug.get_untracked());
            match crate::api::post_json::<_, Nameplate>(&path, &req).await {
                Ok(_) => {
                    self.reload_nameplates();
                    self.show_toast("Menovka pridaná.", "success");
                }
                Err(e) => self.show_toast(&format!("Pridanie zlyhalo: {e}"), "error"),
            }
        });
    }

    /// Update a plate's texts, then refetch the list.
    pub fn update_nameplate(self, id: i64, primary: String, secondary: String) {
        let primary = primary.trim().to_string();
        if primary.is_empty() {
            self.show_toast("Meno nesmie byť prázdne.", "error");
            return;
        }
        let secondary = secondary.trim().to_string();
        leptos::task::spawn_local(async move {
            let req = NameplateTextReq {
                primary_text: primary,
                secondary_text: secondary,
            };
            match crate::api::patch_json::<_, Nameplate>(&nameplate_path(id), &req).await {
                Ok(_) => self.reload_nameplates(),
                Err(e) => self.show_toast(&format!("Uloženie zlyhalo: {e}"), "error"),
            }
        });
    }

    /// Delete a plate (native confirm), then refetch the list.
    pub fn delete_nameplate(self, id: i64) {
        let confirmed = crate::utils::window::window()
            .confirm_with_message("Zmazať túto menovku?")
            .unwrap_or(false);
        if !confirmed {
            return;
        }
        leptos::task::spawn_local(async move {
            match crate::api::delete(&nameplate_path(id)).await {
                Ok(()) => {
                    self.reload_nameplates();
                    self.show_toast("Menovka zmazaná.", "success");
                }
                Err(e) => self.show_toast(&format!("Zmazanie zlyhalo: {e}"), "error"),
            }
        });
    }

    /// Move a plate one step (reorder wants the FULL id set, like scenes).
    pub fn move_nameplate(self, id: i64, up: bool) {
        let mut ids: Vec<i64> = self
            .nameplates
            .get_untracked()
            .iter()
            .map(|p| p.id)
            .collect();
        let Some(pos) = ids.iter().position(|x| *x == id) else {
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
            let req = super::ReorderReq { ids };
            let path = nameplates_order_path(&self.output_slug.get_untracked());
            match crate::api::put_no_content(&path, &req).await {
                Ok(()) => self.reload_nameplates(),
                Err(e) => self.show_toast(&format!("Zmena poradia zlyhala: {e}"), "error"),
            }
        });
    }

    /// Show a person plate on the REAL output; applies the returned active state.
    pub fn show_nameplate_person(self, id: i64) {
        self.set_active(SetActiveReq {
            source: Some("person".to_string()),
            id: Some(id),
        });
    }

    /// Show the virtual song plate on the REAL output.
    pub fn show_song_nameplate(self) {
        self.set_active(SetActiveReq {
            source: Some("song".to_string()),
            id: None,
        });
    }

    /// Hide whatever plate is on air.
    pub fn hide_nameplate(self) {
        self.set_active(SetActiveReq {
            source: None,
            id: None,
        });
    }

    fn set_active(self, req: SetActiveReq) {
        leptos::task::spawn_local(async move {
            let path = nameplates_active_path(&self.output_slug.get_untracked());
            match crate::api::put_json::<_, Option<ActiveNameplate>>(&path, &req).await {
                Ok(active) => self.active_nameplate.set(active),
                Err(e) => self.show_toast(&format!("Menovku sa nepodarilo prepnúť: {e}"), "error"),
            }
        });
    }

    /// Create a "Menovky" overlay scene with a default `lower_third` element and
    /// activate it — the one-click layer setup when the output has none.
    pub fn create_nameplate_layer(self) {
        leptos::task::spawn_local(async move {
            let slug = self.output_slug.get_untracked();
            let scene_req = super::CreateSceneReq {
                name: "Menovky".to_string(),
                kind: SceneKind::Overlay,
            };
            let scene =
                match crate::api::post_json::<_, StreamSceneDef>(&scenes_path(&slug), &scene_req)
                    .await
                {
                    Ok(scene) => scene,
                    Err(e) => {
                        self.show_toast(&format!("Vytvorenie vrstvy zlyhalo: {e}"), "error");
                        return;
                    }
                };
            let props = default_element_props("lower_third");
            if let Err(e) = crate::api::post_json::<StreamElementProps, StreamElementDef>(
                &super::elements_path(scene.id),
                &props,
            )
            .await
            {
                self.show_toast(&format!("Pridanie prvku zlyhalo: {e}"), "error");
                return;
            }
            let overlay_req = super::SetOverlayReq { active: true };
            let _ = crate::api::put_json::<_, StreamShowState>(
                &overlay_path(&slug, scene.id),
                &overlay_req,
            )
            .await;
            self.refresh();
            self.show_toast("Menovková vrstva vytvorená.", "success");
        });
    }

    /// True when the output already has a `lower_third` element (drives whether
    /// the "Vytvoriť menovkovú vrstvu" button is offered).
    pub fn has_lower_third(self) -> bool {
        self.def
            .get()
            .map(|d| {
                d.scenes
                    .iter()
                    .flat_map(|s| &s.elements)
                    .any(|e| matches!(e.props, StreamElementProps::LowerThird { .. }))
            })
            .unwrap_or(false)
    }
}

/// Build a fake `ActiveNameplate` and push it into the preview iframe (Prehrať).
/// `seq` uses the wall clock so each preview keys a fresh layer (re-animates).
pub fn preview_nameplate(
    source: NameplateSource,
    id: Option<i64>,
    primary: String,
    secondary: String,
) {
    let seq = js_sys::Date::now() as u64;
    let active = ActiveNameplate {
        source,
        nameplate_id: id,
        primary,
        secondary,
        seq,
    };
    let json = crate::components::stream::nameplate_preview::serialize_message(Some(active));
    post_nameplate_preview(json);
}

/// Post a serialized nameplate-preview message into the preview iframe's
/// `contentWindow` (same-origin). No-op on the host / when no preview iframe is
/// mounted (no scene open in the workspace).
#[cfg(target_arch = "wasm32")]
fn post_nameplate_preview(json: String) {
    use leptos::wasm_bindgen::{JsCast, JsValue};

    let document = crate::utils::window::document();
    let Ok(Some(el)) = document.query_selector("[data-role=\"stream-preview-frame\"]") else {
        return;
    };
    let Some(iframe) = el.dyn_ref::<leptos::web_sys::HtmlIFrameElement>() else {
        return;
    };
    let Some(win) = iframe.content_window() else {
        return;
    };
    let origin = leptos::web_sys::window()
        .and_then(|w| w.location().origin().ok())
        .unwrap_or_else(|| "*".to_string());
    let _ = win.post_message(&JsValue::from_str(&json), &origin);
}

/// Host stub (see the wasm version).
#[cfg(not(target_arch = "wasm32"))]
fn post_nameplate_preview(_json: String) {}

/// The Menovky panel — mounted by `pages/stream_editor.rs` after `EditorScenes`.
#[component]
pub fn NameplatePanel(ctx: StreamEditorCtx) -> impl IntoView {
    let new_name = RwSignal::new(String::new());
    let new_role = RwSignal::new(String::new());
    let plate_ids = move || {
        ctx.nameplates
            .get()
            .iter()
            .map(|p| p.id)
            .collect::<Vec<_>>()
    };
    let song_active = move || {
        matches!(
            ctx.active_nameplate.get(),
            Some(ActiveNameplate {
                source: NameplateSource::Song,
                ..
            })
        )
    };
    let song_texts = move || ctx.song_preview.get();

    view! {
        <section class="stream-editor__nameplates" data-role="stream-nameplates">
            <header class="stream-editor__panel-head">
                <h2 class="stream-editor__section-title">"Menovky"</h2>
                <Show when=move || !ctx.has_lower_third()>
                    <button
                        type="button"
                        class="stream-editor__btn stream-editor__btn--primary"
                        data-role="stream-nameplate-create-layer"
                        on:click=move |_| ctx.create_nameplate_layer()
                    >
                        "Vytvoriť menovkovú vrstvu"
                    </button>
                </Show>
            </header>

            // Song (virtual) row.
            <div
                class="stream-editor__nameplate stream-editor__nameplate--song"
                data-role="stream-nameplate-song"
                data-active=move || bool_attr(song_active())
            >
                <div class="stream-editor__nameplate-texts">
                    <strong>"Pieseň (aktuálna)"</strong>
                    <span class="stream-editor__nameplate-song-live" data-role="stream-nameplate-song-live">
                        {move || {
                            let (name, lib) = song_texts();
                            if name.is_empty() { "—".to_string() } else { format!("{name} · {lib}") }
                        }}
                    </span>
                </div>
                <div class="stream-editor__nameplate-actions">
                    <button type="button" class="stream-editor__btn" data-role="stream-nameplate-song-show"
                        on:click=move |_| ctx.show_song_nameplate()>"Zobraziť"</button>
                    <button type="button" class="stream-editor__btn stream-editor__btn--ghost" data-role="stream-nameplate-song-preview"
                        on:click=move |_| {
                            let (name, lib) = ctx.song_preview.get_untracked();
                            let name = if name.is_empty() { "Názov piesne".to_string() } else { name };
                            preview_nameplate(NameplateSource::Song, None, name, lib);
                        }>"Prehrať"</button>
                    <button type="button" class="stream-editor__btn stream-editor__btn--ghost" data-role="stream-nameplate-hide"
                        on:click=move |_| ctx.hide_nameplate()>"Skryť"</button>
                </div>
            </div>

            // Person plates.
            <ul class="stream-editor__nameplate-list" data-role="stream-nameplate-list">
                <For
                    each=plate_ids
                    key=|id| *id
                    children=move |id| view! { <NameplateRow ctx=ctx id=id /> }
                />
            </ul>

            // Add form.
            <form
                class="stream-editor__nameplate-add"
                data-role="stream-nameplate-add"
                on:submit=move |ev| {
                    ev.prevent_default();
                    ctx.add_nameplate(new_name.get_untracked(), new_role.get_untracked());
                    new_name.set(String::new());
                    new_role.set(String::new());
                }
            >
                <input type="text" placeholder="Meno" data-role="stream-nameplate-new-name"
                    prop:value=move || new_name.get()
                    on:input=move |ev| new_name.set(event_target_value(&ev)) />
                <input type="text" placeholder="Pozícia (napr. pastor)" data-role="stream-nameplate-new-role"
                    prop:value=move || new_role.get()
                    on:input=move |ev| new_role.set(event_target_value(&ev)) />
                <button type="submit" class="stream-editor__btn stream-editor__btn--primary" data-role="stream-nameplate-add-submit">
                    "Pridať menovku"
                </button>
            </form>
        </section>
    }
}

/// One person-plate row: name + role inputs (commit on blur), on-air highlight,
/// and Zobraziť / Prehrať / Skryť / reorder / delete.
#[component]
fn NameplateRow(ctx: StreamEditorCtx, id: i64) -> impl IntoView {
    let plate = move || ctx.nameplates.get().into_iter().find(|p| p.id == id);
    let name = move || plate().map(|p| p.primary_text).unwrap_or_default();
    let role = move || plate().map(|p| p.secondary_text).unwrap_or_default();
    let is_active = move || {
        matches!(
            ctx.active_nameplate.get(),
            Some(ActiveNameplate { source: NameplateSource::Person, nameplate_id: Some(a), .. }) if a == id
        )
    };
    view! {
        <li
            class="stream-editor__nameplate"
            data-role="stream-nameplate"
            data-nameplate-id=id.to_string()
            data-active=move || bool_attr(is_active())
        >
            <div class="stream-editor__nameplate-texts">
                <input type="text" data-role="stream-nameplate-name" prop:value=name
                    on:change=move |ev| ctx.update_nameplate(id, event_target_value(&ev), role()) />
                <input type="text" data-role="stream-nameplate-role" prop:value=role
                    on:change=move |ev| ctx.update_nameplate(id, name(), event_target_value(&ev)) />
            </div>
            <div class="stream-editor__nameplate-actions">
                <button type="button" class="stream-editor__btn" data-role="stream-nameplate-show"
                    on:click=move |_| ctx.show_nameplate_person(id)>"Zobraziť"</button>
                <button type="button" class="stream-editor__btn stream-editor__btn--ghost" data-role="stream-nameplate-preview"
                    on:click=move |_| preview_nameplate(NameplateSource::Person, Some(id), name(), role())>"Prehrať"</button>
                <button type="button" class="stream-editor__btn stream-editor__btn--ghost" data-role="stream-nameplate-hide"
                    on:click=move |_| ctx.hide_nameplate()>"Skryť"</button>
                <button type="button" class="stream-editor__btn stream-editor__btn--ghost" data-role="stream-nameplate-up"
                    title="Vyššie" on:click=move |_| ctx.move_nameplate(id, true)>"↑"</button>
                <button type="button" class="stream-editor__btn stream-editor__btn--ghost" data-role="stream-nameplate-down"
                    title="Nižšie" on:click=move |_| ctx.move_nameplate(id, false)>"↓"</button>
                <button type="button" class="stream-editor__btn stream-editor__btn--danger" data-role="stream-nameplate-delete"
                    on:click=move |_| ctx.delete_nameplate(id)>"Zmazať"</button>
            </div>
        </li>
    }
}
