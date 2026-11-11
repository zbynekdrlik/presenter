//! The chapter (spec §6.3): verse numbers in a 44 px gutter (each is a control,
//! so a superscript in the line could not be tapped), a swipe or the bottom bar
//! to the next chapter. `data-loaded-for` is the settle marker: its render is
//! the one that shows that chapter.

use leptos::prelude::*;
use presenter_core::bible::resolver::Step;

use super::markup_view::{NumbersChip, VerseBody};
use super::scroll::{save_on_scroll, ScrollPlan};
use super::swipe::SwipeTracker;
use super::{compare, nav, Chapter, ChapterPlace, Study};
use crate::components::version_label::VersionLabel;
use crate::state::study::{
    extend_selection, missing_text, step_chapter, step_label, StudyRoute, StudySheet, StudyVerse,
    StudyView,
};
use crate::state::study_word::has_numbers;

fn step_target(s: Study, dir: Step) -> Option<(String, u16)> {
    let (book, chapter) = s.route.with(|route| match &route.view {
        StudyView::Reader { book, chapter } => Some((book.clone(), *chapter)),
        _ => None,
    })?;
    s.with_structure(|structure| step_chapter(structure?, &book, chapter, dir))?
}

fn turn(s: Study, dir: Step) {
    let Some((book, chapter)) = s.place().and_then(|_| step_target(s, dir)) else {
        return;
    };
    let translation = s.route.with_untracked(|route| route.translation.clone());
    if let Some(code) = translation {
        nav::go(
            s,
            StudyRoute::reader(&code, &book, chapter),
            ScrollPlan::Top,
        );
    }
}

/// The text on screen is the route's chapter. While another one loads the URL
/// has moved on: a verse number there would select in the wrong chapter.
pub(super) fn chapter_settled(s: Study) -> bool {
    let wanted = s.place();
    s.pending.with(Option::is_none)
        && s.chapter.with(|chapter| match chapter {
            Chapter::Ready(place, _) => Some(place) == wanted.as_ref(),
            _ => false,
        })
}

fn on_verse_tap(s: Study, verse: u16) {
    if !chapter_settled_untracked(s) {
        return;
    }
    let route = s.route.get_untracked();
    if route.sheet == Some(StudySheet::Actions) {
        match extend_selection(route.verses, verse) {
            Some(range) => nav::replace(s, route.with_verses(Some(range))),
            None => nav::close_sheet(s),
        }
    } else {
        nav::open_sheet(
            s,
            route
                .with_verses(Some((verse, verse)))
                .with_sheet(Some(StudySheet::Actions)),
        );
    }
}

/// A tap on a verse's text highlights it, so the eye knows where it is (owner,
/// 2026-11-06); a second tap lets go. No history entry: it is the same place.
/// With the verse sheet open the tap grows the selection like the number.
fn on_text_tap(s: Study, verse: u16, on_word: bool) {
    if !chapter_settled_untracked(s) {
        return;
    }
    let route = s.route.get_untracked();
    match route.sheet {
        Some(StudySheet::Actions) => {
            if !on_word {
                on_verse_tap(s, verse);
            }
        }
        None => {
            let again = route.verses == Some((verse, verse)) && !on_word;
            let verses = (!again).then_some((verse, verse));
            if route.verses != verses {
                nav::replace(s, route.with_verses(verses));
            }
        }
        Some(_) => {}
    }
}

pub(super) fn chapter_settled_untracked(s: Study) -> bool {
    leptos::prelude::untrack(|| chapter_settled(s))
}

/// The verse's number at the start of its line (#932), a control (T3): it
/// opens the verse sheet and is never also a tap on the text around it.
#[component]
fn VerseNumber(s: Study, number: u16, settled: Memo<bool>) -> impl IntoView {
    view! {
        <button
            type="button"
            class="study-verse__number"
            data-role="study-verse-number"
            aria-label=format!("Verš {number}")
            disabled=move || !settled.get()
            on:click=move |ev| {
                ev.stop_propagation();
                on_verse_tap(s, number);
            }
        >
            {number}
        </button>
    }
}

