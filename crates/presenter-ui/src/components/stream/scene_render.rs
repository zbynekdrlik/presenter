//! Renders one stream scene's elements (#709; lyrics + verse #710; transitions
//! #716; live draft preview #777).
//!
//! Elements arrive already ordered by `z_order` (the repository's def assembly,
//! #705). Each is mapped to its per-kind component (IMAGE / COUNTDOWN / LYRICS /
//! VERSE / COLOR). Its props are resolved through the preview
//! [`StreamDraftOverride`] (#777): on a `?preview=1` output page the editor pushes
//! the operator's UNSAVED edits in via `postMessage`, so an element being edited
//! renders from the live draft — the preview IS the real renderer. The override
//! is read through a per-element `Memo`, so ONLY the element under edit re-renders
//! on a draft change; every other element (and every production output, which
//! provides no override context) is untouched — the `Memo` fires once and holds.
//!
//! A scene's element SET only changes on a def refetch (which remounts this whole
//! subtree), so building a `Vec` of per-element reactive nodes (no keyed `<For>`)
//! is correct: there is no live in-place list mutation and no scroll container.
//! Live CONTENT changes (a new lyric line / verse) update reactively inside the
//! lyrics/verse elements themselves.
//!
//! SCENE-SWITCH CROSSFADE (#716): this component's own `.stream-scene` div is the
//! crossfade layer. The output page keeps an outgoing scene mounted with
//! `leaving=true` (the `--leaving` class fades opacity to 0) while the incoming
//! one fades in via `@starting-style`; `duration_ms`
//! (`scene.transition_ms ?? kind-level ?? default`, resolved on the output page
//! per #752) sets the inline `transition-duration`. `leaving` is read REACTIVELY
//! (a `Signal`) because a keyed `<For>` does not re-run children when only the
//! leaving flag flips (ui skill #496/#693).

use leptos::prelude::*;
use presenter_core::{SceneKind, StreamElementProps, StreamSceneDef};

use super::draft_preview::{resolve_props, StreamDraftOverride};
use super::element_color::ElementColor;
use super::element_countdown::ElementCountdown;
use super::element_image::ElementImage;
use super::element_lower_third::ElementLowerThird;
use super::element_lyrics::ElementLyrics;
use super::element_verse::ElementVerse;

/// Render one element (already resolved through any draft override) to its
/// per-kind component.
fn render_element(id: i64, z: i32, props: StreamElementProps) -> AnyView {
    match props {
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
            // Countdown ignores `content_transition` (#776): a per-tick fade
            // flickers, so it renders a stable text node with a hard cut.
            content_transition: _,
            r#box,
        } => view! {
            <ElementCountdown id=id timer_id=timer_id style=style frame=frame text_box=r#box z=z />
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
        StreamElementProps::Color {
            color,
            opacity,
            frame,
        } => view! {
            <ElementColor id=id color=color opacity=opacity frame=frame z=z />
        }
        .into_any(),
        StreamElementProps::LowerThird {
            frame,
            bar_color,
            bar_opacity,
            accent_color,
            accent_width_pct,
            primary_style,
            secondary_style,
            padding_pct,
            animation,
            in_ms,
            out_ms,
            // The plate's TEXT + auto-hide are runtime state, not element props:
            // auto_hide_s is applied server-side, so it is not read here.
            auto_hide_s: _,
        } => view! {
            <ElementLowerThird
                id=id
                frame=frame
                bar_color=bar_color
                bar_opacity=bar_opacity
                accent_color=accent_color
                accent_width_pct=accent_width_pct
                primary_style=primary_style
                secondary_style=secondary_style
                padding_pct=padding_pct
                animation=animation
                in_ms=in_ms
                out_ms=out_ms
                z=z
            />
        }
        .into_any(),
    }
}

#[component]
pub fn SceneRender(
    scene: StreamSceneDef,
    /// Whether this scene layer is fading OUT (scheduled for removal). Reactive
    /// so the fade-out class applies without re-running the keyed `<For>` child.
    #[prop(into)]
    leaving: Signal<bool>,
    /// Crossfade duration in ms (`scene.transition_ms ?? kind-level ?? default`,
    /// resolved on the output page per #752), mirrored to the inline
    /// `transition-duration`. REACTIVE so `mark_leaving` re-pointing an outgoing
    /// base to the incoming scene's duration reaches the DOM.
    #[prop(into)]
    duration_ms: Signal<u32>,
) -> impl IntoView {
    let scene_id = scene.id;
    let kind: SceneKind = scene.kind;
    let elements = scene.elements;

    // Preview draft override (present ONLY on a `?preview=1` output page). In a
    // production output `use_context` is `None`, so every element's `Memo` returns
    // its stored props once and never re-fires — zero overhead off the editor.
    let override_sig = use_context::<StreamDraftOverride>();

    let rendered: Vec<AnyView> = elements
        .into_iter()
        .map(|el| {
            let id = el.id;
            let z = el.z_order;
            let base = el.props;
            let effective = Memo::new(move |_| resolve_props(override_sig, id, &base));
            view! { {move || render_element(id, z, effective.get())} }.into_any()
        })
        .collect();

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
