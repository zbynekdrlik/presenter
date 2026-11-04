//! The operator's live socket (#850 B1d, spec §6.7). Every frame is applied
//! by `live_event_handler`, synchronously and in arrival order — the old
//! latest-value signal let a `BibleSlide` overwrite the `Bible` published a
//! moment before it, and a 1.5 s heartbeat overwrite either. The socket is
//! `ws::passive` (5 s silence watchdog, reconnect when a page comes back after
//! more than 30 s hidden — the iOS case), and every reconnect re-reads what
//! the hub never replays.

use std::cell::Cell;
use std::rc::Rc;

use leptos::prelude::*;
use presenter_core::LiveEvent;

use crate::components::stage_follow::on_stage_snapshot;
use crate::state::AppContext;
use crate::ws::passive::{LiveEventHandler, PassiveWsHandle};

/// Where the lazily mounted Menovky view hears every live event (#871): it
/// registers a handler here once it has mounted.
pub(crate) type LiveSink = StoredValue<Option<LiveEventHandler>, LocalStorage>;

/// Bumped by every live event of one kind: a resync answer that was in
/// flight while such an event arrived is older than it and is dropped.
#[derive(Clone, Default)]
pub(super) struct SyncGenerations {
    bible: Rc<Cell<u32>>,
    layout: Rc<Cell<u32>>,
}

fn bump(counter: &Cell<u32>) {
    counter.set(counter.get().wrapping_add(1));
}

/// Apply one live event to the operator's state.
pub(super) fn live_event_handler(
    ctx: &AppContext,
    gens: &SyncGenerations,
    sink: LiveSink,
) -> LiveEventHandler {
    let ctx = ctx.clone();
    let gens = gens.clone();
    Rc::new(move |event: LiveEvent| {
        if let Some(view) = sink.with_value(Clone::clone) {
            view(event.clone());
        }
        apply_event(&ctx, &gens, event);
    })
}

fn apply_event(ctx: &AppContext, gens: &SyncGenerations, event: LiveEvent) {
    match event {
        LiveEvent::Stage { snapshot } => {
            // AbleSet Follow: Live mode opens the stage's song (#853: never
            // in Edit mode). The stage preview always updates.
            on_stage_snapshot(ctx, &snapshot);
            ctx.stage_snapshot.set(Some(snapshot));
        }
        LiveEvent::Timers { overview } => ctx.timers.set(Some(overview)),
        LiveEvent::StageConnection { snapshot } => {
            ctx.stage_connections.update(|conns| {
                match conns.iter_mut().find(|c| c.id == snapshot.id) {
                    Some(existing) => *existing = snapshot,
                    None => conns.push(snapshot),
                }
            })
        }
        LiveEvent::BroadcastLive { enabled } => ctx.broadcast_live.set(enabled),
        LiveEvent::StageLayout { code } => {
            bump(&gens.layout);
            ctx.stage_layout_code.set(code);
        }
        LiveEvent::StageTextMode { mode } => ctx.stage_text_mode.set(mode),
        LiveEvent::Bible { broadcast } => {
            bump(&gens.bible);
            ctx.active_bible_broadcast.set(Some(broadcast));
        }
        LiveEvent::BibleSlide { output } => {
            bump(&gens.bible);
            ctx.active_bible_slide.set(Some(output));
        }
        LiveEvent::BibleCleared => {
            bump(&gens.bible);
            ctx.active_bible_broadcast.set(None);
            ctx.active_bible_slide.set(None);
        }
        LiveEvent::BibleSlidesChanged { .. } => {
            ctx.bible_presentations_version.update(|v| *v += 1);
        }
        LiveEvent::BiblePreferencesChanged { character_limit } => {
            // BibleState may not be mounted; the Bible page reads it from body.
            if let Some(body) = crate::utils::window::document_body() {
                let _ = body.set_attribute("data-bible-char-limit", &character_limit.to_string());
            }
        }
        _ => {}
    }
}

/// `ws_connected` follows the socket; every reconnect after the first one
/// re-reads what changed meanwhile (#574 lists, #799 text mode, and since
/// #850 the Bible state and the stage layout).
pub(super) fn setup_live_sync(ctx: &AppContext, ws: PassiveWsHandle, gens: &SyncGenerations) {
    let ws_connected = ctx.ws_connected;
    Effect::new(move |_| {
        ws_connected.set(ws.state.get() == crate::ws::passive::PassiveWsState::Connected);
    });
    let ctx = ctx.clone();
    let gens = gens.clone();
    Effect::new(move |_| {
        if ws.connects.get() > 1 {
            resync(ctx.clone(), gens.clone());
        }
    });
}

fn resync(ctx: AppContext, gens: SyncGenerations) {
    leptos::task::spawn_local(async move {
        let bible_at = gens.bible.get();
        let layout_at = gens.layout.get();
        let slide = crate::api::bible::get_active_slide_output().await;
        let broadcast = crate::api::bible::get_broadcast().await;
        if gens.bible.get() == bible_at {
            if let (Ok(slide), Ok(broadcast)) = (slide, broadcast) {
                ctx.active_bible_slide.set(slide);
                ctx.active_bible_broadcast.set(broadcast);
            }
        }
        if let Ok(resp) = crate::api::stage::get_layout().await {
            if gens.layout.get() == layout_at {
                ctx.stage_layout_code.set(resp.code);
            }
        }
        if let Ok(libs) = crate::api::libraries::list_libraries().await {
            ctx.libraries.set(libs);
        }
        if let Ok(pls) = crate::api::playlists::list_playlists().await {
            ctx.playlists.set(pls);
        }
        if let Ok(resp) = crate::api::stage::get_text_mode().await {
            ctx.stage_text_mode.set(resp.mode);
        }
        if let Some(lib_id) = ctx.selected_library_id.get_untracked() {
            if let Ok(pres) = crate::api::libraries::list_presentations(&lib_id).await {
                ctx.presentations.set(pres);
            }
        }
    });
}
