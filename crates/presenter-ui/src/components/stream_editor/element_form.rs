//! Per-element property form (#714; live-preview draft #777): the property form
//! edits the SHARED working-copy draft on the ctx (`ctx.draft`), which the canvas
//! overlay and the preview push also read/write — so numeric fields, the on-canvas
//! outline and the live iframe stay in lock-step with NO save round trip. The
//! draft re-seeds ONLY when the selected element changes (and not when the overlay
//! already seeded it), so a live def refetch or a drag never clobbers edits. The
//! Frame section is buffered numeric fields (clamped to the legal range on commit,
//! so a save can never 422 on the frame); per-kind fields (image / color /
//! countdown / lyrics / verse) build on the shared `TextStyleForm`; Save is
//! EXPLICIT (PATCHes the draft; a 422 renders inline via `ctx.prop_error`).

use leptos::prelude::*;
use presenter_core::{
    ContentTransition, ImageFit, StreamElementProps, STREAM_DEFAULT_FADE_MS,
    STREAM_FRAME_POS_MAX_PCT, STREAM_FRAME_POS_MIN_PCT, STREAM_FRAME_SIZE_MAX_PCT,
};

use super::frame_math::MIN_SIZE_PCT;
use super::number_field::{FrameField, FrameNumberField};
use super::props_access::{read_transition, split_color, with_transition_mut, TsSlot};
use super::text_style_form::TextStyleForm;
use super::StreamEditorCtx;

/// The two Presenter timers a countdown element can bind to. `timer_id` is
/// forward-looking (the output page currently always renders
/// `countdown_to_start` — #709 contract); ids 1/2 are the conventional mapping.
const STREAM_TIMERS: &[(i64, &str)] = &[(1, "Odpočet do začiatku"), (2, "Časomiera kázne")];

