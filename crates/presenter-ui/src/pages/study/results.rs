//! The text search view (spec §6.4): `/bible/search` in the reader's
//! translation, within the scope on the route, every hit in the Bible's order
//! (#983). A fully quoted query is an exact phrase. A hit opens the reader at
//! its verse. The translation row runs the search again in another one.

use leptos::prelude::*;
use presenter_core::bible::sermon::translation_short_code;
use presenter_core::bible::SearchOrder;

use super::favourites::switch;
use super::hit::Hit;
use super::scroll::{self, ScrollPlan};
use super::{nav, study_error, Chapter, Search, Study, REMOTE_TRANSLATION};
use crate::api::bible as bible_api;
use crate::state::bible_translations::study_chip_cap;
use crate::state::study::{search_marker, visible_favourites, StudySheet, StudyView};
use crate::state::study_search::search_translation_row;
use crate::state::study_word::{search_scopes, SCOPE_ALL};

/// `/bible/search` in the Bible's order answers at most this many verses
/// (#983: every hit, not the first 100 by relevance).
const SEARCH_LIMIT: u32 = 1_000;

/// The search on the route, in the Bible's order. Only the newest lands.
pub(super) fn fetch_search(s: Study, translation: String, query: String, scope: String) {
    let key = search_marker(&translation, &query, &scope);
    leptos::task::spawn_local(async move {
        let scoped = (scope != SCOPE_ALL).then_some(scope.as_str());
        let answer = bible_api::search(
            &query,
            &translation,
            (Some(SEARCH_LIMIT), scoped),
            SearchOrder::Bible,
        )
        .await;
        if s.search_key() != Some((translation, query, scope)) {
            return;
        }
        s.search.set(match answer {
            Ok(hits) => Search::Ready { key, hits },
            Err(err) => Search::Failed {
                key,
                message: study_error(&err),
            },
        });
        scroll::settle(s);
    });
}

fn count_text(count: usize) -> String {
    if count >= SEARCH_LIMIT as usize {
        format!("Zobrazených prvých {SEARCH_LIMIT} výsledkov")
    } else {
        format!("Výsledkov: {count}")
    }
}

fn current_scope(s: Study) -> String {
    s.route.with(|route| match &route.view {
        StudyView::Search { scope, .. } => scope.clone(),
        _ => SCOPE_ALL.to_string(),
    })
}

fn set_scope(s: Study, wanted: &str) {
    let mut route = s.route.get_untracked();
    if let StudyView::Search { scope, .. } = &mut route.view {
        if scope == wanted {
            return;
        }
        *scope = wanted.to_string();
    }
    s.scroll_plan.set_value(ScrollPlan::Top);
    nav::replace(s, route);
}

/// The reader's favourites that can be searched, in one row above the hits,
/// and ▾ for every installed translation (#983: the header's chips are the
/// reader's, and a phone's focused field hid them). A tap is the reader chip's
/// `switch`: the search runs again in that translation, from the top.
fn translation_row(s: Study) -> impl IntoView {
    let open_list = move |_| nav::open_sheet(s, nav::sheet_route(s, StudySheet::Translations));
    let chips = move || {
        let installed = {
            s.translations.track();
            s.installed_codes()
        };
        let favourites = visible_favourites(&s.favourites.get(), &installed);
        let open = s.route.with(|route| route.translation.clone());
        search_translation_row(
            favourites,
            open.as_deref(),
            study_chip_cap(s.narrow.get()),
            |code| code != REMOTE_TRANSLATION,
        )
    };
    view! {
        <nav
            class="study-scopes study-search-translations"
            data-role="search-translations"
            aria-label="Hľadať v preklade"
        >
            <For
                each=chips
                key=|chip| chip.clone()
                children=move |(_, code)| {
                    let mine = code.clone();
                    let target = code.clone();
                    let pressed = Memo::new(move |_| {
                        s.route
                            .with(|route| route.translation.as_deref() == Some(mine.as_str()))
                    });
                    view! {
                        <button
                            type="button"
                            class="study-chip"
                            data-role="search-translation"
                            data-translation=code.clone()
                            aria-pressed=move || if pressed.get() { "true" } else { "false" }
                            on:click=move |_| switch(s, &target)
                        >
                            {translation_short_code(&code)}
                        </button>
                    }
                }
            />
            <button
                type="button"
                class="study-chip"
                data-role="study-translations-open"
                title="Všetky preklady"
                aria-label="Všetky preklady"
                aria-haspopup="dialog"
                on:click=open_list
            >
                "\u{25BE}"
            </button>
        </nav>
    }
}

