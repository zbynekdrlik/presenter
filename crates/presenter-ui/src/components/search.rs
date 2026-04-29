use crate::state::bible::SelectedBook;
use crate::state::operator::OperatorState;
use crate::state::AppContext;
use leptos::prelude::*;
use presenter_core::SearchResult;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsCast;

/// Search results dropdown with debounced API queries.
#[component]
pub fn SearchResults() -> impl IntoView {
    let ctx = use_ctx!(AppContext);
    let op = use_ctx!(OperatorState);

    // Debounced search effect (for library/song search only, bible is handled in header)
    let timeout_handle: Rc<RefCell<Option<gloo_timers::callback::Timeout>>> =
        Rc::new(RefCell::new(None));

    Effect::new({
        let timeout_handle = Rc::clone(&timeout_handle);
        move || {
            let query = op.search_query.get();
            let trimmed = query.trim().to_string();

            // Cancel existing timer
            timeout_handle.borrow_mut().take();

            if trimmed.is_empty() {
                ctx.search_results.set(Vec::new());
                ctx.search_loading.set(false);
                return;
            }

            ctx.search_loading.set(true);

            let search_results = ctx.search_results;
            let search_loading = ctx.search_loading;

            let timer = gloo_timers::callback::Timeout::new(200, move || {
                leptos::task::spawn_local(async move {
                    let url = format!("/search?query={}&limit=30", urlencoding(&trimmed));
                    match crate::api::get_json::<Vec<SearchResult>>(&url).await {
                        Ok(results) => {
                            search_results.set(results);
                        }
                        Err(_) => {
                            search_results.set(Vec::new());
                        }
                    }
                    search_loading.set(false);
                });
            });
            *timeout_handle.borrow_mut() = Some(timer);
        }
    });

    // Outside-click handler to close search results
    {
        let op_close = op.clone();
        let search_results_close = ctx.search_results;
        let bible_search_query = ctx.bible_search_query;
        let bible_search_results = ctx.bible_search_results;
        let bible_has_searched = ctx.bible_has_searched;
        let handler = wasm_bindgen::closure::Closure::<dyn Fn(web_sys::MouseEvent)>::new(
            move |ev: web_sys::MouseEvent| {
                if !op_close.search_open.get_untracked() {
                    return;
                }
                let target = ev
                    .target()
                    .and_then(|t| t.dyn_into::<web_sys::Element>().ok());
                if let Some(el) = target {
                    let doc = crate::utils::window::document();
                    // Check if click is inside the search form or results
                    let in_search = doc
                        .query_selector("[data-role='global-search-form']")
                        .ok()
                        .flatten()
                        .map(|form| form.contains(Some(&el)))
                        .unwrap_or(false);
                    let in_results = doc
                        .query_selector("[data-role='global-search-results']")
                        .ok()
                        .flatten()
                        .map(|res| res.contains(Some(&el)))
                        .unwrap_or(false);
                    if !in_search && !in_results {
                        op_close.search_open.set(false);
                        op_close.search_query.set(String::new());
                        search_results_close.set(Vec::new());
                        // Also clear bible search
                        bible_search_query.set(String::new());
                        bible_search_results.set(Vec::new());
                        bible_has_searched.set(false);
                    }
                }
            },
        );
        let window = crate::utils::window::window();
        let _ =
            window.add_event_listener_with_callback("mousedown", handler.as_ref().unchecked_ref());
        handler.forget();
    }

    let on_result_click = move |lib_id: String, pres_id: Option<String>| {
        // Navigate to library and select presentation
        ctx.selected_library_id.set(Some(lib_id.clone()));
        ctx.selected_playlist_id.set(None);
        crate::state::session::set("activeLibraryId", &lib_id);
        crate::state::session::remove("activePlaylistId");

        op.search_open.set(false);
        op.search_query.set(String::new());
        ctx.search_results.set(Vec::new());

        if let Some(pid) = pres_id.clone() {
            ctx.selected_presentation_id.set(Some(pid.clone()));
            crate::state::session::set("currentPresentationId", &pid);
        }

        let presentations = ctx.presentations;
        let context_title = ctx.context_title;
        let libraries = ctx.libraries;
        let selected_pres = ctx.selected_presentation;
        let pres_id_clone = pres_id;

        leptos::task::spawn_local(async move {
            if let Ok(libs) = crate::api::libraries::list_libraries().await {
                if let Some(lib) = libs.iter().find(|l| l.id.to_string() == lib_id) {
                    context_title.set(lib.name.clone());
                    presentations.set(lib.presentations.clone());
                }
                libraries.set(libs);
            }
            if let Some(pid) = pres_id_clone {
                if let Ok(detail) = crate::api::presentations::get_presentation(&pid).await {
                    selected_pres.set(Some(detail.presentation));
                }
            }
        });
    };

    // Clone op for multiple closures in view! macro
    let op_visible = op.clone();
    let op_results = op.clone();

    view! {
        <div
            data-role="global-search-results"
            data-visible=move || {
                let open = op_visible.search_open.get();
                let is_bible = ctx.view.get() == "bible";
                if is_bible {
                    let has_query = !ctx.bible_search_query.get().is_empty()
                        && ctx.bible_search_query.get().len() >= 3;
                    let has_results = !ctx.bible_search_results.get().is_empty()
                        || ctx.bible_searching.get()
                        || ctx.bible_has_searched.get();
                    if open && has_query && has_results { "true" } else { "false" }
                } else {
                    let has_query = !op_visible.search_query.get().is_empty();
                    if open && has_query { "true" } else { "false" }
                }
            }
            class="operator__search-results"
        >
            {move || {
                let is_bible = ctx.view.get() == "bible";

                if is_bible {
                    // Render Bible search results
                    let results = ctx.bible_search_results.get();
                    let searching = ctx.bible_searching.get();
                    let has_searched = ctx.bible_has_searched.get();

                    if searching {
                        return view! {
                            <div class="operator__search-loading">"Searching\u{2026}"</div>
                        }.into_any();
                    }
                    if results.is_empty() && has_searched {
                        return view! {
                            <div class="operator__search-empty">"No results found."</div>
                        }.into_any();
                    }
                    if results.is_empty() {
                        return view! {
                            <div class="operator__search-empty">"Type to search\u{2026}"</div>
                        }.into_any();
                    }

                    view! {
                        <section class="operator__search-group" data-kind="bible">
                            <h3>"Bible verses"</h3>
                            {results.into_iter().map(|hit| {
                                let ref_label = hit.reference.to_human_readable();
                                let text_preview = {
                                    let t = &hit.text;
                                    if t.chars().count() > 80 {
                                        let end: usize = t.char_indices().nth(80).map(|(i, _)| i).unwrap_or(t.len());
                                        format!("{}\u{2026}", &t[..end])
                                    } else {
                                        t.clone()
                                    }
                                };
                                let book = hit.reference.book.clone();
                                let book_code = hit.reference.book_code.clone();
                                let book_number = hit.reference.book_number;
                                let chapter = hit.reference.chapter;
                                let verse_start = hit.reference.verse_start;
                                let verse_end = hit.reference.verse_end;

                                let on_click = move |_| {
                                    // Navigate to the verse in the Bible live tab
                                    if let Some(bs) = leptos::prelude::use_context::<crate::state::bible::BibleState>() {
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
                                    }
                                    // Close search
                                    ctx.bible_search_query.set(String::new());
                                    ctx.bible_search_results.set(Vec::new());
                                    ctx.bible_has_searched.set(false);
                                    op.search_open.set(false);
                                };

                                view! {
                                    <div
                                        data-role="bible-search-result"
                                        class="operator__search-result"
                                        on:click=on_click
                                    >
                                        <span class="operator__search-result-title">{ref_label}</span>
                                        <span class="operator__search-result-snippet">{text_preview}</span>
                                    </div>
                                }
                            }).collect_view()}
                        </section>
                    }.into_any()
                } else {
                    // Render library/song search results
                    let results = ctx.search_results.get();
                    if results.is_empty() && ctx.search_loading.get() {
                        return view! { <div class="operator__search-loading">"Searching..."</div> }.into_any();
                    }
                    if results.is_empty() {
                        return view! { <div class="operator__search-empty">"No results"</div> }.into_any();
                    }

                    // Group results by kind: Libraries, Presentations, Slides
                    let (libraries, presentations, slides): (Vec<_>, Vec<_>, Vec<_>) = {
                        let mut libs = Vec::new();
                        let mut pres = Vec::new();
                        let mut slds = Vec::new();
                        for r in results {
                            match r.kind {
                                presenter_core::SearchResultKind::Library => libs.push(r),
                                presenter_core::SearchResultKind::Presentation => pres.push(r),
                                presenter_core::SearchResultKind::Slide => slds.push(r),
                            }
                        }
                        (libs, pres, slds)
                    };

                    let op_for_render = op_results.clone();
                    let render_result = |result: SearchResult| {
                        let kind = format!("{:?}", result.kind).to_lowercase();
                        let lib_id = result.library_id.to_string();
                        let pres_id = result.presentation_id.map(|id| id.to_string());
                        let pres_name = result.presentation_name.clone().unwrap_or_default();
                        let lib_name = result.library_name.clone();
                        let snippet = result.snippet.clone().unwrap_or_default();
                        let pres_id_attr = pres_id.clone().unwrap_or_default();
                        let lib_click = lib_id.clone();
                        let pres_click = pres_id.clone();
                        let pres_id_drag = pres_id.clone().unwrap_or_default();

                        // Clone op for drag handlers
                        let op_drag_start = op_for_render.clone();
                        let op_drag_end = op_for_render.clone();

                        view! {
                            <div
                                data-role="search-result-item"
                                data-kind=kind
                                data-presentation-id=pres_id_attr
                                class="operator__search-result"
                                draggable=if pres_id.is_some() { "true" } else { "false" }
                                on:click=move |_| {
                                    on_result_click(lib_click.clone(), pres_click.clone());
                                }
                                on:dragstart=move |ev: web_sys::DragEvent| {
                                    if let Some(dt) = ev.data_transfer() {
                                        // Set both MIME types for compatibility
                                        let _ = dt.set_data("application/x-presentation-id", &pres_id_drag);
                                        let _ = dt.set_data("application/x-presenter-search", &pres_id_drag);
                                        dt.set_effect_allowed("copy");
                                    }
                                    // Track drag state for JS parity
                                    op_drag_start.search_dragging.set(true);
                                    op_drag_start.dragging_from_search.set(true);
                                }
                                on:dragend=move |_| {
                                    op_drag_end.search_dragging.set(false);
                                    op_drag_end.dragging_from_search.set(false);
                                }
                            >
                                <span class="operator__search-result-title">{pres_name}</span>
                                <span class="operator__search-result-meta">{lib_name}</span>
                                <span class="operator__search-result-snippet">{snippet}</span>
                            </div>
                        }
                    };

                    view! {
                        <>
                            {(!libraries.is_empty()).then(|| view! {
                                <section class="operator__search-group" data-kind="library">
                                    <h3>"Libraries"</h3>
                                    {libraries.into_iter().map(render_result).collect_view()}
                                </section>
                            })}
                            {(!presentations.is_empty()).then(|| view! {
                                <section class="operator__search-group" data-kind="presentation">
                                    <h3>"Presentations"</h3>
                                    {presentations.into_iter().map(render_result).collect_view()}
                                </section>
                            })}
                            {(!slides.is_empty()).then(|| view! {
                                <section class="operator__search-group" data-kind="slide">
                                    <h3>"Slides"</h3>
                                    {slides.into_iter().map(render_result).collect_view()}
                                </section>
                            })}
                        </>
                    }.into_any()
                }
            }}
        </div>
    }
}

fn urlencoding(s: &str) -> String {
    js_sys::encode_uri_component(s)
        .as_string()
        .unwrap_or_default()
}
