use leptos::prelude::*;

use crate::state::AppContext;
use crate::ws;

/// Operator page — primary control surface for worship service management.
#[component]
pub fn OperatorPage() -> impl IntoView {
    let ctx = AppContext::new();
    let (ws_state, _last_event) = ws::use_live_websocket();

    // Load initial data on mount
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

    let ws_indicator = move || {
        let state = ws_state.get();
        match state {
            ws::WsState::Connected => "connected",
            ws::WsState::Connecting => "connecting",
            ws::WsState::Reconnecting => "reconnecting",
            ws::WsState::Disconnected => "disconnected",
        }
    };

    view! {
        <div data-role="operator-page" class="operator-layout">
            <header data-role="operator-header">
                <h1>"Presenter"</h1>
                <span data-role="ws-status">{ws_indicator}</span>
            </header>
            <main data-role="operator-main">
                <aside data-role="library-panel">
                    <h2>"Libraries"</h2>
                    <ul data-role="library-list">
                        {move || {
                            libraries.get().into_iter().map(|lib| {
                                let id = lib.id.to_string();
                                let name = lib.name.clone();
                                view! {
                                    <li data-role="library-item" data-library-id={id}>
                                        {name}
                                    </li>
                                }
                            }).collect::<Vec<_>>()
                        }}
                    </ul>
                </aside>
                <section data-role="presentation-panel">
                    <p>"Select a library to view presentations."</p>
                </section>
                <aside data-role="playlist-panel">
                    <h2>"Playlists"</h2>
                    <ul data-role="playlist-list">
                        {move || {
                            playlists.get().into_iter().map(|pl| {
                                let id = pl.id.to_string();
                                let name = pl.name.clone();
                                view! {
                                    <li data-role="playlist-item" data-playlist-id={id}>
                                        {name}
                                    </li>
                                }
                            }).collect::<Vec<_>>()
                        }}
                    </ul>
                </aside>
            </main>
        </div>
    }
}
