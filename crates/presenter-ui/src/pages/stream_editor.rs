//! Stream-graphics operator editor page (`/ui/stream`, epic #718, PR-6 #713).
//!
//! Own standalone page (NOT an operator.rs tab — that file is already in the
//! 777-line warn band). Fetches the default output's def, subscribes to the
//! generic `/live/ws` feed, and reflects live show-state so a second operator's
//! activation / config change updates this view:
//!   * `StreamState`         → applied directly to `active` (activation does not
//!                             bump `config_revision`).
//!   * `StreamConfigChanged` → refetch the def when its revision advances.
//! The scene UI + all write actions live in `components/stream_editor`. The
//! areas sit in Scény / Menovky / Písma tabs (#829, `editor_tabs.rs`).

use leptos::prelude::*;
use presenter_core::{LiveEvent, StreamShowState};

use crate::components::nameplates::NameplateStore;
use crate::components::stream_editor::editor_fonts::FontPanel;
use crate::components::stream_editor::editor_nameplates::NameplatePanel;
use crate::components::stream_editor::editor_panel::EditorPanel;
use crate::components::stream_editor::editor_preview::EditorPreview;
use crate::components::stream_editor::editor_scenes::EditorScenes;
use crate::components::stream_editor::editor_tabs::{
    initial_tab, mirror_tab_to_url, EditorTab, EditorTabs, TabPanel,
};
use crate::components::stream_editor::output_manage::OutputManage;
use crate::components::stream_editor::output_paths::{initial_output_slug, OutputSelect};
use crate::components::stream_editor::{show_editor_toast, StreamEditorCtx};
use crate::components::version_label::VersionLabel;

