use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::api::bible;
use crate::state::bible::{BibleState, SelectedBook};
use crate::state::AppContext;

// ---------------------------------------------------------------------------
// Clear broadcast button
// ---------------------------------------------------------------------------

#[component]
pub fn ClearBroadcastButton() -> impl IntoView {
    let ctx = use_ctx!(AppContext);

    let on_clear = move |_| {
        let toast_message = ctx.toast_message;
        let toast_variant = ctx.toast_variant;
        let active_broadcast = ctx.active_bible_broadcast;
        leptos::task::spawn_local(async move {
            match bible::clear_broadcast().await {
                Ok(()) => {
                    active_broadcast.set(None);
                    toast_variant.set("success".to_string());
                    toast_message.set(Some("Broadcast cleared".to_string()));
                }
                Err(e) => {
                    toast_variant.set("error".to_string());
                    toast_message.set(Some(format!("Clear failed: {e}")));
                }
            }
        });
    };

    view! {
        <button
            type="button"
            class="operator__list-action"
            data-role="clear-broadcast"
            on:click=on_clear
            style="margin-top: 4px;"
        >"Clear broadcast"</button>
    }
}

// ---------------------------------------------------------------------------
// Bible search
// ---------------------------------------------------------------------------

#[component]
pub fn BibleSearch() -> impl IntoView {
    let bs = use_ctx!(BibleState);
    let search_query = bs.search_query;
    let search_results = bs.search_results;
    let searching = bs.searching;
    let has_searched = bs.has_searched;

    // Debounce timer handle stored in a simple signal
    let timer_handle: RwSignal<Option<i32>> = RwSignal::new(None);

    let on_input = move |ev: web_sys::Event| {
        let target = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok());
        let Some(input) = target else { return };
        let query = input.value();
        search_query.set(query.clone());

        // Cancel pending timer
        if let Some(handle) = timer_handle.get_untracked() {
            crate::utils::window::window().clear_timeout_with_handle(handle);
        }

        if query.len() < 3 {
            search_results.set(Vec::new());
            has_searched.set(false);
            return;
        }

        searching.set(true);
        let translation = bs.selected_translation.get_untracked().unwrap_or_default();

        let cb = wasm_bindgen::closure::Closure::once_into_js(move || {
            leptos::task::spawn_local(async move {
                match bible::search(&query, &translation, Some(20)).await {
                    Ok(hits) => {
                        search_results.set(hits);
                        has_searched.set(true);
                    }
                    Err(_) => {
                        search_results.set(Vec::new());
                        has_searched.set(true);
                    }
                }
                searching.set(false);
            });
        });

        let handle = crate::utils::window::window()
            .set_timeout_with_callback_and_timeout_and_arguments_0(cb.unchecked_ref(), 300)
            .unwrap_or(0);
        timer_handle.set(Some(handle));
    };

    let on_clear = move |_| {
        search_query.set(String::new());
        search_results.set(Vec::new());
        has_searched.set(false);
    };

    view! {
        <div class="bible__search" data-role="bible-search">
            <label class="operator__field">
                <span>"Search verses"</span>
                <div style="display:flex;gap:4px;">
                    <input
                        type="search"
                        data-role="bible-search-input"
                        placeholder="Min 3 characters\u{2026}"
                        prop:value=move || search_query.get()
                        on:input=on_input
                    />
                    <button
                        type="button"
                        data-role="bible-search-clear"
                        class="operator__list-action"
                        on:click=on_clear
                        style="flex-shrink:0;"
                    >"\u{00D7}"</button>
                </div>
            </label>
            {move || {
                let query = search_query.get();
                let is_searching = searching.get();
                let results = search_results.get();
                let searched = has_searched.get();

                if query.len() < 3 {
                    None
                } else if is_searching {
                    Some(view! {
                        <div class="bible__search-results" data-role="bible-search-results">
                            <p class="operator__slides-empty">"Searching\u{2026}"</p>
                        </div>
                    }.into_any())
                } else if results.is_empty() && searched {
                    Some(view! {
                        <div class="bible__search-results" data-role="bible-search-results">
                            <p class="operator__slides-empty">"No results found."</p>
                        </div>
                    }.into_any())
                } else if !results.is_empty() {
                    Some(view! {
                        <BibleSearchResults />
                    }.into_any())
                } else {
                    None
                }
            }}
        </div>
    }
}

