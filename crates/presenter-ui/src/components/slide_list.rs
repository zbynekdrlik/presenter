use leptos::prelude::*;

use crate::api;
use crate::state::operator::OperatorState;
use crate::state::AppContext;

#[component]
pub fn SlideList() -> impl IntoView {
    let ctx = use_context::<AppContext>().expect("AppContext");
    let op = use_context::<OperatorState>().expect("OperatorState");

    let trigger_slide = move |pres_id: String, slide_id: String, next_slide_id: Option<String>| {
        let playlist_id = ctx.selected_playlist_id.get_untracked();
        leptos::task::spawn_local(async move {
            let _ = api::stage::update_state(&api::stage::StageStateRequest {
                presentation_id: pres_id,
                current_slide_id: slide_id,
                next_slide_id,
                playlist_id,
            })
            .await;
        });
    };

    let add_slide = move |_| {
        let pres = ctx.selected_presentation.get_untracked();
        if let Some(p) = pres {
            let pres_id = p.id.to_string();
            leptos::task::spawn_local(async move {
                if let Ok(slides) = api::presentations::insert_slide(&pres_id, None).await {
                    let ctx = use_context::<AppContext>().expect("AppContext");
                    ctx.selected_presentation.update(|p| {
                        if let Some(pres) = p.as_mut() {
                            pres.slides = slides;
                        }
                    });
                }
            });
        }
    };

    let on_line_limit_change = move |ev| {
        let val: String = event_target_value(&ev);
        if let Ok(n) = val.parse::<u32>() {
            op.line_limit.set(n);
            crate::state::session::set("lineLimit", &n.to_string());
        }
    };

    view! {
        <div class="operator__slides">
            <div class="operator__slides-toolbar">
                <label class="operator__line-limit-label">
                    "Line limit: "
                    <input
                        type="number"
                        data-role="line-limit"
                        class="operator__line-limit-input"
                        prop:value=move || op.line_limit.get().to_string()
                        on:change=on_line_limit_change
                        min="1"
                        max="100"
                    />
                </label>
                <button
                    data-role="add-slide"
                    class="operator__add-slide"
                    on:click=add_slide
                >
                    "+ Add Slide"
                </button>
            </div>
            <div class="operator__slide-list" data-role="slide-list">
                {move || {
                    let mode = ctx.mode.get();
                    let pres = ctx.selected_presentation.get();
                    let snapshot = ctx.stage_snapshot.get();
                    let line_limit = op.line_limit.get();

                    let Some(presentation) = pres else {
                        return view! { <div class="operator__no-slides">"Select a presentation to view slides"</div> }.into_any();
                    };

                    let pres_id = presentation.id.to_string();
                    let slides = presentation.slides.clone();
                    let current_slide_id = snapshot.as_ref().and_then(|s| s.current_slide_id.map(|id| id.to_string()));
                    let is_live = mode == "live";

                    let mut current_group: Option<String> = None;

                    slides.into_iter().enumerate().map(|(i, slide)| {
                        let slide_id = slide.id.to_string();
                        let main_text = slide.content.main.value().to_string();
                        let translation_text = slide.content.translation.value().to_string();
                        let stage_text = slide.content.stage.value().to_string();
                        let group_name = slide.content.group.as_ref().map(|g| g.name().to_string());

                        let show_group = if group_name != current_group {
                            current_group.clone_from(&group_name);
                            group_name
                        } else {
                            None
                        };

                        let is_active = current_slide_id.as_deref() == Some(&slide_id);
                        let main_lines = main_text.lines().count() as u32;
                        let main_warning = main_lines > line_limit;

                        let pres_id_click = pres_id.clone();
                        let slide_id_click = slide_id.clone();
                        let next_slide_id = presentation.slides.get(i + 1).map(|s| s.id.to_string());

                        // For edit mode blur handlers
                        let pres_id_edit = pres_id.clone();
                        let slide_id_edit = slide_id.clone();
                        let pres_id_dup = pres_id.clone();
                        let slide_id_dup = slide_id.clone();
                        let pres_id_del = pres_id.clone();
                        let slide_id_del = slide_id.clone();

                        view! {
                            {show_group.map(|g| view! {
                                <div data-role="slide-group" class="operator__slide-group">{g}</div>
                            })}
                            <div
                                class=move || {
                                    let mut c = "stage-control__slide".to_string();
                                    if is_active { c.push_str(" is-active"); }
                                    c
                                }
                                data-slide-id=slide_id.clone()
                            >
                                {if is_live {
                                    // Live mode: clickable slides
                                    let trigger = trigger_slide;
                                    view! {
                                        <div
                                            class="operator__slide-card operator__slide-card--live"
                                            on:click=move |_| {
                                                trigger(pres_id_click.clone(), slide_id_click.clone(), next_slide_id.clone());
                                            }
                                        >
                                            <div class="operator__slide-text--main" attr:data-warning=move || if main_warning { "true" } else { "false" }>
                                                {main_text.clone()}
                                            </div>
                                            {(!translation_text.is_empty()).then(|| view! {
                                                <div class="operator__slide-text--translation">{translation_text.clone()}</div>
                                            })}
                                            {(!stage_text.is_empty()).then(|| view! {
                                                <div class="operator__slide-text--stage">{stage_text.clone()}</div>
                                            })}
                                        </div>
                                    }.into_any()
                                } else {
                                    // Edit mode: textareas
                                    view! {
                                        <div class="operator__slide-card operator__slide-card--edit">
                                            <div data-role="slide-warning" attr:data-visible=move || if main_warning { "true" } else { "false" } class="operator__slide-warning">
                                                {format!("Warning: {main_lines} lines exceeds limit of {line_limit}")}
                                            </div>
                                            <textarea
                                                data-field="main"
                                                class="operator__slide-textarea operator__slide-textarea--main"
                                                prop:value=main_text.clone()
                                                on:blur={
                                                    let pres_id = pres_id_edit.clone();
                                                    let sid = slide_id_edit.clone();
                                                    move |ev| {
                                                        let val = event_target_value(&ev);
                                                        let pres_id = pres_id.clone();
                                                        let sid = sid.clone();
                                                        leptos::task::spawn_local(async move {
                                                            let ctx = use_context::<AppContext>().expect("AppContext");
                                                            let pres = ctx.selected_presentation.get_untracked();
                                                            if let Some(p) = &pres {
                                                                let slide = p.slides.iter().find(|s| s.id.to_string() == sid);
                                                                if let Some(s) = slide {
                                                                    let _ = api::presentations::update_slide(
                                                                        &pres_id, &sid,
                                                                        &val,
                                                                        s.content.translation.value(),
                                                                        s.content.stage.value(),
                                                                    ).await;
                                                                }
                                                            }
                                                        });
                                                    }
                                                }
                                                on:focus={
                                                    let sid = slide_id.clone();
                                                    move |_| {
                                                        let op = use_context::<OperatorState>().expect("OperatorState");
                                                        op.focused_slide_id.set(Some(sid.clone()));
                                                        op.focused_field.set(Some("main".to_string()));
                                                        crate::state::session::set("focusedSlideId", &sid);
                                                    }
                                                }
                                            />
                                            <textarea
                                                data-field="translation"
                                                class="operator__slide-textarea operator__slide-textarea--translation"
                                                prop:value=translation_text.clone()
                                                on:blur={
                                                    let pres_id = pres_id_edit.clone();
                                                    let sid = slide_id_edit.clone();
                                                    move |ev| {
                                                        let val = event_target_value(&ev);
                                                        let pres_id = pres_id.clone();
                                                        let sid = sid.clone();
                                                        leptos::task::spawn_local(async move {
                                                            let ctx = use_context::<AppContext>().expect("AppContext");
                                                            let pres = ctx.selected_presentation.get_untracked();
                                                            if let Some(p) = &pres {
                                                                let slide = p.slides.iter().find(|s| s.id.to_string() == sid);
                                                                if let Some(s) = slide {
                                                                    let _ = api::presentations::update_slide(
                                                                        &pres_id, &sid,
                                                                        s.content.main.value(),
                                                                        &val,
                                                                        s.content.stage.value(),
                                                                    ).await;
                                                                }
                                                            }
                                                        });
                                                    }
                                                }
                                            />
                                            <textarea
                                                data-field="stage"
                                                class="operator__slide-textarea operator__slide-textarea--stage"
                                                prop:value=stage_text.clone()
                                                on:blur={
                                                    let pres_id = pres_id_edit.clone();
                                                    let sid = slide_id_edit.clone();
                                                    move |ev| {
                                                        let val = event_target_value(&ev);
                                                        let pres_id = pres_id.clone();
                                                        let sid = sid.clone();
                                                        leptos::task::spawn_local(async move {
                                                            let ctx = use_context::<AppContext>().expect("AppContext");
                                                            let pres = ctx.selected_presentation.get_untracked();
                                                            if let Some(p) = &pres {
                                                                let slide = p.slides.iter().find(|s| s.id.to_string() == sid);
                                                                if let Some(s) = slide {
                                                                    let _ = api::presentations::update_slide(
                                                                        &pres_id, &sid,
                                                                        s.content.main.value(),
                                                                        s.content.translation.value(),
                                                                        &val,
                                                                    ).await;
                                                                }
                                                            }
                                                        });
                                                    }
                                                }
                                            />
                                            <div class="operator__slide-actions">
                                                <button
                                                    data-action="duplicate"
                                                    class="operator__slide-action-btn"
                                                    on:click=move |_| {
                                                        let pres_id = pres_id_dup.clone();
                                                        let sid = slide_id_dup.clone();
                                                        leptos::task::spawn_local(async move {
                                                            if let Ok(slides) = api::presentations::duplicate_slide(&pres_id, &sid).await {
                                                                let ctx = use_context::<AppContext>().expect("AppContext");
                                                                ctx.selected_presentation.update(|p| {
                                                                    if let Some(pres) = p.as_mut() { pres.slides = slides; }
                                                                });
                                                            }
                                                        });
                                                    }
                                                >
                                                    "Duplicate"
                                                </button>
                                                <button
                                                    data-action="delete-slide"
                                                    class="operator__slide-action-btn operator__slide-action-btn--danger"
                                                    on:click=move |_| {
                                                        let pres_id = pres_id_del.clone();
                                                        let sid = slide_id_del.clone();
                                                        leptos::task::spawn_local(async move {
                                                            if let Ok(slides) = api::presentations::delete_slide(&pres_id, &sid).await {
                                                                let ctx = use_context::<AppContext>().expect("AppContext");
                                                                ctx.selected_presentation.update(|p| {
                                                                    if let Some(pres) = p.as_mut() { pres.slides = slides; }
                                                                });
                                                            }
                                                        });
                                                    }
                                                >
                                                    "Delete"
                                                </button>
                                            </div>
                                        </div>
                                    }.into_any()
                                }}
                            </div>
                        }
                    }).collect_view().into_any()
                }}
            </div>
        </div>
    }
}