/// The `/ui/stream` operator editor page.
#[component]
pub fn StreamEditorPage() -> impl IntoView {
    // Own the <body> for full-page dark styling; restore on unmount so an
    // in-app navigation can't leave a stale class behind. Also mark the <html>
    // element: tablet.css ships a bare global `html { height:100%; overflow:hidden }`
    // (its iOS-bounce guard) that trunk bundles into every page, which clips the
    // editor's tall property panel and stops the page scrolling (#738). The
    // `data-stream-editor` attribute lets stream_editor.css restore normal
    // document scrolling for this page only — mirrors the output page's
    // `data-stream` html override.
    if let Some(body) = crate::utils::window::document_body() {
        let _ = body.set_attribute("class", "stream-editor-page");
    }
    if let Some(html) = crate::utils::window::document().document_element() {
        let _ = html.set_attribute("data-stream-editor", "true");
    }
    on_cleanup(|| {
        if let Some(body) = crate::utils::window::document_body() {
            let _ = body.set_attribute("class", "");
        }
        if let Some(html) = crate::utils::window::document().document_element() {
            let _ = html.remove_attribute("data-stream-editor");
        }
    });

    // #785: open the output from `?output=` / localStorage / the default.
    let output_slug = RwSignal::new(initial_output_slug());
    let toast_msg = RwSignal::new(String::new());
    let toast_visible = RwSignal::new(false);
    let toast_state = RwSignal::new(String::from("info"));
    // #871: the plate list is the shared store (also the operator's Menovky
    // view); it reports through this page's toast.
    let plates = NameplateStore::new(
        output_slug,
        Callback::new(move |(msg, variant): (String, &'static str)| {
            show_editor_toast(toast_msg, toast_visible, toast_state, &msg, variant);
        }),
    );
    let ctx = StreamEditorCtx {
        output_slug,
        outputs: RwSignal::new(Vec::new()),
        // #829: reopen the tab from `?tab=` / localStorage (default Scény).
        tab: RwSignal::new(initial_tab()),
        def: RwSignal::new(None),
        active: RwSignal::new(crate::components::stream_editor::empty_show_state()),
        toast_msg,
        toast_visible,
        toast_state,
        selected_scene: RwSignal::new(None),
        selected_element: RwSignal::new(None),
        prop_error: RwSignal::new(None),
        draft: RwSignal::new(
            crate::components::stream_editor::props_access::default_element_props("image"),
        ),
        draft_element_id: RwSignal::new(None),
        fonts: RwSignal::new(Vec::new()),
        plates,
        selection: StoredValue::new(
            crate::components::stream_editor::selection_intent::SelectionIntent::default(),
        ),
    };

    // #829: the URL names the tab on screen from the start (a tab restored from
    // localStorage too), so a reload / bookmark / output switch carries it.
    mirror_tab_to_url(ctx.tab.get_untracked());

    // Cold load.
    ctx.refresh();
    // #785: load the output list for the header switcher.
    ctx.reload_outputs();
    // #778: load uploaded fonts (the picker's extra families) and inject the
    // generated @font-face stylesheet so picker previews render in-face.
    ctx.reload_fonts();
    crate::components::stream::fonts::ensure_fonts_css_link(0);
    // #779: load the plate list + the plate currently on air + the live song.
    ctx.plates.load();

    // Live reflection: apply activation events directly, refetch on config bump.
    let (_ws_state, last_event) = crate::ws::use_live_websocket("stream");
    Effect::new(move |_| {
        let Some(event) = last_event.get() else {
            return;
        };
        match event {
            LiveEvent::StreamState {
                output,
                active_scene_id,
                active_overlay_ids,
                config_revision,
            } if output == ctx.output_slug.get_untracked() => {
                ctx.active.set(StreamShowState {
                    active_scene_id,
                    active_overlay_ids,
                    config_revision,
                });
            }
            LiveEvent::StreamConfigChanged {
                output,
                config_revision,
            } => {
                // #842: outputs are created and renamed from any editor, so
                // every config change refreshes the output list (a small GET).
                ctx.reload_outputs();
                let current = ctx
                    .def
                    .get_untracked()
                    .map(|d| d.config_revision)
                    .unwrap_or(0);
                if output == ctx.output_slug.get_untracked() && config_revision > current {
                    ctx.refresh();
                }
            }
            // #779: a plate on/off air (highlight), the plate list changed
            // (refetch), the live song (the "Pieseň" row): the shared store.
            other => ctx.plates.apply_event(&other),
        }
    });

    view! {
        <div class="stream-editor" data-role="stream-editor-page">
            <header class="stream-editor__header">
                <div class="stream-editor__header-title">
                    <h1>"Stream Graphics"</h1>
                    <p>"Base scény ako stĺpce, overlay scény hore. Klik na scénu ju aktivuje."</p>
                </div>
                <nav class="stream-editor__header-nav">
                    <OutputSelect ctx=ctx />
                    <a href="/ui/operator" class="stream-editor__link">"← Operator"</a>
                    <span class="stream-editor__version"><VersionLabel /></span>
                </nav>
            </header>
            // #842: the selected output's OBS URL + create / rename outputs.
            <OutputManage ctx=ctx />
            // #829: one area at a time. Every panel stays mounted (hidden, not
            // unmounted), so unsaved work survives a tab switch.
            <EditorTabs ctx=ctx />
            <main class="stream-editor__main">
                <TabPanel ctx=ctx tab=EditorTab::Scenes>
                    <EditorScenes ctx=ctx />
                    <Show when=move || ctx.selected_scene.get().is_some()>
                        <section class="stream-editor__workspace" data-role="stream-workspace">
                            <EditorPanel ctx=ctx />
                            <EditorPreview ctx=ctx />
                        </section>
                    </Show>
                </TabPanel>
                <TabPanel ctx=ctx tab=EditorTab::Nameplates>
                    <NameplatePanel ctx=ctx />
                </TabPanel>
                <TabPanel ctx=ctx tab=EditorTab::Fonts>
                    <FontPanel ctx=ctx />
                </TabPanel>
            </main>
            <div
                class="stream-editor__toast"
                data-role="toast"
                data-visible=move || if ctx.toast_visible.get() { "true" } else { "false" }
                data-state=move || ctx.toast_state.get()
            >
                {move || ctx.toast_msg.get()}
            </div>
        </div>
    }
}
