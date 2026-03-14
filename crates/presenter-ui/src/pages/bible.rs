use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::state::bible::BibleState;
use crate::state::AppContext;
use crate::ws;

/// Bible page - search and broadcast Bible passages.
#[component]
pub fn BiblePage() -> impl IntoView {
    let ctx = use_context::<AppContext>().expect("AppContext");
    let bible_state = BibleState::new();
    provide_context(bible_state.clone());

    let (_ws_state, _last_event) = ws::use_live_websocket();

    // Load translations on mount
    let translations = bible_state.translations;
    let selected_translation = bible_state.selected_translation;
    leptos::task::spawn_local(async move {
        if let Ok(trans) = crate::api::bible::list_translations().await {
            // Set default translation if available
            if let Some(first) = trans.first() {
                if selected_translation.get_untracked().is_none() {
                    selected_translation.set(Some(first.code.clone()));
                }
            }
            translations.set(trans);
        }
    });

    // Load current broadcast state
    let active_broadcast = ctx.active_bible_broadcast;
    leptos::task::spawn_local(async move {
        if let Ok(broadcast) = crate::api::bible::get_broadcast().await {
            active_broadcast.set(broadcast);
        }
    });

    view! {
        <div data-role="bible-page" class="bible-layout">
            <header data-role="bible-header" class="bible-header">
                <h1>"Bible"</h1>
                <TranslationSelector bible_state=bible_state.clone() />
            </header>
            <main data-role="bible-main" class="bible-main">
                <BibleSearch bible_state=bible_state.clone() />
                <BibleResults bible_state=bible_state.clone() />
                <ActiveBroadcast />
            </main>
        </div>
    }
}

/// Translation dropdown selector
#[component]
fn TranslationSelector(bible_state: BibleState) -> impl IntoView {
    let translations = bible_state.translations;
    let selected_translation = bible_state.selected_translation;

    let on_change = move |ev: web_sys::Event| {
        let target = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlSelectElement>().ok());
        if let Some(select) = target {
            let value = select.value();
            if !value.is_empty() {
                selected_translation.set(Some(value));
            }
        }
    };

    view! {
        <div class="bible-translation-selector">
            <label for="bible-translation">"Translation:"</label>
            <select
                id="bible-translation"
                data-role="bible-translation-select"
                on:change=on_change
            >
                {move || {
                    let current = selected_translation.get();
                    translations.get().into_iter().map(|t| {
                        let code = t.code.clone();
                        let is_selected = current.as_ref() == Some(&code);
                        view! {
                            <option value=code.clone() selected=is_selected>
                                {format!("{} ({})", t.name, t.code)}
                            </option>
                        }
                    }).collect_view()
                }}
            </select>
        </div>
    }
}

/// Bible search input
#[component]
fn BibleSearch(bible_state: BibleState) -> impl IntoView {
    let search_query = bible_state.search_query;
    let searching = bible_state.searching;
    let search_results = bible_state.search_results;
    let selected_translation = bible_state.selected_translation;

    let do_search = move || {
        let query = search_query.get_untracked();
        let translation = selected_translation
            .get_untracked()
            .unwrap_or_else(|| "ESV".to_string());

        if query.trim().is_empty() {
            search_results.set(Vec::new());
            return;
        }

        searching.set(true);
        leptos::task::spawn_local(async move {
            match crate::api::bible::search(&query, &translation).await {
                Ok(result) => {
                    search_results.set(result.results);
                }
                Err(_) => {
                    search_results.set(Vec::new());
                }
            }
            searching.set(false);
        });
    };

    let on_input = {
        move |ev: web_sys::Event| {
            let target = ev
                .target()
                .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok());
            if let Some(input) = target {
                search_query.set(input.value());
            }
        }
    };

    let on_keydown = move |ev: web_sys::KeyboardEvent| {
        if ev.key() == "Enter" {
            ev.prevent_default();
            do_search();
        }
    };

    let on_search_click = move |_| {
        do_search();
    };

    view! {
        <section data-role="bible-search" class="bible-search">
            <div class="bible-search-row">
                <input
                    data-role="bible-search-input"
                    type="text"
                    placeholder="Search Bible (e.g., John 3:16, love)"
                    prop:value=move || search_query.get()
                    on:input=on_input
                    on:keydown=on_keydown
                />
                <button
                    type="button"
                    data-role="bible-search-button"
                    on:click=on_search_click
                    disabled=move || searching.get()
                >
                    {move || if searching.get() { "Searching..." } else { "Search" }}
                </button>
            </div>
        </section>
    }
}

