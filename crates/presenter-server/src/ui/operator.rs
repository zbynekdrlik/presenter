use crate::{
    ableset::AbleSetStatusSnapshot,
    router::{BUILD_CHANNEL, VERSION},
    state::AppState,
};
use axum::response::Html;
use leptos::prelude::*;
use presenter_core::{playlist::PlaylistEntryKind, TimersOverview};
use reactive_graph::owner::Owner;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::models::{LibraryRow, PlaylistEntryRow, PlaylistRow, PresentationRow};
use super::scripts::OPERATOR as OPERATOR_SCRIPT_TEMPLATE;
use super::styles::OPERATOR as OPERATOR_STYLES;
use super::utils::{escape_script_tag, format_seconds, format_timer_state, json_safe};
use serde_json::to_string;

#[component]
pub fn OperatorDocument(
    libraries: Vec<LibraryRow>,
    playlists: Vec<PlaylistRow>,
    timers: TimersOverview,
    ableset_status: AbleSetStatusSnapshot,
    libraries_json: String,
    playlists_json: String,
    stage_layouts_json: String,
    stage_layout_code: String,
    initial_view: String,
) -> impl IntoView {
    let initial_library_id = libraries.first().map(|library| library.id.clone());
    let initial_playlist_id = playlists.first().map(|playlist| playlist.id.clone());
    let libraries = Arc::new(libraries);
    let playlists = Arc::new(playlists);
    let timers = Arc::new(timers);
    let ableset_enabled = ableset_status.enabled;
    let ableset_follow_enabled = ableset_status.follow_enabled;
    let ableset_enable_label = if ableset_enabled {
        "Ableton ON"
    } else {
        "Ableton OFF"
    }
    .to_string();
    let ableset_follow_label = if ableset_follow_enabled {
        "Follow ON"
    } else {
        "Follow OFF"
    }
    .to_string();
    let libraries_json = escape_script_tag(&libraries_json);
    let playlists_json = escape_script_tag(&playlists_json);
    let timers_json = json_safe(&*timers);
    let stage_layouts_json = escape_script_tag(&stage_layouts_json);

    let stage_layout_code_safe = stage_layout_code.replace('"', "\\\"");

    let ableset_status_json = json_safe(&ableset_status);

    let operator_script = OPERATOR_SCRIPT_TEMPLATE
        .replace("__LIBRARIES__", &libraries_json)
        .replace("__PLAYLISTS__", &playlists_json)
        .replace("__TIMERS__", &timers_json)
        .replace("__STAGE_LAYOUTS__", &stage_layouts_json)
        .replace("__STAGE_LAYOUT_CODE__", &stage_layout_code_safe)
        .replace("__ABLESET_STATUS__", &ableset_status_json);

    view! {
            <html lang="en">
                <head>
                    <meta charset="utf-8" />
                    <title>"Presenter Operator"</title>
                    <style>{OPERATOR_STYLES}</style>
                </head>
                <body class="operator" data-view={initial_view.clone()} data-mode="live">
                    <header class="operator__header">
                        <div class="operator__header-left">
                            <h1>"Presenter"</h1>
                            <span class="operator__version-badge">
                                "v"{VERSION}
                                {if BUILD_CHANNEL != "release" { format!(" ({})", BUILD_CHANNEL) } else { String::new() }}
                            </span>
                            <form class="operator__search" data-role="global-search-form" role="search" autocomplete="off">
                                <span class="operator__search-icon" aria-hidden="true"></span>
                                <input
                                    type="search"
                                    placeholder="Search libraries, songs, slides"
                                    data-role="global-search-query"
                                    aria-label="Search presenter content"
                                    autocomplete="off"
                                />
                                <button
                                    type="button"
                                    data-role="global-search-clear"
                                    aria-label="Clear search"
                                ><span aria-hidden="true">{ "×" }</span><span class="sr-only">"Clear search"</span></button>
                            </form>
                        </div>
                        <nav class="operator__view-nav">
                            <button
                                type="button"
                                data-role="view-toggle"
                                data-view="worship"
                                data-active={if initial_view == "worship" { "true" } else { "false" }}
                            >"Worship"</button>
                            <button type="button" data-role="view-toggle" data-view="bible" data-active={if initial_view == "bible" { "true" } else { "false" }}>"Bible"</button>
                            <button type="button" data-role="view-toggle" data-view="timers" data-active={if initial_view == "timers" { "true" } else { "false" }}>"Timers"</button>
                            <button type="button" data-role="view-toggle" data-view="settings" data-active={if initial_view == "settings" { "true" } else { "false" }}>"Settings"</button>
                        </nav>
                        <div class="operator__header-right">
                            <div class="operator__stage-layout" aria-label="Stage display mode">
                                <label class="operator__stage-layout-label" for="stage-layout-select">"Stage Output"</label>
                                <select id="stage-layout-select" data-role="stage-layout-select"></select>
                            </div>
                            <div class="operator__stage-preview" data-role="stage-status" data-active="false">
                                <div class="operator__stage-preview-stack">
                                    <div class="operator__stage-preview-panel operator__stage-preview-panel--next" data-role="stage-next">"—"</div>
                                    <div class="operator__stage-preview-song" data-role="stage-song-line">"—"</div>
                                    <div class="operator__stage-preview-actions">
                                        <button
                                            type="button"
                                            class="operator__stage-toggle"
                                            data-role="ableset-enable"
                                            data-state={if ableset_enabled { "on" } else { "off" }}
                                        >{ableset_enable_label}</button>
                                        <button
                                            type="button"
                                            class="operator__stage-toggle"
                                            data-role="ableset-follow"
                                            data-state={if ableset_follow_enabled { "on" } else { "off" }}
                                        >{ableset_follow_label}</button>
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
                                >
                                    <span data-role="stage-monitor-connected" class="operator__stage-monitor-count operator__stage-monitor-count--connected">"0"</span>
                                    <span class="operator__stage-monitor-separator">"/"</span>
                                    <span data-role="stage-monitor-issues" class="operator__stage-monitor-count operator__stage-monitor-count--issues">"0"</span>
                                </button>
                                <button
                                    type="button"
                                    class="operator__clear-button"
                                    data-role="clear-slide"
                                    aria-label="Clear live outputs"
                                >"🧹"</button>
                            </div>
                            <div class="operator__mode-toggle">
                                <button
                                    type="button"
                                    data-role="mode-toggle"
                                    data-mode="live"
                                    data-active="true"
                                >"Live"</button>
                                <button type="button" data-role="mode-toggle" data-mode="edit">"Edit"</button>
                            </div>
                            <button type="button" class="operator__hamburger" data-role="mobile-menu-toggle" aria-label="Menu">"☰"</button>
                        </div>
                    </header>
                    <div class="operator__search-results" data-role="global-search-results"></div>
                    <main class="operator__main">
                        <section class="operator__worship" data-view-panel="worship">
                            <section class="operator__catalog" data-role="catalog">
                                <div class="operator__catalog-top" data-role="catalog-top">
                                    <section class="operator__group operator__group--libraries">
                                        <header class="operator__group-header">
                                            <h2>"Libraries"</h2>
                                            <div class="operator__group-controls">
                                                <button
                                                    type="button"
                                                    class="operator__group-count"
                                                    data-role="library-more"
                                                    aria-label="Show all libraries"
                                                >"0"</button>
                                                <button
                                                    type="button"
                                                    data-role="library-create"
                                                    aria-label="Create library"
                                                    title="Create library"
                                                >"+"</button>
                                            </div>
                                        </header>
                                        <ul class="operator__list" data-role="library-list">
                                            <For
                                                each={
                                                    let libs = Arc::clone(&libraries);
                                                    move || (*libs).clone()
                                                }
                                                key=|library: &LibraryRow| library.id.clone()
                                                children={
                                                    let initial = initial_library_id.clone();
                                                    move |library: LibraryRow| {
                                                        let is_active =
                                                            initial.as_ref().map(|id| id == &library.id).unwrap_or(false);
                                                        view! {
                                                            <li class="operator__list-item">
                                                                <button
                                                                    type="button"
                                                                    class="operator__list-button"
                                                                    data-role="library-item"
                                                                    data-library-id={library.id.clone()}
                                                                    data-active={if is_active { "true" } else { "false" }}
                                                                >
                                                                    <span class="operator__list-label">{library.name.clone()}</span>
                                                                    <span class="operator__list-meta" data-role="library-count">{library.presentation_count}</span>
                                                                </button>
                                                                <div class="operator__list-actions">
                                                                    <button
                                                                        type="button"
                                                                        class="operator__list-action operator__list-action--icon operator__list-action--menu"
                                                                        data-action="library-edit"
                                                                        data-library-id={library.id.clone()}
                                                                        aria-label="Edit library"
                                                                    >{ "⋮" }</button>
                                                                </div>
                                                            </li>
                                                        }
                                                    }
                                                }
                                            />
                                        </ul>
                                    </section>
                                    <section class="operator__group operator__group--playlists">
                                        <header class="operator__group-header">
                                            <h2>"Playlists"</h2>
                                            <div class="operator__group-controls">
                                                <button
                                                    type="button"
                                                    class="operator__group-count"
                                                    data-role="playlist-more"
                                                    aria-label="Show all playlists"
                                                >"0"</button>
                                                <button
                                                    type="button"
                                                    data-role="playlist-create"
                                                    aria-label="Create playlist"
                                                    title="Create playlist"
                                                >"+"</button>
                                            </div>
                                        </header>
                                        <ul class="operator__list" data-role="playlist-list">
                                            <For
                                                each={
                                                    let lists = Arc::clone(&playlists);
                                                    move || (*lists).clone()
                                                }
                                                key=|playlist: &PlaylistRow| playlist.id.clone()
                                                children={
                                                    let initial = initial_playlist_id.clone();
                                                    move |playlist: PlaylistRow| {
                                                        let is_active =
                                                            initial.as_ref().map(|id| id == &playlist.id).unwrap_or(false);
                                                        view! {
                                                            <li class="operator__list-item">
                                                                <button
                                                                    type="button"
                                                                    class="operator__list-button"
                                                                    data-role="playlist-item"
                                                                    data-playlist-id={playlist.id.clone()}
                                                                    data-active={if is_active { "true" } else { "false" }}
                                                                >
                                                                    <span class="operator__list-label">{playlist.name.clone()}</span>
                                                                    <span class="operator__list-meta" data-role="playlist-count">{playlist.entries.len()}</span>
                                                                </button>
                                                                <div class="operator__list-actions">
                                                                    <button
                                                                        type="button"
                                                                        class="operator__list-action operator__list-action--icon operator__list-action--menu"
                                                                        data-action="playlist-edit"
                                                                        data-playlist-id={playlist.id.clone()}
                                                                        aria-label="Edit playlist"
                                                                    >{ "⋮" }</button>
                                                                </div>
                                                            </li>
                                                        }
                                                    }
                                                }
                                            />
                                        </ul>
                                    </section>
                                </div>
                                <div class="operator__catalog-resizer" data-role="catalog-resizer" aria-hidden="true"></div>
                                <div class="operator__catalog-bottom" data-role="catalog-bottom" data-dropzone-target="presentations">
                                    <header class="operator__group-header operator__presentations-header">
                                        <h2 data-role="context-title">"Presentations"</h2>
                                        <div class="operator__group-controls">
                                            <span class="operator__group-count operator__group-count--static" data-role="presentation-count">"—"</span>
                                            <button
                                                type="button"
                                                data-role="presentation-create"
                                                aria-label="Add presentation or separator"
                                                title="Add"
                                            >"+"</button>
                                        </div>
                                    </header>
                                    <ul class="operator__presentation-list" data-role="presentation-list">
                                        <li class="empty">"Select a library or playlist to view presentations."</li>
                                    </ul>
                                </div>
                            </section>
                            <section class="operator__slides-column">
                                <div class="operator__slides-toolbar">
                                    <label class="operator__line-limit" title="Maximum characters per line">
                                        <span>"Line limit"</span>
                                        <input
                                            type="number"
                                            min="10"
                                            max="120"
                                            step="1"
                                            value="32"
                                            data-role="line-limit"
                                        />
                                    </label>
                                    <button type="button" class="operator__slides-add" data-role="add-slide" title="Add slide">"+"</button>
                                </div>
                                <div class="operator__slides" data-role="slides">
                                    <p class="empty">"Select a presentation to load slides."</p>
                                </div>
                            </section>
                        </section>
    <section class="operator__panel operator__panel--bible" data-view-panel="bible">
                            <iframe src="/ui/bible" title="Bible Control"></iframe>
                        </section>
                        <section class="operator__panel operator__panel--timers" data-view-panel="timers">
                            <div class="operator__timers" data-role="timer-cards">
                                {
                                let overview = Arc::clone(&timers);
                                view! {
                                    <article class="operator__timer-card" data-role="timer-countdown">
                                        <header>
                                            <strong>"Countdown"</strong>
                                        </header>
                                        <p class="operator__timer-primary" id="countdown-value">
                                            {format_seconds(overview.countdown_to_start.seconds_remaining)}
                                        </p>
                                        <small id="countdown-target">
                                            {format!("Target {}", overview.countdown_to_start.target.format("%H:%M:%S %Z"))}
                                        </small>
                                    </article>
                                }
                            }
                            {
                                let overview = Arc::clone(&timers);
                                view! {
                                    <article class="operator__timer-card" data-role="timer-preach">
                                        <header>
                                            <strong>"Preach"</strong>
                                            <span class="operator__timer-state" id="preach-state">
                                                {format_timer_state(overview.preach_timer.state)}
                                            </span>
                                        </header>
                                        <p class="operator__timer-primary" id="preach-value">
                                            {format_seconds(overview.preach_timer.seconds_elapsed)}
                                        </p>
                                        <small id="preach-elapsed">
                                            {format!("Elapsed {}", format_seconds(overview.preach_timer.seconds_elapsed))}
                                        </small>
                                    </article>
                                }
                            }
                            </div>
                            <div class="operator__timer-actions" data-role="timer-actions">
                                <div class="operator__timer-group">
                                    <h3>"Countdown"</h3>
                                    <label class="operator__timer-field">
                                        <span>"Service start"</span>
                                        <input
                                            type="text"
                                            inputmode="numeric"
                                            placeholder="18:00"
                                            data-role="countdown-target-input"
                                            aria-label="Countdown target time (HH:MM)"
                                        />
                                    </label>
                                    <p class="operator__timer-help">
                                        "Type HH:MM (or minutes only) and press Enter or Set to update while the timer runs."
                                    </p>
                                    <div class="operator__timer-buttons">
                                        <button type="button" data-role="countdown-start">"Start"</button>
                                        <button type="button" data-role="countdown-offset-minus">"-5 min"</button>
                                        <button type="button" data-role="countdown-offset-plus">"+5 min"</button>
                                    </div>
                                    <div class="operator__timer-links">
                                        <button type="button" data-role="timer-overlay-open">"Open Countdown Overlay"</button>
                                        <button type="button" data-role="timer-overlay-copy">"Copy Overlay URL"</button>
                                    </div>
                                </div>
                                <div class="operator__timer-group">
                                    <h3>"Preach"</h3>
                                    <div class="operator__timer-buttons">
                                        <button type="button" data-command="start_preach">"Start"</button>
                                        <button type="button" data-command="reset_preach">"Reset"</button>
                                    </div>
                                </div>
                            </div>
                        </section>
                        <section class="operator__panel operator__panel--settings" data-view-panel="settings">
                            <iframe src="/ui/settings" title="Settings" class="operator__settings-frame"></iframe>
                        </section>
                    </main>
                    <div class="operator__toast" data-role="toast"></div>
                    <div class="operator__library-modal" data-role="library-modal">
                        <div class="operator__library-modal-panel">
                            <header class="operator__library-modal-header">
                                <h3>"All Libraries"</h3>
                                <button type="button" class="operator__library-modal-close" data-role="library-modal-close" aria-label="Close">"×"</button>
                            </header>
                            <div class="operator__library-modal-body" data-role="library-modal-list"></div>
                        </div>
                    </div>
                    <div class="operator__playlist-modal" data-role="playlist-modal">
                        <div class="operator__playlist-modal-panel">
                            <header class="operator__playlist-modal-header">
                                <h3>"All Playlists"</h3>
                                <button type="button" class="operator__playlist-modal-close" data-role="playlist-modal-close" aria-label="Close">"×"</button>
                            </header>
                            <div class="operator__playlist-modal-body" data-role="playlist-modal-list"></div>
                        </div>
                    </div>
                    <div class="operator__library-edit" data-role="library-edit-modal" data-mode="edit">
                        <div class="operator__library-edit-panel">
                            <form class="operator__library-edit-form" data-role="library-edit-form">
                                <header class="operator__library-edit-header">
                                    <h3 data-role="library-edit-title">"Edit Library"</h3>
                                </header>
                                <div class="operator__library-edit-body">
                                    <label>
                                        <span>"Library name"</span>
                                        <input type="text" data-role="library-edit-name" autocomplete="off" required minlength="1" maxlength="120" />
                                    </label>
                                    <label class="operator__library-edit-favorite">
                                        <input type="checkbox" data-role="library-edit-favorite" />
                                        <span>"Show in dashboard"</span>
                                    </label>
                                </div>
                                <footer class="operator__library-edit-footer">
                                    <button
                                        type="button"
                                        class="operator__library-edit-delete"
                                        data-role="library-edit-delete"
                                    >"Delete library"</button>
                                    <div class="operator__library-edit-actions">
                                        <button type="button" data-role="library-edit-cancel">"Cancel"</button>
                                        <button type="submit" data-role="library-edit-save">"Save changes"</button>
                                    </div>
                                </footer>
                            </form>
                        </div>
                    </div>
                    <div class="operator__library-edit operator__playlist-edit" data-role="playlist-edit-modal" data-mode="edit">
                        <div class="operator__library-edit-panel">
                            <form class="operator__library-edit-form" data-role="playlist-edit-form">
                                <header class="operator__library-edit-header">
                                    <h3 data-role="playlist-edit-title">"Edit Playlist"</h3>
                                </header>
                                <div class="operator__library-edit-body">
                                    <label>
                                        <span>"Playlist name"</span>
                                        <input type="text" data-role="playlist-edit-name" autocomplete="off" required minlength="1" maxlength="160" />
                                    </label>
                                    <label class="operator__library-edit-favorite">
                                        <input type="checkbox" data-role="playlist-edit-dashboard" />
                                        <span>"Show in dashboard"</span>
                                    </label>
                                </div>
                                <footer class="operator__library-edit-footer">
                                    <button
                                        type="button"
                                        class="operator__library-edit-delete"
                                        data-role="playlist-edit-delete"
                                    >"Delete playlist"</button>
                                    <div class="operator__library-edit-actions">
                                        <button type="button" data-role="playlist-edit-cancel">"Cancel"</button>
                                        <button type="submit" data-role="playlist-edit-save">"Save changes"</button>
                                    </div>
                                </footer>
                            </form>
                        </div>
                    </div>
                    <div class="operator__library-edit operator__presentation-edit" data-role="presentation-edit-modal" data-mode="presentation">
                        <div class="operator__library-edit-panel">
                            <form class="operator__library-edit-form" data-role="presentation-edit-form">
                                <header class="operator__library-edit-header">
                                    <h3 data-role="presentation-edit-title">"Rename Presentation"</h3>
                                </header>
                                <div class="operator__library-edit-body">
                                    <label>
                                        <span data-role="presentation-edit-label">"Presentation name"</span>
                                        <input type="text" data-role="presentation-edit-name" autocomplete="off" required minlength="1" maxlength="160" />
                                    </label>
                                </div>
                                <footer class="operator__library-edit-footer">
                                    <button
                                        type="button"
                                        class="operator__library-edit-delete"
                                        data-role="presentation-edit-delete"
                                    >"Delete presentation"</button>
                                    <div class="operator__library-edit-actions">
                                        <button type="button" data-role="presentation-edit-cancel">"Cancel"</button>
                                        <button type="submit" data-role="presentation-edit-save">"Save changes"</button>
                                    </div>
                                </footer>
                            </form>
                        </div>
                    </div>
                    <div class="operator__library-edit operator__presentation-create" data-role="presentation-create-modal">
                        <div class="operator__library-edit-panel">
                            <div class="operator__library-edit-form" data-role="presentation-create-form">
                                <header class="operator__library-edit-header">
                                    <h3>"Create Presentation"</h3>
                                </header>
                                <div class="operator__library-edit-body">
                                    <label>
                                        <span>"Presentation name"</span>
                                        <input type="text" data-role="presentation-create-name" autocomplete="off" maxlength="160" placeholder="New Presentation" />
                                    </label>
                                    <div class="operator__presentation-create-options" data-role="presentation-create-options">
                                        <button type="button" class="operator__create-option" data-role="presentation-create-blank">
                                            <span class="operator__create-option-icon" aria-hidden="true">{"\u{1F4C4}"}</span>
                                            <span class="operator__create-option-label">"Blank"</span>
                                            <span class="operator__create-option-desc">"Empty presentation"</span>
                                        </button>
                                        <button type="button" class="operator__create-option" data-role="presentation-create-paste">
                                            <span class="operator__create-option-icon" aria-hidden="true">{"\u{1F4CB}"}</span>
                                            <span class="operator__create-option-label">"Paste"</span>
                                            <span class="operator__create-option-desc">"Paste song text"</span>
                                        </button>
                                        <button type="button" class="operator__create-option" data-role="presentation-create-import">
                                            <span class="operator__create-option-icon" aria-hidden="true">{"\u{1F4C1}"}</span>
                                            <span class="operator__create-option-label">"Import"</span>
                                            <span class="operator__create-option-desc">".pro file"</span>
                                        </button>
                                    </div>
                                    <div class="operator__presentation-create-paste-area" data-role="presentation-create-paste-area" style="display:none">
                                        <textarea data-role="presentation-create-paste-text" rows="10" placeholder="Paste song text here..."></textarea>
                                        <div class="operator__presentation-create-sub-actions">
                                            <button type="button" data-role="presentation-create-paste-back">"Back"</button>
                                            <button type="button" data-role="presentation-create-paste-confirm">"Create"</button>
                                        </div>
                                    </div>
                                    <div class="operator__presentation-create-import-area" data-role="presentation-create-import-area" style="display:none">
                                        <input type="file" data-role="presentation-create-import-file" accept=".pro" />
                                        <div class="operator__presentation-create-sub-actions">
                                            <button type="button" data-role="presentation-create-import-back">"Back"</button>
                                            <button type="button" data-role="presentation-create-import-confirm">"Import"</button>
                                        </div>
                                    </div>
                                </div>
                                <footer class="operator__library-edit-footer">
                                    <div class="operator__library-edit-actions">
                                        <button type="button" data-role="presentation-create-cancel">"Cancel"</button>
                                    </div>
                                </footer>
                            </div>
                        </div>
                    </div>
                    <script>{operator_script}</script>
                    <footer class="operator__version">
                        "v"{VERSION}
                        {if BUILD_CHANNEL != "release" { format!(" ({})", BUILD_CHANNEL) } else { String::new() }}
                    </footer>
                </body>
            </html>
        }
}

