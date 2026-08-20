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
use presenter_core::{
    BibleSlideOutput, SceneKind, StageDisplaySnapshot, StreamOutputDef, StreamSceneDef,
    StreamShowState,
};

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

    // Shared reactive context for the text elements: countdown (timers + a
    // ticking now), lyrics (worship `stage` snapshot) and verse (`bible` slide).
    let ctx = StreamContext {
        timers: RwSignal::new(None::<TimersReceipt>),
        now_ms: RwSignal::new(js_sys::Date::now()),
        stage: RwSignal::new(None::<StageDisplaySnapshot>),
        bible: RwSignal::new(None::<BibleSlideOutput>),
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

    // Fetch (cold load AND resync) on every WS transition INTO Connected: the
    // live hub does not replay events, so a config/activation change made while
    // this socket was down/zombie would be lost. A `Memo` on the Connected state
    // dedups per-heartbeat re-sets, so the fetch fires only on real transitions —
    // the FIRST connect (the cold load) and each reconnect — exactly once each,
    // never a redundant pre-connect + first-connect double fetch.
    {
        let slug_c = slug.clone();
        let ws_state = ws.state;
        let connected = Memo::new(move |_| ws_state.get() == StreamWsState::Connected);
        Effect::new(move |_| {
            if connected.get() {
                spawn_refetch_def(slug_c.clone(), def, show_state);
                spawn_refetch_timers(ctx.timers);
                // Content cold-load (#710): recover the lyrics/verse currently on
                // stage so a (re)connecting OBS source shows the live look at
                // once, not only after the next trigger — parity with timers.
                spawn_refetch_stage(ctx.stage);
                spawn_refetch_bible(ctx.bible);
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
            // The def is "known" only when every scene the event references
            // resolves in our local def — the base id AND every active overlay
            // id (each to an Overlay scene). A missing id means our def predates
            // the scene, so refetch. No def yet ⇒ not known ⇒ refetch.
            let known = local
                .as_ref()
                .map(|d| {
                    let base_ok = match msg.active_scene_id {
                        None => true,
                        Some(id) => d.scenes.iter().any(|s| s.id == id),
                    };
                    let overlays_ok = msg.active_overlay_ids.iter().all(|oid| {
                        d.scenes
                            .iter()
                            .any(|s| s.id == *oid && s.kind == SceneKind::Overlay)
                    });
                    base_ok && overlays_ok
                })
                .unwrap_or(false);
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

    // Feed WS worship `Stage` snapshots into the lyrics context. A snapshot is
    // always `Some` (a broom/clear arrives as a snapshot with `current == None`,
    // which the lyrics element already renders as nothing), so only-Some mirrors
    // the timers wiring above.
    {
        let ws_stage = ws.stage;
        let ctx_stage = ctx.stage;
        Effect::new(move |_| {
            if let Some(snapshot) = ws_stage.get() {
                ctx_stage.set(Some(snapshot));
            }
        });
    }

    // Feed WS `BibleSlide`/`BibleCleared` into the verse context. Unlike stage,
    // `None` is MEANINGFUL here (a `BibleCleared` clears the verse), so this
    // propagates both `Some` and `None`. It runs once on mount setting `None`
    // (harmless) before the async cold-load seeds the current slide.
    {
        let ws_bible = ws.bible;
        let ctx_bible = ctx.bible;
        Effect::new(move |_| {
            ctx_bible.set(ws_bible.get());
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

/// Cold-load the current worship stage snapshot (drives the lyrics element on a
/// fresh connect / reconnect). `current` is layout-independent, so the selected
/// snapshot reflects the live worship content for the worship layouts a lyrics
/// scene is used with.
fn spawn_refetch_stage(stage: RwSignal<Option<StageDisplaySnapshot>>) {
    leptos::task::spawn_local(async move {
        if let Ok(snapshot) = api::stage::get_snapshot().await {
            stage.set(Some(snapshot));
        }
    });
}

/// Cold-load the current Bible slide output (drives the verse element). `None`
/// (no active slide) is the cleared/transparent state.
fn spawn_refetch_bible(bible: RwSignal<Option<BibleSlideOutput>>) {
    leptos::task::spawn_local(async move {
        if let Ok(output) = api::bible::get_active_slide_output().await {
            bible.set(output);
        }
    });
}
