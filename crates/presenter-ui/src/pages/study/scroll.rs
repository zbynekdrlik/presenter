//! Where the reader's scroll goes: a chosen verse after a search, the same
//! verse after a translation switch, the old offset after Back, and the verse
//! last read after a reopen. A plan waits until its chapter is on screen.

use gloo_timers::callback::Timeout;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use super::{Chapter, Study};
use crate::state::session;
use crate::state::study::{chapter_key, StudyRoute, StudyView, KEY_LAST_VERSE};

/// The gap left above a verse scrolled to the top.
const TOP_GAP_PX: f64 = 8.0;
/// The verse in view is saved this long after the last scroll event.
const SAVE_AFTER_MS: u32 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ScrollPlan {
    /// Leave the scroll alone.
    Keep,
    Top,
    Verse(u16),
    Offset(i32),
}

/// The plan for a route that names verses.
pub(super) fn plan_for_route(s: Study) {
    let verse = s.route.with_untracked(|route| route.verses.map(|v| v.0));
    s.scroll_plan
        .set_value(verse.map_or(ScrollPlan::Top, ScrollPlan::Verse));
}

/// Carry out the plan once the route's chapter is the one on screen. The
/// `requestAnimationFrame` lands after the render effects that show it.
pub(super) fn settle(s: Study) {
    let Some(place) = s.place() else {
        return;
    };
    let on_screen = s.chapter.with_untracked(|shown| match shown {
        Chapter::Ready(held, _) => *held == place,
        Chapter::Missing(held) | Chapter::Failed(held, _) => *held == place,
        Chapter::None => false,
    });
    if !on_screen {
        return;
    }
    let plan = s.scroll_plan.get_value();
    s.scroll_plan.set_value(ScrollPlan::Keep);
    if plan != ScrollPlan::Keep {
        request_animation_frame(move || apply(s, plan));
    }
}

fn apply(s: Study, plan: ScrollPlan) {
    let Some(scroller) = s.scroller.get_untracked() else {
        return;
    };
    match plan {
        ScrollPlan::Keep => {}
        ScrollPlan::Top => scroller.set_scroll_top(0),
        ScrollPlan::Offset(offset) => scroller.set_scroll_top(offset),
        ScrollPlan::Verse(verse) => {
            let selector = format!("[data-role=\"study-verse\"][data-verse=\"{verse}\"]");
            if let Ok(Some(found)) = scroller.query_selector(&selector) {
                let distance = found.get_bounding_client_rect().top()
                    - scroller.get_bounding_client_rect().top()
                    - TOP_GAP_PX;
                scroller.set_scroll_top(scroller.scroll_top() + distance as i32);
            }
        }
    }
}

/// The first verse still showing at the top of the reader.
pub(super) fn verse_in_view(s: Study) -> Option<u16> {
    let scroller = s.scroller.get_untracked()?;
    let top = scroller.get_bounding_client_rect().top() + TOP_GAP_PX;
    let verses = scroller
        .query_selector_all("[data-role=\"study-verse\"]")
        .ok()?;
    (0..verses.length())
        .filter_map(|index| verses.item(index))
        .filter_map(|node| node.dyn_into::<web_sys::Element>().ok())
        .find(|element| element.get_bounding_client_rect().bottom() > top)
        .and_then(|element| element.get_attribute("data-verse")?.parse().ok())
}

/// The history entry being left keeps its scroll offset for Back.
pub(super) fn remember_offset(s: Study) {
    let Some(scroller) = s.scroller.get_untracked() else {
        return;
    };
    let index = s.history.with_untracked(|history| history.position());
    let offset = scroller.scroll_top();
    s.offsets.update_value(|offsets| {
        offsets.insert(index, offset);
    });
}

/// Save the verse in view when the scrolling stops (`study.lastVerse`).
pub(super) fn save_after_scroll(s: Study, ticket: StoredValue<u32>) {
    ticket.update_value(|held| *held = held.wrapping_add(1));
    let mine = ticket.get_value();
    Timeout::new(SAVE_AFTER_MS, move || {
        if ticket.try_get_value() != Some(mine) {
            return;
        }
        if let (Some(place), Some(verse)) = (s.place(), verse_in_view(s)) {
            session::set_persistent(KEY_LAST_VERSE, &format!("{}|{verse}", place.key()));
        }
    })
    .forget();
}

/// The verse saved for this chapter, if the saved one is this chapter's.
pub(super) fn saved_verse(route: &StudyRoute) -> Option<u16> {
    let StudyView::Reader { book, chapter } = &route.view else {
        return None;
    };
    let key = chapter_key(route.translation.as_deref()?, book, *chapter);
    let saved = session::get_persistent(KEY_LAST_VERSE)?;
    let (held, verse) = saved.rsplit_once('|')?;
    (held == key).then(|| verse.parse().ok()).flatten()
}