/// Property form for the currently-selected element.
#[component]
pub fn ElementForm(ctx: StreamEditorCtx) -> impl IntoView {
    // The shared working copy (ctx.draft) — the canvas overlay + preview push read
    // and write the SAME signal. Re-seeded only when the selection changes to a
    // DIFFERENT element than the draft already holds (so a live def refetch, or the
    // overlay's own synchronous seed on drag-start, never clobbers edits). def is
    // read UNTRACKED so a live StreamConfigChanged refetch never reseeds.
    let draft = ctx.draft;
    Effect::new(move |_| {
        let Some(id) = ctx.selected_element.get() else {
            return;
        };
        if ctx.draft_element_id.get_untracked() == Some(id) {
            return;
        }
        if let Some(def) = ctx.def.get_untracked() {
            if let Some(el) = def
                .scenes
                .iter()
                .flat_map(|s| &s.elements)
                .find(|e| e.id == id)
            {
                ctx.seed_draft(id, el.props.clone());
            }
        }
    });

    let kind = move || draft.get().kind_str().to_string();
    let dirty = move || ctx.draft_is_dirty_reactive();

    view! {
        <div class="stream-editor__prop-form" data-role="stream-prop-form">
            <FrameFields draft=draft />

            <Show when=move || kind() == "image">
                <ImageFields draft=draft ctx=ctx />
            </Show>
            <Show when=move || kind() == "color">
                <ColorFields draft=draft />
            </Show>
            <Show when=move || kind() == "countdown">
                <div class="stream-editor__kind-fields" data-role="stream-countdown-fields">
                    <label class="stream-editor__field">
                        <span>"Časovač"</span>
                        <select
                            data-role="stream-countdown-timer"
                            prop:value=move || match draft.get() {
                                StreamElementProps::Countdown { timer_id, .. } => timer_id.to_string(),
                                _ => String::new(),
                            }
                            on:change=move |ev| {
                                if let Ok(v) = event_target_value(&ev).parse::<i64>() {
                                    draft.update(|p| {
                                        if let StreamElementProps::Countdown { timer_id, .. } = p {
                                            *timer_id = v;
                                        }
                                    });
                                }
                            }
                        >
                            {STREAM_TIMERS
                                .iter()
                                .map(|(id, name)| view! { <option value=id.to_string()>{*name}</option> })
                                .collect_view()}
                        </select>
                    </label>
                    <TextStyleForm draft=draft ts_slot=TsSlot::CountdownStyle label="Štýl" role="countdown" />
                </div>
            </Show>
            <Show when=move || kind() == "lyrics">
                <div class="stream-editor__kind-fields" data-role="stream-lyrics-fields">
                    <label class="stream-editor__field stream-editor__field--check">
                        <input
                            type="checkbox"
                            data-role="stream-lyrics-show-main"
                            prop:checked=move || matches!(draft.get(), StreamElementProps::Lyrics { show_main: true, .. })
                            on:change=move |ev| {
                                let on = event_target_checked(&ev);
                                draft.update(|p| {
                                    if let StreamElementProps::Lyrics { show_main, .. } = p { *show_main = on; }
                                });
                            }
                        />
                        <span>"Zobraziť hlavný text"</span>
                    </label>
                    <label class="stream-editor__field stream-editor__field--check">
                        <input
                            type="checkbox"
                            data-role="stream-lyrics-show-translation"
                            prop:checked=move || matches!(draft.get(), StreamElementProps::Lyrics { show_translation: true, .. })
                            on:change=move |ev| {
                                let on = event_target_checked(&ev);
                                draft.update(|p| {
                                    if let StreamElementProps::Lyrics { show_translation, .. } = p { *show_translation = on; }
                                });
                            }
                        />
                        <span>"Zobraziť preklad"</span>
                    </label>
                    <TextStyleForm draft=draft ts_slot=TsSlot::LyricsMain label="Hlavný text" role="main" />
                    <TextStyleForm draft=draft ts_slot=TsSlot::LyricsTranslation label="Preklad" role="translation" />
                </div>
            </Show>
            <Show when=move || kind() == "verse">
                <div class="stream-editor__kind-fields" data-role="stream-verse-fields">
                    <label class="stream-editor__field stream-editor__field--check">
                        <input
                            type="checkbox"
                            data-role="stream-verse-show-secondary"
                            prop:checked=move || matches!(draft.get(), StreamElementProps::Verse { show_secondary: true, .. })
                            on:change=move |ev| {
                                let on = event_target_checked(&ev);
                                draft.update(|p| {
                                    if let StreamElementProps::Verse { show_secondary, .. } = p { *show_secondary = on; }
                                });
                            }
                        />
                        <span>"Zobraziť druhý jazyk"</span>
                    </label>
                    <TextStyleForm draft=draft ts_slot=TsSlot::VerseText label="Text verša" role="text" />
                    <TextStyleForm draft=draft ts_slot=TsSlot::VerseSecondary label="Druhý jazyk" role="secondary" />
                    <TextStyleForm draft=draft ts_slot=TsSlot::VerseReference label="Odkaz" role="reference" />
                </div>
            </Show>

            // Content-transition control: lyrics + verse only. Image/color have
            // no content_transition; countdown carries one in the model but ignores
            // it (a per-tick fade flickers — #776), so the control is hidden for it.
            <Show when=move || kind() == "lyrics" || kind() == "verse">
                <TransitionFields draft=draft />
            </Show>

            <Show when=move || !ctx.prop_error.get().is_empty()>
                <p class="stream-editor__prop-error" data-role="stream-prop-error">
                    {move || ctx.prop_error.get()}
                </p>
            </Show>

            <div class="stream-editor__prop-actions">
                <Show when=dirty>
                    <span class="stream-editor__unsaved" data-role="stream-unsaved">
                        "Neuložené zmeny"
                    </span>
                </Show>
                <button
                    type="button"
                    class="stream-editor__btn stream-editor__btn--primary"
                    data-role="stream-prop-save"
                    data-dirty=move || super::bool_attr(dirty())
                    on:click=move |_| {
                        if let Some(id) = ctx.draft_element_id.get_untracked() {
                            ctx.save_props(id, draft.get_untracked());
                        }
                    }
                >
                    "Uložiť"
                </button>
            </div>
        </div>
    }
}

