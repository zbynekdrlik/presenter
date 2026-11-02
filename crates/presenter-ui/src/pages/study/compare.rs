//! Compare (spec §6.5): on a phone one verse across the favourites, ◀ ▶ and a
//! swipe move by verse; wider, the chapter in 2 or 3 columns aligned by verse
//! number. Each translation loads on its own, so a failed NLT costs only its
//! rows. A step replaces the entry: Back returns to where compare was opened.

use std::collections::HashMap;

use leptos::prelude::*;
use presenter_core::bible::resolver::Step;
use presenter_core::bible::sermon::translation_short_code;

use super::markup_view::{NumbersChip, VerseBody};
use super::reader::chapter_settled_untracked;
use super::scroll::{self, ScrollPlan};
use super::swipe::SwipeTracker;
use super::{nav, study_error, ChapterPlace, Study};
use crate::api::sermons::is_not_found;
use crate::api::study::{load_chapter, ChapterText};
use crate::state::study::{
    chapter_key, step_chapter, visible_favourites, StudyRoute, StudyVerse, StudyView,
};
use crate::state::study_compare::{
    fitted, listed_translations, switch_column, verse_numbers, CompareLayout, MISSING_VERSE,
};
use crate::state::study_word::{has_numbers, STRONGS_TRANSLATION};

/// One translation's chapter in the compare view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CompareCell {
    Loading,
    Ready(ChapterText),
    Failed(String),
}

/// The verse the compare view is on.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Spot {
    book: String,
    chapter: u16,
    verse: u16,
}

fn spot(s: Study) -> Option<Spot> {
    s.route.with(|route| match &route.view {
        StudyView::Compare {
            book,
            chapter,
            verse,
            ..
        } => Some(Spot {
            book: book.clone(),
            chapter: *chapter,
            verse: *verse,
        }),
        _ => None,
    })
}

fn layout(s: Study) -> CompareLayout {
    CompareLayout::for_screen(s.phone.get(), s.wide.get())
}

/// Every translation of the view, whatever fits on this screen.
fn listed(s: Study) -> Vec<String> {
    let (named, reader) = s.route.with(|route| match &route.view {
        StudyView::Compare { translations, .. } => {
            (translations.clone(), route.translation.clone())
        }
        _ => (Vec::new(), None),
    });
    let installed = s.installed_codes();
    let favourites: Vec<String> = visible_favourites(&s.favourites.get(), &installed)
        .into_iter()
        .map(|(_, code)| code)
        .collect();
    listed_translations(&named, reader.as_deref(), &favourites, &installed)
}

fn shown(s: Study) -> Vec<String> {
    fitted(listed(s), layout(s))
}

/// What compare holds of the numbered text, for the word sheet.
pub(super) enum NumberedText {
    /// Not answered yet.
    Waiting,
    /// No KJV column on this screen.
    Absent,
    /// The KJV column's read failed: its retry may still bring the word.
    Failed,
    /// `only` = the one verse a phone shows.
    Here {
        verses: Vec<StudyVerse>,
        only: Option<u16>,
    },
}

pub(super) fn numbered_text(s: Study) -> NumberedText {
    let Some(at) = spot(s) else {
        return NumberedText::Absent;
    };
    if s.translations.with(Vec::is_empty) {
        return NumberedText::Waiting;
    }
    if !shown(s).iter().any(|code| code == STRONGS_TRANSLATION) {
        return NumberedText::Absent;
    }
    let key = chapter_key(STRONGS_TRANSLATION, &at.book, at.chapter);
    s.compare.with(|cells| match cells.get(&key) {
        Some(CompareCell::Ready(chapter)) => NumberedText::Here {
            verses: chapter.verses.clone(),
            only: (layout(s) == CompareLayout::Stacked).then_some(at.verse),
        },
        Some(CompareCell::Failed(_)) => NumberedText::Failed,
        Some(CompareCell::Loading) | None => NumberedText::Waiting,
    })
}

