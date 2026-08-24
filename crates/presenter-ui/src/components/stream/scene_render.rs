//! Renders one stream scene's elements (#709; lyrics + verse #710; transitions
//! #716).
//!
//! Elements arrive already ordered by `z_order` (the repository's def assembly,
//! #705). Each is mapped to its per-kind component (IMAGE / COUNTDOWN / LYRICS /
//! VERSE), and the element's `ContentTransition` (#716) is threaded to the text
//! kinds so a content change fades or cuts (see `transition::CrossfadeText`). A
//! scene's element set only changes on a def refetch, which remounts this whole
//! subtree, so a plain `collect_view()` (no keyed `<For>`) is correct — there is
//! no live in-place mutation and no scroll container to preserve. Live CONTENT
//! changes (a new lyric line / verse) update reactively inside the lyrics/verse
//! elements themselves, not by rebuilding this subtree.
//!
//! SCENE-SWITCH CROSSFADE (#716): this component's own `.stream-scene` div is the
//! crossfade layer. The output page keeps an outgoing scene mounted with
//! `leaving=true` (the `--leaving` class fades opacity to 0) while the incoming
//! one fades in via `@starting-style`; `duration_ms`
//! (`scene.transition_ms ?? kind-level ?? default`, resolved on the output page
//! per #752) sets the inline
//! `transition-duration`. `leaving` is read REACTIVELY (a `Signal`) because a
//! keyed `<For>` does not re-run children when only the leaving flag flips
//! (ui skill #496/#693).

use leptos::prelude::*;
use presenter_core::{SceneKind, StreamElementProps, StreamSceneDef};

use super::element_countdown::ElementCountdown;
use super::element_image::ElementImage;
use super::element_lyrics::ElementLyrics;
use super::element_verse::ElementVerse;

#[component]
pub fn SceneRender(
    scene: StreamSceneDef,
    /// Whether this scene layer is fading OUT (scheduled for removal). Reactive
    /// so the fade-out class applies without re-running the keyed `<For>` child.
    #[prop(into)]
    leaving: Signal<bool>,
    /// Crossfade duration in ms (`scene.transition_ms ?? kind-level ?? default`,
    /// resolved on the output page per #752),
    /// mirrored to the inline `transition-duration`. REACTIVE (a keyed `<For>`
    /// does not re-run the child), so `mark_leaving` re-pointing an outgoing base
    /// to the incoming scene's duration actually reaches the DOM — the fade-out
    /// and its removal timeout then agree (the "incoming governs both fades"
    /// invariant; a plain `u32` was a dead write that popped on differing
    /// per-scene durations).
    #[prop(into)]
    duration_ms: Signal<u32>,
) -> impl IntoView {
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
                    content_transition,
                } => view! {
                    <ElementCountdown
                        id=id
                        timer_id=timer_id
                        style=style
                        frame=frame
                        z=z
                        content_transition=content_transition
                    />
                }
                .into_any(),
                StreamElementProps::Lyrics {
                    show_main,
                    show_translation,
                    main_style,
                    translation_style,
                    frame,
                    content_transition,
                } => view! {
                    <ElementLyrics
                        id=id
                        show_main=show_main
                        show_translation=show_translation
                        main_style=main_style
                        translation_style=translation_style
                        frame=frame
                        z=z
                        content_transition=content_transition
                    />
                }
                .into_any(),
                StreamElementProps::Verse {
                    show_secondary,
                    text_style,
                    secondary_style,
                    reference_style,
                    frame,
                    content_transition,
                } => view! {
                    <ElementVerse
                        id=id
                        show_secondary=show_secondary
                        text_style=text_style
                        secondary_style=secondary_style
                        reference_style=reference_style
                        frame=frame
                        z=z
                        content_transition=content_transition
                    />
                }
                .into_any(),
            }
        })
        .collect_view();

    let style = move || format!("transition-duration:{}ms;", duration_ms.get());

    view! {
        <div
            class="stream-scene"
            class:stream-scene--leaving=leaving
            data-role="stream-scene"
            data-scene-id=scene_id.to_string()
            data-scene-kind=kind.as_str()
            style=style
        >
            {rendered}
        </div>
    }
}
