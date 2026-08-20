//! Transparent stream-graphics OUTPUT page (`/stream/{slug}`, #709, epic #718).
//!
//! The OBS browser source: a transparent WASM page that cold-loads the output
//! definition over REST, subscribes to `/live/ws`, and renders the active BASE
//! scene + any active OVERLAY scenes inside a fixed 16:9 canvas. This lane ships
//! the IMAGE and COUNTDOWN element kinds; LYRICS/VERSE arrive in #710.
//!
//! Data flow (arch §8): fetch `GET /stream/api/outputs/{slug}/def` -> build the
//! scene map -> render active base + overlays. `StreamState` is applied directly
//! (an unknown scene id OR a higher `config_revision` ⇒ refetch the def);
//! `StreamConfigChanged` ⇒ refetch; `Timers` ⇒ drive the countdown; the def is
//! refetched on every WS reconnect. Content-change crossfades are #716 — this
//! lane swaps scenes without animation.

use leptos::prelude::*;
use presenter_core::{SceneKind, StreamOutputDef, StreamSceneDef, StreamShowState};

use crate::api;
use crate::components::stream::scene_render::SceneRender;
use crate::components::stream::StreamContext;
use crate::ws::stream::{self, StreamWsState, TimersReceipt};

/// Countdown re-derivation cadence (smooth ticking between server `Timers`
/// pushes). 250 ms is well under one second so the displayed value never lags a
/// visible tick.
const TICK_INTERVAL_MS: u32 = 250;

