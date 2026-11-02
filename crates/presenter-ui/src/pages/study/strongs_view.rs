//! Where a Strong's number occurs (spec §6.6): the KJV's verses in canonical
//! order, its words marked, scope chips with their counts, 100 per page. The
//! list stays in memory, so Back from a verse finds it, appended pages and
//! scroll included, without asking again.

use leptos::prelude::*;

use super::scroll::{self, ScrollPlan};
use super::word_sheet::Lexicon;
use super::{nav, study_error, Study};
use crate::api::study::{load_occurrences, Occurrence, OccurrencePage};
use crate::state::study::StudyRoute;
use crate::state::study::StudyView;
use crate::state::study_word::{
    marked_segments, occurrences_key, scope_chips, ScopeChip, STRONGS_TRANSLATION,
};

/// The "Ďalšie" button's state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum More {
    Idle,
    Loading,
    Failed(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) enum Occurrences {
    #[default]
    Idle,
    Ready {
        key: String,
        total: u64,
        items: Vec<Occurrence>,
        more: More,
    },
    Failed {
        key: String,
        message: String,
    },
}

/// `(number, scope)` of the route.
fn on_route(s: Study) -> Option<(String, String)> {
    s.route.with(|route| match &route.view {
        StudyView::Strongs { number, scope } => Some((number.clone(), scope.clone())),
        _ => None,
    })
}

fn route_key(s: Study) -> Option<String> {
    on_route(s).map(|(number, scope)| occurrences_key(&number, &scope))
}

/// The route's list is on screen: the scroll may land.
pub(super) fn settled(s: Study) -> bool {
    let key = untrack(|| route_key(s));
    s.occurrences.with_untracked(|held| match held {
        Occurrences::Ready { key: shown, .. } => Some(shown) == key.as_ref(),
        _ => false,
    })
}

fn fetch_first(s: Study, number: String, scope: String) {
    let key = occurrences_key(&number, &scope);
    leptos::task::spawn_local(async move {
        let answer = load_occurrences(&number, &scope, 0).await;
        // A scope left behind has no place to land.
        if untrack(|| route_key(s)).as_ref() != Some(&key) {
            return;
        }
        s.occurrences.set(match answer {
            Ok(page) => {
                s.book_counts.set(Some((number, page.per_book)));
                Occurrences::Ready {
                    key,
                    total: page.total,
                    items: page.items,
                    more: More::Idle,
                }
            }
            Err(err) => Occurrences::Failed {
                key,
                message: study_error(&err),
            },
        });
        scroll::settle(s);
    });
}

pub(super) fn install(s: Study) {
    let wanted = Memo::new(move |_| on_route(s));
    Effect::new(move |_| {
        s.retry.track();
        let Some((number, scope)) = wanted.get() else {
            return;
        };
        let key = occurrences_key(&number, &scope);
        let held = s.occurrences.with_untracked(|held| match held {
            Occurrences::Ready { key: shown, .. } => *shown == key,
            _ => false,
        });
        if !held {
            s.occurrences.set(Occurrences::Idle);
            fetch_first(s, number, scope);
        }
    });
}

/// A page lands only on the list that asked for it, right after what it holds.
fn land_page(s: Study, key: &str, offset: usize, answer: Result<OccurrencePage, String>) {
    s.occurrences.update(|held| {
        let Occurrences::Ready {
            key: shown,
            items,
            more,
            ..
        } = held
        else {
            return;
        };
        if shown != key || items.len() != offset {
            return;
        }
        match answer {
            Ok(page) => {
                items.extend(page.items);
                *more = More::Idle;
            }
            Err(message) => *more = More::Failed(message),
        }
    });
}

fn load_more(s: Study) {
    let Some((number, scope)) = on_route(s) else {
        return;
    };
    let key = occurrences_key(&number, &scope);
    let mut offset = None;
    s.occurrences.update(|held| {
        if let Occurrences::Ready {
            key: shown,
            items,
            more,
            ..
        } = held
        {
            if *shown == key && *more != More::Loading {
                *more = More::Loading;
                offset = Some(items.len());
            }
        }
    });
    let Some(offset) = offset else {
        return;
    };
    leptos::task::spawn_local(async move {
        let answer = load_occurrences(&number, &scope, offset)
            .await
            .map_err(|err| study_error(&err));
        land_page(s, &key, offset, answer);
    });
}

fn set_scope(s: Study, wanted: &str) {
    let mut route = s.route.get_untracked();
    if let StudyView::Strongs { scope, .. } = &mut route.view {
        if scope == wanted {
            return;
        }
        *scope = wanted.to_string();
    }
    s.scroll_plan.set_value(ScrollPlan::Top);
    nav::replace(s, route);
}

fn chip_label(s: Study, chip: &ScopeChip) -> String {
    let name = chip.label.map(ToString::to_string).unwrap_or_else(|| {
        s.with_structure(|found| Some(found?.book(&chip.scope)?.name.clone()))
            .flatten()
            .unwrap_or_else(|| chip.scope.clone())
    });
    format!("{name} ({})", chip.count)
}