#[component]
fn BibleSearchResults() -> impl IntoView {
    let bs = use_ctx!(BibleState);
    let results = bs.search_results;

    view! {
        <div class="bible__search-results" data-role="bible-search-results">
            {move || {
                results.get().into_iter().map(|hit| {
                    let ref_label = hit.reference.to_human_readable();
                    let text_preview = {
                        let t = &hit.text;
                        if t.len() > 80 { format!("{}\u{2026}", &t[..80]) } else { t.clone() }
                    };
                    let book = hit.reference.book.clone();
                    let book_code = hit.reference.book_code.clone();
                    let book_number = hit.reference.book_number;
                    let chapter = hit.reference.chapter;
                    let verse_start = hit.reference.verse_start;
                    let verse_end = hit.reference.verse_end;

                    let on_click = {
                        let bs = bs.clone();
                        move |_| {
                            let books = bs.books.get_untracked();
                            if let Some(found_book) = books.iter().find(|b| {
                                book_code.as_ref().map(|c| c == &b.code).unwrap_or(false)
                                    || b.book == book
                            }) {
                                let verse_counts: Vec<u16> = found_book.chapters.iter().map(|c| c.verse_count).collect();
                                bs.selected_book.set(Some(SelectedBook {
                                    book: found_book.book.clone(),
                                    code: found_book.code.clone(),
                                    number: book_number.unwrap_or(found_book.number),
                                    chapter_count: found_book.chapters.len() as u16,
                                    verse_counts,
                                }));
                            }
                            bs.selected_chapter.set(chapter);
                            bs.verse_start.set(verse_start);
                            bs.verse_end.set(if verse_end > verse_start { Some(verse_end) } else { None });
                            bs.search_query.set(String::new());
                            bs.search_results.set(Vec::new());
                            bs.has_searched.set(false);
                        }
                    };

                    view! {
                        <button
                            type="button"
                            class="bible__search-result"
                            data-role="bible-search-result"
                            on:click=on_click
                        >
                            <strong>{ref_label}</strong>
                            <span class="bible__search-text">{text_preview}</span>
                        </button>
                    }
                }).collect_view()
            }}
        </div>
    }
}

// ---------------------------------------------------------------------------
// Loaded passages history
// ---------------------------------------------------------------------------

#[component]
pub fn LoadedPassagesHistory() -> impl IntoView {
    let bs = use_ctx!(BibleState);
    let history = bs.loaded_passages_history;

    view! {
        {move || {
            let items = history.get();
            if items.is_empty() {
                None
            } else {
                Some(view! {
                    <div class="bible__history" data-role="bible-history">
                        <h4 style="margin:8px 0 4px;font-size:0.85em;opacity:0.7;">"Recent passages"</h4>
                        {items.into_iter().map(|entry| {
                            let label = entry.label.clone();
                            let book_code = entry.book_code.clone();
                            let book_number = entry.book_number;
                            let chapter = entry.chapter;
                            let verse_start = entry.verse_start;
                            let verse_end = entry.verse_end;
                            let translation_code = entry.translation_code.clone();
                            let bs = bs.clone();

                            let on_click = move |_| {
                                let books = bs.books.get_untracked();
                                if let Some(found_book) = books.iter().find(|b| b.code == book_code) {
                                    let verse_counts: Vec<u16> = found_book.chapters.iter().map(|c| c.verse_count).collect();
                                    bs.selected_book.set(Some(SelectedBook {
                                        book: found_book.book.clone(),
                                        code: found_book.code.clone(),
                                        number: book_number,
                                        chapter_count: found_book.chapters.len() as u16,
                                        verse_counts,
                                    }));
                                }
                                bs.selected_chapter.set(chapter);
                                bs.verse_start.set(verse_start);
                                bs.verse_end.set(verse_end);
                                if bs.selected_translation.get_untracked().as_ref() != Some(&translation_code) {
                                    bs.selected_translation.set(Some(translation_code.clone()));
                                }
                            };

                            view! {
                                <button
                                    type="button"
                                    class="bible__history-item"
                                    data-role="bible-history-item"
                                    on:click=on_click
                                >
                                    {label}
                                </button>
                            }
                        }).collect_view()}
                    </div>
                })
            }
        }}
    }
}

// ---------------------------------------------------------------------------
// Selection controls & add-to-presentation
// ---------------------------------------------------------------------------

#[component]
pub fn SelectionControls() -> impl IntoView {
    let bs = use_ctx!(BibleState);

    let selected_count = move || bs.selected_slide_ids.get().len();

    let on_select_all = move |_| {
        let all_ids: std::collections::HashSet<String> =
            bs.slides.get().iter().map(|s| s.id.clone()).collect();
        bs.selected_slide_ids.set(all_ids);
    };

    view! {
        <span data-role="selection-count" class="operator__slides-count">
            {move || format!("{} selected", selected_count())}
        </span>
        <button
            type="button"
            class="operator__list-action"
            data-role="select-all-slides"
            on:click=on_select_all
        >"Select all"</button>
        <AddToPresentationButtons />
    }
}

