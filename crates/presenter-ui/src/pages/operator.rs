use leptos::prelude::*;
use presenter_core::LiveEvent;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

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

#[component]
pub fn OperatorPage() -> impl IntoView {
    let ctx = AppContext::new();
    let op = OperatorState::new();

    // Provide context for all child components
    provide_context(ctx.clone());
    provide_context(op.clone());

    // Set body attributes
    setup_body_attributes(&ctx);

    // Reactive body attribute sync
    {
        let view = ctx.view;
        let mode = ctx.mode;
        let mobile_nav_open = op.mobile_nav_open;
        let line_limit = op.line_limit;
        Effect::new(move || {
            if let Some(body) = crate::utils::window::document_body() {
                let _ = body.set_attribute("data-view", &view.get());
                let _ = body.set_attribute("data-mode", &mode.get());

                // Sync mobile nav class
                if mobile_nav_open.get() {
                    let _ = body.class_list().add_1("operator--mobile-nav-open");
                } else {
                    let _ = body.class_list().remove_1("operator--mobile-nav-open");
                }

                // Sync line-limit CSS custom property
                let ll = line_limit.get();
                let _ = body
                    .style()
                    .set_property("--operator-line-limit-ch", &format!("{ll}ch"));
            }
        });
    }

    // Connect WebSocket
    let (ws_state, last_event) = ws::use_live_websocket();

    // Track ws connected state
    {
        let ws_connected = ctx.ws_connected;
        Effect::new(move || {
            ws_connected.set(ws_state.get() == ws::WsState::Connected);
        });
    }

    // Dispatch WS events
    setup_ws_dispatch(last_event, &ctx);

    // Load initial data
    load_initial_data(&ctx);

    // Load session state
    load_session_presentation(&ctx);

    // Stage monitor polling
    setup_stage_monitor(ctx.clone());

    // Keyboard shortcuts
    setup_keyboard_shortcuts(ctx.clone(), op.clone());

    // Expose test helpers
    crate::utils::test_helpers::expose_globals(&ctx, &op);

    // Catalog resizer
    let catalog_top_style = move || format!("height: {}px", op.catalog_top_height.get());

    view! {
        <Header />
        <SearchResults />
        <main class="operator__main">
            <section class="operator__worship" data-view-panel="worship">
                <section class="operator__catalog" data-role="catalog">
                    <div
                        class="operator__catalog-top"
                        data-role="catalog-top"
                        style=catalog_top_style
                    >
                        <LibraryList />
                        <PlaylistList />
                    </div>
                    <CatalogResizer />
                    <PresentationList />
                </section>
                <SlideList />
            </section>
            <section class="operator__panel operator__panel--bible" data-view-panel="bible">
                <iframe src="/ui/bible" title="Bible Control"></iframe>
            </section>
            <section class="operator__panel operator__panel--timers" data-view-panel="timers">
                <TimerPanel />
            </section>
            <section class="operator__panel operator__panel--settings" data-view-panel="settings">
                <iframe src="/ui/settings" title="Settings" class="operator__settings-frame"></iframe>
            </section>
        </main>
        <Toast />
        <LibraryModals />
        <PlaylistModals />
        <PresentationModals />
        <footer class="operator__version"></footer>
    }
}

#[component]
fn CatalogResizer() -> impl IntoView {
    let op = use_context::<OperatorState>().expect("OperatorState");

    let on_pointerdown = move |ev: web_sys::PointerEvent| {
        let start_y = ev.client_y() as f64;
        let start_height = op.catalog_top_height.get_untracked();
        let pointer_id = ev.pointer_id();

        let target = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok());
        if let Some(el) = &target {
            let _ = el.set_pointer_capture(pointer_id);
        }

        let on_move: Closure<dyn Fn(web_sys::PointerEvent)> =
            Closure::new(move |ev: web_sys::PointerEvent| {
                let dy = ev.client_y() as f64 - start_y;
                let new_height = (start_height + dy).clamp(200.0, 520.0);
                op.catalog_top_height.set(new_height);
            });

        let on_up: Closure<dyn Fn(web_sys::PointerEvent)> =
            Closure::new(move |_ev: web_sys::PointerEvent| {
                let height = op.catalog_top_height.get_untracked();
                crate::state::session::set("catalogTopHeight", &height.to_string());
            });

        if let Some(el) = &target {
            let _ = el
                .add_event_listener_with_callback("pointermove", on_move.as_ref().unchecked_ref());
            let _ =
                el.add_event_listener_with_callback("pointerup", on_up.as_ref().unchecked_ref());
        }
        on_move.forget();
        on_up.forget();
    };

    view! {
        <div
            class="operator__catalog-resizer"
            data-role="catalog-resizer"
            aria-hidden="true"
            on:pointerdown=on_pointerdown
        ></div>
    }
}

