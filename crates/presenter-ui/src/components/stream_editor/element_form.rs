//! Per-element property form (#714): a single working-copy draft
//! (`RwSignal<StreamElementProps>`) seeded from the selected element, a shared
//! Frame section, per-kind fields (image / countdown / lyrics / verse) built on
//! the shared `TextStyleForm`, a content-transition control, and an EXPLICIT
//! Save that PATCHes the draft. A server 422 is rendered inline
//! (`ctx.prop_error`); the def is left unchanged. The draft re-seeds ONLY when
//! the selected element changes, so a live def refetch never clobbers edits.

use leptos::prelude::*;
use presenter_core::{ContentTransition, ImageFit, StreamElementProps, STREAM_DEFAULT_FADE_MS};

use super::props_access::{
    default_element_props, read_frame, read_transition, with_frame_mut, with_transition_mut, TsSlot,
};
use super::text_style_form::TextStyleForm;
use super::StreamEditorCtx;

/// The two Presenter timers a countdown element can bind to. `timer_id` is
/// forward-looking (the output page currently always renders
/// `countdown_to_start` — #709 contract); ids 1/2 are the conventional mapping.
const STREAM_TIMERS: &[(i64, &str)] = &[(1, "Odpočet do začiatku"), (2, "Časomiera kázne")];

/// Property form for the currently-selected element.
#[component]
pub fn ElementForm(ctx: StreamEditorCtx) -> impl IntoView {
    // One working copy; re-seeded whenever the selected element changes. def is
    // read UNTRACKED so a live StreamConfigChanged refetch never clobbers edits.
    let draft = RwSignal::new(default_element_props("image"));
    Effect::new(move |_| {
        let Some(id) = ctx.selected_element.get() else {
            return;
        };
        if let Some(def) = ctx.def.get_untracked() {
            if let Some(el) = def
                .scenes
                .iter()
                .flat_map(|s| &s.elements)
                .find(|e| e.id == id)
            {
                draft.set(el.props.clone());
            }
        }
    });

    let kind = move || draft.get().kind_str().to_string();

    view! {
        <div class="stream-editor__prop-form" data-role="stream-prop-form">
            <FrameFields draft=draft />

            <Show when=move || kind() == "image">
                <ImageFields draft=draft ctx=ctx />
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

            <Show when=move || kind() != "image">
                <TransitionFields draft=draft />
            </Show>

            <Show when=move || !ctx.prop_error.get().is_empty()>
                <p class="stream-editor__prop-error" data-role="stream-prop-error">
                    {move || ctx.prop_error.get()}
                </p>
            </Show>

            <div class="stream-editor__prop-actions">
                <button
                    type="button"
                    class="stream-editor__btn stream-editor__btn--primary"
                    data-role="stream-prop-save"
                    on:click=move |_| {
                        if let Some(id) = ctx.selected_element.get_untracked() {
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
#[component]
fn FrameFields(draft: RwSignal<StreamElementProps>) -> impl IntoView {
    let x = move || read_frame(&draft.get()).x_pct.to_string();
    let y = move || read_frame(&draft.get()).y_pct.to_string();
    let w = move || read_frame(&draft.get()).w_pct.to_string();
    let h = move || read_frame(&draft.get()).h_pct.to_string();
    view! {
        <fieldset class="stream-editor__frame" data-role="stream-frame">
            <legend class="stream-editor__ts-legend">"Rám (% plátna)"</legend>
            <label class="stream-editor__field">
                <span>"X"</span>
                <input type="number" step="0.1" min="0" max="100" data-role="stream-frame-x"
                    prop:value=x
                    on:input=move |ev| {
                        if let Ok(v) = event_target_value(&ev).parse::<f32>() {
                            draft.update(|p| with_frame_mut(p, |fr| fr.x_pct = v));
                        }
                    } />
            </label>
            <label class="stream-editor__field">
                <span>"Y"</span>
                <input type="number" step="0.1" min="0" max="100" data-role="stream-frame-y"
                    prop:value=y
                    on:input=move |ev| {
                        if let Ok(v) = event_target_value(&ev).parse::<f32>() {
                            draft.update(|p| with_frame_mut(p, |fr| fr.y_pct = v));
                        }
                    } />
            </label>
            <label class="stream-editor__field">
                <span>"Šírka"</span>
                <input type="number" step="0.1" min="0" max="100" data-role="stream-frame-w"
                    prop:value=w
                    on:input=move |ev| {
                        if let Ok(v) = event_target_value(&ev).parse::<f32>() {
                            draft.update(|p| with_frame_mut(p, |fr| fr.w_pct = v));
                        }
                    } />
            </label>
            <label class="stream-editor__field">
                <span>"Výška"</span>
                <input type="number" step="0.1" min="0" max="100" data-role="stream-frame-h"
                    prop:value=h
                    on:input=move |ev| {
                        if let Ok(v) = event_target_value(&ev).parse::<f32>() {
                            draft.update(|p| with_frame_mut(p, |fr| fr.h_pct = v));
                        }
                    } />
            </label>
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
    let opacity = move || match draft.get() {
        StreamElementProps::Image { opacity, .. } => opacity.to_string(),
        _ => String::new(),
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
            <label class="stream-editor__field">
                <span>"Priehľadnosť"</span>
                <input type="number" min="0" max="1" step="0.05" data-role="stream-image-opacity"
                    prop:value=opacity
                    on:input=move |ev| {
                        if let Ok(v) = event_target_value(&ev).parse::<f32>() {
                            draft.update(|p| {
                                if let StreamElementProps::Image { opacity, .. } = p { *opacity = v; }
                            });
                        }
                    } />
            </label>
        </div>
    }
}

/// Content-transition control (cut vs crossfade + duration) for the kinds that
/// carry one (countdown / lyrics / verse).
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
