use leptos::prelude::*;
use presenter_core::LiveEvent;

use crate::components::header::Header;
use crate::components::library_list::LibraryList;
use crate::components::library_modal::LibraryModals;
use crate::components::playlist_list::PlaylistList;
use crate::components::playlist_modal::PlaylistModals;
use crate::components::presentation_list::PresentationList;
use crate::components::presentation_modal::PresentationModals;
use crate::components::search::SearchResults;
use crate::components::slide_list::SlideList;
use crate::components::timer_panel::TimerPanel;
use crate::components::toast::Toast;
use crate::state::operator::OperatorState;
use crate::state::AppContext;
use crate::ws;

/// Operator page — primary control surface for worship service management.
#[component]
pub fn OperatorPage() -> impl IntoView {
    let ctx = AppContext::new();
    let op = OperatorState::new();

    // Set body attributes on mount
    setup_body_attributes(&ctx);

    // Connect WebSocket and dispatch events to state
    let (_ws_state, last_event) = ws::use_live_websocket();
    setup_ws_dispatch(last_event, &ctx);

    // Load initial data
    load_initial_data(&ctx);

    // Load initial presentation if session has one
    load_session_presentation(&ctx);

    // Fetch stage connections periodically
    setup_stage_monitor(ctx.clone());

    // Keyboard shortcuts
    setup_keyboard_shortcuts(ctx.clone(), op.clone());

    view! {
        <Header ctx=ctx.clone() op=op.clone() />
        <SearchResults ctx=ctx.clone() op=op.clone() />
        <main class="operator__main">
            <section class="operator__worship" data-view-panel="worship">
                <section class="operator__catalog" data-role="catalog">
                    <div class="operator__catalog-top" data-role="catalog-top">
                        <LibraryList ctx=ctx.clone() op=op.clone() />
                        <PlaylistList ctx=ctx.clone() op=op.clone() />
                    </div>
                    <div class="operator__catalog-resizer" data-role="catalog-resizer" aria-hidden="true"></div>
                    <PresentationList ctx=ctx.clone() op=op.clone() />
                </section>
                <SlideList ctx=ctx.clone() op=op.clone() />
            </section>
            <section class="operator__panel operator__panel--bible" data-view-panel="bible">
                <iframe src="/ui/bible" title="Bible Control"></iframe>
            </section>
            <TimerPanel ctx=ctx.clone() />
            <section class="operator__panel operator__panel--settings" data-view-panel="settings">
                <iframe src="/ui/settings" title="Settings" class="operator__settings-frame"></iframe>
            </section>
        </main>
        <Toast ctx=ctx.clone() />
        <LibraryModals ctx=ctx.clone() op=op.clone() />
        <PlaylistModals ctx=ctx.clone() op=op.clone() />
        <PresentationModals ctx=ctx.clone() op=op.clone() />
        <footer class="operator__version"></footer>
    }
}

/// Set body class and data attributes on mount.
fn setup_body_attributes(ctx: &AppContext) {
    if let Some(body) = crate::utils::window::document_body() {
        let _ = body.set_attribute("class", "operator");
        let _ = body.set_attribute("data-view", &ctx.view.get_untracked());
        let _ = body.set_attribute("data-mode", &ctx.mode.get_untracked());
    }
}

/// Dispatch WebSocket events to the appropriate state signals.
fn setup_ws_dispatch(last_event: ReadSignal<Option<LiveEvent>>, ctx: &AppContext) {
    let stage_snapshot = ctx.stage_snapshot;
    let timers = ctx.timers;
    let stage_connections = ctx.stage_connections;

    Effect::new(move || {
        if let Some(event) = last_event.get() {
            match event {
                LiveEvent::Stage { snapshot } => {
                    stage_snapshot.set(Some(snapshot));
                }
                LiveEvent::Timers { overview } => {
                    timers.set(Some(overview));
                }
                LiveEvent::StageConnection { snapshot } => {
                    stage_connections.update(|conns| {
                        // Update or add the connection
                        if let Some(existing) = conns.iter_mut().find(|c| c.id == snapshot.id) {
                            *existing = snapshot.clone();
                        } else {
                            conns.push(snapshot.clone());
                        }
                    });
                }
                _ => {}
            }
        }
    });
}

/// Load initial libraries, playlists, and stage data.
fn load_initial_data(ctx: &AppContext) {
    let libraries = ctx.libraries;
    leptos::task::spawn_local(async move {
        if let Ok(libs) = crate::api::libraries::list_libraries().await {
            libraries.set(libs);
        }
    });

    let playlists = ctx.playlists;
    leptos::task::spawn_local(async move {
        if let Ok(pls) = crate::api::playlists::list_playlists().await {
            playlists.set(pls);
        }
    });

    let stage_snapshot = ctx.stage_snapshot;
    leptos::task::spawn_local(async move {
        if let Ok(snap) = crate::api::stage::get_snapshot().await {
            stage_snapshot.set(Some(snap));
        }
    });

    let timers = ctx.timers;
    leptos::task::spawn_local(async move {
        if let Ok(t) = crate::api::timers::get_timers().await {
            timers.set(Some(t));
        }
    });
}

/// If session has a stored presentation ID, load it.
fn load_session_presentation(ctx: &AppContext) {
    if let Some(pres_id) = ctx.selected_presentation_id.get_untracked() {
        let selected = ctx.selected_presentation;
        leptos::task::spawn_local(async move {
            if let Ok(pres) = crate::api::presentations::get_presentation(&pres_id).await {
                selected.set(Some(pres));
            }
        });
    }

    // Also load presentations for stored library
    if let Some(lib_id) = ctx.selected_library_id.get_untracked() {
        let presentations = ctx.presentations;
        leptos::task::spawn_local(async move {
            if let Ok(pres) = crate::api::libraries::list_presentations(&lib_id).await {
                presentations.set(pres);
            }
        });
    }
}

/// Periodically refresh stage connections.
fn setup_stage_monitor(ctx: AppContext) {
    let connections = ctx.stage_connections;
    // Initial fetch
    leptos::task::spawn_local(async move {
        if let Ok(conns) = crate::api::stage::get_connections().await {
            connections.set(conns);
        }
    });
}

/// Global keyboard shortcuts.
fn setup_keyboard_shortcuts(ctx: AppContext, op: OperatorState) {
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::JsCast;

    let handler =
        Closure::<dyn Fn(web_sys::KeyboardEvent)>::new(move |ev: web_sys::KeyboardEvent| {
            let key = ev.key();

            // Escape closes modals and search
            if key == crate::utils::keyboard::KEY_ESCAPE {
                if op.open_modal.get_untracked().is_some() {
                    crate::components::modal::close_modal(&op);
                    return;
                }
                if op.search_open.get_untracked() {
                    op.search_open.set(false);
                    op.search_query.set(String::new());
                    return;
                }
            }

            // Don't handle shortcuts when focused on input/textarea
            if let Some(active) = crate::utils::window::document().active_element() {
                let tag = active.tag_name();
                if tag == "INPUT" || tag == "TEXTAREA" || tag == "SELECT" {
                    return;
                }
            }

            // Arrow keys for slide navigation in live mode
            if ctx.mode.get_untracked() == "live"
                && (key == crate::utils::keyboard::KEY_ARROW_LEFT
                    || key == crate::utils::keyboard::KEY_ARROW_RIGHT)
            {
                // Slide navigation would need to find prev/next slide
                // This is a placeholder for the full implementation
            }
        });

    let window = crate::utils::window::window();
    let _ = window.add_event_listener_with_callback("keydown", handler.as_ref().unchecked_ref());
    handler.forget();
}
