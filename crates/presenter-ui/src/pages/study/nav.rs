//! Every move of the study page (spec §6.2): one browser history entry per
//! place or sheet, `history.state` = its index in `StudyHistory`. A `popstate`
//! restores the view from the URL. Closing a sheet is a Back.

use leptos::prelude::*;
use wasm_bindgen::JsValue;

use super::scroll::{self, ScrollPlan};
use super::{Chapter, Study};
use crate::state::session;
use crate::state::study::{
    compare_label, reader_label, search_label, StudyRoute, StudySheet, StudyView, KEY_HISTORY,
    KEY_LAST,
};
use crate::state::study_history::{parse_state, HistoryEntry, StudyHistory};
use crate::state::study_word::strongs_label;

fn browser_history() -> Option<web_sys::History> {
    crate::utils::window::window().history().ok()
}

fn book_name(s: Study, route: &StudyRoute, book: &str, chapter: u16) -> String {
    let shown = s.chapter.with_untracked(|shown| match shown {
        Chapter::Ready(place, text) if place.book == book && place.chapter == chapter => {
            Some(text.book.clone())
        }
        _ => None,
    });
    shown
        .or_else(|| {
            let code = route.translation.as_deref()?;
            s.structures
                .with_untracked(|map| Some(map.get(code)?.book(book)?.name.clone()))
        })
        .unwrap_or_else(|| book.to_string())
}

fn entry(s: Study, route: &StudyRoute) -> HistoryEntry {
    let translation = route.translation.as_deref().unwrap_or_default();
    let label = match &route.view {
        StudyView::Reader { book, chapter } => {
            reader_label(&book_name(s, route, book, *chapter), *chapter, translation)
        }
        StudyView::Search { query, .. } => search_label(query, translation),
        StudyView::Compare {
            book,
            chapter,
            verse,
            ..
        } => compare_label(&book_name(s, route, book, *chapter), *chapter, *verse),
        StudyView::Strongs { number, .. } => strongs_label(number),
        StudyView::Sermons => "Kázne".to_string(),
        StudyView::Sermon { .. } => "Kázeň".to_string(),
    };
    HistoryEntry {
        url: route.to_url(),
        label,
        listed: route.sheet.is_none(),
    }
}

/// `study.last` holds the reader's place; the list lives as long as the tab.
fn remember(s: Study, route: &StudyRoute) {
    if route.sheet.is_none() && matches!(route.view, StudyView::Reader { .. }) {
        // Only the place: the sermon beside it may be gone at the next visit.
        let mut place = route.clone();
        place.pane = None;
        session::set_persistent(KEY_LAST, &place.to_url());
    }
    session::set(KEY_HISTORY, &s.history.with_untracked(|h| h.to_stored()));
}

fn state_value(s: Study) -> JsValue {
    JsValue::from_str(&s.history.with_untracked(StudyHistory::state_text))
}

fn push_entry(s: Study, route: StudyRoute) {
    let entry = entry(s, &route);
    let url = entry.url.clone();
    s.history.update(|history| {
        history.push(entry);
    });
    if let Some(history) = browser_history() {
        let _ = history.push_state_with_url(&state_value(s), "", Some(&url));
    }
    remember(s, &route);
    s.route.set(route);
}

/// Change the current entry: a selection that grew, a translation found for a
/// URL without one.
pub(super) fn replace(s: Study, route: StudyRoute) {
    let mut entry = entry(s, &route);
    // A closed sheet's step stays a step: a highlight on it is no second row.
    if route.sheet.is_none() && s.route.with_untracked(|shown| shown.sheet.is_none()) {
        entry.listed = s.history.with_untracked(StudyHistory::current_listed);
    }
    replace_entry(s, route, entry);
}

fn replace_entry(s: Study, route: StudyRoute, entry: HistoryEntry) {
    let url = entry.url.clone();
    s.history.update(|history| history.replace(entry));
    if let Some(history) = browser_history() {
        let _ = history.replace_state_with_url(&state_value(s), "", Some(&url));
    }
    remember(s, &route);
    s.route.set(route);
}