fn scope_label(s: Study, scope: &str) -> String {
    match scope {
        SCOPE_ALL => "Celá Biblia".to_string(),
        "ot" => "SZ".to_string(),
        "nt" => "NZ".to_string(),
        book => s
            .with_structure(|found| Some(found?.book(book)?.name.clone()))
            .flatten()
            .unwrap_or_else(|| book.to_string()),
    }
}

/// The book the reader last showed, kept behind the search.
fn book_read(s: Study) -> Option<String> {
    s.chapter.with(|shown| match shown {
        Chapter::Ready(place, _) | Chapter::Missing(place) | Chapter::Failed(place, _) => {
            Some(place.book.clone())
        }
        Chapter::None => None,
    })
}

fn scope_row(s: Study) -> impl IntoView {
    view! {
        <nav class="study-scopes" data-role="search-scopes" aria-label="Rozsah">
            <For
                each=move || search_scopes(book_read(s).as_deref(), &current_scope(s))
                key=|scope| scope.clone()
                children=move |scope| {
                    let mine = scope.clone();
                    let target = scope.clone();
                    let label_for = scope.clone();
                    let pressed = Memo::new(move |_| current_scope(s) == mine);
                    view! {
                        <button
                            type="button"
                            class="study-chip"
                            data-role="search-scope"
                            data-scope=scope
                            aria-pressed=move || if pressed.get() { "true" } else { "false" }
                            on:click=move |_| set_scope(s, &target)
                        >
                            {move || scope_label(s, &label_for)}
                        </button>
                    }
                }
            />
        </nav>
    }
}

#[component]
pub(super) fn SearchResults(s: Study) -> impl IntoView {
    let query = move || {
        s.route.with(|route| match &route.view {
            StudyView::Search { query, .. } => query.clone(),
            _ => String::new(),
        })
    };
    let remote = move || s.translation().as_deref() == Some(REMOTE_TRANSLATION);
    let loaded_for = move || {
        s.search.with(|search| match search {
            Search::Ready { key, .. } | Search::Failed { key, .. } => Some(key.clone()),
            Search::Idle => None,
        })
    };
    let state = move || {
        s.search.with(|search| match search {
            Search::Idle => "loading",
            Search::Ready { .. } => "ready",
            Search::Failed { .. } => "error",
        })
    };
    let hits = move || {
        s.search.with(|search| match search {
            Search::Ready { hits, .. } => hits.iter().cloned().enumerate().collect(),
            _ => Vec::new(),
        })
    };
    let count = move || {
        s.search.with(|search| match search {
            Search::Ready { hits, .. } => Some(hits.len()),
            _ => None,
        })
    };
    let failure = move || {
        s.search.with(|search| match search {
            Search::Failed { message, .. } => Some(message.clone()),
            _ => None,
        })
    };

    view! {
        <main class="study-scroll study-scroll--results" node_ref=s.scroller>
            <section
                class="study-results"
                data-role="search-results"
                data-loaded-for=loaded_for
                data-state=state
            >
                {translation_row(s)}
                {move || (!remote()).then(|| scope_row(s))}
                {move || {
                    match count() {
                        Some(0) if remote() => {
                            view! {
                                <p class="study-empty" data-role="search-empty">
                                    "V NLT sa nedá hľadať. Vyber iný preklad."
                                </p>
                            }
                                .into_any()
                        }
                        Some(0) => {
                            view! {
                                <p class="study-empty" data-role="search-empty">
                                    {format!("Nič sa nenašlo: „{}“", query())}
                                </p>
                            }
                                .into_any()
                        }
                        Some(found) => {
                            view! {
                                <p class="study-results__count" data-role="search-count">
                                    {count_text(found)}
                                </p>
                            }
                                .into_any()
                        }
                        None => ().into_any(),
                    }
                }}
                <For
                    each=hits
                    key=|(index, hit)| (*index, hit.reference.to_human_readable())
                    children=move |(_, hit)| view! { <Hit s=s hit=hit query=query() /> }
                />
                {move || {
                    failure()
                        .map(|message| {
                            view! {
                                <div class="study-error" data-role="search-error" role="alert">
                                    <p>{message}</p>
                                    <button
                                        type="button"
                                        class="study-button"
                                        data-role="study-retry"
                                        on:click=move |_| s.retry.update(|n| *n = n.wrapping_add(1))
                                    >
                                        "Skúsiť znova"
                                    </button>
                                </div>
                            }
                        })
                }}
            </section>
        </main>
    }
}
