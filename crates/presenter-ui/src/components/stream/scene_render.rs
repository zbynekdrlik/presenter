//! Renders one stream scene's elements (#709; lyrics + verse #710).
//!
//! Elements arrive already ordered by `z_order` (the repository's def assembly,
//! #705). Each is mapped to its per-kind component (IMAGE / COUNTDOWN / LYRICS /
//! VERSE). A scene's element set only changes on a def refetch, which remounts
//! this whole subtree, so a plain `collect_view()` (no keyed `<For>`) is correct
//! — there is no live in-place mutation and no scroll container to preserve.
//! Live CONTENT changes (a new lyric line / verse) update reactively inside the
//! lyrics/verse elements themselves, not by rebuilding this subtree.

use leptos::prelude::*;
use presenter_core::{SceneKind, StreamElementProps, StreamSceneDef};

use super::element_countdown::ElementCountdown;
use super::element_image::ElementImage;
use super::element_lyrics::ElementLyrics;
use super::element_verse::ElementVerse;

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
                StreamElementProps::Lyrics {
                    show_main,
                    show_translation,
                    main_style,
                    translation_style,
                    frame,
                    ..
                } => view! {
                    <ElementLyrics
                        id=id
                        show_main=show_main
                        show_translation=show_translation
                        main_style=main_style
                        translation_style=translation_style
                        frame=frame
                        z=z
                    />
                }
                .into_any(),
                StreamElementProps::Verse {
                    show_secondary,
                    text_style,
                    secondary_style,
                    reference_style,
                    frame,
                    ..
                } => view! {
                    <ElementVerse
                        id=id
                        show_secondary=show_secondary
                        text_style=text_style
                        secondary_style=secondary_style
                        reference_style=reference_style
                        frame=frame
                        z=z
                    />
                }
                .into_any(),
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
