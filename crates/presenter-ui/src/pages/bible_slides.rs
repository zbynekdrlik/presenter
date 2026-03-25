use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::api::bible::{self, BibleSlideDto};
use crate::api::presentations as pres_api;
use crate::state::bible::BibleState;
use crate::state::AppContext;

// ---------------------------------------------------------------------------
// Slides column
// ---------------------------------------------------------------------------

#[component]
pub fn BibleSlidesColumn() -> impl IntoView {
    let bs = use_ctx!(BibleState);
    let bible_tab = bs.bible_tab;

    view! {
        <section
            class="operator__slides-column operator__slides-column--minimal"
            data-role="slides-column"
        >
            <div class="operator__slides-toolbar operator__slides-toolbar--minimal">
                <AddEmptySlideButton />
            </div>
            <div class="operator__slides" data-role="slides">
                {move || {
                    let tab = bible_tab.get();
                    if tab == "prepared" {
                        view! { <PreparedSlides /> }.into_any()
                    } else {
                        view! { <LiveSlides /> }.into_any()
                    }
                }}
            </div>
        </section>
    }
}

#[component]
fn AddEmptySlideButton() -> impl IntoView {
    let bs = use_ctx!(BibleState);
    let ctx = use_ctx!(AppContext);

    let on_click = move |_| {
        let pres_id = bs.active_presentation_id.get_untracked();
        let Some(id) = pres_id else {
            ctx.show_toast("Select a presentation first", "error");
            return;
        };
        let active_slides = bs.active_presentation_slides;
        let toast_message = ctx.toast_message;
        let toast_variant = ctx.toast_variant;

        let input = bible::AppendSlideInput {
            main: String::new(),
            translation: String::new(),
            stage: String::new(),
            group: None,
            metadata: None,
        };

        leptos::task::spawn_local(async move {
            match bible::append_presentation_slides(&id, &[input]).await {
                Ok(detail) => {
                    active_slides.set(detail.slides);
                    toast_variant.set("success".to_string());
                    toast_message.set(Some("Added empty slide".to_string()));
                }
                Err(e) => {
                    toast_variant.set("error".to_string());
                    toast_message.set(Some(format!("Failed: {e}")));
                }
            }
        });
    };

    view! {
        <button
            type="button"
            class="operator__slides-add"
            data-role="add-empty-slide"
            title="Add empty slide to active presentation"
            on:click=on_click
        >"+"</button>
    }
}

#[component]
fn LiveSlides() -> impl IntoView {
    let bs = use_ctx!(BibleState);
    let slides = bs.slides;

    view! {
        {move || {
            let slide_list = slides.get();
            if slide_list.is_empty() {
                view! {
                    <p class="operator__slides-empty">"Load a passage to populate slides."</p>
                }.into_any()
            } else {
                slide_list.into_iter().map(|slide| {
                    view! { <LiveSlideCard slide=slide /> }
                }).collect_view().into_any()
            }
        }}
    }
}

#[component]
fn PreparedSlides() -> impl IntoView {
    let bs = use_ctx!(BibleState);
    let slides = bs.active_presentation_slides;

    view! {
        {move || {
            let slide_list = slides.get();
            if slide_list.is_empty() {
                view! {
                    <p class="operator__slides-empty">"Select a presentation to view slides."</p>
                }.into_any()
            } else {
                slide_list.into_iter().enumerate().map(|(idx, slide)| {
                    view! { <PreparedSlideCard slide=slide index=idx /> }
                }).collect_view().into_any()
            }
        }}
    }
}

