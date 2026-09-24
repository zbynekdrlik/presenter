//! Stream-graphics operator editor page (`/ui/stream`, epic #718, PR-6 #713).
//!
//! Own standalone page (NOT an operator.rs tab — that file is already in the
//! 777-line warn band). Fetches the default output's def, subscribes to the
//! generic `/live/ws` feed, and reflects live show-state so a second operator's
//! activation / config change updates this view:
//!   * `StreamState`         → applied directly to `active` (activation does not
//!                             bump `config_revision`).
//!   * `StreamConfigChanged` → refetch the def when its revision advances.
//! The scene UI + all write actions live in `components/stream_editor`.

use leptos::prelude::*;
use presenter_core::{LiveEvent, StreamShowState};

use crate::components::stream_editor::editor_fonts::FontPanel;
use crate::components::stream_editor::editor_nameplates::NameplatePanel;
use crate::components::stream_editor::editor_panel::EditorPanel;
use crate::components::stream_editor::editor_preview::EditorPreview;
use crate::components::stream_editor::editor_scenes::EditorScenes;
use crate::components::stream_editor::output_paths::{initial_output_slug, OutputSelect};
use crate::components::stream_editor::StreamEditorCtx;
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

    let ctx = StreamEditorCtx {
        // #785: open the output from `?output=` / localStorage / the default.
        output_slug: RwSignal::new(initial_output_slug()),
        outputs: RwSignal::new(Vec::new()),
        def: RwSignal::new(None),
        active: RwSignal::new(crate::components::stream_editor::empty_show_state()),
        toast_msg: RwSignal::new(String::new()),
        toast_visible: RwSignal::new(false),
        toast_state: RwSignal::new(String::from("info")),
        selected_scene: RwSignal::new(None),
        selected_element: RwSignal::new(None),
        prop_error: RwSignal::new(String::new()),
        draft: RwSignal::new(
            crate::components::stream_editor::props_access::default_element_props("image"),
        ),
        draft_element_id: RwSignal::new(None),
        fonts: RwSignal::new(Vec::new()),
        nameplates: RwSignal::new(Vec::new()),
        active_nameplate: RwSignal::new(None),
        song_preview: RwSignal::new((String::new(), String::new())),
    };

    // Cold load.
    ctx.refresh();
    // #785: load the output list for the header switcher.
    ctx.reload_outputs();
    // #778: load uploaded fonts (the picker's extra families) and inject the
    // generated @font-face stylesheet so picker previews render in-face.
    ctx.reload_fonts();
    crate::components::stream::fonts::ensure_fonts_css_link(0);
    // #779: load the plate list + the plate currently on air + the live song.
    ctx.reload_nameplates();
    ctx.reload_active_nameplate();
    load_song_preview(ctx);

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
            } if output == ctx.output_slug.get_untracked() => {
                let current = ctx
                    .def
                    .get_untracked()
                    .map(|d| d.config_revision)
                    .unwrap_or(0);
                if config_revision > current {
                    ctx.refresh();
                }
            }
            // #779: a plate went on/off air → update the on-air highlight directly.
            LiveEvent::StreamNameplate { output, active }
                if output == ctx.output_slug.get_untracked() =>
            {
                ctx.active_nameplate.set(active);
            }
            // #779: the plate list changed → refetch it.
            LiveEvent::StreamNameplatesChanged { output }
                if output == ctx.output_slug.get_untracked() =>
            {
                ctx.reload_nameplates();
            }
            // #779: keep the "Pieseň" row's live text current.
            LiveEvent::Stage { snapshot } => {
                ctx.song_preview.set((
                    snapshot.song_name.clone().unwrap_or_default(),
                    snapshot.library_name.clone().unwrap_or_default(),
                ));
            }
            _ => {}
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
            <main class="stream-editor__main">
                <EditorScenes ctx=ctx />
                <Show when=move || ctx.selected_scene.get().is_some()>
                    <section class="stream-editor__workspace" data-role="stream-workspace">
                        <EditorPanel ctx=ctx />
                        <EditorPreview ctx=ctx />
                    </section>
                </Show>
                <NameplatePanel ctx=ctx />
                <FontPanel ctx=ctx />
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

/// Cold-load the live song title + library for the "Pieseň" row (#779). The
/// snapshot's `song_name`/`library_name` are layout-independent, so the selected
/// snapshot reflects the live worship song the server resolves the plate from.
fn load_song_preview(ctx: StreamEditorCtx) {
    leptos::task::spawn_local(async move {
        if let Ok(snapshot) = crate::api::stage::get_snapshot().await {
            ctx.song_preview.set((
                snapshot.song_name.clone().unwrap_or_default(),
                snapshot.library_name.clone().unwrap_or_default(),
            ));
        }
    });
}
