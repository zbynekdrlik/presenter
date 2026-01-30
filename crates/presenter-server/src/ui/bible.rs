use super::utils::escape_script_tag;
use super::{scripts, styles};
use crate::state::AppState;
use axum::response::Html;
use leptos::prelude::*;
use presenter_core::{BibleBroadcast, BibleTranslation, DEFAULT_STAGE_LAYOUT_CODE};
use reactive_graph::owner::Owner;
use serde_json::to_string;

#[component]
fn BibleDocument(
    translations: Vec<BibleTranslation>,
    active: Option<BibleBroadcast>,
    translations_json: String,
    active_json: String,
    embed: bool,
) -> impl IntoView {
    let translations_json_safe = escape_script_tag(&translations_json);
    let active_json_safe = escape_script_tag(&active_json);
    let script = scripts::BIBLE
        .replace("__TRANSLATIONS__", &translations_json_safe)
        .replace("__ACTIVE__", &active_json_safe);
    let combined_styles = format!("{}{}", styles::OPERATOR, styles::BIBLE);
    let translations_for_view = translations.clone();
    let translation_count = translations_for_view.len();
    let broadcast_for_view = active.clone();
    let body_class = if embed {
        "operator operator--bible operator--embedded"
    } else {
        "operator operator--bible"
    };

    view! {
            <html lang="en">
                <head>
                    <meta charset="utf-8" />
                    <title>"Presenter Bible Control"</title>
                    <style>{combined_styles}</style>
                </head>
                <body class={body_class} data-view="bible" data-mode="live">
                    {if !embed {
                        view! {
    <header class="operator__header">
                        <div class="operator__header-left">
                            <h1>"Presenter Bible"</h1>
                            <nav class="operator__view-nav">
                                <button type="button" data-role="view-toggle" data-view="worship" data-href="/ui/operator">"Worship"</button>
                                <button type="button" data-role="view-toggle" data-view="bible" data-active="true">"Bible"</button>
                                <button type="button" data-role="view-toggle" data-view="timers" data-href="/ui/operator?view=timers">"Timers"</button>
                                <button type="button" data-role="view-toggle" data-view="settings" data-href="/ui/settings">"Settings"</button>
                            </nav>
                        </div>
                        <div class="operator__header-center">
                            <form class="operator__search" data-role="global-search-form" role="search" autocomplete="off">
                                <span class="operator__search-icon" aria-hidden="true"></span>
                                <input
                                    type="search"
                                    placeholder="Search presenter content"
                                    data-role="global-search-query"
                                    aria-label="Search presenter content"
                                    autocomplete="off"
                                    disabled
                                />
                                <button type="button" data-role="global-search-clear" aria-label="Clear search" disabled>
                                    <span aria-hidden="true">"×"</span>
                                    <span class="sr-only">"Clear search"</span>
                                </button>
                            </form>
                            <div class="operator__search-results" data-role="global-search-results"></div>
                            <div class="operator__stage-layout" aria-label="Stage display mode">
                                <label class="operator__stage-layout-label" for="bible-stage-layout">"Stage Output"</label>
                                <select id="bible-stage-layout" data-role="stage-layout-select" disabled>
                                    <option value={DEFAULT_STAGE_LAYOUT_CODE}>"Worship"</option>
                                </select>
                            </div>
                        </div>
                        <div class="operator__header-right">
                            <div class="operator__stage-preview" data-role="stage-status" data-active="false">
                                <div class="operator__stage-preview-stack">
                                    <div class="operator__stage-preview-panel operator__stage-preview-panel--next" data-role="stage-next">"—"</div>
                                    <div class="operator__stage-preview-song" data-role="stage-song-line">"—"</div>
                                    <div class="operator__stage-preview-actions">
                                        <button type="button" class="operator__stage-toggle" disabled>"Ableton OFF"</button>
                                        <button type="button" class="operator__stage-toggle" disabled>"Follow OFF"</button>
                                    </div>
                                </div>
                                <div class="operator__stage-preview-panel operator__stage-preview-panel--current" data-role="stage-current">"—"</div>
                                <button
                                    type="button"
                                    class="operator__stage-monitor"
                                    data-role="stage-monitor"
                                    data-connected="0"
                                    data-issues="0"
                                    aria-label="Stage display health"
                                    title="Stage displays – no data"
                                    disabled
                                >
                                    <span data-role="stage-monitor-connected" class="operator__stage-monitor-count operator__stage-monitor-count--connected">"0"</span>
                                    <span class="operator__stage-monitor-separator">"/"</span>
                                    <span data-role="stage-monitor-issues" class="operator__stage-monitor-count operator__stage-monitor-count--issues">"0"</span>
                                </button>
                                <button type="button" class="operator__clear-button" data-role="clear-button">
                                    <span aria-hidden="true">"🧹"</span>
                                    <span class="sr-only">"Clear broadcast"</span>
                                </button>
                            </div>
                            <div class="operator__mode-toggle">
                                <button type="button" data-role="mode-toggle" data-mode="live" data-active="true" disabled>"Live"</button>
                                <button type="button" data-role="mode-toggle" data-mode="edit" disabled>"Edit"</button>
                            </div>
                        </div>
                    </header>
                        }.into_any()
                    } else {
                        ().into_view().into_any()
                    }}
                    <main class="operator__main">
                        <aside class="operator__catalog operator__catalog--bible" data-role="catalog">
                            <div class="operator__catalog-top">
                                <section class="operator__group operator__group--translations">
                                    <header class="operator__group-header">
                                        <h2>"Bibles"</h2>
                                        <div class="operator__group-controls">
                                            <button
                                                type="button"
                                                class="operator__group-count"
                                                data-role="bible-dashboard"
                                                aria-label={format!("Show all Bibles ({translation_count} available)")}
                                                data-empty={if translation_count == 0 { "true" } else { "false" }}
                                            >
                                                {format!("({translation_count})")}
                                            </button>
                                            <button
                                                type="button"
                                                data-role="bible-import"
                                                aria-label="Import Bible translation"
                                                title="Import Bible translation"
                                            >
                                                "+"
                                            </button>
                                        </div>
                                    </header>
                                    <ul class="operator__list operator__list--tight" data-role="translation-list">
                                        {translations_for_view.iter().enumerate().map(|(i, translation)| {
                                            let label = if translation.language.is_empty() {
                                                translation.name.clone()
                                            } else {
                                                format!("{} ({})", translation.name, translation.language)
                                            };
                                            let edit_label = label.clone();
                                            view! {
                                                <li
                                                    class="operator__list-item"
                                                    data-role="translation-item"
                                                    data-translation-code={translation.code.clone()}
                                                    data-index={i.to_string()}
                                                >
                                                    <button
                                                        type="button"
                                                        class="operator__list-button"
                                                        data-translation-code={translation.code.clone()}
                                                    >
                                                        <span class="operator__list-label">{label}</span>
                                                    </button>
                                                    <div class="operator__list-actions">
                                                        <button
                                                            type="button"
                                                            class="operator__list-action operator__list-action--icon operator__list-action--menu"
                                                            data-action="bible-edit"
                                                            data-translation-code={translation.code.clone()}
                                                            aria-label={format!("Edit {edit_label}")}
                                                        >
                                                            "⋮"
                                                        </button>
                                                    </div>
                                                </li>
                                            }
                                        }).collect::<Vec<_>>() }
                                    </ul>
                                </section>
                                <section class="operator__group operator__group--reference" data-role="reference-panel">
                                    <header class="operator__group-header">
                                        <div>
                                            <h2>"Reference"</h2>
                                            <p>"Select the passage to load."</p>
                                        </div>
                                    </header>
                                    <div class="operator__form-group">
                                        <label class="operator__field">
                                            <span>"Secondary translation"</span>
                                            <select data-role="secondary-translation">
                                                <option value="">"None"</option>
                                                {translations_for_view.iter().map(|translation| {
                                                    let label = if translation.language.is_empty() {
                                                        translation.name.clone()
                                                    } else {
                                                        format!("{} ({})", translation.name, translation.language)
                                                    };
                                                    view! { <option value={translation.code.clone()}>{label}</option> }
                                                }).collect::<Vec<_>>() }
                                            </select>
                                        </label>
                                        <label class="operator__field">
                                            <span>"Character limit"</span>
                                            <input type="number" data-role="char-limit" value="320" min="1" max="4000" />
                                        </label>
                                        <button type="button" class="operator__list-action operator__list-action--primary" data-role="save-preferences">"Save preferences"</button>
                                    </div>
                                    <div class="operator__divider"></div>
                                    <label class="operator__field">
                                        <span>"Find book"</span>
                                        <input type="search" data-role="book-filter" placeholder="Start typing…" />
                                    </label>
                                    <div class="operator__list operator__list--tight" data-role="book-list"></div>
                                    <div class="operator__reference-grid">
                                        <label class="operator__field">
                                            <span>"Chapter"</span>
                                            <input type="number" data-role="chapter-input" min="1" value="1" />
                                        </label>
                                        <label class="operator__field">
                                            <span>"Verse start"</span>
                                            <input type="number" data-role="verse-start" min="1" value="1" />
                                        </label>
                                        <label class="operator__field">
                                            <span>"Verse end"</span>
                                            <input type="number" data-role="verse-end" min="1" placeholder="All" />
                                        </label>
                                    </div>
                                    <button type="button" class="operator__list-action operator__list-action--primary" data-role="load-button">"Load passage"</button>
                                </section>
                            </div>
                            <div class="operator__catalog-bottom">
                                <section class="operator__group operator__group--passages">
                                    <header class="operator__group-header">
                                        <div>
                                            <h2>"Loaded verses"</h2>
                                            <p>"Quickly reapply recent selections."</p>
                                        </div>
                                    </header>
                                    <ul class="operator__list operator__list--compact" data-role="loaded-passages">
                                        <li class="operator__list-item operator__list-item--empty">"Load a passage to populate this list."</li>
                                    </ul>
                                </section>
                            </div>
                        </aside>
                        <section class="operator__slides-column" data-role="slides-column">
                            <header class="operator__slides-heading">
                                <div>
                                    <h2>"Slides"</h2>
                                    <span data-role="selection-count" class="operator__slides-count">"0 selected"</span>
                                </div>
                                <div class="operator__slides-actions">
                                    <button type="button" class="operator__list-action" data-role="select-all-slides">"Select all"</button>
                                    <button type="button" class="operator__list-action operator__list-action--secondary" data-role="toggle-mode">"Switch to Edit Mode"</button>
                                </div>
                            </header>
                            <section class="operator__panel operator__panel--active" data-role="active-passage">
                                {match broadcast_for_view {
                                    Some(broadcast) => {
                                        let reference = broadcast.passage.reference.to_human_readable();
                                        let translation = broadcast.passage.translation.name.clone();
                                        let text = broadcast.passage.text.clone();
                                        view! {
                                            <article class="operator__active-card">
                                                <header>
                                                    <strong>{reference}</strong>
                                                    <span>{translation}</span>
                                                </header>
                                                <p>{text}</p>
                                            </article>
                                        }.into_any()
                                    }
                                    None => view! {
                                        <article class="operator__active-card operator__active-card--empty">
                                            <header>
                                                <strong>"No active passage"</strong>
                                                <span>""</span>
                                            </header>
                                            <p>"Trigger a slide to broadcast scripture."</p>
                                        </article>
                                    }.into_any(),
                                }}
                            </section>
                            <div class="operator__slides" data-role="slides">
                                <p class="operator__slides-empty">"Load a passage to populate slides."</p>
                            </div>
                            <footer class="operator__slides-footer">
                                <div class="operator__presentation-controls">
                                    <label class="operator__field">
                                        <span>"Existing presentation"</span>
                                        <select data-role="presentation-select">
                                            <option value="">"Select existing…"</option>
                                        </select>
                                    </label>
                                    <label class="operator__field">
                                        <span>"Create new presentation"</span>
                                        <input type="text" data-role="presentation-name" placeholder="New presentation name" />
                                    </label>
                                    <button type="button" class="operator__list-action operator__list-action--primary" data-role="presentation-add">"Add selected slides"</button>
                                    <button type="button" class="operator__list-action" data-role="refresh-presentations">"Refresh list"</button>
                                </div>
                            </footer>
                        </section>
                        <aside class="operator__panel operator__panel--secondary">
                            <header class="operator__panel-header">
                                <h3>"Bible presentations"</h3>
                            </header>
                            <div class="operator__presentation-list" data-role="presentations-list">
                                <p class="operator__slides-empty">"No Bible presentations yet."</p>
                            </div>
                        </aside>
                    </main>
                    <div class="operator__toast" data-role="toast" data-visible="false"></div>
                    <div class="operator__library-modal operator__library-modal--bible" data-role="bible-modal">
                        <div class="operator__library-modal-panel">
                            <header class="operator__library-modal-header">
                                <h3>"All Bibles"</h3>
                                <button
                                    type="button"
                                    class="operator__library-modal-close"
                                    data-role="bible-modal-close"
                                    aria-label="Close"
                                >
                                    "×"
                                </button>
                            </header>
                            <div class="operator__library-modal-body" data-role="bible-modal-list"></div>
                        </div>
                    </div>
                    <div class="operator__library-edit operator__library-edit--bible" data-role="bible-edit-modal" data-mode="edit">
                        <div class="operator__library-edit-panel">
                            <form class="operator__library-edit-form" data-role="bible-edit-form">
                                <header class="operator__library-edit-header">
                                    <h3 data-role="bible-edit-title">"Edit Bible"</h3>
                                </header>
                                <div class="operator__library-edit-body">
                                    <label>
                                        <span>"Bible name"</span>
                                        <input
                                            type="text"
                                            data-role="bible-edit-name"
                                            autocomplete="off"
                                            required
                                            minlength="1"
                                            maxlength="160"
                                        />
                                    </label>
                                    <label>
                                        <span>"Language"</span>
                                        <input
                                            type="text"
                                            data-role="bible-edit-language"
                                            autocomplete="off"
                                            required
                                            minlength="1"
                                            maxlength="60"
                                        />
                                    </label>
                                    <label class="operator__library-edit-favorite">
                                        <input type="checkbox" data-role="bible-edit-dashboard" />
                                        <span>"Show in dashboard"</span>
                                    </label>
                                </div>
                                <footer class="operator__library-edit-footer">
                                    <button
                                        type="button"
                                        class="operator__library-edit-delete"
                                        data-role="bible-edit-delete"
                                    >
                                        "Delete Bible"
                                    </button>
                                    <div class="operator__library-edit-actions">
                                        <button type="button" data-role="bible-edit-cancel">"Cancel"</button>
                                        <button type="submit" data-role="bible-edit-save">"Save changes"</button>
                                    </div>
                                </footer>
                            </form>
                        </div>
                    </div>
                    <script>{script}</script>
                </body>
            </html>
        }
}

pub async fn render_bible_ui(state: &AppState, embed: bool) -> anyhow::Result<Html<String>> {
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
                embed=embed
            />
        }
        .into_view()
        .to_html()
    });

    Ok(Html(format!("<!DOCTYPE html>{html}")))
}