/// Helper to build a TriggerSlideRequest from signal values (for edit mode accuracy).
fn build_trigger_request(
    main_text_sig: RwSignal<String>,
    main_ref_sig: RwSignal<String>,
    trans_text_sig: RwSignal<String>,
    trans_ref_sig: RwSignal<String>,
    slide: &BibleSlideDto,
    bs_translation: RwSignal<Option<String>>,
) -> bible::TriggerSlideRequest {
    let main_text = main_text_sig.get_untracked();
    let main_ref = {
        let r = main_ref_sig.get_untracked();
        if r.is_empty() {
            // Fall back to stage field (where AI puts the reference)
            slide.stage.clone()
        } else {
            r
        }
    };
    let translation_text = trans_text_sig.get_untracked();
    let trans_ref = trans_ref_sig.get_untracked();

    let meta = slide.metadata.as_ref().and_then(|m| m.bible.as_ref());
    bible::TriggerSlideRequest {
        main_text,
        main_reference: main_ref,
        secondary_text: if translation_text.is_empty() {
            None
        } else {
            Some(translation_text)
        },
        secondary_reference: if trans_ref.is_empty() {
            None
        } else {
            Some(trans_ref)
        },
        translation_code: meta
            .and_then(|m| m.translation_code.clone())
            .or_else(|| bs_translation.get_untracked()),
        book: meta.and_then(|m| m.book.clone()),
        book_code: meta.and_then(|m| m.book_code.clone()),
        book_number: meta.and_then(|m| m.book_number),
        chapter: meta.and_then(|m| m.chapter),
        verse_start: meta.and_then(|m| m.verse_start),
        verse_end: meta.and_then(|m| m.verse_end),
    }
}

/// Render the slide body content (shared between live and prepared cards).
/// Bible slides have 4 fields: Main, Translation, Main Reference, Translation Reference.
/// Group is a worship field — not used here.
fn slide_body_view(
    main: String,
    trans: String,
    main_ref: String,
    trans_ref: String,
) -> impl IntoView {
    view! {
        <section class="operator__slide-bodies operator__slide-bodies--bible">
            <div class="operator__slide-text operator__slide-text--main">{main}</div>
            {if !trans.is_empty() {
                Some(view! {
                    <div class="operator__slide-text operator__slide-text--translation operator__slide-text--secondary">{trans}</div>
                })
            } else {
                None
            }}
            {if !main_ref.is_empty() || !trans_ref.is_empty() {
                Some(view! {
                    <footer class="operator__slide-footer">
                        {if !main_ref.is_empty() {
                            Some(view! { <span class="operator__slide-reference">{main_ref}</span> })
                        } else {
                            None
                        }}
                        {if !trans_ref.is_empty() {
                            Some(view! { <span class="operator__slide-reference operator__slide-reference--secondary">{trans_ref}</span> })
                        } else {
                            None
                        }}
                    </footer>
                })
            } else {
                None
            }}
        </section>
    }
}

