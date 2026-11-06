use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::api::bible::{self, BibleSlideDto};
use crate::components::slide_columns::SlideColumnsControl;
use crate::pages::bible_controls::SelectionControls;
use crate::pages::bible_prepared_sermon::{active_sermon_badge, SermonSlides};
use crate::state::bible::{slide_char_count, BibleState};
use crate::state::bible_grid::{is_on_wall, passage_title, slide_caption};
use crate::state::AppContext;

// ---------------------------------------------------------------------------
// Slides column
// ---------------------------------------------------------------------------

#[component]
pub fn BibleSlidesColumn() -> impl IntoView {
    let bs = use_ctx!(BibleState);
    let bible_tab = bs.bible_tab;
    let sermon = active_sermon_badge(&bs);
    let phone = use_ctx!(AppContext).phone_shell;
    let title = worktop_title(&bs);

    view! {
        <section
            class="operator__slides-column operator__slides-column--minimal"
            data-role="slides-column"
        >
            // #875 mockup v3 section 3: on a desktop this row is the work
            // area's top, as on Piesne — the loaded passage (or the open
            // presentation) bold on the left, the controls and "Snímok v
            // riadku − N +" on the right. The phone keeps its "+" and stepper.
            <div class="operator__slides-toolbar operator__slides-toolbar--minimal operator__worktop operator__worktop--bible">
                <Show when=move || !phone.get()>
                    <span class="operator__worktop-title" data-role="bible-worktop-title">
                        {move || title.get()}
                    </span>
                </Show>
                // On a desktop "+ Snímka" goes into the open prepared
                // presentation, so it is offered on the Prepared tab only.
                <Show when=move || {
                    sermon.with(Option::is_none)
                        && (phone.get() || bible_tab.with(|t| t == "prepared"))
                }>
                    <AddEmptySlideButton />
                </Show>
                // Multi-select + add to a prepared presentation, over the grid
                // they act on (spec section 7.1: desktop only, until #851); not
                // over an empty grid, where there is nothing to select (#875).
                <Show when=move || {
                    !phone.get()
                        && bible_tab.with(|t| t == "live")
                        && bs.slides.with(|slides| !slides.is_empty())
                }>
                    <div class="bible__selection" data-role="bible-selection">
                        <SelectionControls />
                    </div>
                </Show>
                <SlideColumnsControl />
            </div>
            <div class="operator__slides" data-role="slides">
                {move || {
                    let tab = bible_tab.get();
                    if tab == "prepared" {
                        match sermon.get() {
                            Some(badge) => view! { <SermonSlides badge=badge /> }.into_any(),
                            None => view! { <PreparedSlides /> }.into_any(),
                        }
                    } else {
                        view! { <LiveSlides /> }.into_any()
                    }
                }}
            </div>
        </section>
    }
}

/// The work area's title: the loaded passage with its translations on the
/// Live tab ("Ján 3:16-18 · SEB + KJV"), the open presentation's name on the
/// Prepared tab; empty while there is nothing to name.
fn worktop_title(bs: &BibleState) -> Memo<String> {
    let bs = *bs;
    Memo::new(move |_| {
        if bs.bible_tab.with(|t| t == "prepared") {
            let Some(id) = bs.active_presentation_id.get() else {
                return String::new();
            };
            return bs.presentations.with(|list| {
                list.iter()
                    .find(|p| p.id == id)
                    .map(|p| p.name.clone())
                    .unwrap_or_default()
            });
        }
        let main = bs.selected_translation.get();
        let secondary = bs.secondary_translation.get();
        bs.slides
            .with(|slides| passage_title(slides, main.as_deref(), secondary.as_deref()))
            .unwrap_or_default()
    })
}