fn setup_body_attributes(ctx: &AppContext) {
    if let Some(body) = crate::utils::window::document_body() {
        let _ = body.set_attribute("class", "operator");
        let _ = body.set_attribute("data-view", &ctx.view.get_untracked());
        let _ = body.set_attribute("data-mode", &ctx.mode.get_untracked());
    }
}

fn setup_ws_dispatch(last_event: ReadSignal<Option<LiveEvent>>, ctx: &AppContext) {
    let stage_snapshot = ctx.stage_snapshot;
    let timers = ctx.timers;
    let stage_connections = ctx.stage_connections;
    let broadcast_live = ctx.broadcast_live;
    let stage_layout_code = ctx.stage_layout_code;

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
                        if let Some(existing) = conns.iter_mut().find(|c| c.id == snapshot.id) {
                            *existing = snapshot.clone();
                        } else {
                            conns.push(snapshot.clone());
                        }
                    });
                }
                LiveEvent::BroadcastLive { enabled } => {
                    broadcast_live.set(enabled);
                }
                LiveEvent::StageLayout { code } => {
                    stage_layout_code.set(code);
                }
                _ => {}
            }
        }
    });
}

fn load_initial_data(ctx: &AppContext) {
    // Libraries
    let libraries = ctx.libraries;
    leptos::task::spawn_local(async move {
        if let Ok(libs) = crate::api::libraries::list_libraries().await {
            libraries.set(libs);
        }
    });

    // Favorites
    let fav_ids = ctx.favorite_library_ids;
    leptos::task::spawn_local(async move {
        if let Ok(favs) = crate::api::libraries::get_favorites().await {
            fav_ids.set(favs.into_iter().collect());
        }
    });

    // Playlists
    let playlists = ctx.playlists;
    leptos::task::spawn_local(async move {
        if let Ok(pls) = crate::api::playlists::list_playlists().await {
            playlists.set(pls);
        }
    });

    // Stage snapshot
    let stage_snapshot = ctx.stage_snapshot;
    leptos::task::spawn_local(async move {
        if let Ok(snap) = crate::api::stage::get_snapshot().await {
            stage_snapshot.set(Some(snap));
        }
    });

    // Timers
    let timers = ctx.timers;
    leptos::task::spawn_local(async move {
        if let Ok(t) = crate::api::timers::get_timers().await {
            timers.set(Some(t));
        }
    });

    // Stage layouts
    let layouts = ctx.stage_layouts;
    let layout_code = ctx.stage_layout_code;
    leptos::task::spawn_local(async move {
        if let Ok(ls) = crate::api::stage::get_layouts().await {
            layouts.set(ls);
        }
        if let Ok(resp) = crate::api::stage::get_layout().await {
            layout_code.set(resp.code);
        }
    });

    // AbleSet status
    let ableset_status = ctx.ableset_status;
    leptos::task::spawn_local(async move {
        if let Ok(status) = crate::api::settings::get_ableset_status().await {
            ableset_status.set(Some(status));
        }
    });

    // Broadcast live
    let broadcast_live = ctx.broadcast_live;
    leptos::task::spawn_local(async move {
        if let Ok(resp) = crate::api::stage::get_broadcast_live().await {
            broadcast_live.set(resp.enabled);
        }
    });
}

fn load_session_presentation(ctx: &AppContext) {
    if let Some(pres_id) = ctx.selected_presentation_id.get_untracked() {
        let selected = ctx.selected_presentation;
        leptos::task::spawn_local(async move {
            if let Ok(detail) = crate::api::presentations::get_presentation(&pres_id).await {
                selected.set(Some(detail.presentation));
            }
        });
    }

    if let Some(lib_id) = ctx.selected_library_id.get_untracked() {
        let presentations = ctx.presentations;
        let context_title = ctx.context_title;
        let libraries = ctx.libraries;
        leptos::task::spawn_local(async move {
            if let Ok(libs) = crate::api::libraries::list_libraries().await {
                if let Some(lib) = libs.iter().find(|l| l.id.to_string() == lib_id) {
                    context_title.set(lib.name.clone());
                    presentations.set(lib.presentations.clone());
                }
                libraries.set(libs);
            }
        });
    }

    if let Some(pl_id) = ctx.selected_playlist_id.get_untracked() {
        let playlists = ctx.playlists;
        let context_title = ctx.context_title;
        let presentations = ctx.presentations;
        leptos::task::spawn_local(async move {
            if let Ok(pls) = crate::api::playlists::list_playlists().await {
                if let Some(pl) = pls.iter().find(|p| p.id.to_string() == pl_id) {
                    context_title.set(pl.name.clone());
                    let summaries: Vec<presenter_core::PresentationSummary> = pl
                        .entries
                        .iter()
                        .filter_map(|e| match &e.kind {
                            presenter_core::playlist::PlaylistEntryKind::Presentation {
                                presentation_id,
                                ..
                            } => Some(presenter_core::PresentationSummary::new(
                                *presentation_id,
                                String::new(),
                            )),
                            _ => None,
                        })
                        .collect();
                    presentations.set(summaries);
                }
                playlists.set(pls);
            }
        });
    }
}