/// Whether the entry behind the current one reads this same place.
fn same_place_behind(s: Study, place: &StudyRoute) -> bool {
    let behind = s
        .history
        .with_untracked(|history| history.previous_url().map(str::to_string));
    behind
        .and_then(|url| StudyRoute::parse(url.split_once('?')?.1))
        .is_some_and(|route| route.with_verses(None) == place.clone().with_verses(None))
}

/// Go to a new place. The sheet over the place being left closes first, so
/// Back never lands on it.
pub(super) fn go(s: Study, route: StudyRoute, plan: ScrollPlan) {
    let current = s.route.get_untracked();
    let route = route.keeping_pane(&current);
    if current == route {
        // The same place asked for again: the reader may have scrolled away.
        s.scroll_plan.set_value(plan);
        scroll::settle(s);
        return;
    }
    scroll::remember_offset(s);
    if current.sheet.is_some() {
        // It stays a step for Back, but it is no second row of the same place.
        // A list that started on the sheet has no other row for it.
        let closed = current.without_sheet();
        let mut step = entry(s, &closed);
        step.listed = !same_place_behind(s, &closed);
        replace_entry(s, closed, step);
    }
    s.scroll_plan.set_value(plan);
    push_entry(s, route);
    scroll::settle(s);
}

/// A sheet over the current place: one Back away from it.
pub(super) fn open_sheet(s: Study, route: StudyRoute) {
    push_entry(s, route);
}

/// Close the open sheet. A sheet that came with the URL has nothing behind it
/// to go back to, so it goes in place, and the verse sheet's selection with it.
pub(super) fn close_sheet(s: Study) {
    if s.route.with_untracked(|route| route.sheet) == Some(StudySheet::Word) {
        close_word_sheet(s);
    } else if s.history.with_untracked(|history| history.can_go_back()) {
        back();
    } else {
        close_in_place(s);
    }
}

fn close_in_place(s: Study) {
    let route = s.route.get_untracked();
    let selected = route.sheet.is_some_and(StudySheet::needs_verses);
    let closed = route.without_sheet();
    replace(
        s,
        if selected {
            closed.with_verses(None)
        } else {
            closed
        },
    );
}

fn is_word_entry(entry: &HistoryEntry) -> bool {
    entry
        .url
        .split_once('?')
        .and_then(|(_, query)| StudyRoute::parse(query))
        .is_some_and(|route| route.sheet == Some(StudySheet::Word))
}

/// What is left to do once the browser finished a walk through the history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum AfterPop {
    /// The walk ended on the list's first entry, still a sheet.
    CloseSheet,
    Go(StudyRoute, ScrollPlan),
}

fn word_run(s: Study) -> (usize, usize) {
    s.history
        .with_untracked(|history| (history.run_behind(is_word_entry), history.behind()))
}

fn walk_then(s: Study, steps: usize, next: AfterPop) {
    s.after_pop.set_value(Some(next));
    step_history(-(steps as i32));
}

/// ✕ and a tap beside it leave the sheet, whatever entries a derivation link
/// put on the way; only Back walks those. A list that starts on the sheet has
/// nothing behind that run: the walk ends on its first entry and closes it there.
fn close_word_sheet(s: Study) {
    let (run, behind) = word_run(s);
    if run == 0 && behind == 0 {
        close_in_place(s);
    } else if run < behind {
        step_history(-((run + 1) as i32));
    } else {
        walk_then(s, run, AfterPop::CloseSheet);
    }
}

/// Leave the word sheet for another place. `go` closes only the top entry, so
/// the entries of derivation links are walked first, or Back would reopen one.
pub(super) fn leave_word_sheet(s: Study, route: StudyRoute, plan: ScrollPlan) {
    match word_run(s).0 {
        0 => go(s, route, plan),
        run => walk_then(s, run, AfterPop::Go(route, plan)),
    }
}

