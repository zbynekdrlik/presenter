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

use crate::components::stream_editor::editor_panel::EditorPanel;
use crate::components::stream_editor::editor_scenes::EditorScenes;
use crate::components::stream_editor::{StreamEditorCtx, DEFAULT_OUTPUT_SLUG};
use crate::components::version_label::VersionLabel;

/// The `/ui/stream` operator editor page.
#[component]
pub fn StreamEditorPage() -> impl IntoView {
    // Own the <body> for full-page dark styling; restore on unmount so an
    // in-app navigation can't leave a stale class behind.
    if let Some(body) = crate::utils::window::document_body() {
        let _ = body.set_attribute("class", "stream-editor-page");
    }
    on_cleanup(|| {
        if let Some(body) = crate::utils::window::document_body() {
            let _ = body.set_attribute("class", "");
        }
    });

    let ctx = StreamEditorCtx {
        def: RwSignal::new(None),
        active: RwSignal::new(StreamShowState {
            active_scene_id: None,
            active_overlay_ids: Vec::new(),
            config_revision: 0,
        }),
        toast_msg: RwSignal::new(String::new()),
        toast_visible: RwSignal::new(false),
        toast_state: RwSignal::new(String::from("info")),
        selected_scene: RwSignal::new(None),
        selected_element: RwSignal::new(None),
        prop_error: RwSignal::new(String::new()),
    };

    // Cold load.
    ctx.refresh();

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
            } if output == DEFAULT_OUTPUT_SLUG => {
                ctx.active.set(StreamShowState {
                    active_scene_id,
                    active_overlay_ids,
                    config_revision,
                });
            }
            LiveEvent::StreamConfigChanged {
                output,
                config_revision,
            } if output == DEFAULT_OUTPUT_SLUG => {
                let current = ctx
                    .def
                    .get_untracked()
                    .map(|d| d.config_revision)
                    .unwrap_or(0);
                if config_revision > current {
                    ctx.refresh();
                }
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
                    <a href="/ui/operator" class="stream-editor__link">"← Operator"</a>
                    <span class="stream-editor__version"><VersionLabel /></span>
                </nav>
            </header>
            <main class="stream-editor__main">
                <EditorScenes ctx=ctx />
                <Show when=move || ctx.selected_scene.get().is_some()>
                    <section class="stream-editor__workspace" data-role="stream-workspace">
                        <EditorPanel ctx=ctx />
                    </section>
                </Show>
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
