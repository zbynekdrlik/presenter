//! The text search view (spec §6.4): `/bible/search` in the reader's
//! translation, within the scope on the route. A fully quoted query is an
//! exact phrase. A hit opens the reader at its verse.

use leptos::prelude::*;
use presenter_core::bible::sermon::translation_short_code;

use super::scroll::ScrollPlan;
use super::{
    add_to_sermon, compare, nav, Chapter, Search, Study, REMOTE_TRANSLATION, SEARCH_LIMIT,
};
use crate::api::bible::BibleSearchHit;
use crate::api::sermons::SermonPassage;
use crate::state::study::{mark_matches, StudyRoute, StudyView};
use crate::state::study_word::{search_scopes, SCOPE_ALL};

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

#[component]
fn Hit(s: Study, hit: BibleSearchHit, query: String) -> impl IntoView {
    let reference = hit.reference.clone();
    let translation = hit.translation.code.clone();
    let label = format!(
        "{} · {}",
        reference.to_human_readable(),
        translation_short_code(&translation)
    );
    let on_add = {
        let reference = reference.clone();
        let translation = translation.clone();
        move |_| {
            let Some(book) = reference.book_code.clone() else {
                return;
            };
            let verse_start = reference.verse_start;
            add_to_sermon::add(
                s,
                SermonPassage {
                    translation: translation.clone(),
                    book_code: book,
                    chapter: reference.chapter,
                    verse_start,
                    verse_end: reference.verse_end.max(verse_start),
                },
            );
        }
    };
    let has_book = reference.book_code.is_some();
    let on_compare = {
        let reference = reference.clone();
        let translation = translation.clone();
        move |_| {
            let Some(book) = reference.book_code.as_deref() else {
                return;
            };
            compare::open_at(
                s,
                &translation,
                (book, reference.chapter, reference.verse_start),
            );
        }
    };
    let on_open = {
        let reference = reference.clone();
        move |_| {
            let Some(book) = reference.book_code.as_deref() else {
                return;
            };
            let verse = reference.verse_start;
            let route = StudyRoute::reader(&translation, book, reference.chapter)
                .with_verses(Some((verse, reference.verse_end.max(verse))));
            nav::go(s, route, ScrollPlan::Verse(verse));
        }
    };
    let snippet = mark_matches(&hit.text, &query)
        .into_iter()
        .map(|(piece, marked)| {
            if marked {
                view! { <mark>{piece}</mark> }.into_any()
            } else {
                view! { <span>{piece}</span> }.into_any()
            }
        })
        .collect_view();

    view! {
        <div class="study-hit-row">
            <button
                type="button"
                class="study-hit"
                data-role="search-hit"
                data-book-code=reference.book_code.clone().unwrap_or_default()
                data-chapter=reference.chapter
                data-verse=reference.verse_start
                on:click=on_open
            >
                <span class="study-hit__label">{label}</span>
                <span class="study-hit__text">{snippet}</span>
            </button>
            {has_book.then(|| view! {
                <button
                    type="button"
                    class="study-icon"
                    data-role="search-hit-compare"
                    aria-label="Porovnať"
                    on:click=on_compare
                >
                    "\u{2261}"
                </button>
            })}
            <button
                type="button"
                class="study-icon"
                data-role="search-hit-add"
                aria-label="Pridať do kázne"
                on:click=on_add
            >
                "+"
            </button>
        </div>
    }
}
