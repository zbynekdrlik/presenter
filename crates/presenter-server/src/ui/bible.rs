use super::utils::escape_script_tag;
use super::{scripts, styles};
use crate::state::AppState;
use axum::response::Html;
use leptos::prelude::*;
use presenter_core::{BibleTranslation, DEFAULT_STAGE_LAYOUT_CODE};
use reactive_graph::owner::Owner;
use serde_json::to_string;

#[component]
fn BibleDocument(
    translations: Vec<BibleTranslation>,
    translations_json: String,
    active_json: String,
) -> impl IntoView {
    let translations_json_safe = escape_script_tag(&translations_json);
    let active_json_safe = escape_script_tag(&active_json);
    let script = scripts::BIBLE
        .replace("__TRANSLATIONS__", &translations_json_safe)
        .replace("__ACTIVE__", &active_json_safe);
    let combined_styles = format!("{}{}", styles::OPERATOR, styles::BIBLE);
    let translations_for_view = translations.clone();

    view! {
        <html lang="en">
            <head>
                <meta charset="utf-8" />
                <title>"Presenter Bible Control"</title>
                <style>{combined_styles}</style>
            </head>
            <body class="operator operator--bible" data-view="bible" data-mode="live">
                <script>"if(window!==window.parent)document.body.classList.add('in-iframe');"</script>
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
                                placeholder="Search Bible verses\u{2026}"
                                data-role="global-search-query"
                                aria-label="Search Bible verses"
                                autocomplete="off"
                            />
                            <button type="button" data-role="global-search-clear" aria-label="Clear search" hidden>
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
                            <button type="button" data-role="mode-toggle" data-mode="live" data-active="true">"Live"</button>
                            <button type="button" data-role="mode-toggle" data-mode="edit">"Edit"</button>
                        </div>
                    </div>
                </header>
                <main class="operator__main">
                    <aside class="operator__catalog operator__catalog--bible" data-role="catalog">
                        <div class="operator__catalog-top">
                            <nav class="bible__tab-nav" data-role="bible-tab-nav">
                                <button type="button" data-role="bible-tab" data-tab="live" data-active="true">"Live"</button>
                                <button type="button" data-role="bible-tab" data-tab="prepared">"Prepared"</button>
                                <button type="button" data-role="bible-tab" data-tab="settings">"Settings"</button>
                            </nav>
                            <div class="bible__tab-panel" data-bible-panel="live" data-visible="true">
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
                                <hr class="operator__divider" />
                                <span data-role="selection-count" class="operator__slides-count">"0 selected"</span>
                                <button type="button" class="operator__list-action" data-role="select-all-slides">"Select all"</button>
                                <label class="operator__field">
                                    <select data-role="presentation-select">
                                        <option value="">"Add to…"</option>
                                    </select>
                                </label>
                                <button type="button" class="operator__list-action operator__list-action--primary" data-role="presentation-add">"Add selected"</button>
                            </div>
                            <div class="bible__tab-panel" data-bible-panel="prepared">
                                <div class="bible__prepared-header">
                                    <h3>"Presentations"</h3>
                                    <button type="button" class="operator__list-action" data-role="presentation-create" aria-label="Create presentation">"+"</button>
                                </div>
                                <div class="bible__prepared-list" data-role="presentations-list">
                                    <p class="operator__slides-empty">"No Bible presentations yet."</p>
                                </div>
                            </div>
                            <div class="bible__tab-panel" data-bible-panel="settings">
                                <div class="operator__form-group">
                                    <label class="operator__field">
                                        <span>"Main translation"</span>
                                        <select data-role="main-translation">
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
                            </div>
                        </div>
                    </aside>
                    <section class="operator__slides-column" data-role="slides-column">
                        <header class="operator__slides-heading">
                            <div>
                                <h2>"Slides"</h2>
                            </div>
                        </header>
                        <div class="operator__slides-toolbar">
                            <button type="button" class="operator__slides-add" data-role="add-empty-slide" title="Add empty slide to active presentation">"+"</button>
                        </div>
                        <div class="operator__slides" data-role="slides">
                            <p class="operator__slides-empty">"Load a passage to populate slides."</p>
                        </div>
                    </section>
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
                <div class="operator__library-edit operator__library-edit--bible-presentation" data-role="bible-presentation-edit-modal">
                    <div class="operator__library-edit-panel">
                        <form class="operator__library-edit-form" data-role="bible-presentation-edit-form">
                            <header class="operator__library-edit-header">
                                <h3>"Edit Presentation"</h3>
                            </header>
                            <div class="operator__library-edit-body">
                                <label>
                                    <span>"Presentation name"</span>
                                    <input
                                        type="text"
                                        data-role="bible-presentation-edit-name"
                                        autocomplete="off"
                                        required
                                        minlength="1"
                                        maxlength="160"
                                    />
                                </label>
                            </div>
                            <footer class="operator__library-edit-footer">
                                <button
                                    type="button"
                                    class="operator__library-edit-delete"
                                    data-role="bible-presentation-edit-delete"
                                >
                                    "Delete presentation"
                                </button>
                                <div class="operator__library-edit-actions">
                                    <button type="button" data-role="bible-presentation-edit-cancel">"Cancel"</button>
                                    <button type="submit" data-role="bible-presentation-edit-save">"Save changes"</button>
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
                translations_json=translations_json.clone()
                active_json=active_json.clone()
            />
        }
        .into_view()
        .to_html()
    });

    Ok(Html(format!("<!DOCTYPE html>{html}")))
}