#[component]
fn AddEmptySlideButton() -> impl IntoView {
    let bs = use_ctx!(BibleState);
    let ctx = use_ctx!(AppContext);
    let phone = ctx.phone_shell;

    let on_click = move |_| {
        let pres_id = bs.active_presentation_id.get_untracked();
        let Some(id) = pres_id else {
            ctx.show_toast("Najprv vyber prezentáciu", "error");
            return;
        };
        let active_slides = bs.active_presentation_slides;
        let toast_message = ctx.toast_message;
        let toast_variant = ctx.toast_variant;

        let input = bible::AppendSlideInput {
            bible_main: String::new(),
            bible_translation: String::new(),
            bible_main_reference: String::new(),
            bible_translation_reference: String::new(),
            metadata: None,
        };

        leptos::task::spawn_local(async move {
            match bible::append_presentation_slides(&id, &[input]).await {
                Ok(detail) => {
                    active_slides.set(detail.slides);
                    toast_variant.set("success".to_string());
                    toast_message.set(Some("Pridaná prázdna snímka".to_string()));
                }
                Err(e) => {
                    toast_variant.set("error".to_string());
                    toast_message.set(Some(format!("Zlyhalo: {e}")));
                }
            }
        });
    };

    view! {
        // Desktop: the Piesne "+ Snímka" button of the title row (#875 v3).
        <button
            type="button"
            class=move || {
                if phone.get() {
                    "operator__slides-add operator__list-action"
                } else {
                    "operator__slides-add operator__worktop-add"
                }
            }
            data-role="add-empty-slide"
            title="Pridať prázdnu snímku do otvorenej prezentácie"
            on:click=on_click
        >{move || if phone.get() { "+" } else { "+ Snímka" }}</button>
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
                    <p class="operator__slides-empty">"Načítaj verše a zobrazia sa tu snímky."</p>
                }.into_any()
            } else {
                slide_list.into_iter().enumerate().map(|(index, slide)| {
                    view! { <LiveSlideCard slide=slide index=index /> }
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
                    <p class="operator__slides-empty">"Vyber prezentáciu vľavo."</p>
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
    let main_ref = main_ref_sig.get_untracked();
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
        verse_start: meta.and_then(|m| m.effective_verse_start()),
        verse_end: meta.and_then(|m| m.effective_verse_end()),
        ..Default::default()
    }
}

/// #828: the small character-count badge of one slide text; warning-coloured
/// (`data-over="true"`) when the count is over the Bible character limit.
fn char_count_badge(field: &'static str, text: &str, limit: u32) -> impl IntoView {
    let (count, over) = slide_char_count(text, limit);
    let title = if over {
        format!("{count} znakov, viac ako limit {limit}")
    } else {
        format!("{count} znakov (limit {limit})")
    };
    view! {
        <span
            class="operator__slide-chars"
            class:operator__slide-chars--over=over
            data-role="slide-char-count"
            data-field=field
            data-over=if over { "true" } else { "false" }
            title=title
        >
            {count}
        </span>
    }
}

/// Render the slide body content (shared between live and prepared cards).
/// Bible slides have 4 fields: Main, Translation, Main Reference, Translation Reference.
/// Group is a worship field — not used here. `limit` is the Bible character
/// limit the count badges compare against (#828).
pub(super) fn slide_body_view(
    main: String,
    trans: String,
    main_ref: String,
    trans_ref: String,
    limit: u32,
) -> impl IntoView {
    let main_badge = char_count_badge("main", &main, limit);
    let translation_badge =
        (!trans.is_empty()).then(|| char_count_badge("translation", &trans, limit));
    view! {
        <section class="operator__slide-bodies operator__slide-bodies--bible">
            <div class="operator__slide-char-counts">{main_badge}{translation_badge}</div>
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
                <span>"Hlavný text"</span>
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
                <span>"Preklad"</span>
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
                    <span>"Odkaz hlavného textu"</span>
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
                    <span>"Odkaz prekladu"</span>
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
                    toast_message.set(Some("Zobrazené na stene".to_string()));
                    if let Ok(broadcast) = bible::get_broadcast().await {
                        active_broadcast.set(broadcast);
                    }
                }
                Err(e) => {
                    toast_variant.set("error".to_string());
                    toast_message.set(Some(format!("Zobrazenie zlyhalo: {e}")));
                }
            }
        });
    }
}

/// The trigger zone's content (#875 v3): the mockup's caption "1 · Ján 3:16",
/// plus "NA STENE" while the card is on the wall. The grid never shows on a
/// phone (the console is its Bible tab).
fn trigger_zone_content(caption: String) -> impl IntoView {
    view! {
        <span class="operator__slide-trigger-icon">"\u{25B6}"</span>
        <span class="operator__slide-caption" data-role="slide-caption">{caption}</span>
        <span class="operator__slide-live-tag" data-role="slide-live-tag">"NA STENE"</span>
    }
}

/// Whether the wall shows what this card would send (its current text).
fn on_wall_memo(
    ctx: &AppContext,
    main_text_sig: RwSignal<String>,
    main_ref_sig: RwSignal<String>,
) -> Memo<bool> {
    let on_air = ctx.active_bible_slide;
    Memo::new(move |_| {
        let main = main_text_sig.get();
        let main_ref = main_ref_sig.get();
        on_air.with(|output| is_on_wall(output.as_ref(), &main, &main_ref))
    })
}

// ---------------------------------------------------------------------------
// Live slide card — broadcast trigger + selection checkboxes
// ---------------------------------------------------------------------------