#[component]
pub(super) fn Reader(s: Study) -> impl IntoView {
    let swipe = StoredValue::new(SwipeTracker::default());
    let settled = Memo::new(move |_| chapter_settled(s));
    let save_ticket = StoredValue::new(0_u32);
    let loaded = Memo::new(move |_| {
        s.chapter.with(|chapter| match chapter {
            Chapter::Ready(place, _) => Some(place.clone()),
            _ => None,
        })
    });
    let held = move |read: fn(&ChapterPlace) -> String| {
        s.chapter.with(|chapter| match chapter {
            Chapter::Ready(place, _) | Chapter::Missing(place) | Chapter::Failed(place, _) => {
                Some(read(place))
            }
            Chapter::None => None,
        })
    };
    let state = move || {
        if s.pending.with(Option::is_some) {
            return "loading";
        }
        s.chapter.with(|chapter| match chapter {
            Chapter::None => "loading",
            Chapter::Ready(..) => "ready",
            Chapter::Missing(_) => "missing",
            Chapter::Failed(..) => "error",
        })
    };
    let verses = move || {
        s.chapter.with(|chapter| match chapter {
            Chapter::Ready(place, text) => text
                .verses
                .iter()
                .map(|verse| (place.key(), verse.clone()))
                .collect(),
            _ => Vec::new(),
        })
    };
    let numbered = Memo::new(move |_| {
        s.chapter.with(|chapter| match chapter {
            Chapter::Ready(_, text) => has_numbers(&text.verses),
            _ => false,
        })
    });
    let missing = move || {
        s.chapter.with(|chapter| {
            matches!(chapter, Chapter::Missing(_))
                .then(|| s.with_structure(missing_text).unwrap_or(missing_text(None)))
        })
    };
    let failure = move || {
        s.chapter.with(|chapter| match chapter {
            Chapter::Failed(_, message) => Some(message.clone()),
            _ => None,
        })
    };

    view! {
        <main
            class="study-scroll"
            data-role="study-scroll"
            node_ref=s.scroller
            use:save_on_scroll=(s, save_ticket)
            on:pointerdown=move |ev| swipe.update_value(|held| held.down(&ev))
            on:pointerup=move |ev| {
                if let Some(dir) = swipe.try_update_value(|held| held.up(&ev)).flatten() {
                    turn(s, dir);
                }
            }
            on:pointercancel=move |_| swipe.update_value(SwipeTracker::cancel)
        >
            <article
                class="study-chapter"
                data-role="study-chapter"
                data-translation=move || held(|place| place.translation.clone())
                data-book-code=move || held(|place| place.book.clone())
                data-chapter=move || held(|place| place.chapter.to_string())
                data-loaded-for=move || loaded.get().map(|place| place.key())
                data-state=state
            >
                // No chapter title (#932): the header names the chapter on
                // screen, the pressed chip its translation.
                <div class="study-chapter__head">
                    <NumbersChip s=s shown=numbered />
                </div>
                <For
                    each=verses
                    key=|(key, verse)| (key.clone(), verse.verse)
                    children=move |(_, verse): (String, StudyVerse)| {
                        let number = verse.verse;
                        let selected = Memo::new(move |_| {
                            s.route
                                .with(|route| {
                                    route
                                        .verses
                                        .is_some_and(|(a, b)| (a..=b).contains(&number))
                                })
                        });
                        view! {
                            <div
                                class="study-verse"
                                data-role="study-verse"
                                data-verse=number
                                data-selected=move || if selected.get() { "true" } else { "false" }
                            >
                                <VerseBody
                                    s=s
                                    verse=verse
                                    class="study-verse__text"
                                    on_tap=Callback::new(move |on_word| on_text_tap(s, number, on_word))
                                    lead=Box::new(move || {
                                        view! { <VerseNumber s=s number=number settled=settled /> }
                                            .into_any()
                                    })
                                />
                            </div>
                        }
                    }
                />
                {move || {
                    missing()
                        .map(|text| {
                            view! {
                                <p class="study-empty" data-role="study-missing">
                                    {text}
                                </p>
                            }
                        })
                }}
                {move || {
                    failure()
                        .map(|message| {
                            view! {
                                <div class="study-error" data-role="study-chapter-error" role="alert">
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
                <footer class="study-version"><VersionLabel /></footer>
            </article>
        </main>
    }
}

/// `◀ kap. 10 · Porovnať · kap. 12 ▶`, above the safe area.
#[component]
pub(super) fn ChapterBar(s: Study) -> impl IntoView {
    let target = move |dir| Memo::new(move |_| step_target(s, dir));
    let previous = target(Step::Prev);
    let next = target(Step::Next);
    let known = Memo::new(move |_| s.with_structure(|found| found.is_some()) == Some(true));
    let label = move |wanted: Memo<Option<(String, u16)>>, end: &'static str| {
        // Without the structure neither end of the Bible is known.
        if !known.get() {
            return "\u{2026}".to_string();
        }
        let Some((code, chapter)) = wanted.get() else {
            return end.to_string();
        };
        let book = s.route.with(|route| match &route.view {
            StudyView::Reader { book, .. } => book.clone(),
            _ => String::new(),
        });
        s.with_structure(|structure| {
            structure.map(|found| step_label(found, &book, (&code, chapter)))
        })
        .flatten()
        .unwrap_or_else(|| format!("kap. {chapter}"))
    };

    view! {
        <footer class="study-bar">
            <button
                type="button"
                class="study-button study-bar__step"
                data-role="study-prev-chapter"
                disabled=move || previous.with(Option::is_none)
                on:click=move |_| turn(s, Step::Prev)
            >
                <span aria-hidden="true">"\u{25C0}"</span>
                <span class="study-bar__label">{move || label(previous, "Začiatok")}</span>
            </button>
            <button
                type="button"
                class="study-button study-bar__compare"
                data-role="study-compare-open"
                disabled=move || s.place().is_none()
                on:click=move |_| compare::open_from_reader(s)
            >
                "Porovnať"
            </button>
            <button
                type="button"
                class="study-button study-bar__step"
                data-role="study-next-chapter"
                disabled=move || next.with(Option::is_none)
                on:click=move |_| turn(s, Step::Next)
            >
                <span class="study-bar__label">{move || label(next, "Koniec")}</span>
                <span aria-hidden="true">"\u{25B6}"</span>
            </button>
        </footer>
    }
}
