//! "Menovky" (lower-third nameplate) editor panel (#779).
//!
//! An output-scoped panel (its own „Menovky" tab since #829, always mounted).
//! The plate LIST itself (the virtual "Pieseň" row, the person rows with inline
//! edit / delete / reorder, Zobraziť / Skryť driving the REAL output, the on-air
//! highlight, the add form) is the shared `components/nameplates` list (#871),
//! also used by the operator's Menovky view. This panel adds the editor-only
//! extras: a Prehrať button that previews the plate animation in the tab's own
//! preview iframe WITHOUT broadcasting (#777 channel sibling), and a one-click
//! "Vytvoriť menovkovú vrstvu" that creates a default-styled `lower_third`
//! overlay scene when the output has none.

use leptos::prelude::*;
use presenter_core::{
    ActiveNameplate, NameplateSource, SceneKind, StreamElementDef, StreamElementProps,
    StreamOutputDef, StreamSceneDef, StreamShowState,
};

use super::editor_tabs::EditorTab;
use super::output_paths::{overlay_path, scenes_path};
use super::props_access::default_element_props;
use super::StreamEditorCtx;
use crate::components::nameplates::plate_list::{
    NameplateAddForm, NameplateRows, NameplateSongRow, RowOptions,
};

impl StreamEditorCtx {
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
        self.def.with(|d| {
            d.as_ref()
                .is_some_and(crate::components::nameplates::def_has_lower_third)
        })
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

/// Take the previewed plate off the Menovky preview („Skryť"). Preview mode
/// ignores the live plate feed and server auto-hide, so only the editor can
/// clear a plate it played there (#829 second review).
pub fn clear_nameplate_preview() {
    post_nameplate_preview(crate::components::stream::nameplate_preview::serialize_message(None));
}

/// Post a serialized nameplate-preview message into the Menovky tab's own
/// preview iframe (same-origin, #829) — the frame the operator sees next to the
/// „Prehrať" buttons. Before #829 it targeted the Scény workspace preview, which
/// the tabs hide while Menovky is open. No-op on the host / when no Menovky
/// preview is mounted (the output has no lower third yet). A click while that
/// frame is still booting is dropped: Prehrať is a momentary action, so it is
/// deliberately NOT replayed when the frame (re)loads.
#[cfg(target_arch = "wasm32")]
fn post_nameplate_preview(json: String) {
    use leptos::wasm_bindgen::JsCast;

    let document = crate::utils::window::document();
    let Ok(Some(el)) = document.query_selector("[data-role=\"stream-nameplate-preview-frame\"]")
    else {
        return;
    };
    if let Some(iframe) = el.dyn_ref::<leptos::web_sys::HtmlIFrameElement>() {
        super::editor_preview::post_to_frame(iframe, &json);
    }
}

/// Host stub (see the wasm version).
#[cfg(not(target_arch = "wasm32"))]
fn post_nameplate_preview(_json: String) {}

/// The Menovky preview URL (#829): the output page in preview mode with every
/// scene that holds a lower third forced on screen — overlay scenes through
/// `overlays=`, the first such base scene through `scene=` — so „Prehrať"
/// animates in a frame the operator sees, whichever scene is open elsewhere.
pub(super) fn nameplate_preview_src(slug: &str, def: Option<&StreamOutputDef>) -> String {
    let mut src = super::editor_preview::preview_base(slug);
    let Some(def) = def else {
        return src;
    };
    let has_plate = |s: &&StreamSceneDef| {
        s.elements
            .iter()
            .any(|e| matches!(e.props, StreamElementProps::LowerThird { .. }))
    };
    if let Some(base) = def
        .scenes
        .iter()
        .filter(has_plate)
        .find(|s| s.kind == SceneKind::Base)
    {
        src.push_str(&format!("&scene={}", base.id));
    }
    let overlays: Vec<String> = def
        .scenes
        .iter()
        .filter(has_plate)
        .filter(|s| s.kind == SceneKind::Overlay)
        .map(|s| s.id.to_string())
        .collect();
    if !overlays.is_empty() {
        src.push_str(&format!("&overlays={}", overlays.join(",")));
    }
    src
}

/// The Menovky tab's own 16:9 preview (#829): the real output page with the
/// lower-third scene(s) forced on, where „Prehrať" plays. The panel mounts it
/// only while the Menovky tab is shown and the output has a lower third — an
/// output page is a full live client, so no hidden copy keeps running.
#[component]
fn NameplatePreviewFrame(ctx: StreamEditorCtx) -> impl IntoView {
    // A Memo: a def refetch that keeps the same lower-third scenes must not
    // reload the frame (which would drop a plate mid-animation).
    let src = Memo::new(move |_| {
        let slug = ctx.output_slug.get();
        ctx.def.with(|d| nameplate_preview_src(&slug, d.as_ref()))
    });
    view! {
        <div
            class="stream-editor__preview-box stream-editor__nameplate-preview"
            data-role="stream-nameplate-preview-box"
        >
            <iframe
                class="stream-editor__preview-frame"
                data-role="stream-nameplate-preview-frame"
                src=move || src.get()
                title="Náhľad menovky"
            ></iframe>
        </div>
    }
}

/// The editor's extras on the shared rows: „Prehrať" in the tab's own
/// preview, and „Skryť" also clears that preview (#829).
const EDITOR_ROWS: RowOptions = RowOptions {
    preview: Some(preview_nameplate),
    after_hide: Some(clear_nameplate_preview),
    numbered: false,
};

/// The Menovky panel — mounted by `pages/stream_editor.rs` in the Menovky tab.
#[component]
pub fn NameplatePanel(ctx: StreamEditorCtx) -> impl IntoView {
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

            // #829: „Prehrať" plays here, in the tab the operator is looking at.
            <Show when=move || ctx.tab.get() == EditorTab::Nameplates && ctx.has_lower_third()>
                <NameplatePreviewFrame ctx=ctx />
            </Show>

            // #871: the shared plate list (song row, person rows, add form).
            <NameplateSongRow store=ctx.plates options=EDITOR_ROWS />
            <NameplateRows store=ctx.plates options=EDITOR_ROWS />
            <NameplateAddForm store=ctx.plates />
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(id: i64, kind: SceneKind, kinds: &[&str]) -> StreamSceneDef {
        StreamSceneDef {
            id,
            name: format!("scene-{id}"),
            kind,
            position: 0,
            is_active: false,
            transition_ms: None,
            elements: kinds
                .iter()
                .enumerate()
                .map(|(i, k)| StreamElementDef {
                    id: id * 10 + i as i64,
                    z_order: i as i32,
                    props: default_element_props(k),
                })
                .collect(),
        }
    }

    fn def(scenes: Vec<StreamSceneDef>) -> StreamOutputDef {
        StreamOutputDef {
            id: 1,
            slug: "stream".to_string(),
            name: "Stream".to_string(),
            default_transition_ms: 300,
            base_transition_ms: None,
            overlay_transition_ms: None,
            active_scene_id: None,
            config_revision: 1,
            scenes,
        }
    }

    #[test]
    fn overlay_lower_thirds_are_forced_on() {
        let d = def(vec![
            scene(1, SceneKind::Base, &["color"]),
            scene(2, SceneKind::Overlay, &["lower_third"]),
            scene(3, SceneKind::Overlay, &["image"]),
            scene(4, SceneKind::Overlay, &["color", "lower_third"]),
        ]);
        assert_eq!(
            nameplate_preview_src("stream", Some(&d)),
            "/stream/stream?preview=1&overlays=2,4"
        );
    }

    #[test]
    fn a_base_lower_third_is_forced_as_the_scene() {
        let d = def(vec![
            scene(5, SceneKind::Base, &["verse"]),
            scene(6, SceneKind::Base, &["lower_third"]),
            scene(7, SceneKind::Base, &["lower_third"]),
        ]);
        assert_eq!(
            nameplate_preview_src("timer", Some(&d)),
            "/stream/timer?preview=1&scene=6"
        );
    }

    #[test]
    fn without_a_lower_third_or_a_def_it_is_the_plain_preview() {
        let d = def(vec![scene(1, SceneKind::Base, &["color"])]);
        assert_eq!(
            nameplate_preview_src("stream", Some(&d)),
            "/stream/stream?preview=1"
        );
        assert_eq!(
            nameplate_preview_src("stream", None),
            "/stream/stream?preview=1"
        );
    }
}