#[component]
fn Hit(s: Study, number: String, item: Occurrence) -> impl IntoView {
    let label = format!("{} {}:{} · KJV", item.book, item.chapter, item.verse);
    let segments = item.markup.as_ref().map(|markup| markup.0.as_slice());
    let text = marked_segments(&item.text, segments, &number)
        .into_iter()
        .map(|(piece, marked)| {
            if marked {
                view! { <mark data-role="strongs-mark">{piece}</mark> }.into_any()
            } else {
                view! { <span>{piece}</span> }.into_any()
            }
        })
        .collect_view();
    let (book, chapter, verse) = (item.book_code.clone(), item.chapter, item.verse);
    let on_open = move |_| {
        let route = StudyRoute::reader(STRONGS_TRANSLATION, &book, chapter)
            .with_verses(Some((verse, verse)));
        nav::go(s, route, ScrollPlan::Verse(verse));
    };
    view! {
        <button
            type="button"
            class="study-hit"
            data-role="strongs-hit"
            data-book-code=item.book_code.clone()
            data-chapter=chapter
            data-verse=verse
            on:click=on_open
        >
            <span class="study-hit__label">{label}</span>
            <span class="study-hit__text">{text}</span>
        </button>
    }
}

fn scope_row(s: Study) -> impl IntoView {
    let chips = move || {
        let (number, current) = on_route(s).unwrap_or_default();
        s.book_counts.with(|held| match held {
            Some((counted, per_book)) if *counted == number => {
                let counts: Vec<_> = per_book
                    .iter()
                    .map(|book| (book.book_code.clone(), book.count))
                    .collect();
                scope_chips(&counts, &current)
            }
            _ => Vec::new(),
        })
    };
    view! {
        <nav class="study-scopes" data-role="search-scopes" aria-label="Rozsah">
            <For
                each=chips
                key=|chip| chip.clone()
                children=move |chip| {
                    let scope = chip.scope.clone();
                    let mine = chip.scope.clone();
                    let role_scope = chip.scope.clone();
                    let pressed = Memo::new(move |_| {
                        on_route(s).is_some_and(|(_, current)| current == mine)
                    });
                    view! {
                        <button
                            type="button"
                            class="study-chip"
                            data-role="search-scope"
                            data-scope=role_scope
                            aria-pressed=move || if pressed.get() { "true" } else { "false" }
                            on:click=move |_| set_scope(s, &scope)
                        >
                            {move || chip_label(s, &chip)}
                        </button>
                    }
                }
            />
        </nav>
    }
}

fn more_view(s: Study, left: u64, more: More) -> impl IntoView {
    let failure = match &more {
        More::Failed(message) => Some(message.clone()),
        _ => None,
    };
    view! {
        {failure.map(|message| {
            view! {
                <p class="study-results__count" data-role="strongs-more-error" role="alert">
                    {message}
                </p>
            }
        })}
        <button
            type="button"
            class="study-button"
            data-role="strongs-more"
            disabled=more == More::Loading
            on:click=move |_| load_more(s)
        >
            {format!("Ďalšie ({left})")}
        </button>
    }
}

#[component]
pub(super) fn StrongsView(s: Study) -> impl IntoView {
    let number = move || on_route(s).map(|(number, _)| number).unwrap_or_default();
    let lemma = move || {
        s.lexicon.with(|entries| match entries.get(&number()) {
            Some(Lexicon::Ready(entry)) => Some(entry.lemma.clone()),
            _ => None,
        })
    };
    let rtl = move || number().starts_with('H').then_some("rtl");
    let loaded_for = move || {
        s.occurrences.with(|held| match held {
            Occurrences::Ready { key, .. } | Occurrences::Failed { key, .. } => Some(key.clone()),
            Occurrences::Idle => None,
        })
    };
    let state = move || {
        s.occurrences.with(|held| match held {
            Occurrences::Idle => "loading",
            Occurrences::Ready { .. } => "ready",
            Occurrences::Failed { .. } => "error",
        })
    };
    let items = move || {
        s.occurrences.with(|held| match held {
            Occurrences::Ready { items, .. } => items.clone(),
            _ => Vec::new(),
        })
    };
    let count = move || {
        s.occurrences.with(|held| match held {
            Occurrences::Ready { total: 0, .. } => Some(view! {
                <p class="study-empty" data-role="strongs-empty">"V tomto rozsahu sa toto číslo nevyskytuje."</p>
            }.into_any()),
            Occurrences::Ready { total, .. } => Some(view! {
                <p class="study-results__count" data-role="strongs-count">{format!("Veršov: {total}")}</p>
            }.into_any()),
            _ => None,
        })
    };
    let more = move || {
        s.occurrences.with(|held| match held {
            Occurrences::Ready {
                total, items, more, ..
            } => {
                let left = total.saturating_sub(items.len() as u64);
                (left > 0).then(|| more_view(s, left, more.clone()))
            }
            _ => None,
        })
    };
    let failure = move || {
        s.occurrences.with(|held| match held {
            Occurrences::Failed { message, .. } => Some(message.clone()),
            _ => None,
        })
    };
    view! {
        <main class="study-scroll study-scroll--results" data-role="study-scroll" node_ref=s.scroller>
            <section
                class="study-results"
                data-role="strongs-results"
                data-loaded-for=loaded_for
                data-state=state
            >
                <h2 class="study-results__title" data-role="strongs-title">
                    {number}
                    " "
                    <span dir=rtl>{lemma}</span>
                </h2>
                {scope_row(s)}
                {count}
                <For
                    each=items
                    key=|item| (item.book_code.clone(), item.chapter, item.verse)
                    children=move |item| view! { <Hit s=s number=number() item=item /> }
                />
                {more}
                {move || {
                    failure()
                        .map(|message| {
                            view! {
                                <div class="study-error" data-role="strongs-error" role="alert">
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
