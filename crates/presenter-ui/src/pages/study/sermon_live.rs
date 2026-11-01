//! An open sermon follows other devices (spec §7.6): `BibleSlidesChanged` of its
//! presentation reads it again; `Editor::apply` ignores anything older. The
//! socket opens with the first sermon, and stays while the page lives.

use std::rc::Rc;

use leptos::prelude::*;
use presenter_core::LiveEvent;

use super::sermon_editor::open_editor;
use super::Study;
use crate::ws::passive::{use_passive_websocket, LiveEventHandler};

fn follow(s: Study, sermon: Option<&str>) {
    if let Some(e) = open_editor(s) {
        if sermon.is_none_or(|id| id == e.id()) {
            e.follow();
        }
    }
}

/// Once per page. A sermon's id is its presentation's. A (re)connect reads
/// again: the hub never replays.
pub(super) fn listen_live(s: Study) {
    if s.live.with_untracked(Option::is_some) {
        return;
    }
    s.live.set(Some(0));
    s.owner.with_value(|owner| {
        owner.with(|| {
            let on_event: LiveEventHandler = Rc::new(move |event| {
                if let LiveEvent::BibleSlidesChanged { presentation_id } = event {
                    follow(s, Some(&presentation_id));
                }
            });
            let ws = use_passive_websocket("study", on_event);
            Effect::new(move |previous: Option<u32>| {
                let connects = ws.connects.get();
                s.live.set(Some(connects));
                if previous.is_some_and(|seen| seen != connects) {
                    follow(s, None);
                }
                connects
            });
        });
    });
}