#[component]
fn LiveSlideCard(slide: BibleSlideDto, index: usize) -> impl IntoView {
    let bs = use_ctx!(BibleState);
    let ctx = use_ctx!(AppContext);
    let mode = ctx.mode;

    let slide_id = slide.id.clone();
    let main_ref = slide.bible_main_reference.clone();
    let trans_ref_initial = slide.bible_translation_reference.clone();

    let main_text_sig = RwSignal::new(slide.bible_main.clone());
    let trans_text_sig = RwSignal::new(slide.bible_translation.clone());
    let main_ref_sig = RwSignal::new(main_ref.clone());
    let trans_ref_sig = RwSignal::new(trans_ref_initial);
    let character_limit = bs.character_limit;
    let caption = slide_caption(index, &slide);
    let on_wall = on_wall_memo(&ctx, main_text_sig, main_ref_sig);

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
            class:is-on-wall=move || on_wall.get()
            data-on-wall=move || if on_wall.get() { "true" } else { "false" }
            data-role="slide-card"
            data-slide-id=slide_id.clone()
        >
            <div
                class="operator__slide-trigger-zone"
                data-role="slide-trigger-zone"
                on:click=on_trigger.clone()
                title="Kliknutím zobrazíš snímku na stene"
            >
                {trigger_zone_content(caption)}
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
                        >"Zobraziť"</button>
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
                    slide_body_view(main, trans, mref, tref, character_limit.get())
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
/// Uses the Bible-specific PATCH endpoint that updates text + metadata references.
/// Does NOT send stage or group (server handles stage for Resolume compatibility).
fn save_bible_slide_from_dom(pres_id: &str, slide_id: &str) {
    let doc = crate::utils::window::document();
    let main = get_bible_field_value(&doc, slide_id, "main");
    let translation = get_bible_field_value(&doc, slide_id, "translation");
    let main_ref = get_bible_field_value(&doc, slide_id, "main-ref");
    let trans_ref = get_bible_field_value(&doc, slide_id, "translation-ref");

    let pres_id = pres_id.to_string();
    let sid = slide_id.to_string();
    leptos::task::spawn_local(async move {
        let _ =
            bible::update_bible_slide(&pres_id, &sid, &main, &translation, &main_ref, &trans_ref)
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
    let main_ref = slide.bible_main_reference.clone();

    let main_text_sig = RwSignal::new(slide.bible_main.clone());
    let trans_text_sig = RwSignal::new(slide.bible_translation.clone());
    let main_ref_sig = RwSignal::new(main_ref.clone());
    let trans_ref_sig = RwSignal::new(slide.bible_translation_reference.clone());
    let character_limit = bs.character_limit;
    let caption = slide_caption(index, &slide);
    let on_wall = on_wall_memo(&ctx, main_text_sig, main_ref_sig);

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
            if let Ok(confirmed) = window.confirm_with_message("Zmazať túto snímku?") {
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
                        toast_message.set(Some("Snímka zmazaná".to_string()));
                    }
                    Err(e) => {
                        toast_variant.set("error".to_string());
                        toast_message.set(Some(format!("Zmazanie zlyhalo: {e}")));
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
                        toast_message.set(Some(format!("Zmena poradia zlyhala: {e}")));
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
    let main_ref_ro = main_ref.clone();
    let trans_ref_ro = trans_ref_sig.get_untracked();

    view! {
        <div
            class="operator__slide-card operator__slide-card--bible"
            class:operator__slide-card--drag-over=is_drag_over
            class:operator__slide-card--edit=move || mode.get() == "edit"
            class:is-on-wall=move || on_wall.get()
            data-on-wall=move || if on_wall.get() { "true" } else { "false" }
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
                title="Potiahnutím zmeníš poradie"
            >"\u{2630}"</div>

            // Trigger zone (live mode only)
            {
                let on_trigger = on_trigger.clone();
                let caption = caption.clone();
                move || {
                    if mode.get() != "edit" {
                        Some(view! {
                            <div
                                class="operator__slide-trigger-zone operator__slide-trigger-zone--full"
                                data-role="slide-trigger-zone"
                                on:click=on_trigger.clone()
                                title="Kliknutím zobrazíš snímku na stene"
                            >
                                {trigger_zone_content(caption.clone())}
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
                        Some(slide_body_view(
                            main.clone(),
                            trans.clone(),
                            mref.clone(),
                            tref.clone(),
                            character_limit.get(),
                        ))
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
                                    title="Zmazať snímku"
                                    aria-label="Zmazať snímku"
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
