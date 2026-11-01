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
use crate::state::study_history::HistoryEntry;

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

fn push_entry(s: Study, route: StudyRoute) {
    let entry = entry(s, &route);
    let url = entry.url.clone();
    let mut index = 0;
    s.history.update(|history| index = history.push(entry));
    if let Some(history) = browser_history() {
        let _ = history.push_state_with_url(&JsValue::from_f64(index as f64), "", Some(&url));
    }
    remember(s, &route);
    s.route.set(route);
}

/// Change the current entry: a selection that grew, a translation found for a
/// URL without one.
pub(super) fn replace(s: Study, route: StudyRoute) {
    let entry = entry(s, &route);
    let url = entry.url.clone();
    s.history.update(|history| history.replace(entry));
    let index = s.history.with_untracked(|history| history.position());
    if let Some(history) = browser_history() {
        let _ = history.replace_state_with_url(&JsValue::from_f64(index as f64), "", Some(&url));
    }
    remember(s, &route);
    s.route.set(route);
}

/// Go to a new place. The sheet over the place being left closes first, so
/// Back never lands on it.
pub(super) fn go(s: Study, route: StudyRoute, plan: ScrollPlan) {
    let current = s.route.get_untracked();
    if current == route {
        return;
    }
    scroll::remember_offset(s);
    if current.sheet.is_some() {
        replace(s, current.without_sheet());
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
/// to go back to, so it goes in place.
pub(super) fn close_sheet(s: Study) {
    if s.history.with_untracked(|history| history.can_go_back()) {
        back();
    } else {
        let route = s.route.get_untracked().without_sheet();
        replace(s, route);
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
    let mut updated = entry(s, &route);
    updated.label = label;
    s.history.update(|history| history.replace(updated));
    remember(s, &route);
}

/// Take the place of the current history entry and follow the browser's moves.
pub(super) fn install(s: Study) {
    let route = s.route.get_untracked();
    let index = browser_history()
        .and_then(|history| history.state().ok())
        .and_then(|state| state.as_f64());
    match index {
        // A reload: the same entry.
        Some(index) => s.history.update(|history| history.moved_to(index as usize)),
        None => {
            let entry = entry(s, &route);
            let mut index = 0;
            s.history.update(|history| index = history.push(entry));
            if let Some(history) = browser_history() {
                let _ = history.replace_state_with_url(
                    &JsValue::from_f64(index as f64),
                    "",
                    Some(&route.to_url()),
                );
            }
        }
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
        let index = event.state().as_f64().map(|index| index as usize);
        if let Some(index) = index {
            s.history.update(|history| history.moved_to(index));
        }
        let sheet_only = s.route.with_untracked(|current| {
            current.without_sheet().with_verses(None) == route.without_sheet().with_verses(None)
        });
        if !sheet_only {
            let offset = index
                .and_then(|index| s.offsets.with_value(|offsets| offsets.get(&index).copied()));
            s.scroll_plan
                .set_value(offset.map_or(ScrollPlan::Keep, ScrollPlan::Offset));
        }
        remember(s, &route);
        s.route.set(route);
        scroll::settle(s);
    });
}
