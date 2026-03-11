pub mod api;
pub mod components;
pub mod pages;
pub mod state;
pub mod utils;
pub mod ws;

use leptos::prelude::*;
use wasm_bindgen::prelude::*;

/// Main entry point for the WASM application.
/// Called from JavaScript after the WASM module loads.
#[wasm_bindgen(start)]
pub fn main() {
    console_error_panic_hook::set_once();

    mount_to_body(App);
}

/// Root application component with client-side routing.
#[component]
fn App() -> impl IntoView {
    let pathname = utils::window::current_pathname();

    // Mark body as WASM-ready for E2E test detection
    if let Some(body) = utils::window::document_body() {
        let _ = body.set_attribute("data-wasm-ready", "true");
    }

    // Client-side routing based on pathname
    let page_view = move || match pathname.as_str() {
        "/ui-next/operator" => view! { <pages::operator::OperatorPage /> }.into_any(),
        "/ui-next/bible" => view! { <pages::bible::BiblePage /> }.into_any(),
        "/ui-next/tablet" => view! { <pages::tablet::TabletPage /> }.into_any(),
        "/ui-next/settings" => view! { <pages::settings::SettingsPage /> }.into_any(),
        _ => view! {
            <div data-role="not-found">
                <h1>"Page not found"</h1>
                <p>"The requested page does not exist."</p>
            </div>
        }
        .into_any(),
    };

    view! {
        {page_view}
    }
}