/// The shared Frame (x/y/w/h %) numeric inputs — all four kinds have a frame.
/// Each is a BUFFERED [`FrameNumberField`] (#777): typing edits a local text
/// buffer, and the parsed value is CLAMPED to the legal range on commit, so a
/// save can never 422 on the frame. X/Y allow off-canvas positions
/// (`STREAM_FRAME_POS_MIN/MAX_PCT`, -200..=300) for slide-in authoring; W/H stay
/// positive (`MIN_SIZE_PCT..=STREAM_FRAME_SIZE_MAX_PCT`). Direct manipulation on
/// the canvas overlay writes the SAME draft, so these fields update live during a
/// drag/resize (and vice-versa).
#[component]
fn FrameFields(draft: RwSignal<StreamElementProps>) -> impl IntoView {
    view! {
        <fieldset class="stream-editor__frame" data-role="stream-frame">
            <legend class="stream-editor__ts-legend">"Rám (% plátna)"</legend>
            <FrameNumberField
                draft=draft
                field=FrameField::X
                label="X"
                role="stream-frame-x"
                min=STREAM_FRAME_POS_MIN_PCT
                max=STREAM_FRAME_POS_MAX_PCT
                step=0.1
            />
            <FrameNumberField
                draft=draft
                field=FrameField::Y
                label="Y"
                role="stream-frame-y"
                min=STREAM_FRAME_POS_MIN_PCT
                max=STREAM_FRAME_POS_MAX_PCT
                step=0.1
            />
            <FrameNumberField
                draft=draft
                field=FrameField::W
                label="Šírka"
                role="stream-frame-w"
                min=MIN_SIZE_PCT
                max=STREAM_FRAME_SIZE_MAX_PCT
                step=0.1
            />
            <FrameNumberField
                draft=draft
                field=FrameField::H
                label="Výška"
                role="stream-frame-h"
                min=MIN_SIZE_PCT
                max=STREAM_FRAME_SIZE_MAX_PCT
                step=0.1
            />
        </fieldset>
    }
}

/// Image-kind fields: asset_id (numeric v1 + picker button wired in #715), fit,
/// opacity.
#[component]
fn ImageFields(draft: RwSignal<StreamElementProps>, ctx: StreamEditorCtx) -> impl IntoView {
    let asset_id = move || match draft.get() {
        StreamElementProps::Image { asset_id, .. } => asset_id.to_string(),
        _ => String::new(),
    };
    let fit = move || match draft.get() {
        StreamElementProps::Image { fit, .. } => match fit {
            ImageFit::Contain => "contain",
            ImageFit::Cover => "cover",
            ImageFit::Stretch => "stretch",
        },
        _ => "contain",
    };
    view! {
        <div class="stream-editor__kind-fields" data-role="stream-image-fields">
            <label class="stream-editor__field">
                <span>"Asset ID"</span>
                <input type="number" min="1" step="1" data-role="stream-image-asset-id"
                    prop:value=asset_id
                    on:input=move |ev| {
                        if let Ok(v) = event_target_value(&ev).parse::<i64>() {
                            draft.update(|p| {
                                if let StreamElementProps::Image { asset_id, .. } = p { *asset_id = v; }
                            });
                        }
                    } />
                <super::editor_assets::AssetPickerButton draft=draft ctx=ctx />
            </label>
            <label class="stream-editor__field">
                <span>"Prispôsobenie"</span>
                <select
                    data-role="stream-image-fit"
                    prop:value=fit
                    on:change=move |ev| {
                        let v = event_target_value(&ev);
                        let f = match v.as_str() {
                            "cover" => ImageFit::Cover,
                            "stretch" => ImageFit::Stretch,
                            _ => ImageFit::Contain,
                        };
                        draft.update(|p| {
                            if let StreamElementProps::Image { fit, .. } = p { *fit = f; }
                        });
                    }
                >
                    <option value="contain">"Contain"</option>
                    <option value="cover">"Cover"</option>
                    <option value="stretch">"Stretch"</option>
                </select>
            </label>
            // Opacity edited as an integer PERCENT, buffered + committed on blur
            // (#776) — the wire stays 0..=1.
            <super::percent_input::PercentInput draft=draft role="stream-image-opacity" />
        </div>
    }
}

