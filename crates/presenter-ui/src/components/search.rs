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

    // Debounced library / song search (Biblia has no header search, #850 B2c).
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

        if let Some(pid) = pres_id {
            // #853: the operator's pick replaces any Follow open still loading;
            // the song loads like a song-list click (#515 seq guard, #556 F6
            // retry, still-selected check, back to the shown song on failure).
            ctx.follow_pending.set_value(None);
            ctx.selected_presentation_id.set(Some(pid.clone()));
            crate::state::session::set("currentPresentationId", &pid);
            let load = crate::state::song_load::SongLoad {
                id: pid,
                selected_id: ctx.selected_presentation_id,
                shown: ctx.selected_presentation,
                slides_cache: None,
                slide_edit_seq: op.slide_edit_seq,
                save_status: op.save_status,
                seq_at_select: op.slide_edit_seq.get_untracked(),
                on_failure: crate::state::song_load::OnFailure::SelectShown,
                follow_pending: ctx.follow_pending,
                selected_entry_index: ctx.selected_entry_index,
                shown_entry_index: ctx.shown_entry_index,
            };
            leptos::task::spawn_local(crate::state::song_load::show_when_loaded(load));
        }

        let presentations = ctx.presentations;
        let context_title = ctx.context_title;
        let libraries = ctx.libraries;
        leptos::task::spawn_local(async move {
            if let Ok(libs) = crate::api::libraries::list_libraries().await {
                if let Some(lib) = libs.iter().find(|l| l.id.to_string() == lib_id) {
                    context_title.set(lib.name.clone());
                    presentations.set(lib.presentations.clone());
                }
                libraries.set(libs);
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
                let has_query = !op_visible.search_query.get().is_empty();
                if open && has_query && ctx.view.get() != "bible" { "true" } else { "false" }
            }
            class="operator__search-results"
        >
            {move || {
                    // Render library/song search results
                    let results = ctx.search_results.get();
                    if results.is_empty() && ctx.search_loading.get() {
                        return view! { <div class="operator__search-loading">"Hľadám\u{2026}"</div> }.into_any();
                    }
                    if results.is_empty() {
                        return view! { <div class="operator__search-empty">"Nič sa nenašlo"</div> }.into_any();
                    }

                    // Group results by kind: Libraries, Presentations
                    let (libraries, presentations): (Vec<_>, Vec<_>) = {
                        let mut libs = Vec::new();
                        let mut pres = Vec::new();
                        for r in results {
                            match r.kind {
                                presenter_core::SearchResultKind::Library => libs.push(r),
                                presenter_core::SearchResultKind::Presentation => pres.push(r),
                            }
                        }
                        (libs, pres)
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
                                    <h3>"Knižnice"</h3>
                                    {libraries.into_iter().map(render_result).collect_view()}
                                </section>
                            })}
                            {(!presentations.is_empty()).then(|| view! {
                                <section class="operator__search-group" data-kind="presentation">
                                    <h3>"Piesne"</h3>
                                    {presentations.into_iter().map(render_result).collect_view()}
                                </section>
                            })}
                        </>
                    }.into_any()
            }}
        </div>
    }
}

fn urlencoding(s: &str) -> String {
    js_sys::encode_uri_component(s)
        .as_string()
        .unwrap_or_default()
}
