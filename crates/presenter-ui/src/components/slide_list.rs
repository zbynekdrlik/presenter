use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::api;
use crate::state::operator::OperatorState;
use crate::state::AppContext;

/// Format text with `<br>` for line breaks and highlight lines exceeding limit.
fn format_multiline(text: &str, limit: u32) -> String {
    text.lines()
        .map(|line| {
            let escaped = html_escape(line);
            if limit > 0 && line.len() as u32 > limit {
                format!("<span class=\"operator__slide-overflow\">{escaped}</span>")
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

/// Get a textarea/input value from the DOM by slide_id and field name.
fn get_field_value(doc: &web_sys::Document, slide_id: &str, field: &str) -> String {
    let selector = format!(
        "[data-slide-id=\"{}\"] [data-field=\"{}\"]",
        slide_id, field
    );
    if let Ok(Some(el)) = doc.query_selector(&selector) {
        if let Ok(ta) = el.clone().dyn_into::<web_sys::HtmlTextAreaElement>() {
            return ta.value();
        }
        if let Ok(inp) = el.dyn_into::<web_sys::HtmlInputElement>() {
            return inp.value();
        }
    }
    String::new()
}

/// Restore focus to a field after save/re-render.
fn restore_pending_focus(op: &OperatorState) {
    if let Some((slide_id, field, sel_start, sel_end)) = op.pending_focus.get_untracked() {
        let op = op.clone();
        // Use a small timeout to let the DOM update
        gloo_timers::callback::Timeout::new(0, move || {
            let doc = crate::utils::window::document();
            let selector = format!(
                "[data-slide-id=\"{}\"] [data-field=\"{}\"]",
                slide_id, field
            );
            if let Ok(Some(el)) = doc.query_selector(&selector) {
                if let Ok(ta) = el.clone().dyn_into::<web_sys::HtmlTextAreaElement>() {
                    let _ = ta.focus();
                    let _ = ta.set_selection_range(sel_start, sel_end);
                } else if let Ok(inp) = el.dyn_into::<web_sys::HtmlInputElement>() {
                    let _ = inp.focus();
                    let _ = inp.set_selection_range(sel_start, sel_end);
                }
            }
            op.pending_focus.set(None);
        })
        .forget();
    }
}

/// Capture current selection range from a textarea event.
fn capture_selection(ev: &web_sys::Event) -> (u32, u32) {
    if let Some(target) = ev.target() {
        if let Ok(ta) = target.clone().dyn_into::<web_sys::HtmlTextAreaElement>() {
            let start = ta.selection_start().ok().flatten().unwrap_or(0);
            let end = ta.selection_end().ok().flatten().unwrap_or(0);
            return (start, end);
        }
        if let Ok(inp) = target.dyn_into::<web_sys::HtmlInputElement>() {
            let start = inp.selection_start().ok().flatten().unwrap_or(0);
            let end = inp.selection_end().ok().flatten().unwrap_or(0);
            return (start, end);
        }
    }
    (0, 0)
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
            // Capture signal OUTSIDE async block
            let selected_presentation_signal = ctx.selected_presentation;
            leptos::task::spawn_local(async move {
                if let Ok(slides) = api::presentations::insert_slide(&pres_id, None).await {
                    selected_presentation_signal.update(|p| {
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
            // Use persistent storage so setting survives tab close
            crate::state::session::set_persistent("lineLimit", &n.to_string());
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
            {
                // Clone op for each handler that moves it into a closure
                let op_dragover = op.clone();
                let op_drop = op.clone();
                view! {
                    <div
                        class="operator__slides"
                        data-role="slides"
                        on:dragover=move |ev: web_sys::DragEvent| {
                            if op_dragover.dragging_slide_id.get_untracked().is_some() {
                                ev.prevent_default();
                            }
                        }
                        on:drop=move |ev: web_sys::DragEvent| {
                            ev.prevent_default();
                            if let Some(dragged_id) = op_drop.dragging_slide_id.get_untracked() {
                                if let Some(target) = ev.target() {
                                    if let Ok(el) = target.dyn_into::<web_sys::Element>() {
                                        if let Some(card) = el.closest("[data-slide-id]").ok().flatten() {
                                            let target_id = card.get_attribute("data-slide-id").unwrap_or_default();
                                            if target_id != dragged_id {
                                                let pres = ctx.selected_presentation.get_untracked();
                                                if let Some(p) = pres {
                                                    let pres_id = p.id.to_string();
                                                    let mut slide_ids: Vec<String> = p.slides.iter().map(|s| s.id.to_string()).collect();
                                                    if let Some(drag_pos) = slide_ids.iter().position(|id| id == &dragged_id) {
                                                        slide_ids.remove(drag_pos);
                                                    }
                                                    if let Some(target_pos) = slide_ids.iter().position(|id| id == &target_id) {
                                                        slide_ids.insert(target_pos, dragged_id);
                                                    }
                                                    let selected_pres = ctx.selected_presentation;
                                                    leptos::task::spawn_local(async move {
                                                        if let Ok(slides) = api::presentations::reorder_slides(&pres_id, slide_ids).await {
                                                            selected_pres.update(|p| {
                                                                if let Some(pres) = p.as_mut() {
                                                                    pres.slides = slides;
                                                                }
                                                            });
                                                        }
                                                    });
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            op_drop.dragging_slide_id.set(None);
                        }
                    >
                    {
                    // Clone op inside the block so we can use it in nested closures
                    let op = op.clone();
                    move || {
                    let mode = ctx.mode.get();
                    let pres = ctx.selected_presentation.get();
                    let snapshot = ctx.stage_snapshot.get();
                    let line_limit = op.line_limit.get();
                    let focused_slide = op.focused_slide_id.get();

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

                        // Track inherited vs explicit group for placeholder
                        let inherited_group = if group_name.is_none() {
                            current_group.clone()
                        } else {
                            None
                        };

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
                        let is_focused = focused_slide.as_deref() == Some(&slide_id);
                        let main_warning = field_has_warning(&main_text, line_limit);
                        let translation_warning = field_has_warning(&translation_text, line_limit);
                        let stage_warning = field_has_warning(&stage_text, line_limit);
                        let any_warning = slide_has_any_warning(&main_text, &translation_text, &stage_text, line_limit);

                        // Format text with HTML for live mode display
                        let main_html = format_multiline(&main_text, line_limit);
                        let translation_html = format_multiline(&translation_text, line_limit);
                        let stage_html = format_multiline(&stage_text, line_limit);

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
                        let group_placeholder = inherited_group.clone().unwrap_or_default();

                        let trigger = trigger_slide;

                        // Clone for drag
                        let slide_id_drag = slide_id.clone();

                        // Clone for on:click handler
                        let pres_id_click = pres_id.clone();
                        let slide_id_click = slide_id.clone();
                        let next_slide_click = next_slide_id.clone();

                        // Clone for on:pointerdown handler
                        let pres_id_pointer = pres_id.clone();
                        let slide_id_pointer = slide_id.clone();
                        let next_slide_pointer = next_slide_id.clone();

                        view! {
                            {show_group.map(|g| view! {
                                <div class="operator__slide-group" data-role="slide-group">{g}</div>
                            })}
                            <article
                                class=move || {
                                    let mut c = "operator__slide-card stage-control__slide".to_string();
                                    if is_active { c.push_str(" is-active"); }
                                    // BLOCKER #2 fix: Add is-focused class for edit mode visibility
                                    if is_focused { c.push_str(" is-focused"); }
                                    c
                                }
                                data-slide-id=slide_id_for_article.clone()
                                data-slide-index=slide_index
                                attr:data-group-inherited=if group_inherited { "true" } else { "false" }
                                on:click={
                                    let slide_id = slide_id_click.clone();
                                    let pres_id_trigger = pres_id_click.clone();
                                    let slide_id_trigger = slide_id_click.clone();
                                    let next_slide_trigger = next_slide_click.clone();
                                    let op = op.clone();
                                    move |ev: web_sys::MouseEvent| {
                                        // Skip if click target is an interactive element
                                        if let Some(target) = ev.target() {
                                            if let Ok(el) = target.dyn_into::<web_sys::Element>() {
                                                let tag = el.tag_name().to_lowercase();
                                                if tag == "button" || tag == "textarea" || tag == "input" {
                                                    return;
                                                }
                                                if el.get_attribute("data-action").is_some() {
                                                    return;
                                                }
                                            }
                                        }

                                        // CRITICAL #4 fix: Always update focused_slide_id (both modes)
                                        op.focused_slide_id.set(Some(slide_id.clone()));
                                        crate::state::session::set("focusedSlideId", &slide_id);

                                        // Only trigger in live mode
                                        if is_live {
                                            // CRITICAL #3: Skip-click-trigger debounce check
                                            if let Some((skip_id, expires)) = op.skip_click_trigger.get_untracked() {
                                                if skip_id == slide_id && js_sys::Date::now() < expires {
                                                    return;
                                                }
                                            }
                                            trigger(pres_id_trigger.clone(), slide_id_trigger.clone(), next_slide_trigger.clone());
                                        }
                                    }
                                }
                                on:pointerdown={
                                    let slide_id = slide_id_pointer.clone();
                                    let pres_id = pres_id_pointer.clone();
                                    let next_slide = next_slide_pointer.clone();
                                    let op = op.clone();
                                    move |ev: web_sys::PointerEvent| {
                                        // Skip if not left click
                                        if ev.button() != 0 { return; }
                                        // Skip interactive elements
                                        if let Some(target) = ev.target() {
                                            if let Ok(el) = target.dyn_into::<web_sys::Element>() {
                                                let tag = el.tag_name().to_lowercase();
                                                if tag == "button" || tag == "textarea" || tag == "input" {
                                                    return;
                                                }
                                            }
                                        }

                                        // In live mode, set skip trigger and trigger immediately
                                        if is_live {
                                            let now = js_sys::Date::now();
                                            op.skip_click_trigger.set(Some((slide_id.clone(), now + 250.0)));
                                            trigger(pres_id.clone(), slide_id.clone(), next_slide.clone());
                                        }
                                    }
                                }
                            >
                                <header class="operator__slide-header">
                                    <div class="operator__slide-header-left">
                                        // BLOCKER #5: Drag handle for reordering
                                        {is_edit.then(|| {
                                            let slide_id = slide_id_drag.clone();
                                            let op_start = op.clone();
                                            let op_end = op.clone();
                                            view! {
                                                <button
                                                    type="button"
                                                    class="operator__slide-handle"
                                                    data-role="slide-drag-handle"
                                                    draggable="true"
                                                    tabindex="-1"
                                                    aria-label="Reorder slide"
                                                    on:dragstart=move |ev: web_sys::DragEvent| {
                                                        if let Some(dt) = ev.data_transfer() {
                                                            let _ = dt.set_data("application/x-slide-id", &slide_id);
                                                            dt.set_effect_allowed("move");
                                                        }
                                                        op_start.dragging_slide_id.set(Some(slide_id.clone()));
                                                    }
                                                    on:dragend=move |_| {
                                                        op_end.dragging_slide_id.set(None);
                                                    }
                                                >
                                                    "\u{2195}"
                                                </button>
                                            }
                                        })}
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
                                        // Capture signal OUTSIDE async blocks
                                        let selected_pres_save = ctx.selected_presentation;
                                        let selected_pres_dup = ctx.selected_presentation;
                                        let selected_pres_del = ctx.selected_presentation;
                                        view! {
                                            <div class="operator__slide-controls">
                                                <button type="button" data-action="save"
                                                    on:click=move |_| {
                                                        let pres_id = pres_id_save.clone();
                                                        let sid = slide_id_save.clone();
                                                        let selected_pres = selected_pres_save;
                                                        leptos::task::spawn_local(async move {
                                                            let p = selected_pres.get_untracked();
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
                                                        let selected_pres = selected_pres_dup;
                                                        leptos::task::spawn_local(async move {
                                                            if let Ok(slides) = api::presentations::duplicate_slide(&pres_id, &sid).await {
                                                                selected_pres.update(|p| {
                                                                    if let Some(pres) = p.as_mut() { pres.slides = slides; }
                                                                });
                                                            }
                                                        });
                                                    }
                                                >"Duplicate"</button>
                                                <button type="button" data-action="delete"
                                                    on:click=move |_| {
                                                        // CRITICAL #1: Add delete confirmation
                                                        let window = crate::utils::window::window();
                                                        let confirmed = window.confirm_with_message(&format!("Delete slide {}?", slide_index + 1)).unwrap_or(false);
                                                        if !confirmed { return; }

                                                        let pres_id = pres_id_del.clone();
                                                        let sid = slide_id_del.clone();
                                                        let selected_pres = selected_pres_del;
                                                        leptos::task::spawn_local(async move {
                                                            if let Ok(slides) = api::presentations::delete_slide(&pres_id, &sid).await {
                                                                selected_pres.update(|p| {
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
                                        view! {
                                            <div
                                                class="operator__slide-text operator__slide-text--main"
                                                data-field-display="main"
                                                attr:data-warning=if main_warning { "true" } else { "false" }
                                                inner_html=main_html
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
                                        // Create reactive warning signals for real-time updates
                                        let main_warn_sig = RwSignal::new(main_warning);
                                        let trans_warn_sig = RwSignal::new(translation_warning);
                                        let stage_warn_sig = RwSignal::new(stage_warning);
                                        let any_warn_sig = RwSignal::new(any_warning);

                                        view! {
                                            <div
                                                class="operator__slide-text operator__slide-text--main"
                                                data-field-display="main"
                                                attr:data-warning=move || if main_warn_sig.get() { "true" } else { "false" }
                                                inner_html=main_html.clone()
                                            >
                                            </div>
                                            {(!translation_text.is_empty()).then(|| view! {
                                                <div
                                                    class="operator__slide-text operator__slide-text--translation"
                                                    data-field-display="translation"
                                                    attr:data-warning=move || if trans_warn_sig.get() { "true" } else { "false" }
                                                    inner_html=translation_html.clone()
                                                >
                                                </div>
                                            })}
                                            {(!stage_text.is_empty()).then(|| view! {
                                                <div
                                                    class="operator__slide-text operator__slide-text--stage"
                                                    data-field-display="stage"
                                                    attr:data-warning=move || if stage_warn_sig.get() { "true" } else { "false" }
                                                    inner_html=stage_html.clone()
                                                >
                                                </div>
                                            })}
                                            <div class="operator__slide-warning" data-role="slide-warning"
                                                attr:data-visible=move || if any_warn_sig.get() { "true" } else { "false" }
                                            >
                                                {format!("Line exceeds {line_limit} characters")}
                                            </div>
                                            <div class="operator__slide-editor">
                                                <label>
                                                    <span>"Main"</span>
                                                    <textarea
                                                        data-field="main"
                                                        rows="2"
                                                        prop:value=main_text.clone()
                                                        on:input=move |ev| {
                                                                // CRITICAL #2: Real-time warning update
                                                                let val = event_target_value(&ev);
                                                                main_warn_sig.set(field_has_warning(&val, line_limit));
                                                                let t = trans_warn_sig.get_untracked();
                                                                let s = stage_warn_sig.get_untracked();
                                                                any_warn_sig.set(main_warn_sig.get_untracked() || t || s);
                                                            }
                                                        on:blur={
                                                            let pres_id = pres_id_edit.clone();
                                                            let sid = slide_id_edit.clone();
                                                            let selected_pres = ctx.selected_presentation;
                                                            let op = op.clone();
                                                            move |ev| {
                                                                let (sel_start, sel_end) = capture_selection(&ev);
                                                                // BLOCKER #3: Set pending focus before save
                                                                op.pending_focus.set(Some((sid.clone(), "main".to_string(), sel_start, sel_end)));

                                                                let val = event_target_value(&ev);
                                                                let pres_id = pres_id.clone();
                                                                let sid = sid.clone();
                                                                let op = op.clone();
                                                                leptos::task::spawn_local(async move {
                                                                    let pres = selected_pres.get_untracked();
                                                                    if let Some(p) = &pres {
                                                                        let slide = p.slides.iter().find(|s| s.id.to_string() == sid);
                                                                        if let Some(s) = slide {
                                                                            let _ = api::presentations::update_slide(
                                                                                &pres_id, &sid,
                                                                                &val,
                                                                                s.content.translation.value(),
                                                                                s.content.stage.value(),
                                                                            ).await;
                                                                            // Restore focus after save
                                                                            restore_pending_focus(&op);
                                                                        }
                                                                    }
                                                                });
                                                            }
                                                        }
                                                        on:focus={
                                                            let sid = slide_id.clone();
                                                            move |_| {
                                                                op.focused_slide_id.set(Some(sid.clone()));
                                                                op.focused_field.set(Some("main".to_string()));
                                                                crate::state::session::set("focusedSlideId", &sid);
                                                                crate::state::session::set("focusedField", "main");
                                                            }
                                                        }
                                                    />
                                                </label>
                                                <label>
                                                    <span>"Translation"</span>
                                                    <textarea
                                                        data-field="translation"
                                                        rows="2"
                                                        prop:value=translation_text.clone()
                                                        on:input=move |ev| {
                                                                let val = event_target_value(&ev);
                                                                trans_warn_sig.set(field_has_warning(&val, line_limit));
                                                                let m = main_warn_sig.get_untracked();
                                                                let s = stage_warn_sig.get_untracked();
                                                                any_warn_sig.set(m || trans_warn_sig.get_untracked() || s);
                                                            }
                                                        on:blur={
                                                            let pres_id = pres_id_edit.clone();
                                                            let sid = slide_id_edit.clone();
                                                            let selected_pres = ctx.selected_presentation;
                                                            let op = op.clone();
                                                            move |ev| {
                                                                let (sel_start, sel_end) = capture_selection(&ev);
                                                                op.pending_focus.set(Some((sid.clone(), "translation".to_string(), sel_start, sel_end)));

                                                                let val = event_target_value(&ev);
                                                                let pres_id = pres_id.clone();
                                                                let sid = sid.clone();
                                                                let op = op.clone();
                                                                leptos::task::spawn_local(async move {
                                                                    let pres = selected_pres.get_untracked();
                                                                    if let Some(p) = &pres {
                                                                        let slide = p.slides.iter().find(|s| s.id.to_string() == sid);
                                                                        if let Some(s) = slide {
                                                                            let _ = api::presentations::update_slide(
                                                                                &pres_id, &sid,
                                                                                s.content.main.value(),
                                                                                &val,
                                                                                s.content.stage.value(),
                                                                            ).await;
                                                                            restore_pending_focus(&op);
                                                                        }
                                                                    }
                                                                });
                                                            }
                                                        }
                                                        // BLOCKER #4 fix: Add on:focus to track field focus
                                                        on:focus={
                                                            let sid = slide_id.clone();
                                                            move |_| {
                                                                op.focused_slide_id.set(Some(sid.clone()));
                                                                op.focused_field.set(Some("translation".to_string()));
                                                                crate::state::session::set("focusedSlideId", &sid);
                                                                crate::state::session::set("focusedField", "translation");
                                                            }
                                                        }
                                                    />
                                                </label>
                                                <label>
                                                    <span>"Stage"</span>
                                                    <textarea
                                                        data-field="stage"
                                                        rows="2"
                                                        prop:value=stage_text.clone()
                                                        on:input=move |ev| {
                                                                let val = event_target_value(&ev);
                                                                stage_warn_sig.set(field_has_warning(&val, line_limit));
                                                                let m = main_warn_sig.get_untracked();
                                                                let t = trans_warn_sig.get_untracked();
                                                                any_warn_sig.set(m || t || stage_warn_sig.get_untracked());
                                                            }
                                                        on:blur={
                                                            let pres_id = pres_id_edit.clone();
                                                            let sid = slide_id_edit.clone();
                                                            let selected_pres = ctx.selected_presentation;
                                                            let op = op.clone();
                                                            move |ev| {
                                                                let (sel_start, sel_end) = capture_selection(&ev);
                                                                op.pending_focus.set(Some((sid.clone(), "stage".to_string(), sel_start, sel_end)));

                                                                let val = event_target_value(&ev);
                                                                let pres_id = pres_id.clone();
                                                                let sid = sid.clone();
                                                                let op = op.clone();
                                                                leptos::task::spawn_local(async move {
                                                                    let pres = selected_pres.get_untracked();
                                                                    if let Some(p) = &pres {
                                                                        let slide = p.slides.iter().find(|s| s.id.to_string() == sid);
                                                                        if let Some(s) = slide {
                                                                            let _ = api::presentations::update_slide(
                                                                                &pres_id, &sid,
                                                                                s.content.main.value(),
                                                                                s.content.translation.value(),
                                                                                &val,
                                                                            ).await;
                                                                            restore_pending_focus(&op);
                                                                        }
                                                                    }
                                                                });
                                                            }
                                                        }
                                                        // BLOCKER #4 fix: Add on:focus to track field focus
                                                        on:focus={
                                                            let sid = slide_id.clone();
                                                            move |_| {
                                                                op.focused_slide_id.set(Some(sid.clone()));
                                                                op.focused_field.set(Some("stage".to_string()));
                                                                crate::state::session::set("focusedSlideId", &sid);
                                                                crate::state::session::set("focusedField", "stage");
                                                            }
                                                        }
                                                    />
                                                </label>
                                                <label>
                                                    <span>"Group"</span>
                                                    <input
                                                        type="text"
                                                        data-field="group"
                                                        prop:value=group_display.clone()
                                                        // CRITICAL #8 fix: Show inherited group as placeholder
                                                        placeholder=group_placeholder
                                                        // BLOCKER #1 fix: Add blur handler to save group changes
                                                        on:blur={
                                                            let pres_id = pres_id_edit.clone();
                                                            let sid = slide_id_edit.clone();
                                                            let selected_pres = ctx.selected_presentation;
                                                            let op = op.clone();
                                                            move |ev| {
                                                                let (sel_start, sel_end) = capture_selection(&ev);
                                                                op.pending_focus.set(Some((sid.clone(), "group".to_string(), sel_start, sel_end)));

                                                                let group_val = event_target_value(&ev);
                                                                let pres_id = pres_id.clone();
                                                                let sid = sid.clone();
                                                                let op = op.clone();

                                                                // Get all field values from DOM to ensure we save current state
                                                                let doc = crate::utils::window::document();
                                                                let main = get_field_value(&doc, &sid, "main");
                                                                let translation = get_field_value(&doc, &sid, "translation");
                                                                let stage = get_field_value(&doc, &sid, "stage");

                                                                let group = if group_val.trim().is_empty() { None } else { Some(group_val.trim().to_string()) };

                                                                leptos::task::spawn_local(async move {
                                                                    let _ = api::presentations::update_slide_with_group(
                                                                        &pres_id, &sid,
                                                                        &main,
                                                                        &translation,
                                                                        &stage,
                                                                        group,
                                                                    ).await;
                                                                    // Refetch presentation to update group display
                                                                    if let Ok(detail) = api::presentations::get_presentation(&pres_id).await {
                                                                        selected_pres.set(Some(detail.presentation));
                                                                    }
                                                                    restore_pending_focus(&op);
                                                                });
                                                            }
                                                        }
                                                        // BLOCKER #4 fix: Add on:focus to track field focus
                                                        on:focus={
                                                            let sid = slide_id.clone();
                                                            move |_| {
                                                                op.focused_slide_id.set(Some(sid.clone()));
                                                                op.focused_field.set(Some("group".to_string()));
                                                                crate::state::session::set("focusedSlideId", &sid);
                                                                crate::state::session::set("focusedField", "group");
                                                            }
                                                        }
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
                }
            }
        </section>
    }
}
