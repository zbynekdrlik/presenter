use crate::state::AppState;
use axum::response::Html;
use leptos::prelude::*;
use reactive_graph::owner::Owner;

use super::scripts::TABLET as TABLET_SCRIPT_TEMPLATE;
use super::styles::TABLET as TABLET_STYLES;
use super::utils::escape_script_tag;

#[component]
fn TabletDocument(presentations_json: String) -> impl IntoView {
    let presentations_json_safe = escape_script_tag(&presentations_json);
    let script = TABLET_SCRIPT_TEMPLATE.replace("__PRESENTATIONS__", &presentations_json_safe);

    view! {
        <html lang="en">
            <head>
                <meta charset="utf-8" />
                <meta name="viewport" content="width=device-width, initial-scale=1.0" />
                <title>"Bible Tablet"</title>
                <style>{TABLET_STYLES}</style>
            </head>
            <body class="tablet">
                <header class="tablet-header">
                    <h1>"Bible Tablet"</h1>
                    <div class="tablet-scale">
                        <label for="scale-slider">"Text size"</label>
                        <input type="range" id="scale-slider" data-role="scale-slider"
                            min="50" max="200" value="100" step="10" />
                        <span data-role="scale-value">"100%"</span>
                    </div>
                </header>
                <main class="tablet-layout">
                    <aside class="tablet-sidebar">
                        <button type="button" class="tablet-sidebar__close" data-role="sidebar-close">
                            "×"
                        </button>
                        <section class="tablet-panel">
                            <h2>"Presentations"</h2>
                            <div class="tablet-list" data-role="presentation-list">
                                "Loading..."
                            </div>
                        </section>
                    </aside>
                    <section class="tablet-main">
                        <header class="tablet-main__header">
                            <button type="button" class="tablet-back-button" data-role="sidebar-toggle">
                                "← Presentations"
                            </button>
                            <h2 data-role="context-title">"Select a presentation"</h2>
                        </header>
                        <div class="tablet-slides" data-role="slides">
                            <p class="tablet-slides__empty">"Select a presentation to view slides."</p>
                        </div>
                    </section>
                </main>
                <div class="tablet-toast" data-role="toast" data-visible="false"></div>
                <script>{script}</script>
            </body>
        </html>
    }
}

pub async fn render_tablet_ui(state: &AppState) -> anyhow::Result<Html<String>> {
    let summaries = state.list_bible_presentations().await?;

    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct PresentationDto {
        id: String,
        name: String,
        slide_count: usize,
    }

    let mut dtos = Vec::with_capacity(summaries.len());
    for summary in summaries {
        let slide_count = state
            .bible_presentation_detail(summary.id)
            .await?
            .map(|p| p.slides.len())
            .unwrap_or(0);
        dtos.push(PresentationDto {
            id: summary.id.to_string(),
            name: summary.name,
            slide_count,
        });
    }

    let presentations_json = serde_json::to_string(&dtos).unwrap_or_else(|_| "[]".to_string());

    let owner = Owner::new_root(None);
    let html = owner.with(|| {
        view! { <TabletDocument presentations_json=presentations_json.clone() /> }
            .into_view()
            .to_html()
    });

    Ok(Html(format!("<!DOCTYPE html>{html}")))
}
