use std::collections::HashMap;

use leptos::prelude::*;
use presenter_core::{resolve_sequence, ResolvedSlide};
use wasm_bindgen::JsCast;

use crate::api;
use crate::state::operator::OperatorState;
use crate::state::AppContext;
use crate::utils::color::group_pill_style;

use super::slide_list_scroll::{handle_wheel_event, scroll_slide_into_view, scroll_slides_to_top};
use super::slide_list_utils::{
    apply_focused_class, field_has_warning, format_multiline, reorder_slide_ids,
    slide_has_any_warning,
};

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
        // Check mode before restoring - only restore in edit mode
        if crate::state::session::get("mode").as_deref() != Some("edit") {
            op.pending_focus.set(None);
            return;
        }
        // Check modal not open
        let doc = crate::utils::window::document();
        if doc
            .body()
            .and_then(|b| b.get_attribute("data-modal-open"))
            .is_some()
        {
            return;
        }

        let op = op.clone();
        // Use requestAnimationFrame for proper timing after DOM updates
        let closure = wasm_bindgen::closure::Closure::once(Box::new(move || {
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
        }) as Box<dyn FnOnce()>);
        let window = crate::utils::window::window();
        let _ = window.request_animation_frame(closure.as_ref().unchecked_ref());
        closure.forget();
    }
}