/// Shared editor section for Bible slide cards (Main, Translation, Main Reference, Translation Reference).
/// Used by both LiveSlideCard and PreparedSlideCard to ensure one consistent editor layout.
fn bible_slide_editor_view(
    main_text_sig: RwSignal<String>,
    trans_text_sig: RwSignal<String>,
    main_ref_sig: RwSignal<String>,
    trans_ref_sig: RwSignal<String>,
    on_blur: impl Fn(web_sys::FocusEvent) + Clone + 'static,
) -> impl IntoView {
    let on_blur2 = on_blur.clone();
    let on_blur3 = on_blur.clone();
    let on_blur4 = on_blur.clone();
    view! {
        <section class="operator__slide-editor operator__slide-editor--bible">
            <label>
                <span>"Main"</span>
                <textarea
                    data-field="main"
                    data-role="slide-main-edit"
                    rows="2"
                    prop:value=move || main_text_sig.get()
                    on:input=move |ev: web_sys::Event| {
                        let target = ev.target().and_then(|t| t.dyn_into::<web_sys::HtmlTextAreaElement>().ok());
                        if let Some(ta) = target { main_text_sig.set(ta.value()); }
                    }
                    on:blur={
                        let on_blur = on_blur.clone();
                        move |ev| on_blur(ev)
                    }
                ></textarea>
            </label>
            <label>
                <span>"Translation"</span>
                <textarea
                    data-field="translation"
                    data-role="slide-translation-edit"
                    rows="2"
                    prop:value=move || trans_text_sig.get()
                    on:input=move |ev: web_sys::Event| {
                        let target = ev.target().and_then(|t| t.dyn_into::<web_sys::HtmlTextAreaElement>().ok());
                        if let Some(ta) = target { trans_text_sig.set(ta.value()); }
                    }
                    on:blur={
                        let on_blur = on_blur2.clone();
                        move |ev| on_blur(ev)
                    }
                ></textarea>
            </label>
            <div class="operator__slide-editor-grid">
                <label>
                    <span>"Main Reference"</span>
                    <input type="text"
                        data-field="main-ref"
                        data-role="slide-main-ref"
                        prop:value=move || main_ref_sig.get()
                        on:input=move |ev: web_sys::Event| {
                            let target = ev.target().and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok());
                            if let Some(el) = target { main_ref_sig.set(el.value()); }
                        }
                        on:blur={
                            let on_blur = on_blur3.clone();
                            move |ev| on_blur(ev)
                        }
                    />
                </label>
                <label>
                    <span>"Translation Reference"</span>
                    <input type="text"
                        data-field="translation-ref"
                        data-role="slide-translation-ref"
                        prop:value=move || trans_ref_sig.get()
                        on:input=move |ev: web_sys::Event| {
                            let target = ev.target().and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok());
                            if let Some(el) = target { trans_ref_sig.set(el.value()); }
                        }
                        on:blur={
                            let on_blur = on_blur4.clone();
                            move |ev| on_blur(ev)
                        }
                    />
                </label>
            </div>
        </section>
    }
}