/// Color-kind fields (#753): a native color picker (`<input type=color>`, which
/// emits a `#rrggbb`) + an opacity input. Opacity is THE transparency control,
/// so the color stays a solid RGB — no alpha byte here (unlike a `TextStyle`
/// color). `split_color` strips any stored alpha for the picker's value.
#[component]
fn ColorFields(draft: RwSignal<StreamElementProps>) -> impl IntoView {
    let color = move || match draft.get() {
        StreamElementProps::Color { color, .. } => split_color(&color).0,
        _ => "#000000".to_string(),
    };
    view! {
        <div class="stream-editor__kind-fields" data-role="stream-color-fields">
            <label class="stream-editor__field">
                <span>"Farba"</span>
                <input type="color" data-role="stream-color-value"
                    prop:value=color
                    on:input=move |ev| {
                        let rgb = event_target_value(&ev);
                        draft.update(|p| {
                            if let StreamElementProps::Color { color, .. } = p { *color = rgb; }
                        });
                    } />
            </label>
            // Opacity edited as an integer PERCENT, buffered + committed on blur
            // (#776) — the wire stays 0..=1.
            <super::percent_input::PercentInput draft=draft role="stream-color-opacity" />
        </div>
    }
}

/// Content-transition control (cut vs crossfade + duration) for lyrics + verse.
/// Countdown carries a `content_transition` in the model but ignores it (a
/// per-tick fade flickers — #776), so the editor hides this control for it.
#[component]
fn TransitionFields(draft: RwSignal<StreamElementProps>) -> impl IntoView {
    let is_fade = move || {
        matches!(
            read_transition(&draft.get()),
            Some(ContentTransition::Fade { .. })
        )
    };
    let duration = move || match read_transition(&draft.get()) {
        Some(ContentTransition::Fade { duration_ms }) => duration_ms.to_string(),
        _ => STREAM_DEFAULT_FADE_MS.to_string(),
    };
    view! {
        <fieldset class="stream-editor__transition" data-role="stream-transition">
            <legend class="stream-editor__ts-legend">"Prechod obsahu"</legend>
            <label class="stream-editor__field stream-editor__field--check">
                <input
                    type="checkbox"
                    data-role="stream-transition-fade"
                    prop:checked=is_fade
                    on:change=move |ev| {
                        let on = event_target_checked(&ev);
                        draft.update(|p| with_transition_mut(p, |t| {
                            *t = if on {
                                ContentTransition::Fade { duration_ms: STREAM_DEFAULT_FADE_MS }
                            } else {
                                ContentTransition::Cut
                            };
                        }));
                    }
                />
                <span>"Prelínať (crossfade)"</span>
            </label>
            <Show when=is_fade>
                <label class="stream-editor__field">
                    <span>"Trvanie (ms)"</span>
                    <input type="number" min="0" max="10000" step="50" data-role="stream-transition-ms"
                        prop:value=duration
                        on:input=move |ev| {
                            if let Ok(v) = event_target_value(&ev).parse::<u32>() {
                                draft.update(|p| with_transition_mut(p, |t| {
                                    if let ContentTransition::Fade { duration_ms } = t { *duration_ms = v; }
                                }));
                            }
                        } />
                </label>
            </Show>
        </fieldset>
    }
}
