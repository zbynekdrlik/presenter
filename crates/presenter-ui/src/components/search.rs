use leptos::prelude::*;

use crate::state::operator::OperatorState;
use crate::state::AppContext;

/// Search results dropdown.
#[component]
pub fn SearchResults(ctx: AppContext, op: OperatorState) -> impl IntoView {
    let results = ctx.search_results;
    let query = op.search_query;
    let search_open = op.search_open;

    // Debounced search effect
    let ctx_clone = ctx.clone();
    Effect::new(move || {
        let q = query.get();
        let trimmed = q.trim().to_string();
        if trimmed.is_empty() {
            ctx_clone.search_results.set(Vec::new());
            search_open.set(false);
            return;
        }
        search_open.set(true);
        ctx_clone.search_loading.set(true);
        let search_results = ctx_clone.search_results;
        let loading = ctx_clone.search_loading;
        // Simple debounce via timeout
        let handle = gloo_timers::callback::Timeout::new(200, move || {
            leptos::task::spawn_local(async move {
                let url = format!(
                    "/search?query={}&limit=30",
                    js_sys::encode_uri_component(&trimmed)
                );
                match crate::api::get_json::<Vec<presenter_core::SearchResult>>(&url).await {
                    Ok(r) => search_results.set(r),
                    Err(_) => search_results.set(Vec::new()),
                }
                loading.set(false);
            });
        });
        handle.forget();
    });

    view! {
        <div
            class="operator__search-results"
            data-role="global-search-results"
            style:display=move || if search_open.get() && !results.get().is_empty() { "block" } else { "none" }
        >
            <ul>
                {move || {
                    results.get().into_iter().map(|result| {
                        let label = result.presentation_name.clone().or(Some(result.library_name.clone())).unwrap_or_default();
                        let snippet = result.snippet.clone().unwrap_or_default();
                        view! {
                            <li data-role="search-result">
                                <button type="button" class="operator__search-result-button">
                                    <span class="operator__search-result-label">{label}</span>
                                    <span class="operator__search-result-snippet">{snippet}</span>
                                </button>
                            </li>
                        }
                    }).collect::<Vec<_>>()
                }}
            </ul>
        </div>
    }
}