fn finish_walk(s: Study) {
    match s.after_pop.try_update_value(Option::take).flatten() {
        Some(AfterPop::CloseSheet) => close_in_place(s),
        Some(AfterPop::Go(route, plan)) => go(s, route, plan),
        None => {}
    }
}

pub(super) fn back() {
    if let Some(history) = browser_history() {
        let _ = history.back();
    }
}

pub(super) fn step_history(steps: i32) {
    if let Some(history) = browser_history() {
        let _ = history.go_with_delta(steps);
    }
}

pub(super) fn sheet_route(s: Study, sheet: StudySheet) -> StudyRoute {
    s.route.get_untracked().with_sheet(Some(sheet))
}

/// The chapter's real book name reached the history row.
pub(super) fn label_current(s: Study, book: &str) {
    let route = s.route.get_untracked();
    let (StudyView::Reader { chapter, .. }, Some(translation)) =
        (&route.view, route.translation.as_deref())
    else {
        return;
    };
    let label = reader_label(book, *chapter, translation);
    s.history.update(|history| history.relabel(label));
    remember(s, &route);
}

/// This list's index in a `history.state`; `None` for an entry of no list or of
/// another one.
fn index_in_list(s: Study, state: &JsValue) -> Option<usize> {
    let state = state.as_string()?;
    let (generation, index) = parse_state(&state)?;
    let ours = s
        .history
        .with_untracked(|history| history.generation() == generation);
    ours.then_some(index)
}

/// A new list at this entry: the tab came in from elsewhere (or from an older
/// list), and what lies behind in the browser is not the study page's. ◀ and a
/// sheet's ✕ only ever walk this list, so they can never leave the page.
fn start_list(s: Study, route: &StudyRoute) {
    let generation = format!(
        "{:x}{:x}",
        js_sys::Date::now() as u64,
        (js_sys::Math::random() * 1e9) as u64
    );
    let first = entry(s, route);
    let url = first.url.clone();
    s.history.set(StudyHistory::starting(generation, first));
    s.offsets.update_value(std::collections::HashMap::clear);
    if let Some(history) = browser_history() {
        let _ = history.replace_state_with_url(&state_value(s), "", Some(&url));
    }
}

/// Is this page a reload of (or a return to) an entry the browser kept?
pub(super) fn has_entry_state() -> bool {
    browser_history()
        .and_then(|history| history.state().ok())
        .is_some_and(|state| state.is_string())
}

/// Take the place of the current history entry and follow the browser's moves.
pub(super) fn install(s: Study) {
    let route = s.route.get_untracked();
    let state = browser_history()
        .and_then(|history| history.state().ok())
        .unwrap_or(JsValue::NULL);
    match index_in_list(s, &state) {
        // A reload: the same entry.
        Some(index) => s.history.update(|history| history.moved_to(index)),
        None => start_list(s, &route),
    }
    remember(s, &route);

    let _ = window_event_listener(leptos::ev::popstate, move |event| {
        let search = crate::utils::window::window()
            .location()
            .search()
            .unwrap_or_default();
        let Some(route) = StudyRoute::parse(&search) else {
            return;
        };
        scroll::remember_offset(s);
        let index = index_in_list(s, &event.state());
        match index {
            Some(index) => s.history.update(|history| history.moved_to(index)),
            None => start_list(s, &route),
        }
        let sheet_only = s
            .route
            .with_untracked(|current| current.only_sheet_differs(&route));
        if !sheet_only {
            let offset = index
                .and_then(|index| s.offsets.with_value(|offsets| offsets.get(&index).copied()));
            let opening = route
                .verses
                .map_or(ScrollPlan::Top, |(verse, _)| ScrollPlan::Verse(verse));
            s.scroll_plan
                .set_value(offset.map_or(opening, ScrollPlan::Offset));
        }
        remember(s, &route);
        s.route.set(route);
        scroll::settle(s);
        finish_walk(s);
    });
}
