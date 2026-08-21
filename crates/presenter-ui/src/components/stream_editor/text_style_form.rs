//! Shared `TextStyle` sub-form (#714), used 6× across the countdown / lyrics /
//! verse property forms. Reads + writes ONE `TextStyle` slot of the element
//! draft (`RwSignal<StreamElementProps>`) via [`props_access`], so there is a
//! single working copy and no per-field signal wall.
//!
//! E2E scoping: the whole group is a `<fieldset data-role="stream-ts-{role}">`;
//! inner controls use FIXED data-roles (`stream-ts-font`, `-size`, `-color`,
//! `-alpha`, `-weight`, `-align-left|center|right`, `-line-height`,
//! `-shadow-enable`, `-shadow-x|y|blur|color`) selected WITHIN that group, so a
//! Verse's three groups stay distinguishable by their wrapper role.

use leptos::prelude::*;
use presenter_core::{StreamElementProps, TextAlign, STREAM_FONT_FAMILIES};

use super::props_access::{default_shadow, join_color, read_ts, split_color, with_ts_mut, TsSlot};

/// One labelled `TextStyle` editor bound to `draft` at `slot`.
#[component]
pub fn TextStyleForm(
    draft: RwSignal<StreamElementProps>,
    slot: TsSlot,
    /// Human label shown above the group (e.g. "Hlavný text").
    label: &'static str,
    /// Data-role discriminator for the wrapper (e.g. "main", "translation").
    role: &'static str,
) -> impl IntoView {
    let group_role = format!("stream-ts-{role}");

    // --- reactive readers (each subscribes to `draft`) ---
    let font = move || {
        read_ts(&draft.get(), slot)
            .map(|ts| ts.font_family)
            .unwrap_or_default()
    };
    let size = move || {
        read_ts(&draft.get(), slot)
            .map(|ts| ts.size_pct.to_string())
            .unwrap_or_default()
    };
    let color_rgb = move || {
        read_ts(&draft.get(), slot)
            .map(|ts| split_color(&ts.color).0)
            .unwrap_or_else(|| "#000000".to_string())
    };
    let alpha = move || {
        read_ts(&draft.get(), slot)
            .map(|ts| split_color(&ts.color).1.to_string())
            .unwrap_or_else(|| "255".to_string())
    };
    let weight = move || {
        read_ts(&draft.get(), slot)
            .map(|ts| ts.weight.to_string())
            .unwrap_or_default()
    };
    let line_height = move || {
        read_ts(&draft.get(), slot)
            .map(|ts| ts.line_height.to_string())
            .unwrap_or_default()
    };
    let align_is = move |a: TextAlign| {
        read_ts(&draft.get(), slot)
            .map(|ts| ts.align == a)
            .unwrap_or(false)
    };
    let has_shadow = move || {
        read_ts(&draft.get(), slot)
            .map(|ts| ts.shadow.is_some())
            .unwrap_or(false)
    };
    let shadow_x = move || {
        read_ts(&draft.get(), slot)
            .and_then(|ts| ts.shadow.map(|s| s.x_px.to_string()))
            .unwrap_or_default()
    };
    let shadow_y = move || {
        read_ts(&draft.get(), slot)
            .and_then(|ts| ts.shadow.map(|s| s.y_px.to_string()))
            .unwrap_or_default()
    };
    let shadow_blur = move || {
        read_ts(&draft.get(), slot)
            .and_then(|ts| ts.shadow.map(|s| s.blur_px.to_string()))
            .unwrap_or_default()
    };
    let shadow_color = move || {
        read_ts(&draft.get(), slot)
            .and_then(|ts| ts.shadow.map(|s| s.color))
            .unwrap_or_else(|| "#000000".to_string())
    };

    // Font <option> list from the fixed v1 whitelist.
    let font_options = STREAM_FONT_FAMILIES
        .iter()
        .map(|f| view! { <option value=*f>{*f}</option> })
        .collect_view();

    view! {
        <fieldset class="stream-editor__ts-group" data-role=group_role>
            <legend class="stream-editor__ts-legend">{label}</legend>

            <label class="stream-editor__field">
                <span>"Písmo"</span>
                <select
                    data-role="stream-ts-font"
                    prop:value=font
                    on:change=move |ev| {
                        let v = event_target_value(&ev);
                        draft.update(|p| with_ts_mut(p, slot, |ts| ts.font_family = v.clone()));
                    }
                >
                    {font_options}
                </select>
            </label>

            <label class="stream-editor__field">
                <span>"Veľkosť (% výšky)"</span>
                <input
                    type="number" step="0.1" min="0" max="100"
                    data-role="stream-ts-size"
                    prop:value=size
                    on:input=move |ev| {
                        if let Ok(v) = event_target_value(&ev).parse::<f32>() {
                            draft.update(|p| with_ts_mut(p, slot, |ts| ts.size_pct = v));
                        }
                    }
                />
            </label>

            <label class="stream-editor__field">
                <span>"Farba"</span>
                <input
                    type="color"
                    data-role="stream-ts-color"
                    prop:value=color_rgb
                    on:input=move |ev| {
                        let rgb = event_target_value(&ev);
                        draft.update(|p| with_ts_mut(p, slot, |ts| {
                            let a = split_color(&ts.color).1;
                            ts.color = join_color(&rgb, a);
                        }));
                    }
                />
                <input
                    type="number" min="0" max="255" step="1"
                    data-role="stream-ts-alpha"
                    prop:value=alpha
                    on:input=move |ev| {
                        if let Ok(a) = event_target_value(&ev).parse::<u8>() {
                            draft.update(|p| with_ts_mut(p, slot, |ts| {
                                let rgb = split_color(&ts.color).0;
                                ts.color = join_color(&rgb, a);
                            }));
                        }
                    }
                />
            </label>

            <label class="stream-editor__field">
                <span>"Hrúbka"</span>
                <input
                    type="number" min="1" max="1000" step="1"
                    data-role="stream-ts-weight"
                    prop:value=weight
                    on:input=move |ev| {
                        if let Ok(v) = event_target_value(&ev).parse::<u16>() {
                            draft.update(|p| with_ts_mut(p, slot, |ts| ts.weight = v));
                        }
                    }
                />
            </label>

            <div class="stream-editor__field stream-editor__align">
                <span>"Zarovnanie"</span>
                <div class="stream-editor__align-buttons">
                    <button
                        type="button" class="stream-editor__btn stream-editor__btn--ghost"
                        data-role="stream-ts-align-left"
                        data-active=move || if align_is(TextAlign::Left) { "true" } else { "false" }
                        on:click=move |_| draft.update(|p| with_ts_mut(p, slot, |ts| ts.align = TextAlign::Left))
                    >"◧"</button>
                    <button
                        type="button" class="stream-editor__btn stream-editor__btn--ghost"
                        data-role="stream-ts-align-center"
                        data-active=move || if align_is(TextAlign::Center) { "true" } else { "false" }
                        on:click=move |_| draft.update(|p| with_ts_mut(p, slot, |ts| ts.align = TextAlign::Center))
                    >"◫"</button>
                    <button
                        type="button" class="stream-editor__btn stream-editor__btn--ghost"
                        data-role="stream-ts-align-right"
                        data-active=move || if align_is(TextAlign::Right) { "true" } else { "false" }
                        on:click=move |_| draft.update(|p| with_ts_mut(p, slot, |ts| ts.align = TextAlign::Right))
                    >"◨"</button>
                </div>
            </div>

            <label class="stream-editor__field">
                <span>"Riadkovanie"</span>
                <input
                    type="number" min="0.5" max="3" step="0.1"
                    data-role="stream-ts-line-height"
                    prop:value=line_height
                    on:input=move |ev| {
                        if let Ok(v) = event_target_value(&ev).parse::<f32>() {
                            draft.update(|p| with_ts_mut(p, slot, |ts| ts.line_height = v));
                        }
                    }
                />
            </label>

            <label class="stream-editor__field stream-editor__field--check">
                <input
                    type="checkbox"
                    data-role="stream-ts-shadow-enable"
                    prop:checked=has_shadow
                    on:change=move |ev| {
                        let on = event_target_checked(&ev);
                        draft.update(|p| with_ts_mut(p, slot, |ts| {
                            ts.shadow = if on { Some(default_shadow()) } else { None };
                        }));
                    }
                />
                <span>"Tieň"</span>
            </label>

            <Show when=has_shadow>
                <div class="stream-editor__shadow-fields">
                    <label class="stream-editor__field">
                        <span>"Tieň X"</span>
                        <input
                            type="number" step="1"
                            data-role="stream-ts-shadow-x"
                            prop:value=shadow_x
                            on:input=move |ev| {
                                if let Ok(v) = event_target_value(&ev).parse::<f32>() {
                                    draft.update(|p| with_ts_mut(p, slot, |ts| {
                                        if let Some(s) = ts.shadow.as_mut() { s.x_px = v; }
                                    }));
                                }
                            }
                        />
                    </label>
                    <label class="stream-editor__field">
                        <span>"Tieň Y"</span>
                        <input
                            type="number" step="1"
                            data-role="stream-ts-shadow-y"
                            prop:value=shadow_y
                            on:input=move |ev| {
                                if let Ok(v) = event_target_value(&ev).parse::<f32>() {
                                    draft.update(|p| with_ts_mut(p, slot, |ts| {
                                        if let Some(s) = ts.shadow.as_mut() { s.y_px = v; }
                                    }));
                                }
                            }
                        />
                    </label>
                    <label class="stream-editor__field">
                        <span>"Rozmazanie"</span>
                        <input
                            type="number" min="0" step="1"
                            data-role="stream-ts-shadow-blur"
                            prop:value=shadow_blur
                            on:input=move |ev| {
                                if let Ok(v) = event_target_value(&ev).parse::<f32>() {
                                    draft.update(|p| with_ts_mut(p, slot, |ts| {
                                        if let Some(s) = ts.shadow.as_mut() { s.blur_px = v; }
                                    }));
                                }
                            }
                        />
                    </label>
                    <label class="stream-editor__field">
                        <span>"Farba tieňa"</span>
                        <input
                            type="color"
                            data-role="stream-ts-shadow-color"
                            prop:value=shadow_color
                            on:input=move |ev| {
                                let c = event_target_value(&ev);
                                draft.update(|p| with_ts_mut(p, slot, |ts| {
                                    if let Some(s) = ts.shadow.as_mut() { s.color = c.clone(); }
                                }));
                            }
                        />
                    </label>
                </div>
            </Show>
        </fieldset>
    }
}
