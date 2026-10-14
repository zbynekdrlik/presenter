//! Interpreter display `/ui/interpreter` (#852 spec §8). Its own page, NOT a
//! stage layout: it never follows the global layout, so the preacher's
//! `preach` timer keeps working. `?layout=tablet` (default) is S1b;
//! `?layout=tv` arrives with unit C5b.
//!
//! The tablet layout is a grid with a slot per area (§8.2): transcript
//! (left), side column (tips, Bible, numbers and names) and the status bar.
//! Areas render only when they have data, so until listening exists the tips
//! area is the whole page (§8.1, §9.6) — `data-transcript="absent"`. C5a
//! fills the transcript and Bible slots without moving the tips.
//!
//! Data: the main live hub through the per-event `ws::passive` socket
//! (`InterpreterTips` = the full active list) and `GET /interpreter/tips` on
//! load and after every (re)connect (the hub never replays).

use crate::components::interpreter::{TipsArea, TipsModel};
use crate::components::version_label::VersionLabel;
use crate::state::interpreter::InterpreterLayout;
use crate::ws::passive::{
    use_passive_websocket, LiveEventHandler, PassiveWsHandle, PassiveWsState,
};
use leptos::prelude::*;
use presenter_core::LiveEvent;
use std::rc::Rc;

#[component]
pub fn InterpreterPage() -> impl IntoView {
    if let Some(body) = crate::utils::window::document_body() {
        let _ = body.set_attribute("class", "interpreter");
    }
    let layout =
        InterpreterLayout::from_param(crate::utils::window::url_param("layout").as_deref());
    let model = TipsModel::new();
    let on_event: LiveEventHandler = Rc::new(move |event| {
        if let LiveEvent::InterpreterTips { tips, .. } = event {
            model.apply_live(tips);
        }
    });
    let ws = use_passive_websocket("interpreter", on_event);
    // Resync on load (connects = 0) and after every (re)connect.
    Effect::new(move |_| {
        let _ = ws.connects.get();
        model.resync();
    });
    view! {
        <div class="interpreter__root" data-role="interpreter" data-layout=layout.as_str()>
            {match layout {
                InterpreterLayout::Tablet => view! { <TabletLayout model=model /> }.into_any(),
                InterpreterLayout::Tv => view! { <TvLayoutPending /> }.into_any(),
            }}
            <StatusBar ws=ws />
        </div>
    }
}

/// Spec §8.2. `data-transcript` stays "absent" until C5a adds the transcript
/// column; the side column then takes its 38 %.
#[component]
fn TabletLayout(model: TipsModel) -> impl IntoView {
    view! {
        <main class="interpreter__grid" data-role="interpreter-grid" data-transcript="absent">
            <aside class="interpreter__side" data-role="interpreter-side">
                <TipsArea model=model />
            </aside>
        </main>
    }
}

/// `?layout=tv` is unit C5b (spec §16); until then the page says so
/// instead of guessing a layout for a stage TV.
#[component]
fn TvLayoutPending() -> impl IntoView {
    view! {
        <main class="interpreter__tv-pending" data-role="interpreter-tv-pending">
            <p>"Rozloženie pre televízor pripravujeme. Na tablete otvor /ui/interpreter."</p>
        </main>
    }
}

/// Connection dot + version (spec §8.2 bottom bar; C5a adds the mic, ASR and
/// type controls).
#[component]
fn StatusBar(ws: PassiveWsHandle) -> impl IntoView {
    let label = move || match ws.state.get() {
        PassiveWsState::Connected => "spojenie OK",
        PassiveWsState::Connecting => "pripájam…",
        PassiveWsState::Reconnecting => "bez spojenia — obnovujem…",
    };
    view! {
        <footer class="interpreter__status" data-role="interpreter-status">
            <span
                class="interpreter__dot"
                data-role="interpreter-ws"
                data-ws=move || ws.state.get().as_str()
            ></span>
            <span>{label}</span>
            <span class="interpreter__version"><VersionLabel /></span>
        </footer>
    }
}