fn shows_numbers(s: Study) -> bool {
    let NumberedText::Here { verses, .. } = numbered_text(s) else {
        return false;
    };
    has_numbers(&verses)
}

fn place_of(spot: &Spot, translation: &str) -> ChapterPlace {
    ChapterPlace {
        translation: translation.to_string(),
        book: spot.book.clone(),
        chapter: spot.chapter,
    }
}

/// Ask for what is not there yet. A chapter left behind drops its cell, so its
/// late answer has nowhere to land.
fn load(s: Study, places: Vec<ChapterPlace>) {
    let wanted: Vec<String> = places.iter().map(ChapterPlace::key).collect();
    s.compare
        .update(|cells| cells.retain(|key, _| wanted.contains(key)));
    for place in places {
        let key = place.key();
        let holds = s.compare.with_untracked(|cells| {
            matches!(
                cells.get(&key),
                Some(CompareCell::Ready(_) | CompareCell::Loading)
            )
        });
        if holds {
            continue;
        }
        if let Some(text) = s.cache.try_update_value(|cache| cache.get(&key)).flatten() {
            s.compare.update(|cells| {
                cells.insert(key, CompareCell::Ready(text));
            });
            continue;
        }
        s.compare.update(|cells| {
            cells.insert(key.clone(), CompareCell::Loading);
        });
        leptos::task::spawn_local(fetch(s, place, key));
    }
}

async fn fetch(s: Study, place: ChapterPlace, key: String) {
    let answer = load_chapter(&place.translation, &place.book, place.chapter).await;
    // A step back to this chapter must not pay for it twice (the NLT's limit).
    if let Ok(text) = &answer {
        if !text.verses.is_empty() {
            s.cache
                .update_value(|cache| cache.put(key.clone(), text.clone()));
        }
    }
    let waiting = s
        .compare
        .with_untracked(|cells| cells.get(&key) == Some(&CompareCell::Loading));
    if !waiting {
        return;
    }
    let cell = match answer {
        Ok(text) => CompareCell::Ready(text),
        Err(err) if is_not_found(&err) => CompareCell::Ready(ChapterText {
            book: String::new(),
            verses: Vec::new(),
        }),
        Err(err) => CompareCell::Failed(study_error(&err)),
    };
    s.compare.update(|cells| {
        cells.insert(key, cell);
    });
}

pub(super) fn install(s: Study) {
    let wanted = Memo::new(move |_| {
        let Some(spot) = spot(s) else {
            return Vec::new();
        };
        shown(s)
            .iter()
            .map(|code| place_of(&spot, code))
            .collect::<Vec<_>>()
    });
    Effect::new(move |_| {
        s.retry.track();
        load(s, wanted.get());
    });
}

fn open(s: Study, verse: u16) {
    let Some(place) = s.place() else {
        return;
    };
    let route = StudyRoute::compare(
        Some(place.translation),
        (&place.book, place.chapter, verse),
        Vec::new(),
    );
    nav::go(s, route, ScrollPlan::Top);
}

/// From a search hit, in the hit's translation.
pub(super) fn open_at(s: Study, translation: &str, place: (&str, u16, u16)) {
    let route = StudyRoute::compare(Some(translation.to_string()), place, Vec::new());
    nav::go(s, route, ScrollPlan::Top);
}

/// From the bottom bar: the selected verse, else the one in view.
pub(super) fn open_from_reader(s: Study) {
    let selected = s.route.with_untracked(|route| route.verses.map(|v| v.0));
    // While a chapter loads, the text on screen is the previous one.
    let in_view = || chapter_settled_untracked(s).then(|| scroll::verse_in_view(s))?;
    open(s, selected.or_else(in_view).unwrap_or(1));
}

pub(super) fn open_from_selection(s: Study) {
    if let Some(verse) = s.route.with_untracked(|route| route.verses.map(|v| v.0)) {
        open(s, verse);
    }
}

