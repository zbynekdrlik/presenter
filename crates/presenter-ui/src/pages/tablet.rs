use leptos::prelude::*;

use crate::ws;

/// Tablet page — touch-optimized Bible viewer with slide triggering.
#[component]
pub fn TabletPage() -> impl IntoView {
    let (_ws_state, _last_event) = ws::use_live_websocket();

    view! {
        <div data-role="tablet-page" class="tablet-layout">
            <header data-role="tablet-header">
                <h1>"Tablet"</h1>
            </header>
            <main data-role="tablet-main">
                <p>"Tablet interface — coming soon."</p>
            </main>
        </div>
    }
}
