//! The sermon beside the text on a wide screen (spec §7.6): the preacher reads
//! on the left and builds on the right. The route stays the reader's or the
//! search's and `pane` names the sermon, so everything about the text works as
//! it does alone, and a move in the text leaves the editor where it is.

use leptos::prelude::*;

use super::add_to_sermon::set_active;
use super::scroll::{saved_verse, ScrollPlan};
use super::sermon_editor::SermonEditor;
use super::{nav, start_route, Study};
use crate::state::phone_shell::watch_media;
use crate::state::session;
use crate::state::study::{StudyRoute, StudyView, KEY_LAST};

/// The width the sheets dock at too (`study.css`).
const WIDE_QUERY: &str = "(min-width: 1024px)";

/// Where a sermon opened on a wide screen reads: the last place.
fn reading_route() -> StudyRoute {
    session::get_persistent(KEY_LAST)
        .and_then(|url| StudyRoute::parse(url.split_once('?')?.1))
        .unwrap_or_else(start_route)
}

/// Keep the route on the layout this width has.
pub(super) fn follow_width(s: Study) {
    watch_media(WIDE_QUERY, move |wide| {
        if s.wide.get_untracked() != wide {
            s.wide.set(wide);
        }
    });
    Effect::new(move |_| {
        let wide = s.wide.get();
        let route = s.route.get();
        let Some(next) = route.for_width(wide, reading_route()) else {
            return;
        };
        if let StudyView::Sermon { id } = &route.view {
            // What the reader adds goes to the sermon beside it.
            set_active(s, Some(id.clone()));
            let verse = saved_verse(&next);
            s.scroll_plan
                .set_value(verse.map_or(ScrollPlan::Top, ScrollPlan::Verse));
        }
        nav::replace(s, next);
    });
}

#[component]
pub(super) fn SermonPane(s: Study) -> impl IntoView {
    let open = Memo::new(move |_| s.route.with(|route| route.pane.clone()));
    move || {
        open.get().map(|id| {
            view! {
                <aside class="study-sermon-pane" data-role="study-sermon-pane">
                    <SermonEditor s=s id=id />
                </aside>
            }
        })
    }
}