pub async fn render_operator_ui(
    state: &AppState,
    initial_view: &str,
) -> anyhow::Result<Html<String>> {
    let library_summaries = state.library_summaries(None).await?;
    let favorite_ids: HashSet<_> = state
        .library_favorites()
        .await?
        .into_iter()
        .map(|id| id.to_string())
        .collect();
    let playlists = state.playlists().await?;
    let timers = state.timers_overview().await?;
    let stage_layouts = state.stage_displays().await?;
    let stage_layout_code = state.stage_layout_code().await;
    let ableset_status = state.ableset_status_snapshot().await;

    let mut presentation_lookup: HashMap<String, String> = HashMap::new();

    let library_rows: Vec<LibraryRow> = library_summaries
        .into_iter()
        .map(|summary| {
            let presentations: Vec<PresentationRow> = summary
                .presentations
                .into_iter()
                .map(|presentation| {
                    presentation_lookup
                        .insert(presentation.id.to_string(), presentation.name.clone());
                    PresentationRow {
                        id: presentation.id.to_string(),
                        name: presentation.name,
                    }
                })
                .collect();
            LibraryRow {
                id: summary.id.to_string(),
                name: summary.name,
                presentation_count: summary.presentation_count,
                presentations,
                is_favorite: favorite_ids.contains(&summary.id.to_string()),
            }
        })
        .collect();

    let playlist_rows: Vec<PlaylistRow> = playlists
        .into_iter()
        .map(|playlist| {
            let entries = playlist
                .entries
                .into_iter()
                .map(|entry| match entry.kind {
                    PlaylistEntryKind::Presentation {
                        presentation_id, ..
                    } => {
                        let presentation_id_str = presentation_id.to_string();
                        let name = presentation_lookup
                            .get(&presentation_id_str)
                            .cloned()
                            .unwrap_or_else(|| "Untitled presentation".to_string());
                        PlaylistEntryRow {
                            entry_id: entry.id.to_string(),
                            entry_type: "presentation".to_string(),
                            name,
                            presentation_id: Some(presentation_id_str),
                        }
                    }
                    PlaylistEntryKind::Separator { name } => PlaylistEntryRow {
                        entry_id: entry.id.to_string(),
                        entry_type: "separator".to_string(),
                        name,
                        presentation_id: None,
                    },
                })
                .collect();
            PlaylistRow {
                id: playlist.id.to_string(),
                name: playlist.name,
                entries,
                show_in_dashboard: playlist.show_in_dashboard,
            }
        })
        .collect();

    let libraries_json = to_string(&library_rows)?;
    let playlists_json = to_string(&playlist_rows)?;
    let stage_layouts_json = to_string(&stage_layouts)?;

    let owner = Owner::new_root(None);
    let html = owner.with(|| {
        view! {
            <OperatorDocument
                libraries=library_rows.clone()
                playlists=playlist_rows.clone()
                timers=timers.clone()
                ableset_status=ableset_status.clone()
                libraries_json=libraries_json.clone()
                playlists_json=playlists_json.clone()
                stage_layouts_json=stage_layouts_json.clone()
                stage_layout_code=stage_layout_code.clone()
                initial_view=initial_view.to_string()
            />
        }
        .into_view()
        .to_html()
    });

    Ok(Html(format!("<!DOCTYPE html>{html}")))
}