fn setup_stage_monitor(ctx: AppContext) {
    let connections = ctx.stage_connections;

    // Initial fetch
    leptos::task::spawn_local(async move {
        if let Ok(conns) = crate::api::stage::get_connections().await {
            connections.set(conns);
        }
    });

    // Periodic refresh every 60s
    let connections = ctx.stage_connections;
    let interval = Closure::<dyn Fn()>::new(move || {
        let connections = connections;
        leptos::task::spawn_local(async move {
            if let Ok(conns) = crate::api::stage::get_connections().await {
                connections.set(conns);
            }
        });
    });

    let window = crate::utils::window::window();
    let _ = window.set_interval_with_callback_and_timeout_and_arguments_0(
        interval.as_ref().unchecked_ref(),
        60_000,
    );
    interval.forget();
}

fn setup_keyboard_shortcuts(ctx: AppContext, op: OperatorState) {
    let handler =
        Closure::<dyn Fn(web_sys::KeyboardEvent)>::new(move |ev: web_sys::KeyboardEvent| {
            let key = ev.key();

            // Escape: close modals first, then search
            if key == crate::utils::keyboard::KEY_ESCAPE {
                if op.open_modal.get_untracked().is_some() {
                    crate::components::modal::close_modal(&op);
                    ev.prevent_default();
                    return;
                }
                if op.search_open.get_untracked() {
                    op.search_open.set(false);
                    op.search_query.set(String::new());
                    ctx.search_results.set(Vec::new());
                    ev.prevent_default();
                    return;
                }
            }

            // Don't process shortcuts when in input fields (except Escape above)
            if let Some(active) = crate::utils::window::document().active_element() {
                let tag = active.tag_name();
                if tag == "INPUT" || tag == "TEXTAREA" || tag == "SELECT" {
                    return;
                }
            }

            let is_live = ctx.mode.get_untracked() == "live";

            // Space in live mode: focus search
            if key == " " && is_live {
                ev.prevent_default();
                if let Ok(Some(input)) = crate::utils::window::document()
                    .query_selector("[data-role='global-search-query']")
                {
                    if let Ok(html_el) = input.dyn_into::<web_sys::HtmlElement>() {
                        let _ = html_el.focus();
                    }
                }
                return;
            }

            // Arrow keys for slide navigation in live mode
            if is_live
                && (key == crate::utils::keyboard::KEY_ARROW_LEFT
                    || key == crate::utils::keyboard::KEY_ARROW_RIGHT)
            {
                ev.prevent_default();
                navigate_slides(&ctx, key == crate::utils::keyboard::KEY_ARROW_RIGHT);
            }
        });

    let window = crate::utils::window::window();
    let _ = window.add_event_listener_with_callback("keydown", handler.as_ref().unchecked_ref());
    handler.forget();
}

fn navigate_slides(ctx: &AppContext, forward: bool) {
    let snapshot = ctx.stage_snapshot.get_untracked();
    let pres = ctx.selected_presentation.get_untracked();
    let playlist_id = ctx.selected_playlist_id.get_untracked();

    let Some(presentation) = pres else { return };
    let slides = &presentation.slides;
    if slides.is_empty() {
        return;
    }

    let current_slide_id = snapshot
        .as_ref()
        .and_then(|s| s.current_slide_id.as_ref().map(|id| id.to_string()));
    let pres_id = presentation.id.to_string();

    let current_idx = current_slide_id
        .as_ref()
        .and_then(|cid| slides.iter().position(|s| s.id.to_string() == *cid));

    let next_idx = if forward {
        match current_idx {
            Some(i) if i + 1 < slides.len() => Some(i + 1),
            Some(_) => None, // at end
            None => Some(0), // no current, go to first
        }
    } else {
        match current_idx {
            Some(i) if i > 0 => Some(i - 1),
            Some(_) => None, // at start
            None => Some(0),
        }
    };

    if let Some(idx) = next_idx {
        let slide_id = slides[idx].id.to_string();
        let next_slide_id = slides.get(idx + 1).map(|s| s.id.to_string());

        leptos::task::spawn_local(async move {
            let _ = crate::api::stage::update_state(&crate::api::stage::StageStateRequest {
                presentation_id: pres_id,
                current_slide_id: slide_id,
                next_slide_id,
                playlist_id,
            })
            .await;
        });
    }
}
