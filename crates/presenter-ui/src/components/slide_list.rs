use std::collections::HashMap;

use leptos::prelude::*;
use presenter_core::{resolve_sequence, ResolvedSlide};
use wasm_bindgen::JsCast;

use crate::api;
use crate::state::operator::{OperatorState, SaveStatus};
use crate::state::AppContext;
use crate::utils::color::group_pill_style;

use super::slide_list_scroll::{handle_wheel_event, scroll_slide_into_view, scroll_slides_to_top};
use super::slide_list_utils::{
    apply_focused_class, field_has_warning, format_multiline, is_interactive_tag,
    resolve_group_display, slide_has_any_warning,
};
use super::slide_reorder::{handle_slide_drop, render_song_bubble};
use super::slide_save::{
    capture_selection, reconcile_after_seq_mismatch, save_all_fields_from_dom,
};
use super::slide_selection::{
    cut_memo, render_insertion_bar, render_select_checkbox, selected_memo,
    setup_clipboard_keyboard, setup_paste_chooser_clear_on_live,
    setup_selection_clear_on_switch, SlideSelectionPanel,
};

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

    // #515: per-slide stage-layout markers of the open presentation
    // (slide_id → layout_code), maintained by the picker module (clears on
    // presentation switch + stale-response guard).
    let slide_stage_markers =
        crate::components::slide_stage_layout_picker::use_slide_stage_markers(&ctx);

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

    // #554: clear selection/clipboard on song switch + the window C/X/V/Escape
    // shortcut listener. F1: also disarm the paste-target chooser on a
    // switch to live mode (Escape/Clear can't reach it there — see
    // `setup_paste_chooser_clear_on_live`). All one-time setups.
    setup_selection_clear_on_switch(ctx.clone(), op.clone());
    setup_clipboard_keyboard(ctx.clone(), op.clone());
    setup_paste_chooser_clear_on_live(ctx.clone(), op.clone());

    let trigger_slide = move |pres_id: String, slide_id: String, next_slide_id: Option<String>| {
        let playlist_id = ctx.selected_playlist_id.get_untracked();
        // #496: send the selected playlist-entry index so the server marks the
        // triggered occurrence of a repeated song active (None outside a playlist).
        let entry_index = ctx.selected_entry_index.get_untracked();
        op.triggering_slide_id.set(Some(slide_id.clone()));
        let triggering_signal = op.triggering_slide_id;
        leptos::task::spawn_local(async move {
            let _ = api::stage::update_state(&api::stage::StageStateRequest {
                presentation_id: pres_id,
                current_slide_id: slide_id,
                next_slide_id,
                playlist_id,
                entry_index,
            })
            .await;
            triggering_signal.set(None);
        });
    };

    let add_slide = move |_: web_sys::MouseEvent| {
        let pres = ctx.selected_presentation.get_untracked();
        if let Some(p) = pres {
            let pres_id = p.id.to_string();
            let pres_id_for_reconcile = pres_id.clone();
            let selected_presentation_signal = ctx.selected_presentation;
            // #556 F3: bump slide_edit_seq the same way every other
            // slide-structure mutation does, so a slower concurrent
            // reorder/edit response can't silently resurrect stale
            // structure once this insert lands.
            op.slide_edit_seq.update(|seq| *seq += 1);
            let my_seq = op.slide_edit_seq.get_untracked();
            let slide_edit_seq = op.slide_edit_seq;
            let save_status = op.save_status;
            leptos::task::spawn_local(async move {
                if let Ok(slides) = api::presentations::insert_slide(&pres_id, None).await {
                    if slide_edit_seq.get_untracked() == my_seq {
                        selected_presentation_signal.update(|p| {
                            if let Some(pres) = p.as_mut() {
                                pres.slides = slides;
                            }
                        });
                    } else {
                        reconcile_after_seq_mismatch(
                            pres_id_for_reconcile,
                            slides,
                            selected_presentation_signal,
                            slide_edit_seq,
                            save_status,
                        )
                        .await;
                    }
                }
            });
        }
    };

    view! {
        <section class="operator__slides-column">
            <div class="operator__slides-area">
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
                <SlideSelectionPanel />
                {
                    // Clone op for each handler that moves it into a closure
                    let op_dragover = op.clone();
                    let op_drop = op.clone();
                    let op_bubble = op.clone();
                    let choosing_paste_target_for_class = op.choosing_paste_target;
                    view! {
                        <div
                            // #554/#602: single column while ACTIVELY CHOOSING a
                            // paste target so every inter-slide gap is one
                            // unambiguous full-width "paste here" bar. Bound to
                            // `choosing_paste_target`, NOT to "a clipboard
                            // exists" (#602 defect 3) — a completed Copy-paste
                            // deliberately keeps the clipboard non-empty for a
                            // re-paste, which used to leave this grid stuck in
                            // single-column forever.
                            class=move || {
                                if choosing_paste_target_for_class.get() {
                                    "operator__slides operator__slides--clipboard"
                                } else {
                                    "operator__slides"
                                }
                            }
                            data-role="slides"
                            on:wheel=handle_wheel_event
                        on:dragover=move |ev: web_sys::DragEvent| {
                            if op_dragover.dragging_slide_id.get_untracked().is_some() {
                                ev.prevent_default();
                            }
                        }
                        // #552/#556 F1/F4: the whole guarded reorder round-trip
                        // (failure badge, slide_edit_seq staleness guard,
                        // order-merge reconciliation, in-flight-typing flush +
                        // focus restore) lives in `slide_reorder::handle_slide_drop`.
                        on:drop=move |ev: web_sys::DragEvent| {
                            handle_slide_drop(ev, ctx.selected_presentation, &op_drop);
                        }
                    >
                    // #556 F5: the song bubble is now an IN-FLOW, sticky first
                    // child of this same scrolling grid (see the
                    // `.operator__slides-bubble` / `.operator__slides` CSS)
                    // instead of an absolutely-positioned overlay pinned to a
                    // fixed screen position. A fixed overlay only ever
                    // protected whatever slide happened to render at that
                    // exact spot (row 1, at scrollTop=0) — every OTHER row
                    // that later scrolled to that same physical position was
                    // silently covered, stealing its drag handle. Being a
                    // real grid item that reserves its own row means no
                    // slide card is ever laid out underneath it, regardless
                    // of scroll position.
                    {render_song_bubble(ctx.clone(), op_bubble.clone())}
                    {
                    // Clone op + ctx inside the block so we can use them in
                    // nested closures (#554 needs the whole ctx per card).
                    let op = op.clone();
                    let ctx = ctx.clone();
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
                    // #554/#602: the ONE tracked read in this render closure —
                    // actively CHOOSING a paste target re-renders the list
                    // with "paste here" bars in every gap (deliberate,
                    // infrequent, never racing in-flight typing: shortcuts are
                    // guarded and the panel buttons blur+save first). Bound to
                    // `choosing_paste_target`, not `clipboard.is_some()`, so
                    // the bars disappear the moment a paste completes even
                    // when Copy deliberately keeps the clipboard alive.
                    let clipboard_active = op.choosing_paste_target.get();
                    let slide_count = raw_slides.len();

                    let mut prev_effective: Option<String> = None;

                    let cards = raw_slides
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

                        // Edit-mode group input's OWN value (empty if this
                        // slide doesn't have an explicit group) — the user's
                        // own typed content, tied to the initial DOM render;
                        // it never needs to react to a DIFFERENT slide's
                        // group cascade.
                        let group_edit_value = explicit_group_name.clone().unwrap_or_default();

                        // #556 F2: the header badge text/inherited-flag and
                        // the Group input's placeholder (ghost text showing
                        // what this slide would inherit) DO depend on the
                        // whole presentation's group cascade, which can
                        // change from ANOTHER slide's save without this
                        // per-slide-list render pass re-running at all — the
                        // Group save path is deliberately untracked (see
                        // `slide_save.rs`). Recompute both from a fresh
                        // `resolve_group_display` call inside a small
                        // fine-grained Memo, gated on `groups_version`
                        // (bumped after every save) — the same isolation
                        // trick this file already uses for `stage_snapshot`.
                        let sid_for_group_display = slide_id.clone();
                        let selected_pres_for_group = ctx.selected_presentation;
                        let groups_version = op.groups_version;
                        let group_display = Memo::new(move |_| {
                            let _ = groups_version.get();
                            selected_pres_for_group
                                .read_untracked()
                                .as_ref()
                                .map(|p| resolve_group_display(p, &sid_for_group_display))
                                .unwrap_or_default()
                        });

                        // is_active is now computed reactively in the class= closure
                        // using ctx.stage_snapshot.get() directly (see class closure below)
                        let is_focused = focused_slide.as_deref() == Some(&slide_id);
                        let main_warning = field_has_warning(&main_text, line_limit);
                        let translation_warning = field_has_warning(&translation_text, line_limit);
                        // #515: the stage field is free-form speaker/reading
                        // text (it feeds the fullscreen "stage" layout) and
                        // is explicitly EXEMPT from the lyrics per-line
                        // character-limit warning — it may run longer than
                        // a projected lyric line.
                        let stage_warning = false;
                        let any_warning =
                            slide_has_any_warning(&main_text, &translation_text, line_limit);

                        // Format text with HTML for live mode display. The
                        // stage field never gets the overflow highlight
                        // (limit 0 disables it) for the same reason.
                        let main_html = format_multiline(&main_text, line_limit);
                        let translation_html = format_multiline(&translation_text, line_limit);
                        let stage_html = format_multiline(&stage_text, 0);

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

                        // #554: per-card selection/cut markers — fine-grained
                        // Memos so checking a box never rebuilds the list.
                        let selected_marker = selected_memo(&op, slide_id.clone());
                        let cut_marker = cut_memo(&op, slide_id.clone());

                        // Clone for save-status badge in slide header
                        let slide_id_for_badge = slide_id.clone();

                        // Clones for the per-slide stage-layout control (#515)
                        let pres_id_for_marker = pres_id.clone();
                        let slide_id_for_marker = slide_id.clone();

                        // #554: a "paste here" bar before this card while the
                        // clipboard is active (gap i = before slide i).
                        let insertion_bar =
                            clipboard_active.then(|| render_insertion_bar(ctx.clone(), op.clone(), i));

                        view! {
                            {insertion_bar}
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
                                    // #554: selection + cut marking.
                                    if selected_marker.get() {
                                        c.push_str(" operator__slide-card--selected");
                                    }
                                    if cut_marker.get() {
                                        c.push_str(" operator__slide-card--cut");
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
                                                if is_interactive_tag(&tag) {
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
                                                if is_interactive_tag(&tag) {
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
                                        // #554: multi-select checkbox (edit mode only)
                                        {is_edit.then(|| render_select_checkbox(
                                            ctx.clone(),
                                            op.clone(),
                                            slide_id.clone(),
                                            i,
                                        ))}
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
                                    {
                                        // #556 F2: reactive — see `group_display` above.
                                        // `<Show>`'s fallback renders nothing at all
                                        // (same as the old `Option::map` returning
                                        // `None`), preserving "no badge for a slide
                                        // with no group" exactly.
                                        view! {
                                            <Show when=move || group_display.get().badge_text.is_some() fallback=|| ()>
                                                {move || {
                                                    let display = group_display.get();
                                                    let g = display.badge_text.unwrap_or_default();
                                                    let color_style = group_colors.get()
                                                        .get(&g)
                                                        .map(|c| group_pill_style(c))
                                                        .unwrap_or_default();
                                                    let class = if display.badge_inherited {
                                                        "operator__slide-group operator__slide-group--inherited"
                                                    } else {
                                                        "operator__slide-group"
                                                    };
                                                    view! {
                                                        <span class=class style=color_style data-role="slide-group">{g}</span>
                                                    }
                                                }}
                                            </Show>
                                        }
                                    }
                                    // #515: per-slide stage-layout marker —
                                    // selector in edit mode, badge in live mode.
                                    <crate::components::slide_stage_layout_picker::SlideStageLayoutControl
                                        pres_id=pres_id_for_marker
                                        slide_id=slide_id_for_marker
                                        is_edit=is_edit
                                        markers=slide_stage_markers
                                    />
                                    {
                                        // Memo: derives this slide's status from the shared map.
                                        // The body runs whenever any slide saves, but the Memo
                                        // only NOTIFIES subscribers when THIS slide's value
                                        // changes — so `<Show>` and its render closure only
                                        // re-evaluate when our own status changes.
                                        let sid_for_memo = slide_id_for_badge.clone();
                                        let save_status = op.save_status;
                                        let badge_status = Memo::new(move |_| {
                                            save_status.get().get(&sid_for_memo).map(|(s, _)| *s)
                                        });
                                        view! {
                                            <Show when=move || badge_status.get().is_some()>
                                                {move || {
                                                    let Some(status) = badge_status.get() else {
                                                        return view! { <span></span> }.into_any();
                                                    };
                                                    match status {
                                                        SaveStatus::Saving => view! {
                                                            <span
                                                                class="operator__slide-save-indicator"
                                                                data-role="slide-save-indicator"
                                                                data-status="saving"
                                                            >"Saving…"</span>
                                                        }.into_any(),
                                                        SaveStatus::Saved => view! {
                                                            <span
                                                                class="operator__slide-save-indicator"
                                                                data-role="slide-save-indicator"
                                                                data-status="saved"
                                                            >"Saved ✓"</span>
                                                        }.into_any(),
                                                        SaveStatus::Failed => view! {
                                                            <span
                                                                class="operator__slide-save-indicator"
                                                                data-role="slide-save-indicator"
                                                                data-status="failed"
                                                            >"Save failed"</span>
                                                        }.into_any(),
                                                    }
                                                }}
                                            </Show>
                                        }
                                    }
                                    {is_edit.then(|| {
                                        let selected_pres_dup = ctx.selected_presentation;
                                        let selected_pres_del = ctx.selected_presentation;
                                        let slide_edit_seq_dup = op.slide_edit_seq;
                                        let slide_edit_seq_del = op.slide_edit_seq;
                                        let save_status_dup = op.save_status;
                                        let save_status_del = op.save_status;
                                        view! {
                                            <div class="operator__slide-controls">
                                                <button type="button" data-action="duplicate"
                                                    on:click=move |_| {
                                                        let pres_id = pres_id_dup.clone();
                                                        let pres_id_for_reconcile = pres_id.clone();
                                                        let sid = slide_id_dup.clone();
                                                        let selected_pres = selected_pres_dup;
                                                        // #556 F3: bump slide_edit_seq + apply the same
                                                        // guard+merge policy as the reorder response (F1),
                                                        // so a slower concurrent reorder/edit response can't
                                                        // silently resurrect stale structure once this
                                                        // duplicate lands.
                                                        slide_edit_seq_dup.update(|seq| *seq += 1);
                                                        let my_seq = slide_edit_seq_dup.get_untracked();
                                                        leptos::task::spawn_local(async move {
                                                            if let Ok(slides) = api::presentations::duplicate_slide(&pres_id, &sid).await {
                                                                if slide_edit_seq_dup.get_untracked() == my_seq {
                                                                    selected_pres.update(|p| {
                                                                        if let Some(pres) = p.as_mut() { pres.slides = slides; }
                                                                    });
                                                                } else {
                                                                    reconcile_after_seq_mismatch(pres_id_for_reconcile, slides, selected_pres, slide_edit_seq_dup, save_status_dup).await;
                                                                }
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
                                                        let pres_id_for_reconcile = pres_id.clone();
                                                        let sid = slide_id_del.clone();
                                                        let selected_pres = selected_pres_del;
                                                        // #556 F3: same seq bump + guard+merge as duplicate.
                                                        slide_edit_seq_del.update(|seq| *seq += 1);
                                                        let my_seq = slide_edit_seq_del.get_untracked();
                                                        leptos::task::spawn_local(async move {
                                                            if let Ok(slides) = api::presentations::delete_slide(&pres_id, &sid).await {
                                                                if slide_edit_seq_del.get_untracked() == my_seq {
                                                                    selected_pres.update(|p| {
                                                                        if let Some(pres) = p.as_mut() { pres.slides = slides; }
                                                                    });
                                                                } else {
                                                                    reconcile_after_seq_mismatch(pres_id_for_reconcile, slides, selected_pres, slide_edit_seq_del, save_status_del).await;
                                                                }
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
                                                        on:input=move |_ev| {
                                                                // #515: stage text is free-form — it never
                                                                // contributes a line-limit warning, so
                                                                // stage_warn_sig stays at its initial `false`.
                                                                let m = main_warn_sig.get_untracked();
                                                                let t = trans_warn_sig.get_untracked();
                                                                any_warn_sig.set(m || t);
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
                                                        // CRITICAL #8 fix: Show inherited group as placeholder.
                                                        // #556 F2: reactive — see `group_display` above, so an
                                                        // earlier slide's group save updates this placeholder
                                                        // live instead of only after a reload.
                                                        placeholder=move || group_display.get().edit_placeholder
                                                        on:blur={
                                                            let pres_id = pres_id_edit.clone();
                                                            let sid = slide_id_edit.clone();
                                                            let op = op.clone();
                                                            move |ev| {
                                                                let (sel_start, sel_end) = capture_selection(&ev);
                                                                // #552/#553: this used to be a bespoke path that did a
                                                                // full presentation refetch + tracked `.set()` on every
                                                                // save (forcing a full slide-list rebuild) and a manual
                                                                // async focus-restore to fight the resulting teardown —
                                                                // which lost the race badly enough to freeze the page
                                                                // (any click elsewhere re-blurred the just-refocused
                                                                // input, retriggering the same cycle forever). Use the
                                                                // same unified, untracked save path as every other
                                                                // field: no full rebuild, so no focus to restore, and
                                                                // its slide_edit_seq guard now also protects group edits
                                                                // from a stale response overwriting a newer one.
                                                                save_all_fields_from_dom(&pres_id, &sid, "group", sel_start, sel_end, ctx.selected_presentation, &op);
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
                    }).collect_view();

                    // #554: the trailing gap (after the last slide).
                    let trailing_bar = clipboard_active
                        .then(|| render_insertion_bar(ctx.clone(), op.clone(), slide_count));
                    view! {
                        {cards}
                        {trailing_bar}
                    }
                    .into_any()
                    }}
                    </div>
                    }
                }
            </div>
        </section>
    }
}