fn collect_selected_inputs(bs: &BibleState) -> Option<Vec<bible::AppendSlideInput>> {
    let selected_ids = bs.selected_slide_ids.get_untracked();
    if selected_ids.is_empty() {
        return None;
    }
    let slides = bs.slides.get_untracked();
    let inputs: Vec<bible::AppendSlideInput> = slides
        .iter()
        .filter(|s| selected_ids.contains(&s.id))
        .map(|s| bible::AppendSlideInput {
            main: s.main.clone(),
            translation: s.translation.clone(),
            stage: s.stage.clone(),
            group: s.group.clone(),
            metadata: s.metadata.clone(),
        })
        .collect();
    if inputs.is_empty() {
        None
    } else {
        Some(inputs)
    }
}

#[component]
fn AddToPresentationButtons() -> impl IntoView {
    let bs = use_ctx!(BibleState);
    let ctx = use_ctx!(AppContext);
    let presentations = bs.presentations;

    let on_add_new = {
        let bs = bs.clone();
        let ctx = ctx.clone();
        move |_| {
            let Some(inputs) = collect_selected_inputs(&bs) else {
                ctx.show_toast("No slides selected", "error");
                return;
            };
            // Prompt user for presentation name
            let window = crate::utils::window::window();
            let name = match window.prompt_with_message("Presentation name:") {
                Ok(Some(n)) if !n.trim().is_empty() => n.trim().to_string(),
                _ => return, // User cancelled or entered empty name
            };
            let bs_pres = bs.presentations;
            let active_pres = bs.active_presentation_id;
            let selected_ids = bs.selected_slide_ids;
            let toast_message = ctx.toast_message;
            let toast_variant = ctx.toast_variant;
            leptos::task::spawn_local(async move {
                let detail = match bible::create_presentation(&name).await {
                    Ok(d) => d,
                    Err(e) => {
                        toast_variant.set("error".to_string());
                        toast_message.set(Some(format!("Failed to create: {e}")));
                        return;
                    }
                };
                let pres_id = detail.id;
                match bible::append_presentation_slides(&pres_id, &inputs).await {
                    Ok(_) => {
                        toast_variant.set("success".to_string());
                        toast_message.set(Some(format!(
                            "Added {} slide(s) to new presentation",
                            inputs.len()
                        )));
                        selected_ids.set(std::collections::HashSet::new());
                        if let Ok(pres) = bible::list_presentations().await {
                            bs_pres.set(pres);
                        }
                        active_pres.set(Some(pres_id));
                    }
                    Err(e) => {
                        toast_variant.set("error".to_string());
                        toast_message.set(Some(format!("Failed to add slides: {e}")));
                    }
                }
            });
        }
    };

    view! {
        <button
            type="button"
            class="operator__list-action operator__list-action--primary"
            data-role="presentation-add"
            on:click=on_add_new
        >"+ New presentation"</button>
        <div class="bible__add-pres-list">
            {move || {
                let pres_list = presentations.get();
                if pres_list.is_empty() {
                    None
                } else {
                    Some(pres_list.into_iter().map(|p| {
                        let pres_id = p.id.clone();
                        let label = format!("{} ({})", p.name, p.slide_count);
                        let bs = bs.clone();
                        let ctx = ctx.clone();
                        let on_click = move |_| {
                            let Some(inputs) = collect_selected_inputs(&bs) else {
                                ctx.show_toast("No slides selected", "error");
                                return;
                            };
                            let pres_id = pres_id.clone();
                            let bs_pres = bs.presentations;
                            let selected_ids = bs.selected_slide_ids;
                            let toast_message = ctx.toast_message;
                            let toast_variant = ctx.toast_variant;
                            leptos::task::spawn_local(async move {
                                match bible::append_presentation_slides(&pres_id, &inputs).await {
                                    Ok(_) => {
                                        toast_variant.set("success".to_string());
                                        toast_message.set(Some(format!(
                                            "Added {} slide(s)",
                                            inputs.len()
                                        )));
                                        selected_ids.set(std::collections::HashSet::new());
                                        if let Ok(pres) = bible::list_presentations().await {
                                            bs_pres.set(pres);
                                        }
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
                                class="operator__list-action"
                                data-role="presentation-add-existing"
                                on:click=on_click
                            >{label}</button>
                        }
                    }).collect_view())
                }
            }}
        </div>
    }
}