/// The verse or chapter one step away; a phone walks verses across chapters.
fn step_target(s: Study, dir: Step) -> Option<Spot> {
    let at = spot(s)?;
    let stacked = layout(s) == CompareLayout::Stacked;
    s.with_structure(|structure| {
        let structure = structure?;
        if stacked {
            // A chapter the reader's translation lacks has no verse steps.
            let stepped = structure.step(&at.book, at.chapter, at.verse, dir);
            let (book, chapter, verse) = stepped.or_else(|| {
                step_chapter(structure, &at.book, at.chapter, dir)
                    .map(|(book, chapter)| (book, chapter, 1))
            })?;
            Some(Spot {
                book,
                chapter,
                verse,
            })
        } else {
            let (book, chapter) = step_chapter(structure, &at.book, at.chapter, dir)?;
            Some(Spot {
                book,
                chapter,
                verse: 1,
            })
        }
    })?
}

fn go_to(s: Study, target: Spot) {
    let mut route = s.route.get_untracked();
    if let StudyView::Compare {
        book,
        chapter,
        verse,
        ..
    } = &mut route.view
    {
        *book = target.book;
        *chapter = target.chapter;
        *verse = target.verse;
    }
    nav::replace(s, route);
    s.scroll_plan.set_value(ScrollPlan::Keep);
}

fn turn(s: Study, dir: Step) {
    if let Some(target) = step_target(s, dir) {
        go_to(s, target);
    }
}

/// Back restores its offset; anything else lands on the verse.
fn land(s: Study) {
    let plan = s.scroll_plan.get_value();
    s.scroll_plan.set_value(ScrollPlan::Keep);
    match plan {
        ScrollPlan::Offset(offset) => request_animation_frame(move || {
            if let Some(scroller) = s.scroller.get_untracked() {
                scroller.set_scroll_top(offset);
            }
        }),
        _ => scroll_to_current(s),
    }
}

/// The columns open at their verse; a phone shows nothing else.
fn scroll_to_current(s: Study) {
    request_animation_frame(move || {
        let Some(scroller) = s.scroller.get_untracked() else {
            return;
        };
        let found = scroller
            .query_selector("[data-role=\"compare-row\"][data-current=\"true\"]")
            .ok()
            .flatten();
        let target = match found {
            Some(row) if layout_untracked(s) != CompareLayout::Stacked => {
                let header = 60.0;
                let distance = row.get_bounding_client_rect().top()
                    - scroller.get_bounding_client_rect().top()
                    - header;
                scroller.scroll_top() + distance as i32
            }
            _ => 0,
        };
        scroller.set_scroll_top(target);
    });
}

fn layout_untracked(s: Study) -> CompareLayout {
    untrack(|| layout(s))
}

fn open_reader(s: Study, translation: &str, at: &Spot, verse: u16) {
    nav::go(
        s,
        StudyRoute::reader(translation, &at.book, at.chapter).with_verses(Some((verse, verse))),
        ScrollPlan::Verse(verse),
    );
}

fn switch(s: Study, column: usize, code: &str) {
    let next = switch_column(&listed(s), column, code);
    let mut route = s.route.get_untracked();
    if let StudyView::Compare { translations, .. } = &mut route.view {
        *translations = next;
    }
    nav::replace(s, route);
}