/// Unified save function that saves ALL fields from DOM atomically.
/// This prevents data loss when editing multiple fields before blur.
/// Takes `RwSignal` (which is `Copy`) instead of `&AppContext` to allow
/// use in multiple `move` closures without moving the entire context.
fn save_all_fields_from_dom(
    pres_id: &str,
    slide_id: &str,
    _current_field: &str,
    _sel_start: u32,
    _sel_end: u32,
    selected_pres: RwSignal<Option<presenter_core::Presentation>>,
    _op: &OperatorState,
) {
    let doc = crate::utils::window::document();

    // Get ALL field values from the DOM (not from signals which may be stale)
    let main = get_field_value(&doc, slide_id, "main");
    let translation = get_field_value(&doc, slide_id, "translation");
    let stage = get_field_value(&doc, slide_id, "stage");
    let group_val = get_field_value(&doc, slide_id, "group");
    let group = if group_val.trim().is_empty() {
        None
    } else {
        Some(group_val.trim().to_string())
    };

    // Compare to signal to skip no-op saves
    let pres = selected_pres.get_untracked();
    if let Some(p) = &pres {
        if let Some(slide) = p.slides.iter().find(|s| s.id.to_string() == slide_id) {
            let orig = &slide.content;
            let orig_group = orig.group.as_ref().map(|g| g.name().to_string());
            if orig.main.value() == main
                && orig.translation.value() == translation
                && orig.stage.value() == stage
                && orig_group == group
            {
                return;
            }
        }
    }

    let pres_id = pres_id.to_string();
    let sid = slide_id.to_string();

    leptos::task::spawn_local(async move {
        // Save all fields atomically. Do NOT update the selected_pres signal
        // after save — that triggers a Leptos re-render which recreates textarea
        // elements with prop:value from the signal, destroying any in-progress
        // DOM edits the user is making in another field. The signal data becomes
        // stale but is refreshed on presentation switch or page reload.
        // Do NOT call restore_pending_focus — it refocuses the blurred field,
        // which then triggers another blur on whatever field Playwright/user
        // moved to, creating a race condition with concurrent saves.
        let _ = api::presentations::update_slide_with_group(
            &pres_id,
            &sid,
            &main,
            &translation,
            &stage,
            group.clone(),
        )
        .await;
    });
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

/// Renders the draggable song bubble above the slide list.
/// Returns a reactive closure that shows the bubble when a presentation is
/// selected, or nothing otherwise.
fn render_song_bubble(ctx: AppContext, op: OperatorState) -> impl IntoView {
    move || {
        let presentation = ctx.selected_presentation.get();
        let Some(pres) = presentation else {
            return "".into_any();
        };
        let pres_id = pres.id.to_string();
        let pres_id_drag = pres_id.clone();
        let pres_name = pres.name.clone();
        let op_drag = op.clone();
        let op_end = op.clone();
        view! {
            <div
                class="operator__slides-bubble"
                data-role="slides-song-bubble"
                data-presentation-id=pres_id
                draggable="true"
                title="Drag into a playlist"
                on:dragstart=move |ev: web_sys::DragEvent| {
                    if let Some(dt) = ev.data_transfer() {
                        let _ = dt.set_data("text/plain", &pres_id_drag);
                        let _ = dt.set_data("application/x-presentation-id", &pres_id_drag);
                        dt.set_effect_allowed("copy");
                    }
                    op_drag.search_dragging.set(true);
                    op_drag.dragging_from_search.set(true);
                }
                on:dragend=move |_| {
                    op_end.search_dragging.set(false);
                    op_end.dragging_from_search.set(false);
                }
            >
                <span class="operator__slides-bubble-name">{pres_name}</span>
            </div>
        }
        .into_any()
    }
}

#[component]
pub fn SlideList() -> impl IntoView {
    let ctx = use_ctx!(AppContext);
    let op = use_ctx!(OperatorState);

    let group_colors = RwSignal::new(HashMap::<String, String>::new());
    {
        leptos::task::spawn_local(async move {
            if let Ok(colors) = api::presentations::fetch_group_colors().await {
                group_colors.set(colors);
            }
        });
    }

    // Scroll active slide into view when stage's current slide changes.
    {
        let stage_snapshot = ctx.stage_snapshot;
        Effect::new(move |prev_id: Option<Option<String>>| {
            let current_id = stage_snapshot
                .get()
                .and_then(|s| s.current_slide_id.map(|id| id.to_string()));
            if current_id != prev_id.flatten() {
                if let Some(ref slide_id) = current_id {
                    let slide_id = slide_id.clone();
                    gloo_timers::callback::Timeout::new(0, move || {
                        scroll_slide_into_view(&slide_id);
                    })
                    .forget();
                }
            }
            current_id
        });
    }

    // Scroll to top when the operator opens a different presentation.
    // Issue #271 concern 3: new song should load with the first slide
    // visible, not at the previous song's scroll position.
    {
        let selected_presentation_id = ctx.selected_presentation_id;
        Effect::new(move |prev_id: Option<Option<String>>| {
            let current_id = selected_presentation_id.get();
            if current_id != prev_id.flatten() && current_id.is_some() {
                gloo_timers::callback::Timeout::new(0, scroll_slides_to_top).forget();
            }
            current_id
        });
    }

    let trigger_slide = move |pres_id: String, slide_id: String, next_slide_id: Option<String>| {
        let playlist_id = ctx.selected_playlist_id.get_untracked();
        op.triggering_slide_id.set(Some(slide_id.clone()));
        let triggering_signal = op.triggering_slide_id;
        leptos::task::spawn_local(async move {
            let _ = api::stage::update_state(&api::stage::StageStateRequest {
                presentation_id: pres_id,
                current_slide_id: slide_id,
                next_slide_id,
                playlist_id,
            })
            .await;
            triggering_signal.set(None);
        });
    };

    let add_slide = move |_: web_sys::MouseEvent| {
        let pres = ctx.selected_presentation.get_untracked();
        if let Some(p) = pres {
            let pres_id = p.id.to_string();
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

    view! {
        <section class="operator__slides-column">
            <div class="operator__slides-area">
                {render_song_bubble(ctx.clone(), op.clone())}
                <Show
                    when=move || ctx.selected_presentation.with(|p| p.is_some())
                    fallback=|| ()
                >
                    <button
                        type="button"
                        class="operator__slides-add-floating"
                        data-role="add-slide"
                        title="Add slide"
                        on:click=add_slide
                    >
                        "+"
                    </button>
                </Show>
                {
                    // Clone op for each handler that moves it into a closure
                    let op_dragover = op.clone();
                    let op_drop = op.clone();
                    view! {
                        <div
                            class="operator__slides"
                            data-role="slides"
                            on:wheel=handle_wheel_event
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
                                            if let Some(p) = ctx.selected_presentation.get_untracked() {
                                                let pres_id = p.id.to_string();
                                                let ids: Vec<String> = p.slides.iter().map(|s| s.id.to_string()).collect();
                                                if let Some(new_ids) = reorder_slide_ids(ids, &dragged_id, &target_id) {
                                                    let selected_pres = ctx.selected_presentation;
                                                    leptos::task::spawn_local(async move {
                                                        if let Ok(slides) = api::presentations::reorder_slides(&pres_id, new_ids).await {
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
                    // NOTE: stage_snapshot is NOT read here (no .get()) to prevent
                    // slide save from triggering full re-render. Slide save calls
                    // broadcast_stage_snapshots() on server → LiveEvent::Stage via WS
                    // → stage_snapshot.set(). Each slide card reads stage_snapshot
                    // reactively in its class= closure, which only updates the class
                    // attribute without destroying textarea DOM elements.
                    let line_limit = op.line_limit.get();
                    // Use get_untracked() so focused_slide_id changes do NOT trigger
                    // full slide list re-render. The is-focused class is applied
                    // via DOM manipulation in on:focus handlers instead.
                    let focused_slide = op.focused_slide_id.get_untracked();

                    let Some(presentation) = pres else {
                        return view! { <p class="empty">"Select a presentation to load slides."</p> }.into_any();
                    };

                    let pres_id = presentation.id.to_string();
                    let raw_slides = presentation.slides.clone();
                    let resolved: Vec<ResolvedSlide> = resolve_sequence(&raw_slides);
                    let is_live = mode == "live";
                    let is_edit = !is_live;

                    let mut prev_effective: Option<String> = None;

                    raw_slides
                        .iter()
                        .cloned()
                        .zip(resolved.into_iter())
                        .enumerate()
                        .map(|(i, (raw_slide, resolved_slide))| {
                        let slide_id = resolved_slide.id.to_string();
                        let main_text = resolved_slide.main.value().to_string();
                        let translation_text = resolved_slide.translation.value().to_string();
                        let stage_text = resolved_slide.stage.value().to_string();

                        // The explicit group for this slide (None if inherited).
                        let explicit_group_name = raw_slide
                            .content
                            .group
                            .as_ref()
                            .map(|g| g.name().to_string());

                        // The effective (inherited or explicit) group for display.
                        let effective_group_name = resolved_slide
                            .effective_group
                            .as_ref()
                            .map(|g| g.name().to_string());

                        // Is this slide the first one showing this effective group?
                        let group_is_new = effective_group_name != prev_effective;
                        prev_effective = effective_group_name.clone();
                        let group_inherited =
                            effective_group_name.is_some() && !group_is_new;

                        // Header badge: render the effective group; dim if inherited.
                        // Suppress entirely for "true blank" slides — empty main AND no
                        // explicit group — so empty bookend slides created by the paste
                        // pipeline (#275) render as truly empty rather than picking up
                        // an inherited badge from the previous section.
                        let group_badge_text =
                            if main_text.is_empty() && explicit_group_name.is_none() {
                                None
                            } else {
                                effective_group_name.clone()
                            };
                        let group_badge_inherited = group_inherited;

                        // Edit-mode group input:
                        // - value = explicit group (empty if this slide doesn't have one)
                        // - placeholder = effective group (shows what would be inherited)
                        let group_edit_value = explicit_group_name.clone().unwrap_or_default();
                        let group_edit_placeholder = if explicit_group_name.is_none() {
                            effective_group_name.clone().unwrap_or_default()
                        } else {
                            String::new()
                        };

                        // is_active is now computed reactively in the class= closure
                        // using ctx.stage_snapshot.get() directly (see class closure below)
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

                        // Clone for class closure (is-loading check)
                        let slide_id_class = slide_id.clone();

                        view! {
                            <article
                                class=move || {
                                    let mut c = "operator__slide-card operator__slide-card--worship".to_string();
                                    // Read stage_snapshot reactively HERE (in class closure)
                                    // so it only updates this element's class, not the entire view.
                                    let snap = ctx.stage_snapshot.get();
                                    let active_id = snap.as_ref().and_then(|s| s.current_slide_id.map(|id| id.to_string()));
                                    if active_id.as_deref() == Some(&slide_id_class) {
                                        c.push_str(" is-active");
                                    }
                                    // is-focused is managed via DOM in apply_focused_class()
                                    if is_focused { c.push_str(" is-focused"); }
                                    // Add is-loading class during trigger operation
                                    if op.triggering_slide_id.get().as_deref() == Some(&slide_id_class) {
                                        c.push_str(" is-loading");
                                    }
                                    c
                                }
                                data-slide-id=slide_id_for_article.clone()
                                data-slide-index=slide_index
                                data-group-inherited=if group_inherited { "true" } else { "false" }
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
                                    {group_badge_text.clone().map(|g| {
                                        let color_style = group_colors.get()
                                            .get(&g)
                                            .map(|c| group_pill_style(c))
                                            .unwrap_or_default();
                                        let class = if group_badge_inherited {
                                            "operator__slide-group operator__slide-group--inherited"
                                        } else {
                                            "operator__slide-group"
                                        };
                                        view! {
                                            <span class=class style=color_style data-role="slide-group">{g}</span>
                                        }
                                    })}
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
                                                data-warning=if main_warning { "true" } else { "false" }
                                                inner_html=main_html
                                            >
                                            </div>
                                            {(!translation_text.is_empty()).then(|| view! {
                                                <div
                                                    class="operator__slide-text operator__slide-text--translation"
                                                    data-field-display="translation"
                                                    data-warning=if translation_warning { "true" } else { "false" }
                                                    inner_html=translation_html
                                                >
                                                </div>
                                            })}
                                            {(!stage_text.is_empty()).then(|| view! {
                                                <div
                                                    class="operator__slide-text operator__slide-text--stage"
                                                    data-field-display="stage"
                                                    data-warning=if stage_warning { "true" } else { "false" }
                                                    inner_html=stage_html
                                                >
                                                </div>
                                            })}
                                            <div class="operator__slide-warning" data-role="slide-warning"
                                                data-visible=move || if any_warning { "true" } else { "false" }
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
                                                data-warning=move || if main_warn_sig.get() { "true" } else { "false" }
                                                inner_html=main_html.clone()
                                            >
                                            </div>
                                            {(!translation_text.is_empty()).then(|| view! {
                                                <div
                                                    class="operator__slide-text operator__slide-text--translation"
                                                    data-field-display="translation"
                                                    data-warning=move || if trans_warn_sig.get() { "true" } else { "false" }
                                                    inner_html=translation_html.clone()
                                                >
                                                </div>
                                            })}
                                            {(!stage_text.is_empty()).then(|| view! {
                                                <div
                                                    class="operator__slide-text operator__slide-text--stage"
                                                    data-field-display="stage"
                                                    data-warning=move || if stage_warn_sig.get() { "true" } else { "false" }
                                                    inner_html=stage_html.clone()
                                                >
                                                </div>
                                            })}
                                            <div class="operator__slide-warning" data-role="slide-warning"
                                                data-visible=move || if any_warn_sig.get() { "true" } else { "false" }
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
                                                            let op = op.clone();
                                                            move |ev| {
                                                                let (sel_start, sel_end) = capture_selection(&ev);
                                                                // Use unified save that gets ALL fields from DOM
                                                                save_all_fields_from_dom(&pres_id, &sid, "main", sel_start, sel_end, ctx.selected_presentation, &op);
                                                            }
                                                        }
                                                        on:focus={
                                                            let sid = slide_id.clone();
                                                            move |_| {
                                                                op.focused_slide_id.set(Some(sid.clone()));
                                                                op.focused_field.set(Some("main".to_string()));
                                                                crate::state::session::set("focusedSlideId", &sid);
                                                                crate::state::session::set("focusedField", "main");
                                                                // Apply is-focused class via DOM since focused_slide_id
                                                                // uses get_untracked() to avoid re-renders
                                                                apply_focused_class(&sid);
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
                                                            let op = op.clone();
                                                            move |ev| {
                                                                let (sel_start, sel_end) = capture_selection(&ev);
                                                                // Use unified save that gets ALL fields from DOM
                                                                save_all_fields_from_dom(&pres_id, &sid, "translation", sel_start, sel_end, ctx.selected_presentation, &op);
                                                            }
                                                        }
                                                        on:focus={
                                                            let sid = slide_id.clone();
                                                            move |_| {
                                                                op.focused_slide_id.set(Some(sid.clone()));
                                                                op.focused_field.set(Some("translation".to_string()));
                                                                crate::state::session::set("focusedSlideId", &sid);
                                                                crate::state::session::set("focusedField", "translation");
                                                                apply_focused_class(&sid);
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
                                                            let op = op.clone();
                                                            move |ev| {
                                                                let (sel_start, sel_end) = capture_selection(&ev);
                                                                // Use unified save that gets ALL fields from DOM
                                                                save_all_fields_from_dom(&pres_id, &sid, "stage", sel_start, sel_end, ctx.selected_presentation, &op);
                                                            }
                                                        }
                                                        on:focus={
                                                            let sid = slide_id.clone();
                                                            move |_| {
                                                                op.focused_slide_id.set(Some(sid.clone()));
                                                                op.focused_field.set(Some("stage".to_string()));
                                                                crate::state::session::set("focusedSlideId", &sid);
                                                                crate::state::session::set("focusedField", "stage");
                                                                apply_focused_class(&sid);
                                                            }
                                                        }
                                                    />
                                                </label>
                                                <label>
                                                    <span>"Group"</span>
                                                    <input
                                                        type="text"
                                                        data-field="group"
                                                        prop:value=group_edit_value.clone()
                                                        // CRITICAL #8 fix: Show inherited group as placeholder
                                                        placeholder=group_edit_placeholder
                                                        on:blur={
                                                            let pres_id = pres_id_edit.clone();
                                                            let sid = slide_id_edit.clone();
                                                            let op = op.clone();
                                                            let selected_pres = ctx.selected_presentation;
                                                            move |ev| {
                                                                let (sel_start, sel_end) = capture_selection(&ev);
                                                                // For group changes, we need to refetch full presentation
                                                                // to update group inheritance across all slides
                                                                op.pending_focus.set(Some((sid.clone(), "group".to_string(), sel_start, sel_end)));

                                                                let doc = crate::utils::window::document();
                                                                let main = get_field_value(&doc, &sid, "main");
                                                                let translation = get_field_value(&doc, &sid, "translation");
                                                                let stage = get_field_value(&doc, &sid, "stage");
                                                                let group_val = get_field_value(&doc, &sid, "group");
                                                                let group = if group_val.trim().is_empty() { None } else { Some(group_val.trim().to_string()) };

                                                                let pres_id = pres_id.clone();
                                                                let sid = sid.clone();
                                                                let op = op.clone();
                                                                leptos::task::spawn_local(async move {
                                                                    let _ = api::presentations::update_slide_with_group(
                                                                        &pres_id, &sid,
                                                                        &main, &translation, &stage, group,
                                                                    ).await;
                                                                    // Refetch to update group inheritance display
                                                                    if let Ok(detail) = api::presentations::get_presentation(&pres_id).await {
                                                                        selected_pres.set(Some(detail.presentation));
                                                                    }
                                                                    restore_pending_focus(&op);
                                                                });
                                                            }
                                                        }
                                                        on:focus={
                                                            let sid = slide_id.clone();
                                                            move |_| {
                                                                op.focused_slide_id.set(Some(sid.clone()));
                                                                op.focused_field.set(Some("group".to_string()));
                                                                crate::state::session::set("focusedSlideId", &sid);
                                                                crate::state::session::set("focusedField", "group");
                                                                apply_focused_class(&sid);
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
            </div>
        </section>
    }
}
