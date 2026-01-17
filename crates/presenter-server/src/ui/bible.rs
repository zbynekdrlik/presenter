use crate::state::AppState;
use axum::response::Html;
use leptos::prelude::*;
use presenter_core::{BibleBroadcast, BibleTranslation};
use reactive_graph::owner::Owner;
use serde_json::to_string;

use super::scripts;
use super::styles;

#[component]
fn BibleDocument(
    translations: Vec<BibleTranslation>,
    active: Option<BibleBroadcast>,
    translations_json: String,
    active_json: String,
) -> impl IntoView {
    let translations_json_safe = translations_json.replace("</script>", r"<\/script>");
    let active_json_safe = active_json.replace("</script>", r"<\/script>");
    let script = scripts::BIBLE
        .replace("__TRANSLATIONS__", &translations_json_safe)
        .replace("__ACTIVE__", &active_json_safe);

    view! {
        <html lang="en">
            <head>
                <meta charset="utf-8" />
                <title>"Presenter Bible Control"</title>
                <style>{styles::BIBLE}</style>
            </head>
            <body class="bible">
                <header class="bible__header">
                    <div>
                        <h1>"Bible Control"</h1>
                        <p>"Search, trigger, and clear Bible passages for broadcast."</p>
                    </div>
                    <button type="button" class="bible__clear" data-role="clear-button">"Clear Broadcast"</button>
                </header>
                <form class="bible__search" data-role="search-form">
                    <label>
                        <span>"Translation"</span>
                        <select data-role="translation-select">
                            <For
                                each={move || translations.clone()}
                                key=|translation: &BibleTranslation| translation.code.clone()
                                children=move |translation: BibleTranslation| {
                                    let label = if translation.language.is_empty() {
                                        translation.name.clone()
                                    } else {
                                        format!("{} ({})", translation.name, translation.language)
                                    };
                                    view! { <option value={translation.code.clone()}>{label}</option> }
                                }
                            />
                        </select>
                    </label>
                    <label class="bible__search-input">
                        <span>"Query"</span>
                        <input type="search" placeholder="e.g. John 3:16 or love" data-role="query-input" />
                    </label>
                    <button type="submit" class="bible__search-button">"Search"</button>
                </form>
                <section class="bible__active" data-role="active-passage">
                    {move || {
                        match &active {
                            Some(broadcast) => {
                                let reference = broadcast.passage.reference.to_human_readable();
                                let translation = broadcast.passage.translation.name.clone();
                                let text = broadcast.passage.text.clone();
                                view! {
                                    <div class="bible__active-card">
                                        <header>
                                            <strong data-role="active-reference">{reference}</strong>
                                            <span class="bible__active-translation">{translation}</span>
                                        </header>
                                        <p data-role="active-text">{text}</p>
                                    </div>
                                }
                                .into_any()
                            }
                            None => view! {
                                <div class="bible__active-card bible__active-card--empty">
                                    <header>
                                        <strong data-role="active-reference">"No active passage"</strong>
                                        <span class="bible__active-translation">""</span>
                                    </header>
                                    <p class="bible__empty" data-role="active-text">"Select a verse to broadcast."</p>
                                </div>
                            }
                            .into_any(),
                        }
                    }}
                </section>
                <section class="bible__results" data-role="results">
                    <p class="bible__empty">"Search for a verse or phrase above."</p>
                </section>
                <div class="bible__toast" data-role="toast" data-visible="false"></div>
                <script>{script}</script>
            </body>
        </html>
    }
}

pub async fn render_bible_ui(state: &AppState) -> anyhow::Result<Html<String>> {
    let translations = state.list_bible_translations().await?;
    let active = state.active_bible_broadcast().await;

    let translations_json = to_string(&translations).unwrap_or_else(|_| "[]".to_string());
    let active_json = to_string(&active).unwrap_or_else(|_| "null".to_string());

    let owner = Owner::new_root(None);
    let html = owner.with(|| {
        view! {
            <BibleDocument
                translations=translations.clone()
                active=active.clone()
                translations_json=translations_json.clone()
                active_json=active_json.clone()
            />
        }
        .into_view()
        .to_html()
    });

    Ok(Html(format!("<!DOCTYPE html>{html}")))
}
