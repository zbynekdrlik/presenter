//! An open sermon follows other devices (spec §7.6): `BibleSlidesChanged` of its
//! presentation reads it again; `Editor::apply` ignores anything older.

use std::rc::Rc;

use leptos::prelude::*;
use presenter_core::LiveEvent;

use super::sermon_editor::{open_editor, reload};
use super::Study;
use crate::ws::passive::{use_passive_websocket, LiveEventHandler};

/// Another device wrote: the open sermon reads again. Skipped while a write of
/// ours is out: its answer carries the same or a newer sermon, and a write
/// another device got in between ends in that write's 409 reload.
fn follow(s: Study, sermon: Option<&str>) {
    let Some(e) = open_editor(s) else {
        return;
    };
    if sermon.is_none_or(|id| id == e.id()) && !e.writing() {
        reload(e);
    }
}

/// The socket of the sermon views, once per page. A sermon's id is its
/// presentation's. A (re)connect reads again: the hub never replays.
pub(super) fn listen_live(s: Study) {
    if s.live.get_value() {
        return;
    }
    s.live.set_value(true);
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
                if previous.is_some_and(|seen| seen != connects) {
                    follow(s, None);
                }
                connects
            });
        });
    });
}
