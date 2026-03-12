use leptos::prelude::*;

use crate::api;
use crate::state::operator::OperatorState;
use crate::state::AppContext;

/// Format text with `<br>` for line breaks and highlight lines exceeding limit.
fn format_multiline(text: &str, limit: u32) -> String {
    text.lines()
        .map(|line| {
            let escaped = html_escape(line);
            if limit > 0 && line.len() as u32 > limit {
                format!("<span class=\"operator__slide-line-over\">{escaped}</span>")
            } else {
                escaped
            }
        })
        .collect::<Vec<_>>()
        .join("<br>")
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn field_has_warning(text: &str, limit: u32) -> bool {
    limit > 0 && text.lines().any(|line| line.len() as u32 > limit)
}

fn slide_has_any_warning(main: &str, translation: &str, stage: &str, limit: u32) -> bool {
    field_has_warning(main, limit)
        || field_has_warning(translation, limit)
        || field_has_warning(stage, limit)
}

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
                        prop:value=move || op.line_limit.get().to_string()
                        on:input=on_line_limit_change
                    />
                </label>
                <button
                    type="button"
                    class="operator__slides-add"
                    data-role="add-slide"
                    title="Add slide"
                    on:click=add_slide
                >
                    "+"
                </button>
            </div>
            <div class="operator__slides" data-role="slides">
                {move || {
                    let mode = ctx.mode.get();
                    let pres = ctx.selected_presentation.get();
                    let snapshot = ctx.stage_snapshot.get();
                    let line_limit = op.line_limit.get();

                    let Some(presentation) = pres else {
                        return view! { <p class="empty">"Select a presentation to load slides."</p> }.into_any();
                    };

                    let pres_id = presentation.id.to_string();
                    let slides = presentation.slides.clone();
                    let current_slide_id = snapshot.as_ref().and_then(|s| s.current_slide_id.map(|id| id.to_string()));
                    let is_live = mode == "live";
                    let is_edit = !is_live;

                    let mut current_group: Option<String> = None;

                    slides.into_iter().enumerate().map(|(i, slide)| {
                        let slide_id = slide.id.to_string();
                        let main_text = slide.content.main.value().to_string();
                        let translation_text = slide.content.translation.value().to_string();
                        let stage_text = slide.content.stage.value().to_string();
                        let group_name = slide.content.group.as_ref().map(|g| g.name().to_string());

                        let group_inherited = if group_name != current_group {
                            current_group.clone_from(&group_name);
                            false
                        } else {
                            group_name.is_some()
                        };

                        let show_group = if !group_inherited {
                            group_name.clone()
                        } else {
                            None
                        };

                        let is_active = current_slide_id.as_deref() == Some(&slide_id);
                        let main_warning = field_has_warning(&main_text, line_limit);
                        let translation_warning = field_has_warning(&translation_text, line_limit);
                        let stage_warning = field_has_warning(&stage_text, line_limit);
                        let any_warning = slide_has_any_warning(&main_text, &translation_text, &stage_text, line_limit);

                        // Format text with HTML for live mode display
                        let main_html = format_multiline(&main_text, line_limit);
                        let translation_html = format_multiline(&translation_text, line_limit);
                        let stage_html = format_multiline(&stage_text, line_limit);

                        let pres_id_click = pres_id.clone();
                        let slide_id_click = slide_id.clone();
                        let next_slide_id = presentation.slides.get(i + 1).map(|s| s.id.to_string());

                        let pres_id_edit = pres_id.clone();
                        let slide_id_edit = slide_id.clone();
                        let pres_id_dup = pres_id.clone();
                        let slide_id_dup = slide_id.clone();
                        let pres_id_del = pres_id.clone();
                        let slide_id_del = slide_id.clone();
                        let slide_id_for_article = slide_id.clone();
                        let slide_index = i;

                        let group_display = group_name.clone().unwrap_or_default();

                        view! {
                            {show_group.map(|g| view! {
                                <div class="operator__slide-group" data-role="slide-group">{g}</div>
                            })}
                            <article
                                class=move || {
                                    let mut c = "operator__slide-card stage-control__slide".to_string();
                                    if is_active { c.push_str(" is-active"); }
                                    c
                                }
                                data-slide-id=slide_id_for_article
                                data-slide-index=slide_index
                                attr:data-group-inherited=if group_inherited { "true" } else { "false" }
                            >
                                <header class="operator__slide-header">
                                    <div class="operator__slide-header-left">
                                        <span class="operator__slide-index">
                                            {i + 1}
                                            {any_warning.then(|| view! {
                                                <sup>"!"</sup>
                                            })}
                                        </span>
                                    </div>
                                    {is_edit.then(|| {
                                        let pres_id_save = pres_id_edit.clone();
                                        let slide_id_save = slide_id_edit.clone();
                                        view! {
                                            <div class="operator__slide-controls">
                                                <button type="button" data-action="save"
                                                    on:click=move |_| {
                                                        let pres_id = pres_id_save.clone();
                                                        let sid = slide_id_save.clone();
                                                        leptos::task::spawn_local(async move {
                                                            let ctx = use_context::<AppContext>().expect("AppContext");
                                                            let p = ctx.selected_presentation.get_untracked();
                                                            if let Some(p) = &p {
                                                                if let Some(s) = p.slides.iter().find(|s| s.id.to_string() == sid) {
                                                                    let _ = api::presentations::update_slide(
                                                                        &pres_id, &sid,
                                                                        s.content.main.value(),
                                                                        s.content.translation.value(),
                                                                        s.content.stage.value(),
                                                                    ).await;
                                                                }
                                                            }
                                                        });
                                                    }
                                                >"Save"</button>
                                                <button type="button" data-action="duplicate"
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
                                                >"Duplicate"</button>
                                                <button type="button" data-action="delete"
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
                                                >"Delete"</button>
                                            </div>
                                        }
                                    })}
                                </header>
                                <section class="operator__slide-bodies">
                                    {if is_live {
                                        let trigger = trigger_slide;
                                        view! {
                                            <div
                                                class="operator__slide-text operator__slide-text--main"
                                                data-field-display="main"
                                                attr:data-warning=if main_warning { "true" } else { "false" }
                                                inner_html=main_html
                                                on:click=move |_| {
                                                    trigger(pres_id_click.clone(), slide_id_click.clone(), next_slide_id.clone());
                                                }
                                            >
                                            </div>
                                            {(!translation_text.is_empty()).then(|| view! {
                                                <div
                                                    class="operator__slide-text operator__slide-text--translation"
                                                    data-field-display="translation"
                                                    attr:data-warning=if translation_warning { "true" } else { "false" }
                                                    inner_html=translation_html
                                                >
                                                </div>
                                            })}
                                            {(!stage_text.is_empty()).then(|| view! {
                                                <div
                                                    class="operator__slide-text operator__slide-text--stage"
                                                    data-field-display="stage"
                                                    attr:data-warning=if stage_warning { "true" } else { "false" }
                                                    inner_html=stage_html
                                                >
                                                </div>
                                            })}
                                            <div class="operator__slide-warning" data-role="slide-warning"
                                                attr:data-visible=move || if any_warning { "true" } else { "false" }
                                            >
                                                {format!("Line exceeds {line_limit} characters")}
                                            </div>
                                        }.into_any()
                                    } else {
                                        view! {
                                            <div
                                                class="operator__slide-text operator__slide-text--main"
                                                data-field-display="main"
                                                attr:data-warning=if main_warning { "true" } else { "false" }
                                                inner_html=main_html.clone()
                                            >
                                            </div>
                                            {(!translation_text.is_empty()).then(|| view! {
                                                <div
                                                    class="operator__slide-text operator__slide-text--translation"
                                                    data-field-display="translation"
                                                    attr:data-warning=if translation_warning { "true" } else { "false" }
                                                    inner_html=translation_html.clone()
                                                >
                                                </div>
                                            })}
                                            {(!stage_text.is_empty()).then(|| view! {
                                                <div
                                                    class="operator__slide-text operator__slide-text--stage"
                                                    data-field-display="stage"
                                                    attr:data-warning=if stage_warning { "true" } else { "false" }
                                                    inner_html=stage_html.clone()
                                                >
                                                </div>
                                            })}
                                            <div class="operator__slide-warning" data-role="slide-warning"
                                                attr:data-visible=move || if any_warning { "true" } else { "false" }
                                            >
                                                {format!("Line exceeds {line_limit} characters")}
                                            </div>
                                            <div class="operator__slide-editor">
                                                <label>
                                                    <span>"Main"</span>
                                                    <textarea
                                                        data-field="main"
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
                                                </label>
                                                <label>
                                                    <span>"Translation"</span>
                                                    <textarea
                                                        data-field="translation"
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
                                                </label>
                                                <label>
                                                    <span>"Stage"</span>
                                                    <textarea
                                                        data-field="stage"
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
                                                </label>
                                                <label>
                                                    <span>"Group"</span>
                                                    <input
                                                        type="text"
                                                        data-field="group"
                                                        prop:value=group_display
                                                    />
                                                </label>
                                            </div>
                                        }.into_any()
                                    }}
                                </section>
                            </article>
                        }
                    }).collect_view().into_any()
                }}
            </div>
        </section>
    }
}
