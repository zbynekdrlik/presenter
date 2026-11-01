//! Every move of the study page (spec §6.2): one browser history entry per
//! place or sheet, `history.state` = its index in `StudyHistory`. A `popstate`
//! restores the view from the URL. Closing a sheet is a Back.

use leptos::prelude::*;
use wasm_bindgen::JsValue;

use super::scroll::{self, ScrollPlan};
use super::{Chapter, Study};
use crate::state::session;
use crate::state::study::{
    reader_label, search_label, StudyRoute, StudySheet, StudyView, KEY_HISTORY, KEY_LAST,
};
use crate::state::study_history::{parse_state, HistoryEntry, StudyHistory};

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
        StudyView::Search { query } => search_label(query, translation),
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
        session::set_persistent(KEY_LAST, &route.to_url());
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
    let entry = entry(s, &route);
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

/// Go to a new place. The sheet over the place being left closes first, so
/// Back never lands on it.
pub(super) fn go(s: Study, route: StudyRoute, plan: ScrollPlan) {
    let current = s.route.get_untracked();
    if current == route {
        // The same place asked for again: the reader may have scrolled away.
        s.scroll_plan.set_value(plan);
        scroll::settle(s);
        return;
    }
    scroll::remember_offset(s);
    if current.sheet.is_some() {
        // It stays a step for Back, but it is no second row of the same place.
        let closed = current.without_sheet();
        let mut step = entry(s, &closed);
        step.listed = false;
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
    if s.history.with_untracked(|history| history.can_go_back()) {
        back();
        return;
    }
    let route = s.route.get_untracked();
    let selected = route.sheet == Some(StudySheet::Actions);
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
        let sheet_only = s.route.with_untracked(|current| {
            current.without_sheet().with_verses(None) == route.without_sheet().with_verses(None)
        });
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
    });
}