/// Trigger handler shared between live and prepared cards.
fn make_trigger_handler(
    ctx: &AppContext,
    slide: &BibleSlideDto,
    main_text_sig: RwSignal<String>,
    main_ref_sig: RwSignal<String>,
    trans_text_sig: RwSignal<String>,
    trans_ref_sig: RwSignal<String>,
    bs_translation: RwSignal<Option<String>>,
) -> impl Fn(web_sys::MouseEvent) + Clone + 'static {
    let ctx = ctx.clone();
    let slide = slide.clone();
    move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        let req = build_trigger_request(
            main_text_sig,
            main_ref_sig,
            trans_text_sig,
            trans_ref_sig,
            &slide,
            bs_translation,
        );

        let toast_message = ctx.toast_message;
        let toast_variant = ctx.toast_variant;
        let active_broadcast = ctx.active_bible_broadcast;
        leptos::task::spawn_local(async move {
            match bible::trigger_slide(&req).await {
                Ok(_) => {
                    toast_variant.set("success".to_string());
                    toast_message.set(Some("Triggered".to_string()));
                    if let Ok(broadcast) = bible::get_broadcast().await {
                        active_broadcast.set(broadcast);
                    }
                }
                Err(e) => {
                    toast_variant.set("error".to_string());
                    toast_message.set(Some(format!("Trigger failed: {e}")));
                }
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Live slide card — broadcast trigger + selection checkboxes
// ---------------------------------------------------------------------------

#[component]
fn LiveSlideCard(slide: BibleSlideDto) -> impl IntoView {
    let bs = use_ctx!(BibleState);
    let ctx = use_ctx!(AppContext);
    let mode = ctx.mode;

    let slide_id = slide.id.clone();
    let main_ref = slide.main_reference.clone().unwrap_or_default();
    let trans_ref_initial = slide.translation_reference.clone().unwrap_or_default();

    let main_text_sig = RwSignal::new(slide.main.clone());
    let trans_text_sig = RwSignal::new(slide.translation.clone());
    let group_label_sig = RwSignal::new(slide.group.clone().unwrap_or_default());
    let main_ref_sig = RwSignal::new(main_ref.clone());
    let trans_ref_sig = RwSignal::new(trans_ref_initial);

    let is_selected = {
        let sid = slide_id.clone();
        move || bs.selected_slide_ids.get().contains(&sid)
    };
    let is_selected_for_checkbox = {
        let sid = slide_id.clone();
        move || bs.selected_slide_ids.get().contains(&sid)
    };

    let on_trigger = make_trigger_handler(
        &ctx,
        &slide,
        main_text_sig,
        main_ref_sig,
        trans_text_sig,
        trans_ref_sig,
        bs.selected_translation,
    );

    let on_select = {
        let sid = slide_id.clone();
        move |_| {
            let sid = sid.clone();
            bs.selected_slide_ids.update(|ids| {
                if ids.contains(&sid) {
                    ids.remove(&sid);
                } else {
                    ids.insert(sid.clone());
                }
            });
        }
    };

    view! {
        <div
            class="operator__slide-card operator__slide-card--bible"
            class:operator__slide-card--edit=move || mode.get() == "edit"
            class:is-selected=is_selected
            data-role="slide-card"
            data-slide-id=slide_id.clone()
        >
            <div
                class="operator__slide-trigger-zone"
                data-role="slide-trigger-zone"
                on:click=on_trigger.clone()
                title="Click to trigger this slide"
            >
                <span class="operator__slide-trigger-icon">"\u{25B6}"</span>
                <span>{if !main_ref.is_empty() { main_ref.clone() } else { "Trigger".to_string() }}</span>
            </div>
            <div
                class="operator__slide-select-zone"
                data-role="slide-select-zone"
                on:click=on_select.clone()
            >
                <header class="operator__slide-header">
                    <div class="operator__slide-header-left">
                        <label class="operator__slide-index operator__slide-index--select">
                            <input type="checkbox"
                                data-role="slide-select"
                                prop:checked=is_selected_for_checkbox
                                on:change=move |_| {}
                            />
                        </label>
                    </div>
                    <div class="operator__slide-controls operator__slide-controls--compact">
                        <button type="button"
                            class="operator__list-action operator__list-action--primary"
                            data-role="slide-trigger"
                            on:click=on_trigger.clone()
                        >"Trigger"</button>
                    </div>
                </header>
                {bible_slide_editor_view(
                    main_text_sig, trans_text_sig, main_ref_sig, trans_ref_sig,
                    |_: web_sys::FocusEvent| {},
                )}
                {move || {
                    let main = main_text_sig.get();
                    let trans = trans_text_sig.get();
                    let mref = main_ref_sig.get();
                    let tref = trans_ref_sig.get();
                    slide_body_view(main, trans, mref, tref)
                }}
            </div>
        </div>
    }
}

// ---------------------------------------------------------------------------
// Helper: read a field value from the DOM by data-slide-id + data-field
// ---------------------------------------------------------------------------

fn get_bible_field_value(doc: &web_sys::Document, slide_id: &str, field: &str) -> String {
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

/// Save all editable fields from DOM atomically — prevents stale signal bugs.
/// Maps: main-ref → stage (Resolume reads this). Group is a worship field — not used for Bible.
fn save_bible_slide_from_dom(pres_id: &str, slide_id: &str) {
    let doc = crate::utils::window::document();
    let main = get_bible_field_value(&doc, slide_id, "main");
    let translation = get_bible_field_value(&doc, slide_id, "translation");
    let stage = get_bible_field_value(&doc, slide_id, "main-ref");

    let pres_id = pres_id.to_string();
    let sid = slide_id.to_string();
    leptos::task::spawn_local(async move {
        let _ =
            pres_api::update_slide_with_group(&pres_id, &sid, &main, &translation, &stage, None)
                .await;
    });
}

// ---------------------------------------------------------------------------
// Prepared slide card — drag-drop reordering + edit mode + delete
// ---------------------------------------------------------------------------

#[component]
fn PreparedSlideCard(slide: BibleSlideDto, index: usize) -> impl IntoView {
    let bs = use_ctx!(BibleState);
    let ctx = use_ctx!(AppContext);
    let mode = ctx.mode;

    let slide_id = slide.id.clone();
    // Use main_reference from metadata, fall back to stage field (where AI puts the reference)
    let main_ref = slide
        .main_reference
        .clone()
        .unwrap_or_else(|| slide.stage.clone());

    let main_text_sig = RwSignal::new(slide.main.clone());
    let trans_text_sig = RwSignal::new(slide.translation.clone());
    let group_label_sig = RwSignal::new(slide.group.clone().unwrap_or_default());
    let main_ref_sig = RwSignal::new(main_ref.clone());
    let trans_ref_sig = RwSignal::new(slide.translation_reference.clone().unwrap_or_default());

    let on_trigger = make_trigger_handler(
        &ctx,
        &slide,
        main_text_sig,
        main_ref_sig,
        trans_text_sig,
        trans_ref_sig,
        bs.selected_translation,
    );

    // -- Delete handler --
    let slide_id_for_delete = slide_id.clone();
    let on_delete = {
        let ctx = ctx.clone();
        let bs = bs.clone();
        move |ev: web_sys::MouseEvent| {
            ev.stop_propagation();
            let pres_id = bs.active_presentation_id.get_untracked();
            let Some(pid) = pres_id else { return };
            let sid = slide_id_for_delete.clone();

            let window = crate::utils::window::window();
            if let Ok(confirmed) = window.confirm_with_message("Delete this slide?") {
                if !confirmed {
                    return;
                }
            }

            let active_slides = bs.active_presentation_slides;
            let toast_message = ctx.toast_message;
            let toast_variant = ctx.toast_variant;
            leptos::task::spawn_local(async move {
                match bible::delete_presentation_slide(&pid, &sid).await {
                    Ok(()) => {
                        if let Ok(detail) = bible::get_presentation(&pid).await {
                            active_slides.set(detail.slides);
                        }
                        toast_variant.set("success".to_string());
                        toast_message.set(Some("Slide deleted".to_string()));
                    }
                    Err(e) => {
                        toast_variant.set("error".to_string());
                        toast_message.set(Some(format!("Delete failed: {e}")));
                    }
                }
            });
        }
    };

    // -- Drag-drop handlers for reorder --
    let drag_source = bs.drag_source_idx;
    let drag_over = bs.drag_over_idx;

    let on_dragstart = move |ev: web_sys::DragEvent| {
        drag_source.set(Some(index));
        if let Some(dt) = ev.data_transfer() {
            let _ = dt.set_data("text/plain", &index.to_string());
            dt.set_effect_allowed("move");
        }
    };

    let on_dragover = move |ev: web_sys::DragEvent| {
        ev.prevent_default();
        drag_over.set(Some(index));
        if let Some(dt) = ev.data_transfer() {
            dt.set_drop_effect("move");
        }
    };

    let on_dragleave = move |_ev: web_sys::DragEvent| {
        drag_over.update(|v| {
            if *v == Some(index) {
                *v = None;
            }
        });
    };

    let on_drop = {
        let bs = bs.clone();
        let ctx = ctx.clone();
        move |ev: web_sys::DragEvent| {
            ev.prevent_default();
            drag_over.set(None);
            let src = drag_source.get_untracked();
            drag_source.set(None);

            let Some(from_idx) = src else { return };
            let to_idx = index;
            if from_idx == to_idx {
                return;
            }

            let pres_id = bs.active_presentation_id.get_untracked();
            let Some(pid) = pres_id else { return };

            let mut current_slides = bs.active_presentation_slides.get_untracked();
            if from_idx >= current_slides.len() || to_idx >= current_slides.len() {
                return;
            }
            let slide = current_slides.remove(from_idx);
            current_slides.insert(to_idx, slide);

            let slide_ids: Vec<String> = current_slides.iter().map(|s| s.id.clone()).collect();
            bs.active_presentation_slides.set(current_slides);

            let active_slides = bs.active_presentation_slides;
            let toast_message = ctx.toast_message;
            let toast_variant = ctx.toast_variant;
            leptos::task::spawn_local(async move {
                match bible::reorder_presentation_slides(&pid, slide_ids).await {
                    Ok(()) => {
                        if let Ok(detail) = bible::get_presentation(&pid).await {
                            active_slides.set(detail.slides);
                        }
                    }
                    Err(e) => {
                        toast_variant.set("error".to_string());
                        toast_message.set(Some(format!("Reorder failed: {e}")));
                        if let Ok(detail) = bible::get_presentation(&pid).await {
                            active_slides.set(detail.slides);
                        }
                    }
                }
            });
        }
    };

    let is_drag_over = move || drag_over.get() == Some(index);

    // -- Blur handler for edit mode: save from DOM --
    let slide_id_for_blur = slide_id.clone();
    let on_field_blur = {
        let bs = bs.clone();
        move |_ev: web_sys::FocusEvent| {
            let Some(pid) = bs.active_presentation_id.get_untracked() else {
                return;
            };
            save_bible_slide_from_dom(&pid, &slide_id_for_blur);
        }
    };

    // Static values for read-only body view
    let main_ro = main_text_sig.get_untracked();
    let trans_ro = trans_text_sig.get_untracked();
    let group_ro = group_label_sig.get_untracked();
    let main_ref_ro = main_ref.clone();
    let trans_ref_ro = trans_ref_sig.get_untracked();

    view! {
        <div
            class="operator__slide-card operator__slide-card--bible"
            class:operator__slide-card--drag-over=is_drag_over
            class:operator__slide-card--edit=move || mode.get() == "edit"
            data-role="slide-card"
            data-slide-id=slide_id.clone()
            on:dragover=on_dragover
            on:dragleave=on_dragleave
            on:drop=on_drop
        >
            // Drag handle — initiates the drag
            <div
                class="bible__slide-handle"
                data-role="slide-drag-handle"
                draggable="true"
                on:dragstart=on_dragstart
                title="Drag to reorder"
            >"\u{2630}"</div>

            // Trigger zone (live mode only)
            {
                let on_trigger = on_trigger.clone();
                let main_ref_trigger = main_ref.clone();
                move || {
                    if mode.get() != "edit" {
                        Some(view! {
                            <div
                                class="operator__slide-trigger-zone operator__slide-trigger-zone--full"
                                data-role="slide-trigger-zone"
                                on:click=on_trigger.clone()
                                title="Click to trigger this slide"
                            >
                                <span class="operator__slide-trigger-icon">"\u{25B6}"</span>
                                <span>{if !main_ref_trigger.is_empty() { main_ref_trigger.clone() } else { "Trigger".to_string() }}</span>
                            </div>
                        })
                    } else {
                        None
                    }
                }
            }

            // Edit mode: shared editor (main, translation, main ref, translation ref)
            {
                let on_blur = on_field_blur.clone();
                move || {
                    if mode.get() == "edit" {
                        Some(bible_slide_editor_view(
                            main_text_sig, trans_text_sig, main_ref_sig, trans_ref_sig,
                            on_blur.clone(),
                        ))
                    } else {
                        None
                    }
                }
            }

            // Read-only body (live mode)
            {
                let main = main_ro.clone();
                let trans = trans_ro.clone();
                let mref = main_ref_ro.clone();
                let tref = trans_ref_ro.clone();
                move || {
                    if mode.get() != "edit" {
                        Some(slide_body_view(main.clone(), trans.clone(), mref.clone(), tref.clone()))
                    } else {
                        None
                    }
                }
            }

            // Delete button (edit mode only)
            {
                let on_delete = on_delete.clone();
                move || {
                    if mode.get() == "edit" {
                        Some(view! {
                            <div class="operator__slide-actions">
                                <button
                                    type="button"
                                    class="operator__slide-delete-btn"
                                    data-action="delete"
                                    data-role="slide-delete"
                                    title="Delete slide"
                                    on:click=on_delete.clone()
                                >"\u{2715}"</button>
                            </div>
                        })
                    } else {
                        None
                    }
                }
            }
        </div>
    }
}
