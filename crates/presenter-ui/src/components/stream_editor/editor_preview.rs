//! Live preview iframe for the selected scene (#715). Embeds the REAL output
//! page (`/stream/{slug}?preview=1&scene=<id>`, #709's contract) in a 16:9
//! checkerboard box so the operator sees exactly what OBS will render, WITH the
//! transparency visible. `preview=1` excludes the connection from the stage/
//! output counts (handled page-side, #709). A "live" toggle drops the forced
//! `scene` param to watch the real (un-forced) output instead.
//!
//! The output page is a PARALLEL lane (not on this tree): this component only
//! builds the URL. The "iframe actually renders the forced scene" behavior is
//! exercised by the E2E in integrated CI, not in this worktree.

use leptos::prelude::*;

use super::{StreamEditorCtx, DEFAULT_OUTPUT_SLUG};

/// The 16:9 preview of the selected scene (or the live output when toggled).
#[component]
pub fn EditorPreview(ctx: StreamEditorCtx) -> impl IntoView {
    // false = forced selected scene; true = live (un-forced) output.
    let live = RwSignal::new(false);

    let src = move || {
        let base = format!("/stream/{DEFAULT_OUTPUT_SLUG}?preview=1");
        if live.get() {
            base
        } else {
            match ctx.selected_scene.get() {
                Some(id) => format!("{base}&scene={id}"),
                None => base,
            }
        }
    };

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
                    src=src
                    title="Náhľad stream scény"
                ></iframe>
            </div>
        </section>
    }
}
