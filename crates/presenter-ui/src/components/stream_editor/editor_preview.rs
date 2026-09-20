//! Live preview iframe + interaction overlay for the selected scene (#715, #777).
//!
//! Embeds the REAL output page (`/stream/{slug}?preview=1&scene=<id>`, #709) in a
//! 16:9 checkerboard box, so the operator sees exactly what OBS renders WITH the
//! transparency visible. #777 makes the preview LIVE: on every draft change the
//! editor `postMessage`s the unsaved element props into the iframe (same origin),
//! and the output page (preview-only) renders that override — so the preview
//! equals the output by construction, no save round trip. A [`CanvasOverlay`]
//! sits on top for drag/resize; the iframe is `pointer-events:none` while a scene
//! is being edited. A "live" toggle drops the forced `scene` param AND the
//! overlay to watch the real (un-forced) output.

use leptos::prelude::*;

use super::canvas_overlay::CanvasOverlay;
use super::StreamEditorCtx;
use crate::components::stream::draft_preview::serialize_message;

/// The 16:9 preview of the selected scene (or the live output when toggled).
#[component]
pub fn EditorPreview(ctx: StreamEditorCtx) -> impl IntoView {
    // false = forced selected scene (overlay editing); true = live output.
    let live = RwSignal::new(false);
    let iframe_ref = NodeRef::<leptos::html::Iframe>::new();

    let src = move || {
        let base = format!("/stream/{}?preview=1", ctx.output_slug.get());
        if live.get() {
            base
        } else {
            match ctx.selected_scene.get() {
                Some(id) => format!("{base}&scene={id}"),
                None => base,
            }
        }
    };

    // The overlay is active while editing a scene (not in live mode); the iframe
    // then ignores pointer events so the overlay receives them.
    let overlay_active = move || !live.get() && ctx.selected_scene.get().is_some();
    let iframe_style = move || {
        if overlay_active() {
            "pointer-events:none;"
        } else {
            ""
        }
    };

    // Mirror the shared draft into the output iframe on every change (live
    // preview). In "Naživo" mode the iframe must show the REAL saved output, so
    // the override is CLEARED (never leak an unsaved draft onto the live view).
    // `None` element ⇒ a clear message.
    Effect::new(move |_| {
        let id = if live.get() {
            None
        } else {
            ctx.draft_element_id.get()
        };
        let props = id.and(Some(ctx.draft.get()));
        push_draft(iframe_ref, serialize_message(id, props));
    });

    view! {
        <section class="stream-editor__preview" data-role="stream-preview">
            <header class="stream-editor__preview-head">
                <h2 class="stream-editor__section-title">"Náhľad"</h2>
                <button
                    type="button"
                    class="stream-editor__btn stream-editor__btn--ghost"
                    data-role="stream-preview-live-toggle"
                    data-live=move || if live.get() { "true" } else { "false" }
                    on:click=move |_| live.update(|v| *v = !*v)
                >
                    {move || if live.get() { "Naživo" } else { "Vybraná scéna" }}
                </button>
            </header>
            <div class="stream-editor__preview-box">
                <iframe
                    class="stream-editor__preview-frame"
                    data-role="stream-preview-frame"
                    node_ref=iframe_ref
                    src=src
                    style=iframe_style
                    title="Náhľad stream scény"
                    on:load=move |_| {
                        // Re-push once the iframe (re)loads — its listener is fresh.
                        // In live mode, push a clear (no draft override on the live view).
                        let id = if live.get_untracked() {
                            None
                        } else {
                            ctx.draft_element_id.get_untracked()
                        };
                        let props = id.and(Some(ctx.draft.get_untracked()));
                        push_draft(iframe_ref, serialize_message(id, props));
                    }
                ></iframe>
                <Show when=overlay_active>
                    <CanvasOverlay ctx=ctx />
                </Show>
            </div>
        </section>
    }
}

/// Post a serialized draft message into the preview iframe's `contentWindow`
/// (same-origin). No-op on the host (no browser).
#[cfg(target_arch = "wasm32")]
fn push_draft(iframe_ref: NodeRef<leptos::html::Iframe>, json: String) {
    use leptos::wasm_bindgen::JsValue;

    let Some(iframe) = iframe_ref.get_untracked() else {
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
fn push_draft(_iframe_ref: NodeRef<leptos::html::Iframe>, _json: String) {}