/// Bible search results list
#[component]
fn BibleResults(bible_state: BibleState) -> impl IntoView {
    let search_results = bible_state.search_results;
    let selected_translation = bible_state.selected_translation;
    let ctx = use_context::<AppContext>().expect("AppContext");
    let toast_message = ctx.toast_message;
    let toast_variant = ctx.toast_variant;
    let active_broadcast = ctx.active_bible_broadcast;

    view! {
        <section data-role="bible-results" class="bible-results">
            {move || {
                let results = search_results.get();
                if results.is_empty() {
                    view! {
                        <p class="bible-results-empty">"Enter a search query to find Bible passages."</p>
                    }.into_any()
                } else {
                    results.into_iter().map(|hit| {
                        let reference = hit.reference.clone();
                        let text = hit.text.clone();
                        let ref_for_attr = reference.clone();
                        let ref_for_display = reference.clone();
                        let ref_for_click = reference.clone();
                        let translation = selected_translation.get_untracked().unwrap_or_else(|| "ESV".to_string());

                        let on_click = {
                            let reference = ref_for_click;
                            let translation = translation.clone();
                            move |_| {
                                let reference = reference.clone();
                                let translation = translation.clone();
                                leptos::task::spawn_local(async move {
                                    match crate::api::bible::broadcast(&reference, &translation).await {
                                        Ok(broadcast) => {
                                            active_broadcast.set(Some(broadcast));
                                            toast_variant.set("success".to_string());
                                            toast_message.set(Some(format!("Broadcasting: {}", reference)));
                                        }
                                        Err(_) => {
                                            toast_variant.set("error".to_string());
                                            toast_message.set(Some("Failed to broadcast passage".to_string()));
                                        }
                                    }
                                });
                            }
                        };

                        view! {
                            <article
                                class="bible-result-item"
                                data-role="bible-result"
                                data-reference=ref_for_attr
                                on:click=on_click
                            >
                                <h3 class="bible-result-reference">{ref_for_display}</h3>
                                <p class="bible-result-text">{text}</p>
                            </article>
                        }
                    }).collect_view().into_any()
                }
            }}
        </section>
    }
}

/// Active broadcast display with clear button
#[component]
fn ActiveBroadcast() -> impl IntoView {
    let ctx = use_context::<AppContext>().expect("AppContext");
    let active_broadcast = ctx.active_bible_broadcast;
    let toast_message = ctx.toast_message;
    let toast_variant = ctx.toast_variant;

    let on_clear = move |_| {
        leptos::task::spawn_local(async move {
            match crate::api::bible::clear_broadcast().await {
                Ok(()) => {
                    active_broadcast.set(None);
                    toast_variant.set("info".to_string());
                    toast_message.set(Some("Bible broadcast cleared".to_string()));
                }
                Err(_) => {
                    toast_variant.set("error".to_string());
                    toast_message.set(Some("Failed to clear broadcast".to_string()));
                }
            }
        });
    };

    view! {
        <section data-role="bible-broadcast" class="bible-broadcast">
            {move || {
                if let Some(broadcast) = active_broadcast.get() {
                    view! {
                        <div class="bible-broadcast-active" data-role="bible-broadcast-active">
                            <header class="bible-broadcast-header">
                                <h3>"Active Broadcast"</h3>
                                <button
                                    type="button"
                                    class="bible-broadcast-clear"
                                    data-role="bible-clear-broadcast"
                                    on:click=on_clear
                                >
                                    "Clear"
                                </button>
                            </header>
                            <div class="bible-broadcast-content">
                                <strong class="bible-broadcast-reference">
                                    {broadcast.reference_label.clone().unwrap_or_else(|| broadcast.passage.reference.to_human_readable())}
                                </strong>
                                <p class="bible-broadcast-text">{broadcast.passage.text.clone()}</p>
                                <small class="bible-broadcast-translation">{broadcast.passage.translation.name.clone()}</small>
                            </div>
                        </div>
                    }.into_any()
                } else {
                    view! {
                        <div class="bible-broadcast-inactive" data-role="bible-broadcast-inactive">
                            <p>"No active broadcast"</p>
                        </div>
                    }.into_any()
                }
            }}
        </section>
    }
}
