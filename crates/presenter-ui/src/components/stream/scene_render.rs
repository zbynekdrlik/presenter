//! Renders one stream scene's elements (#709).
//!
//! Elements arrive already ordered by `z_order` (the repository's def assembly,
//! #705). Each is mapped to its per-kind component; this lane ships IMAGE +
//! COUNTDOWN and skips LYRICS/VERSE (#710) by rendering nothing for them. A
//! scene's element set only changes on a def refetch, which remounts this whole
//! subtree, so a plain `collect_view()` (no keyed `<For>`) is correct — there is
//! no live in-place mutation and no scroll container to preserve.

use leptos::prelude::*;
use presenter_core::{SceneKind, StreamElementProps, StreamSceneDef};

use super::element_countdown::ElementCountdown;
use super::element_image::ElementImage;

#[component]
pub fn SceneRender(scene: StreamSceneDef) -> impl IntoView {
    let scene_id = scene.id;
    let kind: SceneKind = scene.kind;
    let elements = scene.elements;

    let rendered = elements
        .into_iter()
        .map(|el| {
            let id = el.id;
            let z = el.z_order;
            match el.props {
                StreamElementProps::Image {
                    asset_id,
                    fit,
                    frame,
                    opacity,
                } => view! {
                    <ElementImage id=id asset_id=asset_id fit=fit frame=frame opacity=opacity z=z />
                }
                .into_any(),
                StreamElementProps::Countdown {
                    timer_id,
                    style,
                    frame,
                    ..
                } => view! {
                    <ElementCountdown id=id timer_id=timer_id style=style frame=frame z=z />
                }
                .into_any(),
                // #710 — lyrics + verse elements. Rendered as nothing here so a
                // mixed scene stays transparent for them.
                StreamElementProps::Lyrics { .. } | StreamElementProps::Verse { .. } => {
                    ().into_any()
                }
            }
        })
        .collect_view();

    view! {
        <div
            class="stream-scene"
            data-role="stream-scene"
            data-scene-id=scene_id.to_string()
            data-scene-kind=kind.as_str()
        >
            {rendered}
        </div>
    }
}
