use leptos::prelude::*;

use crate::state::operator::OperatorState;
use crate::state::AppContext;

/// Slide list component - displays slides in live mode (click to trigger) or edit mode (textarea fields).
#[component]
pub fn SlideList(ctx: AppContext, op: OperatorState) -> impl IntoView {
    let presentation = ctx.selected_presentation;
    let mode = ctx.mode;
    let snapshot = ctx.stage_snapshot;
    let line_limit = op.line_limit;
    let focused = op.focused_slide_id;
    let playlist_id = ctx.selected_playlist_id;

    let active_slide_id = move || {
        snapshot
            .get()
            .and_then(|s| s.current_slide_id)
            .map(|id| id.to_string())
    };

    let trigger_slide = move |pres_id: String, slide_id: String, next_id: Option<String>| {
        let pl_id = playlist_id.get();
        leptos::task::spawn_local(async move {
            let _ = crate::api::stage::update_state(&crate::api::stage::StageStateRequest {
                presentation_id: pres_id,
                current_slide_id: slide_id,
                next_slide_id: next_id,
                playlist_id: pl_id,
            })
            .await;
        });
    };

    view! {
        <section class="operator__slides-column">
            <div class="operator__slides-toolbar">
                <label class="operator__line-limit" title="Maximum characters per line">
                    <span>"Line limit"</span>
                    <input
                        type="number"
                        min="10"
                        max="120"
                        step="1"
                        data-role="line-limit"
                        prop:value=move || line_limit.get().to_string()
                        on:change=move |ev| {
                            if let Ok(v) = event_target_value(&ev).parse::<u32>() {
                                line_limit.set(v);
                                crate::state::session::set("lineLimit", &v.to_string());
                            }
                        }
                    />
                </label>
                <button
                    type="button"
                    class="operator__slides-add"
                    data-role="add-slide"
                    title="Add slide"
                >"+"</button>
            </div>
            <div class="operator__slides" data-role="slides">
                {move || {
                    let pres = presentation.get();
                    match pres {
                        None => view! {
                            <p class="empty">"Select a presentation to load slides."</p>
                        }.into_any(),
                        Some(pres) => {
                            let pres_id = pres.id.to_string();
                            let slides = pres.slides.clone();
                            let is_live = mode.get() == "live";
                            let slide_count = slides.len();

                            view! {
                                <div class="operator__slides-inner">
                                    {slides.into_iter().enumerate().map(|(idx, slide)| {
                                        let slide_id = slide.id.to_string();
                                        let pres_id = pres_id.clone();
                                        let main_text = slide.content.main.value().to_string();
                                        let translation_text = slide.content.translation.value().to_string();
                                        let stage_text = slide.content.stage.value().to_string();
                                        let group = slide.content.group.as_ref().map(|g| g.name().to_string());
                                        let slide_id_cmp = slide_id.clone();
                                        let slide_id_trigger = slide_id.clone();
                                        let slide_id_focus = slide_id.clone();
                                        let limit = line_limit.get();

                                        // Check for line limit warnings
                                        let has_warning = main_text.lines().any(|line| line.len() > limit as usize);

                                        // Compute next slide ID for stage triggering
                                        let next_id: Option<String> = if idx + 1 < slide_count {
                                            // We need the slides list for this, but we only have the current slide
                                            // This will be None for now; the server resolves it when not provided
                                            None
                                        } else {
                                            None
                                        };

                                        let is_focused = {
                                            let sid = slide_id.clone();
                                            move || focused.get().as_deref() == Some(sid.as_str())
                                        };

                                        view! {
                                            <div
                                                class="operator__slide-card"
                                                class:active=move || active_slide_id().as_deref() == Some(slide_id_cmp.as_str())
                                                class:focused=is_focused
                                                data-role="slide-card"
                                                data-slide-id={slide_id.clone()}
                                            >
                                                {group.map(|g| view! {
                                                    <div class="operator__slide-group" data-role="slide-group">{g}</div>
                                                })}
                                                {if is_live {
                                                    let pres_id = pres_id.clone();
                                                    let sid = slide_id_trigger.clone();
                                                    let next = next_id.clone();
                                                    view! {
                                                        <div
                                                            class="operator__slide-content operator__slide-content--live"
                                                            on:click=move |_| {
                                                                trigger_slide(pres_id.clone(), sid.clone(), next.clone());
                                                            }
                                                        >
                                                            <div class="operator__slide-main">
                                                                {main_text.clone()}
                                                            </div>
                                                            {(!translation_text.is_empty()).then(|| view! {
                                                                <div class="operator__slide-translation">
                                                                    {translation_text.clone()}
                                                                </div>
                                                            })}
                                                            {(!stage_text.is_empty()).then(|| view! {
                                                                <div class="operator__slide-stage">
                                                                    {stage_text.clone()}
                                                                </div>
                                                            })}
                                                        </div>
                                                    }.into_any()
                                                } else {
                                                    let sid_main = slide_id_focus.clone();
                                                    let sid_trans = slide_id_focus.clone();
                                                    let sid_stage = slide_id_focus.clone();
                                                    view! {
                                                        <div class="operator__slide-content operator__slide-content--edit">
                                                            <textarea
                                                                class="operator__slide-field"
                                                                data-field="main"
                                                                rows="3"
                                                                prop:value={main_text.clone()}
                                                                on:focus=move |_| {
                                                                    focused.set(Some(sid_main.clone()));
                                                                    crate::state::session::set("focusedSlideId", &sid_main);
                                                                }
                                                            />
                                                            <textarea
                                                                class="operator__slide-field"
                                                                data-field="translation"
                                                                rows="2"
                                                                prop:value={translation_text.clone()}
                                                                on:focus=move |_| {
                                                                    focused.set(Some(sid_trans.clone()));
                                                                    crate::state::session::set("focusedSlideId", &sid_trans);
                                                                }
                                                            />
                                                            <textarea
                                                                class="operator__slide-field"
                                                                data-field="stage"
                                                                rows="2"
                                                                prop:value={stage_text.clone()}
                                                                on:focus=move |_| {
                                                                    focused.set(Some(sid_stage.clone()));
                                                                    crate::state::session::set("focusedSlideId", &sid_stage);
                                                                }
                                                            />
                                                        </div>
                                                    }.into_any()
                                                }}
                                                {has_warning.then(|| view! {
                                                    <div class="operator__slide-warning" data-role="slide-warning">
                                                        "Line exceeds character limit"
                                                    </div>
                                                })}
                                            </div>
                                        }
                                    }).collect::<Vec<_>>()}
                                </div>
                            }.into_any()
                        }
                    }
                }}
            </div>
        </section>
    }
}
