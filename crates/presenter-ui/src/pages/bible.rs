use leptos::prelude::*;

use crate::state::bible::BibleState;
use crate::ws;

/// Bible page — search and broadcast Bible passages.
#[component]
pub fn BiblePage() -> impl IntoView {
    let bible_state = BibleState::new();
    let (_ws_state, _last_event) = ws::use_live_websocket();

    // Load translations on mount
    let translations = bible_state.translations;
    leptos::task::spawn_local(async move {
        if let Ok(trans) = crate::api::bible::list_translations().await {
            translations.set(trans);
        }
    });

    view! {
        <div data-role="bible-page" class="bible-layout">
            <header data-role="bible-header">
                <h1>"Bible"</h1>
            </header>
            <main data-role="bible-main">
                <section data-role="bible-search">
                    <input
                        data-role="bible-search-input"
                        type="text"
                        placeholder="Search Bible..."
                    />
                </section>
                <section data-role="bible-results">
                    <p>"Enter a search query to find Bible passages."</p>
                </section>
            </main>
        </div>
    }
}