/// What one translation has at one verse.
fn row(
    s: Study,
    cells: &HashMap<String, CompareCell>,
    code: &str,
    at: &Spot,
    verse: u16,
) -> AnyView {
    let key = place_of(at, code).key();
    let held = match cells.get(&key) {
        Some(CompareCell::Failed(message)) => {
            let message = message.clone();
            return view! {
                <div class="study-compare__text" data-kind="failed" data-role="compare-failed">
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
            .into_any();
        }
        Some(CompareCell::Ready(chapter)) => chapter
            .verses
            .iter()
            .find(|held| held.verse == verse)
            .cloned(),
        Some(CompareCell::Loading) | None => {
            return view! {
                <p class="study-compare__text" data-kind="loading">"Načítava sa\u{2026}"</p>
            }
            .into_any();
        }
    };
    match held {
        Some(verse) => view! {
            <VerseBody s=s verse=verse class="study-compare__text" kind="text" />
        }
        .into_any(),
        None => view! {
            <p class="study-compare__text" data-kind="missing">{MISSING_VERSE}</p>
        }
        .into_any(),
    }
}

fn row_view(
    s: Study,
    cells: &HashMap<String, CompareCell>,
    code: &str,
    at: &Spot,
    verse: u16,
    label: String,
) -> impl IntoView {
    let current = verse == at.verse;
    let body = row(s, cells, code, at, verse);
    let (target, place) = (code.to_string(), at.clone());
    view! {
        <div
            class="study-compare__row"
            data-role="compare-row"
            data-translation=code.to_string()
            data-verse=verse
            data-current=if current { "true" } else { "false" }
        >
            <button
                type="button"
                class="study-compare__open"
                data-role="compare-open"
                on:click=move |_| open_reader(s, &target, &place, verse)
            >
                {label}
            </button>
            {body}
        </div>
    }
}

fn stacked(s: Study, codes: &[String], at: &Spot) -> AnyView {
    s.compare.with(|cells| {
        codes
            .iter()
            .map(|code| {
                let label = translation_short_code(code).to_string();
                row_view(s, cells, code, at, at.verse, label)
            })
            .collect_view()
            .into_any()
    })
}

fn column_chip(s: Study, column: usize, code: String) -> impl IntoView {
    let options = s
        .translations
        .get()
        .into_iter()
        .map(|translation| {
            let selected = translation.code == code;
            let label = translation_short_code(&translation.code).to_string();
            view! {
                <option value=translation.code prop:selected=selected>{label}</option>
            }
        })
        .collect_view();
    view! {
        <select
            class="study-compare__column"
            data-role="compare-column"
            aria-label=format!("Stĺpec {}", column + 1)
            on:change=move |ev| switch(s, column, &event_target_value(&ev))
        >
            {options}
        </select>
    }
}

fn chips(s: Study, codes: &[String]) -> impl IntoView {
    codes
        .iter()
        .enumerate()
        .map(|(column, code)| column_chip(s, column, code.clone()))
        .collect_view()
}

fn columns(s: Study, codes: &[String], at: &Spot) -> AnyView {
    s.compare.with(|cells| {
        let numbers =
            verse_numbers(codes.iter().filter_map(
                |code| match cells.get(&place_of(at, code).key()) {
                    Some(CompareCell::Ready(chapter)) => Some(chapter.verses.as_slice()),
                    _ => None,
                },
            ));
        // Before any answer the one verse keeps the grid's shape.
        let numbers = if numbers.is_empty() {
            vec![at.verse]
        } else {
            numbers
        };
        numbers
            .into_iter()
            .flat_map(|verse| {
                codes
                    .iter()
                    .map(move |code| row_view(s, cells, code, at, verse, verse.to_string()))
            })
            .collect_view()
            .into_any()
    })
}

/// The book's name and `11:1`.
fn title_parts(s: Study, at: &Spot, codes: &[String]) -> (String, String) {
    let book = s.compare.with(|cells| {
        codes
            .iter()
            .find_map(|code| match cells.get(&place_of(at, code).key()) {
                Some(CompareCell::Ready(chapter)) if !chapter.book.is_empty() => {
                    Some(chapter.book.clone())
                }
                _ => None,
            })
    });
    (
        book.unwrap_or_else(|| at.book.clone()),
        format!("{}:{}", at.chapter, at.verse),
    )
}

fn title(s: Study, at: &Spot, codes: &[String]) -> String {
    let (book, place) = title_parts(s, at, codes);
    format!("{book} {place}")
}

#[component]
pub(super) fn CompareView(s: Study) -> impl IntoView {
    let swipe = StoredValue::new(SwipeTracker::default());
    let codes = Memo::new(move |_| shown(s));
    let place = Memo::new(move |_| spot(s));
    let settled = Memo::new(move |_| {
        let at = place.get()?;
        let codes = codes.get();
        let ready = !codes.is_empty()
            && s.compare.with(|cells| {
                codes.iter().all(|code| {
                    matches!(
                        cells.get(&place_of(&at, code).key()),
                        Some(CompareCell::Ready(_) | CompareCell::Failed(_))
                    )
                })
            });
        ready.then(|| format!("{}.{}.{}", at.book, at.chapter, at.verse))
    });
    // A retry passes through "not settled": it lands nowhere, a rotation does.
    Effect::new(move |previous: Option<Option<(String, &'static str)>>| {
        let landed = settled.get().map(|key| (key, layout(s).key()));
        if landed.is_none() {
            return previous.flatten();
        }
        if previous.flatten() != landed {
            land(s);
        }
        landed
    });
    let columns_count = move || match layout(s) {
        CompareLayout::Columns(_) => Some(codes.with(Vec::len).max(1)),
        CompareLayout::Stacked => None,
    };
    let column_chips =
        move || matches!(layout(s), CompareLayout::Columns(_)).then(|| chips(s, &codes.get()));
    let body = move || {
        let at = place.get()?;
        let codes = codes.get();
        Some(match layout(s) {
            CompareLayout::Stacked => stacked(s, &codes, &at),
            CompareLayout::Columns(_) => columns(s, &codes, &at),
        })
    };
    let heading = move || place.get().map(|at| title(s, &at, &codes.get()));
    let numbered = Memo::new(move |_| shows_numbers(s));
    // The bar has little room: a long name gives way, the place never does.
    let bar_part = move |part: usize| {
        let (book, place) = title_parts(s, &place.get()?, &codes.get());
        Some(if part == 0 { book } else { format!(" {place}") })
    };
    let step = move |dir| Memo::new(move |_| step_target(s, dir));
    let (previous, next) = (step(Step::Prev), step(Step::Next));

    view! {
        <main
            class="study-scroll"
            data-role="study-scroll"
            node_ref=s.scroller
            on:pointerdown=move |ev| swipe.update_value(|held| held.down(&ev))
            on:pointerup=move |ev| {
                if let Some(dir) = swipe.try_update_value(|held| held.up(&ev)).flatten() {
                    turn(s, dir);
                }
            }
            on:pointercancel=move |_| swipe.update_value(SwipeTracker::cancel)
        >
            <section
                class="study-compare"
                data-role="study-compare"
                data-layout=move || layout(s).key()
                data-columns=columns_count
                data-loaded-for=move || settled.get()
            >
                <div class="study-chapter__head">
                    <h2 class="study-compare__title" data-role="compare-title">{heading}</h2>
                    <NumbersChip s=s shown=numbered />
                </div>
                <div class="study-compare__grid">{column_chips} {body}</div>
            </section>
        </main>
        <footer class="study-bar study-bar--compare">
            <button
                type="button"
                class="study-button study-bar__step"
                data-role="compare-prev"
                aria-label="Predchádzajúci"
                disabled=move || previous.with(Option::is_none)
                on:click=move |_| turn(s, Step::Prev)
            >
                <span aria-hidden="true">"\u{25C0}"</span>
            </button>
            <span class="study-compare__nav" data-role="compare-place">
                <span class="study-compare__book">{move || bar_part(0)}</span>
                <span class="study-compare__verse">{move || bar_part(1)}</span>
            </span>
            <button
                type="button"
                class="study-button study-bar__step"
                data-role="compare-next"
                aria-label="Nasledujúci"
                disabled=move || next.with(Option::is_none)
                on:click=move |_| turn(s, Step::Next)
            >
                <span aria-hidden="true">"\u{25B6}"</span>
            </button>
        </footer>
    }
}