#[component]
pub fn StreamOutputPage(slug: String) -> impl IntoView {
    // OBS alpha: the output must be transparent. `tablet.css` ships a bare global
    // `html { background:#1e293b }`, so overriding only `body` is not enough —
    // mark BOTH the body (class) and the html element (data attr) so
    // `stream_output.css` can force each transparent.
    if let Some(body) = crate::utils::window::document_body() {
        let _ = body.set_attribute("class", "stream");
    }
    if let Some(html) = crate::utils::window::document().document_element() {
        let _ = html.set_attribute("data-stream", "true");
    }

    // Preview mode (editor iframe, `?preview=1&scene=<id>&overlays=<id,id>`, #715
    // extends): force a specific base scene + overlay set and skip the wake lock
    // (mirrors the `/stage?preview=1` precedent, #460 — a preview iframe is not a
    // real display).
    let preview = crate::utils::window::url_flag_enabled("preview");
    let forced_scene: Option<i64> = if preview {
        crate::utils::window::url_param("scene").and_then(|s| s.trim().parse::<i64>().ok())
    } else {
        None
    };
    let forced_overlays: Option<Vec<i64>> = if preview {
        crate::utils::window::url_param("overlays").map(|s| parse_id_list(&s))
    } else {
        None
    };

    if !preview {
        crate::components::stage::wake_lock::start_wake_lock_guard();
    }

    let def = RwSignal::new(None::<StreamOutputDef>);
    let show_state = RwSignal::new(None::<StreamShowState>);

    // Shared reactive context for the countdown element (timers + a ticking now).
    let ctx = StreamContext {
        timers: RwSignal::new(None::<TimersReceipt>),
        now_ms: RwSignal::new(js_sys::Date::now()),
    };
    provide_context(ctx);

    // Tick `now_ms` so the countdown re-derives its remaining time smoothly.
    {
        let now_ms = ctx.now_ms;
        let interval = gloo_timers::callback::Interval::new(TICK_INTERVAL_MS, move || {
            now_ms.set(js_sys::Date::now());
        });
        interval.forget();
    }

    let ws = stream::use_stream_websocket();

    // Cold load: fetch the def + the current timers immediately (before the WS
    // even connects) so the output paints as fast as possible.
    spawn_refetch_def(slug.clone(), def, show_state);
    spawn_refetch_timers(ctx.timers);

    // Resync on every WS (re)connect: the live hub does not replay events, so a
    // config/activation change made while this socket was down/zombie would be
    // lost. A `Memo` on the Connected state dedups per-heartbeat re-sets, so the
    // refetch fires only on real transitions (first connect + each reconnect).
    {
        let slug_c = slug.clone();
        let ws_state = ws.state;
        let connected = Memo::new(move |_| ws_state.get() == StreamWsState::Connected);
        Effect::new(move |_| {
            if connected.get() {
                spawn_refetch_def(slug_c.clone(), def, show_state);
                spawn_refetch_timers(ctx.timers);
            }
        });
    }

    // Apply `StreamState`: directly when the referenced scene is known and the
    // revision is current; otherwise refetch the def (unknown scene id OR a newer
    // config revision means our def is stale).
    {
        let slug_c = slug.clone();
        let ws_stream_state = ws.stream_state;
        Effect::new(move |_| {
            let Some(msg) = ws_stream_state.get() else {
                return;
            };
            if msg.output != slug_c {
                return;
            }
            let local = def.get_untracked();
            let local_rev = local.as_ref().map(|d| d.config_revision).unwrap_or(0);
            let known = match msg.active_scene_id {
                None => true,
                Some(id) => local
                    .as_ref()
                    .map(|d| d.scenes.iter().any(|s| s.id == id))
                    .unwrap_or(false),
            };
            if msg.config_revision > local_rev || !known {
                spawn_refetch_def(slug_c.clone(), def, show_state);
            } else {
                show_state.set(Some(StreamShowState {
                    active_scene_id: msg.active_scene_id,
                    active_overlay_ids: msg.active_overlay_ids.clone(),
                    config_revision: msg.config_revision,
                }));
            }
        });
    }

    // `StreamConfigChanged` ⇒ the config changed under us ⇒ refetch the def.
    {
        let slug_c = slug.clone();
        let ws_config = ws.config_changed;
        Effect::new(move |_| {
            let Some(msg) = ws_config.get() else {
                return;
            };
            if msg.output != slug_c {
                return;
            }
            spawn_refetch_def(slug_c.clone(), def, show_state);
        });
    }

    // Feed WS `Timers` snapshots into the countdown context.
    {
        let ws_timers = ws.timers;
        let ctx_timers = ctx.timers;
        Effect::new(move |_| {
            if let Some(receipt) = ws_timers.get() {
                ctx_timers.set(Some(receipt));
            }
        });
    }

    let ws_state = ws.state;
    let ws_state_attr = move || match ws_state.get() {
        StreamWsState::Connecting => "connecting",
        StreamWsState::Connected => "connected",
        StreamWsState::Reconnecting => "reconnecting",
        StreamWsState::Disconnected => "disconnected",
    };
    let config_rev_attr = move || {
        show_state
            .get()
            .map(|s| s.config_revision.to_string())
            .unwrap_or_default()
    };
    let slug_attr = slug.clone();

    // Render the effective base scene + overlays. In preview mode the forced
    // query values override the live show-state (the editor drives which scene to
    // show); otherwise the live show-state selects them.
    let render_scenes = move || {
        let Some(d) = def.get() else {
            return ().into_any();
        };
        let ss = show_state.get();
        let live_base = ss.as_ref().and_then(|s| s.active_scene_id);
        let live_overlays = ss
            .as_ref()
            .map(|s| s.active_overlay_ids.clone())
            .unwrap_or_default();
        let base_id = if preview {
            forced_scene.or(live_base)
        } else {
            live_base
        };
        let overlay_ids = if preview {
            forced_overlays.clone().unwrap_or(live_overlays)
        } else {
            live_overlays
        };

        let mut scenes: Vec<StreamSceneDef> = Vec::new();
        if let Some(bid) = base_id {
            if let Some(scene) = d
                .scenes
                .iter()
                .find(|s| s.id == bid && s.kind == SceneKind::Base)
            {
                scenes.push(scene.clone());
            }
        }
        for oid in &overlay_ids {
            if let Some(scene) = d
                .scenes
                .iter()
                .find(|s| s.id == *oid && s.kind == SceneKind::Overlay)
            {
                scenes.push(scene.clone());
            }
        }

        scenes
            .into_iter()
            .map(|scene| view! { <SceneRender scene=scene /> }.into_any())
            .collect_view()
            .into_any()
    };

    view! {
        <div
            class="stream-canvas"
            data-role="stream-canvas"
            data-slug=slug_attr
            data-ws-state=ws_state_attr
            data-config-revision=config_rev_attr
        >
            {render_scenes}
        </div>
    }
}

/// Parse a comma-separated id list (`"3,7,9"`) into i64s, dropping unparseable
/// entries. Used only for the preview `overlays=` query param.
fn parse_id_list(s: &str) -> Vec<i64> {
    s.split(',')
        .filter_map(|p| p.trim().parse::<i64>().ok())
        .collect()
}

/// Fetch the output def and derive the show-state from it (cold load / refetch).
fn spawn_refetch_def(
    slug: String,
    def: RwSignal<Option<StreamOutputDef>>,
    show_state: RwSignal<Option<StreamShowState>>,
) {
    leptos::task::spawn_local(async move {
        if let Ok(d) = api::stream::get_output_def(&slug).await {
            let active_overlay_ids = d
                .scenes
                .iter()
                .filter(|s| s.kind == SceneKind::Overlay && s.is_active)
                .map(|s| s.id)
                .collect();
            show_state.set(Some(StreamShowState {
                active_scene_id: d.active_scene_id,
                active_overlay_ids,
                config_revision: d.config_revision,
            }));
            def.set(Some(d));
        }
    });
}

/// Fetch the current timers overview and stamp it with the receipt time.
fn spawn_refetch_timers(timers: RwSignal<Option<TimersReceipt>>) {
    leptos::task::spawn_local(async move {
        if let Ok(overview) = api::timers::get_timers().await {
            timers.set(Some(TimersReceipt {
                overview,
                received_at_ms: js_sys::Date::now(),
            }));
        }
    });
}
