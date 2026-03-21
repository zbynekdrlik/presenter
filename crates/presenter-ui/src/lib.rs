#[macro_use]
mod context_macros;
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
    let page_view = move || {
        let p = pathname.as_str();
        if p == "/ui/operator" || p.starts_with("/ui/operator/") {
            // Extract view from path: /ui/operator/bible → "bible"
            let initial_view = p
                .strip_prefix("/ui/operator/")
                .filter(|v| !v.is_empty())
                .unwrap_or("")
                .to_string();
            view! { <pages::operator::OperatorPage initial_view=initial_view /> }.into_any()
        } else if p == "/ui/tablet" {
            view! { <pages::tablet::TabletPage /> }.into_any()
        } else {
            view! {
                <div data-role="not-found">
                    <h1>"Page not found"</h1>
                    <p>"The requested page does not exist."</p>
                </div>
            }
            .into_any()
        }
    };

    view! {
        {page_view}
    }
}
