use axum::response::Html;
use leptos::prelude::*;
use reactive_graph::owner::Owner;

use super::styles;

#[component]
fn HomeDocument() -> impl IntoView {
    view! {
        <html lang="en">
            <head>
                <meta charset="utf-8" />
                <title>"Presenter surfaces"</title>
                <style>{styles::HOME}</style>
            </head>
            <body class="home">
                <main class="home__container">
                    <header class="home__header">
                        <h1>"Presenter Demo Environment"</h1>
                        <p>"Quick links to control surfaces and stage displays for live verification."</p>
                    </header>
                    <section class="home__section">
                        <h2>"Control Surfaces"</h2>
                        <ul class="home__links">
                            <li><a href="/ui/operator">"Operator UI"</a></li>
                            <li><a href="/ui/tablet">"Tablet UI"</a></li>
                            <li><a href="/ui/bible">"Bible Control"</a></li>
                            <li><a href="/ui/settings" target="_blank" rel="noopener">"Settings"</a></li>
                        </ul>
                    </section>
                    <section class="home__section">
                        <h2>"Stage Displays"</h2>
                        <ul class="home__links">
                            <li><a href="/stage">"Stage Output"</a></li>
                            <li><a href="/ui/camera">"Camera Crew"</a></li>
                            <li><a href="/overlays/timer">"Timer Overlay"</a></li>
                        </ul>
                    </section>
                </main>
            </body>
        </html>
    }
}

pub async fn render_home_ui() -> anyhow::Result<Html<String>> {
    let owner = Owner::new_root(None);
    let html = owner.with(|| view! { <HomeDocument /> }.into_view().to_html());
    Ok(Html(format!("<!DOCTYPE html>{html}")))
}
