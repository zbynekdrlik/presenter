use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::api::bible::{self, BibleSlideDto};
use crate::state::bible::BibleState;
use crate::state::AppContext;

// ---------------------------------------------------------------------------
// Slides column
// ---------------------------------------------------------------------------

#[component]
pub fn BibleSlidesColumn() -> impl IntoView {
    let bs = use_context::<BibleState>().expect("BibleState");
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
    let bs = use_context::<BibleState>().expect("BibleState");
    let ctx = use_context::<AppContext>().expect("AppContext");

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
    let bs = use_context::<BibleState>().expect("BibleState");
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
                    view! { <BibleSlideCard slide=slide source="live" /> }
                }).collect_view().into_any()
            }
        }}
    }
}

#[component]
fn PreparedSlides() -> impl IntoView {
    let bs = use_context::<BibleState>().expect("BibleState");
    let slides = bs.active_presentation_slides;

    view! {
        {move || {
            let slide_list = slides.get();
            if slide_list.is_empty() {
                view! {
                    <p class="operator__slides-empty">"Select a presentation to view slides."</p>
                }.into_any()
            } else {
                slide_list.into_iter().map(|slide| {
                    view! { <BibleSlideCard slide=slide source="prepared" /> }
                }).collect_view().into_any()
            }
        }}
    }
}

#[component]
fn BibleSlideCard(slide: BibleSlideDto, source: &'static str) -> impl IntoView {
    let bs = use_context::<BibleState>().expect("BibleState");
    let ctx = use_context::<AppContext>().expect("AppContext");
    let mode = ctx.mode;

    let slide_id = slide.id.clone();
    let main_ref = slide.main_reference.clone().unwrap_or_default();

    // Store text in signals so closures can read them repeatedly
    let main_text_sig = RwSignal::new(slide.main.clone());
    let trans_text_sig = RwSignal::new(slide.translation.clone());
    let group_label_sig = RwSignal::new(slide.group.clone().unwrap_or_default());

    let is_selected = {
        let sid = slide_id.clone();
        move || bs.selected_slide_ids.get().contains(&sid)
    };

    let on_trigger = {
        let ctx = ctx.clone();
        let slide = slide.clone();
        let bs_translation = bs.selected_translation;
        move |_| {
            let main_text = slide.main.clone();
            let main_ref = slide
                .main_reference
                .clone()
                .unwrap_or_else(|| "Bible".to_string());
            let translation_text = slide.translation.clone();
            let trans_ref = slide.translation_reference.clone().unwrap_or_default();

            let meta = slide.metadata.as_ref().and_then(|m| m.bible.as_ref());
            let req = bible::TriggerSlideRequest {
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
            };

            let toast_message = ctx.toast_message;
            let toast_variant = ctx.toast_variant;
            leptos::task::spawn_local(async move {
                match bible::trigger_slide(&req).await {
                    Ok(_) => {
                        toast_variant.set("success".to_string());
                        toast_message.set(Some("Triggered".to_string()));
                    }
                    Err(e) => {
                        toast_variant.set("error".to_string());
                        toast_message.set(Some(format!("Trigger failed: {e}")));
                    }
                }
            });
        }
    };

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

    let is_live = source == "live";

    if is_live {
        view! {
            <div
                class="operator__slide-card operator__slide-card--bible"
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
                    {move || {
                        let is_edit = mode.get() == "edit";
                        if is_edit {
                            view! {
                                <div class="operator__slide-edit">
                                    <textarea
                                        data-role="slide-main-edit"
                                        rows="3"
                                        prop:value=move || main_text_sig.get()
                                        on:input=move |ev: web_sys::Event| {
                                            let target = ev.target().and_then(|t| t.dyn_into::<web_sys::HtmlTextAreaElement>().ok());
                                            if let Some(ta) = target { main_text_sig.set(ta.value()); }
                                        }
                                    ></textarea>
                                    <textarea
                                        data-role="slide-translation-edit"
                                        rows="2"
                                        prop:value=move || trans_text_sig.get()
                                        on:input=move |ev: web_sys::Event| {
                                            let target = ev.target().and_then(|t| t.dyn_into::<web_sys::HtmlTextAreaElement>().ok());
                                            if let Some(ta) = target { trans_text_sig.set(ta.value()); }
                                        }
                                    ></textarea>
                                </div>
                            }.into_any()
                        } else {
                            let main = main_text_sig.get();
                            let trans = trans_text_sig.get();
                            let group = group_label_sig.get();
                            view! {
                                <div class="operator__slide-content">
                                    <p class="operator__slide-main">{main}</p>
                                    {if !trans.is_empty() {
                                        Some(view! { <p class="operator__slide-translation">{trans}</p> })
                                    } else {
                                        None
                                    }}
                                    {if !group.is_empty() {
                                        Some(view! { <small class="operator__slide-group">{group}</small> })
                                    } else {
                                        None
                                    }}
                                </div>
                            }.into_any()
                        }
                    }}
                </div>
            </div>
        }
        .into_any()
    } else {
        // Prepared slide layout
        let main = main_text_sig.get_untracked();
        let trans = trans_text_sig.get_untracked();
        view! {
            <div
                class="operator__slide-card operator__slide-card--bible"
                data-role="slide-card"
                data-slide-id=slide_id.clone()
            >
                <div
                    class="operator__slide-trigger-zone operator__slide-trigger-zone--full"
                    data-role="slide-trigger-zone"
                    on:click=on_trigger.clone()
                    title="Click to trigger this slide"
                >
                    <span class="operator__slide-trigger-icon">"\u{25B6}"</span>
                    <span>{if !main_ref.is_empty() { main_ref.clone() } else { "Trigger".to_string() }}</span>
                </div>
                <div class="operator__slide-content" style="padding: 0.5rem 0.75rem;">
                    <p class="operator__slide-main">{main}</p>
                    {if !trans.is_empty() {
                        Some(view! { <p class="operator__slide-translation">{trans}</p> })
                    } else {
                        None
                    }}
                </div>
            </div>
        }
        .into_any()
    }
}
