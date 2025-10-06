use crate::{
    ableset::AbleSetStatusSnapshot,
    android_stage::AndroidStageDisplayStatusSnapshot,
    osc::OscStatusSnapshot,
    resolume::{ResolumeConnectionSnapshot, ResolumeConnectionState},
    state::{AppState, FeatureFlags},
};
use axum::response::Html;
use chrono::{DateTime, Utc};
use leptos::prelude::*;
use presenter_core::{
    playlist::PlaylistEntryKind, AbleSetSettings, BibleBroadcast, BibleTranslation, OscSettings,
    TimerState, TimersOverview,
};
use reactive_graph::owner::Owner;
use serde::Serialize;
use serde_json::{json, to_string};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

const OPERATOR_SCRIPT_TEMPLATE: &str = include_str!("operator_script.js");
const TABLET_SCRIPT_TEMPLATE: &str = include_str!("tablet_script.js");
const BIBLE_SCRIPT_TEMPLATE: &str = include_str!("bible_script.js");
const SETTINGS_SCRIPT_TEMPLATE: &str = include_str!("settings_script.js");

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryRow {
    pub id: String,
    pub name: String,
    pub presentation_count: usize,
    pub presentations: Vec<PresentationRow>,
    pub is_favorite: bool,
}

#[derive(Clone, Serialize)]
pub struct PresentationRow {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistRow {
    pub id: String,
    pub name: String,
    pub entries: Vec<PlaylistEntryRow>,
    #[serde(default)]
    pub show_in_dashboard: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistEntryRow {
    pub entry_id: String,
    #[serde(rename = "type")]
    pub entry_type: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presentation_id: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsHostRow {
    pub id: String,
    pub label: String,
    pub host: String,
    pub port: u16,
    pub is_enabled: bool,
    pub created_at: String,
    pub created_at_display: String,
    pub updated_at: String,
    pub updated_at_display: String,
    pub status_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_latency_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<ResolumeConnectionSnapshot>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsAndroidDisplayRow {
    pub id: String,
    pub label: String,
    pub host: String,
    pub port: u16,
    pub launch_component: String,
    pub is_enabled: bool,
    pub created_at: String,
    pub created_at_display: String,
    pub updated_at: String,
    pub updated_at_display: String,
    pub status_state: String,
    pub last_attempt_display: String,
    pub last_success_display: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<AndroidStageDisplayStatusSnapshot>,
}

fn format_timer_state(state: TimerState) -> &'static str {
    match state {
        TimerState::Idle => "Idle",
        TimerState::Running => "Running",
        TimerState::Paused => "Paused",
        TimerState::Completed => "Completed",
    }
}

fn format_seconds(total_seconds: i64) -> String {
    let total = total_seconds.max(0);
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let seconds = total % 60;
    if hours > 0 {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

fn format_seconds_compact(total_seconds: i64) -> String {
    let total = total_seconds.max(0);
    if total < 60 {
        total.to_string()
    } else {
        let minutes = total / 60;
        let seconds = total % 60;
        format!("{minutes:02}:{seconds:02}")
    }
}

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
    let libraries_json = libraries_json.replace("</script>", r"<\/script>");
    let playlists_json = playlists_json.replace("</script>", r"<\/script>");
    let timers_json = to_string(&*timers).unwrap_or_else(|_| "{}".to_string());
    let timers_json = timers_json.replace("</script>", r"<\/script>");
    let stage_layouts_json = stage_layouts_json.replace("</script>", r"<\/script>");

    let stage_layout_code_safe = stage_layout_code.replace('"', "\\\"");

    let ableset_status_json = to_string(&ableset_status)
        .unwrap_or_else(|_| "{}".to_string())
        .replace("</script>", r"<\/script>");

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
                <body class="operator" data-view="worship" data-mode="live">
                    <header class="operator__header">
                        <div class="operator__header-left">
                            <h1>"Presenter Operator"</h1>
                            <nav class="operator__view-nav">
                                <button
                                    type="button"
                                    data-role="view-toggle"
                                    data-view="worship"
                                    data-active="true"
                                >"Worship"</button>
                                <button type="button" data-role="view-toggle" data-view="bible">"Bible"</button>
                                <button type="button" data-role="view-toggle" data-view="timers">"Timers"</button>
                                <button
                                    type="button"
                                    data-role="view-toggle"
                                    data-view="settings"
                                >"Settings"</button>
                            </nav>
                        </div>
                        <div class="operator__header-center">
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
                                ><span aria-hidden="true">{ "×" }</span><span class="sr-only">Clear search</span></button>
                            </form>
                            <div class="operator__search-results" data-role="global-search-results"></div>
                            <div class="operator__stage-layout" aria-label="Stage display mode">
                                <label class="operator__stage-layout-label" for="stage-layout-select">"Stage Output"</label>
                                <select id="stage-layout-select" data-role="stage-layout-select"></select>
                            </div>
                        </div>
                        <div class="operator__header-right">
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
                        </div>
                    </header>
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
                                    <div class="operator__library-edit-actions">
                                        <button type="button" data-role="presentation-edit-cancel">"Cancel"</button>
                                        <button type="submit" data-role="presentation-edit-save">"Save changes"</button>
                                    </div>
                                </footer>
                            </form>
                        </div>
                    </div>
                    <script>{operator_script}</script>
                </body>
            </html>
        }
}

pub async fn render_operator_ui(state: &AppState) -> anyhow::Result<Html<String>> {
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
            />
        }
        .into_view()
        .to_html()
    });

    Ok(Html(format!("<!DOCTYPE html>{html}")))
}
const OPERATOR_STYLES: &str = r#"
:root {
    --operator-bg: #f5f6f8;
    --operator-panel: #ffffff;
    --operator-border: #d7d9e0;
    --operator-text: #191a1d;
    --operator-muted: #6b6f7b;
    --operator-accent: #3b7cff;
    --operator-accent-dark: #2554c1;
    --operator-radius: 12px;
    --shadow-soft: 0 12px 28px rgba(15, 23, 42, 0.08);
    --shadow-inner: inset 0 0 0 1px rgba(15, 23, 42, 0.04);
}

.sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
}

body.operator {
    margin: 0;
    min-height: 100vh;
    height: 100vh;
    display: flex;
    flex-direction: column;
    font-family: "Inter", "Segoe UI", system-ui, sans-serif;
    background: var(--operator-bg);
    color: var(--operator-text);
    overflow: hidden;
    --operator-line-limit-ch: 32;
    --operator-line-line-height: 1.35;
}

.operator__header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 1rem 1.5rem;
    background: linear-gradient(90deg, #111827, #1f2937);
    color: #ffffff;
    box-shadow: var(--shadow-soft);
    position: sticky;
    top: 0;
    z-index: 10;
}

.operator__header h1 {
    margin: 0;
    font-size: 1.25rem;
    font-weight: 600;
}

.operator__header-left {
    display: flex;
    align-items: center;
    gap: 1.5rem;
}

.operator__header-center {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    position: relative;
    margin: 0 1.5rem;
}

.operator__search {
    width: min(100%, 420px);
    background: rgba(255, 255, 255, 0.12);
    border-radius: 999px;
    display: flex;
    align-items: center;
    padding: 0.35rem 0.75rem;
    gap: 0.5rem;
    border: 1px solid rgba(255, 255, 255, 0.18);
    box-shadow: inset 0 0 0 1px rgba(0, 0, 0, 0.05);
}

.operator__search input {
    flex: 1;
    border: none;
    background: transparent;
    color: #ffffff;
    font-size: 0.85rem;
    outline: none;
}

.operator__search input::placeholder {
    color: rgba(255, 255, 255, 0.6);
}

.operator__search button {
    border: none;
    background: transparent;
    color: rgba(255, 255, 255, 0.7);
    font-size: 1rem;
    cursor: pointer;
    padding: 0;
}

.operator__search button:hover {
    color: #ffffff;
}

.operator__search-icon {
    width: 1rem;
    height: 1rem;
    border-radius: 50%;
    border: 2px solid rgba(255, 255, 255, 0.6);
    position: relative;
}

.operator__search [data-role="global-search-clear"] {
    border: none;
    background: transparent;
    color: rgba(248, 250, 252, 0.75);
    cursor: pointer;
    padding: 0;
    margin: 0;
    font-size: 1.1rem;
    line-height: 1;
    transition: color 0.2s ease;
}

.operator__search [data-role="global-search-clear"]:hover {
    color: #ffffff;
}

.operator__search [data-role="global-search-clear"][hidden] {
    display: none;
}

.operator__search-icon::after {
    content: '';
    position: absolute;
    width: 0.55rem;
    height: 0.15rem;
    background: rgba(255, 255, 255, 0.6);
    top: 0.75rem;
    left: 0.55rem;
    transform: rotate(45deg);
    border-radius: 999px;
}

.operator__search-results {
    position: absolute;
    top: 3.2rem;
    left: 50%;
    transform: translateX(-50%);
    width: min(100%, 520px);
    background: #ffffff;
    color: var(--operator-text);
    border-radius: 14px;
    border: 1px solid rgba(15, 23, 42, 0.12);
    box-shadow: 0 18px 38px rgba(15, 23, 42, 0.18);
    max-height: 420px;
    overflow-y: auto;
    display: none;
    z-index: 20;
}

.operator__search-results[data-visible="true"] {
    display: block;
}

.operator__search-group {
    padding: 0.75rem 1rem;
}

.operator__search-group + .operator__search-group {
    border-top: 1px solid rgba(15, 23, 42, 0.08);
}

.operator__search-group h3 {
    margin: 0 0 0.35rem 0;
    font-size: 0.75rem;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--operator-muted);
}

.operator__search-result {
    list-style: none;
    margin: 0;
    padding: 0;
}

.operator__search-result button {
    width: 100%;
    border: none;
    background: transparent;
    text-align: left;
    padding: 0.4rem 0.55rem;
    border-radius: 10px;
    cursor: pointer;
    display: flex;
    flex-direction: column;
    gap: 0.2rem;
}

.operator__search-result button:hover {
    background: rgba(59, 124, 255, 0.12);
}

.operator__search-result-title {
    font-weight: 600;
    font-size: 0.9rem;
    color: #0f172a;
}

.operator__search-result-meta {
    font-size: 0.75rem;
    color: var(--operator-muted);
}

.operator__search-result-snippet {
    font-size: 0.75rem;
    color: rgba(15, 23, 42, 0.72);
}

.operator__search-empty {
    margin: 0;
    font-size: 0.8rem;
    color: var(--operator-muted);
}

.operator__view-nav {
    font-size: 0.7rem;
    letter-spacing: 0.16em;
    text-transform: uppercase;
    color: #cbd5f5;
    opacity: 0.75;
}

.operator__mode-toggle {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
    background: rgba(255, 255, 255, 0.08);
    border-radius: 999px;
    padding: 0.25rem;
}

.operator__view-nav button,
.operator__mode-toggle button {
    border: none;
    background: transparent;
    color: inherit;
    padding: 0.45rem 0.9rem;
    border-radius: 999px;
    font-size: 0.85rem;
    cursor: pointer;
    transition: background 0.2s ease, color 0.2s ease;
}

.operator__view-nav button[data-active="true"],
.operator__mode-toggle button[data-active="true"] {
    background: #ffffff;
    color: #1f2937;
    box-shadow: 0 6px 12px rgba(15, 23, 42, 0.15);
}

.operator__main {
    flex: 1;
    display: flex;
    position: relative;
    overflow: hidden;
    min-height: 0;
}

.operator__worship {
    flex: 1;
    display: flex;
    gap: 1.5rem;
    min-height: 0;
}

.operator__sidebar {
    flex: 0 0 280px;
    background: var(--operator-panel);
    border: 1px solid var(--operator-border);
    border-radius: var(--operator-radius);
    padding: 1rem 1.25rem;
    display: flex;
    flex-direction: column;
    gap: 1.25rem;
    overflow-y: auto;
    max-height: calc(100vh - 5.5rem);
    position: sticky;
    top: calc(4.75rem);
}

.operator__group-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 0.75rem;
    margin-bottom: 0.75rem;
}

.operator__presentations-header h2 {
    margin: 0;
    font-size: 0.95rem;
    font-weight: 600;
    color: var(--operator-muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
}

.operator__group h2 {
    margin: 0;
    font-size: 0.95rem;
    font-weight: 600;
    color: var(--operator-muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
}

.operator__group-controls {
    display: flex;
    align-items: center;
    gap: 0.45rem;
}

.operator__group-controls [data-role$="create"] {
    font-size: 0.85rem;
    padding: 0.3rem 0.7rem;
    border-radius: 999px;
    border: none;
    background: rgba(59, 124, 255, 0.16);
    color: var(--operator-accent-dark);
    cursor: pointer;
    transition: background 0.2s ease, color 0.2s ease;
}

.operator__group-controls [data-role$="create"]:hover {
    background: rgba(59, 124, 255, 0.28);
    color: #ffffff;
}

.operator__group-count {
    border: 1px solid rgba(59, 124, 255, 0.35);
    background: rgba(59, 124, 255, 0.12);
    color: var(--operator-accent-dark);
    border-radius: 999px;
    padding: 0.25rem 0.65rem;
    font-size: 0.85rem;
    cursor: pointer;
    min-width: 2.5rem;
    text-align: center;
    transition: background 0.2s ease, color 0.2s ease;
}

.operator__group-count:hover {
    background: rgba(59, 124, 255, 0.24);
    color: #ffffff;
}

.operator__group-count[disabled] {
    opacity: 0.55;
    cursor: default;
}

.operator__group-count[data-empty="true"] {
    opacity: 0.6;
}

.operator__list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
}

.operator__list-item {
    display: flex;
    align-items: center;
    gap: 0.35rem;
}

.operator__favorites-empty {
    color: var(--operator-muted);
    font-size: 0.9rem;
    margin: 0.4rem 0 0;
}

.operator__list-button {
    width: 100%;
    text-align: left;
    display: flex;
    align-items: center;
    gap: 0.4rem;
    background: rgba(99, 102, 241, 0.08);
    border: 1px solid transparent;
    border-radius: 10px;
    padding: 0.55rem 0.75rem;
    font-size: 0.9rem;
    color: var(--operator-text);
    cursor: pointer;
    transition: background 0.2s ease, border 0.2s ease;
}

.operator__list-favorite {
    border: none;
    background: transparent;
    color: rgba(59, 124, 255, 0.45);
    font-size: 1rem;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 2rem;
    height: 2rem;
    cursor: pointer;
    transition: color 0.2s ease, transform 0.2s ease;
}

.operator__list-favorite[aria-pressed="true"] {
    color: #f59e0b;
    transform: scale(1.1);
}

.operator__list-favorite:focus-visible {
    outline: 2px solid rgba(59, 124, 255, 0.6);
    outline-offset: 2px;
}

.operator__list-favorite--inline {
    width: 1.75rem;
    height: 1.75rem;
    font-size: 0.95rem;
    margin-right: 0.25rem;
}

.operator__list-label {
    flex: 1;
}

.operator__list-meta {
    font-size: 0.75rem;
    color: var(--operator-muted);
    background: rgba(59, 124, 255, 0.16);
    border-radius: 999px;
    padding: 0.1rem 0.4rem;
}

.operator__list-button:hover {
    border-color: rgba(59, 124, 255, 0.45);
}

.operator__list-button[data-active="true"] {
    background: rgba(59, 124, 255, 0.18);
    border-color: rgba(59, 124, 255, 0.6);
    font-weight: 600;
}

.operator__list-row {
    display: flex;
    align-items: center;
    gap: 0.35rem;
}

.operator__list-row--modal {
    padding: 0.1rem 0;
}

.operator__list-row > .operator__list-button {
    flex: 1;
}

.operator__list-actions {
    display: flex;
    gap: 0.25rem;
    align-items: center;
}

.operator__list-action {
    border: 1px solid transparent;
    border-radius: 8px;
    background: rgba(148, 163, 184, 0.12);
    color: var(--operator-muted);
    font-size: 0.75rem;
    padding: 0.35rem 0.55rem;
    cursor: pointer;
    transition: background 0.2s ease, color 0.2s ease;
}

.operator__list-action--icon {
    width: 2.1rem;
    height: 2.1rem;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    font-size: 1rem;
}

.operator__list-action:hover {
    background: rgba(59, 124, 255, 0.16);
    color: var(--operator-text);
}

.operator__list-action--danger {
    background: rgba(239, 68, 68, 0.12);
    color: rgb(239, 68, 68);
}

.operator__list-action--danger:hover {
    background: rgba(239, 68, 68, 0.24);
    color: rgb(248, 113, 113);
}

.operator__list-action--menu {
    color: rgba(100, 116, 139, 0.9);
    background: transparent;
}

.operator__list-action--menu:hover {
    background: rgba(59, 124, 255, 0.16);
    color: var(--operator-accent-dark);
}

.operator__playlist-modal-body ul {
    list-style: none;
    margin: 0;
    padding: 0;
}

.operator__playlist-modal-body li + li {
    margin-top: 0.4rem;
}

.operator__workspace {
    flex: 1;
    display: flex;
    gap: 1.5rem;
    padding: 0;
    overflow: hidden;
    min-height: 0;
}

.operator__presentations {
    flex: 0 0 320px;
    background: var(--operator-panel);
    border-radius: var(--operator-radius);
    border: 1px solid var(--operator-border);
    display: flex;
    flex-direction: column;
    overflow: hidden;
}

.operator__presentations header {
    padding: 0.9rem 1rem;
    border-bottom: 1px solid rgba(15, 23, 42, 0.06);
}

.operator__presentation-list {
    list-style: none;
    margin: 0;
    padding: 0.75rem;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    overflow-y: auto;
}

.operator__presentation-list[data-dropzone="append"] {
    background: rgba(59, 124, 255, 0.08);
    outline: 2px dashed rgba(59, 124, 255, 0.5);
    outline-offset: -6px;
}

.operator__catalog-bottom[data-dropzone="append"] {
    background: rgba(59, 124, 255, 0.04);
}

.operator__presentation-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.65rem;
    background: rgba(15, 23, 42, 0.05);
    border-radius: 10px;
    padding: 0.55rem 0.75rem;
    border: 1px solid transparent;
    cursor: pointer;
    transition: background 0.2s ease, border 0.2s ease;
}

.operator__presentation-item[data-drop-position] {
    position: relative;
}

.operator__presentation-item[data-drop-position="before"]::before,
.operator__presentation-item[data-drop-position="after"]::after {
    content: '';
    position: absolute;
    left: 12px;
    right: 12px;
    border-top: 3px solid rgba(59, 124, 255, 0.85);
    border-radius: 999px;
    pointer-events: none;
}

.operator__presentation-item[data-drop-position="before"]::before {
    top: -6px;
}

.operator__presentation-item[data-drop-position="after"]::after {
    bottom: -6px;
}

.operator__presentation-item.is-active {
    background: rgba(59, 124, 255, 0.2);
    border-color: rgba(59, 124, 255, 0.5);
}

.operator__presentation-item.is-stage-active {
    box-shadow: 0 0 0 2px rgba(59, 124, 255, 0.35);
}

.operator__presentation-meta {
    font-size: 0.75rem;
    color: var(--operator-muted);
    margin-left: auto;
    margin-right: 0.35rem;
}

.operator__presentation-actions {
    display: inline-flex;
    gap: 0.35rem;
}

.operator__presentation-actions button {
    border: none;
    background: rgba(15, 23, 42, 0.12);
    color: var(--operator-muted);
    border-radius: 999px;
    padding: 0.1rem 0.45rem;
    cursor: pointer;
}

.operator__presentation-actions button:hover {
    background: rgba(59, 124, 255, 0.2);
    color: var(--operator-accent-dark);
}

.operator__slides-panel {
    flex: 1;
    background: var(--operator-panel);
    border-radius: var(--operator-radius);
    border: 1px solid var(--operator-border);
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
}

.operator__slides-toolbar {
    display: flex;
    justify-content: flex-end;
    align-items: center;
    gap: 0.75rem;
    padding: 0.75rem 1rem;
    border-bottom: 1px solid rgba(15, 23, 42, 0.06);
}

.operator__line-limit {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.78rem;
    color: var(--operator-muted);
    transition: opacity 0.2s ease;
}

.operator__line-limit[hidden] {
    display: none !important;
}

.operator__line-limit input {
    width: 3.5rem;
    border-radius: 8px;
    border: 1px solid rgba(15, 23, 42, 0.2);
    padding: 0.35rem 0.45rem;
    font-size: 0.85rem;
    text-align: center;
}

.operator__line-limit[data-disabled="true"] {
    opacity: 0.35;
}

.operator__line-limit[data-disabled="true"] input {
    pointer-events: none;
}

body.operator[data-mode="live"] .operator__line-limit {
    display: none !important;
}

.operator__slides-actions button {
    border: none;
    border-radius: 8px;
    padding: 0.45rem 0.85rem;
    background: var(--operator-accent);
    color: #ffffff;
    font-weight: 500;
    cursor: pointer;
    box-shadow: 0 10px 18px rgba(59, 124, 255, 0.28);
}

.operator__slides-clear:hover {
    background: #dc2626;
}

.operator__header-right {
    display: flex;
    align-items: center;
    gap: 1.5rem;
}

.operator__stage-preview {
    position: relative;
    display: inline-flex;
    align-items: stretch;
    gap: 1rem;
    padding: 0.65rem 1rem;
    background: #101828;
    border: 1px solid rgba(148, 163, 184, 0.25);
    color: #f8fafc;
    min-width: 0;
    border-radius: 14px;
    box-shadow: inset 0 0 0 1px rgba(15, 23, 42, 0.25);
}

.operator__stage-preview[data-active="false"] {
    opacity: 0.6;
}

.operator__stage-monitor {
    position: absolute;
    right: 0.35rem;
    bottom: 0.25rem;
    padding: 0;
    border: none;
    background: none;
    color: #e2e8f0;
    font-size: 0.78rem;
    font-weight: 700;
    display: inline-flex;
    align-items: baseline;
    gap: 0.2rem;
    cursor: pointer;
    font-variant-numeric: tabular-nums;
    text-shadow: 0 0 6px rgba(15, 23, 42, 0.85);
}

.operator__stage-monitor:hover {
    color: #38bdf8;
}

.operator__stage-monitor:focus-visible {
    outline: 2px solid rgba(56, 189, 248, 0.65);
    outline-offset: 2px;
}

.operator__stage-monitor--alert {
    color: #f87171;
}

.operator__stage-monitor-separator {
    opacity: 0.6;
}

.operator__stage-monitor-count {
    font-variant-numeric: tabular-nums;
    min-width: 1.4ch;
    text-align: right;
    display: inline-block;
}

.operator__stage-monitor-count--connected {
    color: #4ade80;
}

.operator__stage-monitor-count--issues {
    color: #64748b;
    transition: color 0.2s ease;
}

.operator__stage-monitor--alert .operator__stage-monitor-count--issues {
    color: #f87171;
    font-size: 1.15rem;
    font-weight: 800;
    animation: operatorStageMonitorPulse 1s ease-in-out infinite;
    text-shadow: 0 0 8px rgba(248, 113, 113, 0.45);
}

@keyframes operatorStageMonitorPulse {
    0%, 100% {
        opacity: 1;
    }
    50% {
        opacity: 0.35;
    }
}

.operator__stage-preview-stack {
    display: flex;
    flex-direction: column;
    justify-content: flex-start;
    gap: 0.5rem;
    min-width: 12rem;
    align-items: center;
}

.operator__stage-preview-song {
    font-size: 0.82rem;
    font-weight: 400;
    letter-spacing: 0.01em;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 100%;
    text-align: center;
}

.operator__stage-preview-actions {
    display: flex;
    gap: 0.5rem;
    justify-content: center;
}

.operator__stage-toggle {
    border: 1px solid rgba(148, 163, 184, 0.35);
    border-radius: 8px;
    background: rgba(15, 23, 42, 0.6);
    color: #f1f5f9;
    padding: 0.35rem 0.7rem;
    font-size: 0.75rem;
    font-weight: 600;
    cursor: pointer;
    transition: background 0.2s ease, border-color 0.2s ease;
}

.operator__stage-toggle[data-state="off"] {
    background: rgba(15, 23, 42, 0.25);
    color: rgba(226, 232, 240, 0.75);
    border-color: rgba(148, 163, 184, 0.25);
}

.operator__stage-toggle:disabled {
    opacity: 0.55;
    cursor: not-allowed;
}

.operator__stage-preview-panel {
    width: 180px;
    min-height: 70px;
    display: flex;
    align-items: center;
    justify-content: center;
    text-align: center;
    font-size: 0.95rem;
    line-height: 1.3;
    padding: 0.35rem 0.5rem;
    background: rgba(15, 23, 42, 0.82);
    border: 1px solid rgba(148, 163, 184, 0.3);
    border-radius: 10px;
}

.operator__stage-preview-panel--current {
    background: rgba(59, 124, 255, 0.28);
    font-weight: 600;
}

.operator__stage-preview-panel--next {
    min-height: 3.5rem;
    font-size: 0.82rem;
    padding: 0.45rem 0.6rem;
}

.operator__clear-button {
    position: absolute;
    top: -0.45rem;
    right: -0.45rem;
    width: 2.1rem;
    height: 2.1rem;
    border-radius: 999px;
    border: 1px solid rgba(148, 163, 184, 0.45);
    background: rgba(15, 23, 42, 0.92);
    color: rgba(226, 232, 240, 0.92);
    font-size: 1.1rem;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    transition: background 0.2s ease, transform 0.2s ease;
}

.operator__clear-button:hover {
    background: rgba(59, 124, 255, 0.6);
    transform: translateY(-1px);
}

.operator__clear-button[disabled] {
    opacity: 0.45;
    cursor: default;
    transform: none;
}

.operator__mode-toggle {
    display: inline-flex;
    flex-direction: column;
    align-items: stretch;
    gap: 0.4rem;
    background: rgba(15, 23, 42, 0.12);
    padding: 0.45rem 0.5rem;
    border-radius: 18px;
}

.operator__mode-toggle button {
    border: none;
    background: transparent;
    color: rgba(226, 232, 240, 0.75);
    padding: 0.35rem 1.1rem;
    border-radius: 12px;
    cursor: pointer;
    transition: background 0.2s ease, color 0.2s ease;
    text-transform: uppercase;
    font-size: 0.75rem;
    letter-spacing: 0.08em;
}

.operator__mode-toggle button[data-active="true"] {
    background: rgba(59, 124, 255, 0.25);
    color: #ffffff;
}

.operator__slides-add {
    border: none;
    border-radius: 8px;
    padding: 0.35rem 0.75rem;
    background: var(--operator-accent);
    color: #ffffff;
    font-weight: 600;
    cursor: pointer;
    box-shadow: 0 10px 18px rgba(59, 124, 255, 0.28);
    transition: background 0.2s ease;
}

.operator__slides-add:hover {
    background: var(--operator-accent-dark);
}

.operator__group-count--static {
    cursor: default;
    border: 1px solid rgba(59, 124, 255, 0.2);
}

.operator__group-count--static:hover {
    background: rgba(59, 124, 255, 0.16);
    color: var(--operator-accent-dark);
}

.operator__slides {
    flex: 1;
    overflow-y: auto;
    padding: 0.35rem;
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 0.9rem;
    min-height: 0;
}

.operator__slide-card {
    background: #ffffff;
    border-radius: 12px;
    border: 1px solid rgba(15, 23, 42, 0.08);
    padding: 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    box-shadow: var(--shadow-inner);
    transition: border-color 0.2s ease, box-shadow 0.2s ease;
}

.operator__slide-card.is-active {
    border-color: rgba(59, 124, 255, 0.6);
    box-shadow: 0 0 0 3px rgba(59, 124, 255, 0.18);
}

.operator__slide-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
}

.operator__slide-header-left {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
}

.operator__slide-handle {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.75rem;
    height: 1.75rem;
    border-radius: 0.6rem;
    border: 1px solid rgba(15, 23, 42, 0.12);
    background: rgba(15, 23, 42, 0.04);
    color: var(--operator-muted);
    font-size: 0.95rem;
    cursor: grab;
    transition: background 0.2s ease, border-color 0.2s ease, color 0.2s ease;
}

.operator__slide-handle:hover {
    background: rgba(59, 124, 255, 0.12);
    border-color: rgba(59, 124, 255, 0.35);
    color: var(--operator-accent-dark);
}

.operator__slide-handle:active {
    cursor: grabbing;
    background: rgba(59, 124, 255, 0.2);
}

.operator__slide-index {
    font-size: 0.75rem;
    color: var(--operator-muted);
    font-weight: 500;
}

.operator__slide-warning-dot {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    margin-left: 0.35rem;
    font-size: 0.7rem;
    color: #fb923c;
}

.operator__slide-controls {
    display: inline-flex;
    gap: 0.35rem;
}

.operator__slide-controls button {
    border: none;
    background: rgba(15, 23, 42, 0.06);
    color: var(--operator-muted);
    padding: 0.35rem 0.55rem;
    border-radius: 8px;
    cursor: pointer;
    font-size: 0.75rem;
}

.operator__slide-bodies {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    min-height: 9.5rem;
}

.operator__slide-text {
    white-space: pre-wrap;
    line-height: 1.45;
    text-align: center;
    padding: 0.35rem 0.5rem;
}

.operator__slide-overflow {
    color: #ef4444;
    font-weight: 600;
}

.operator__slide-overflow[data-overflow-line="true"] {
    display: inline;
}

.operator__slide-text--main {
    font-weight: 600;
    font-size: 1rem;
    color: #0f172a;
}

.operator__slide-text--translation {
    color: #1d4ed8;
    font-style: italic;
}

.operator__slide-text--stage {
    color: #0f766e;
    font-family: 'IBM Plex Mono', 'SFMono-Regular', Menlo, Monaco, Consolas, 'Liberation Mono', 'Courier New', monospace;
    font-size: 0.95rem;
}

.operator__slide-group {
    font-size: 0.68rem;
    color: var(--operator-muted);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    text-align: center;
    margin-top: auto;
    min-height: 1rem;
    display: flex;
    align-items: flex-end;
    justify-content: center;
}

.operator__slide-group[data-hidden="true"] {
    visibility: hidden;
}

.operator__slide-text.is-warning {
    color: #dc2626;
}

.operator__slide-card[data-warning="true"] {
    box-shadow: 0 0 0 2px rgba(220, 38, 38, 0.12);
}

.operator__slide-warning {
    font-size: 0.75rem;
    color: #dc2626;
    text-align: center;
    margin-top: -0.1rem;
    display: none;
}

.operator__slide-warning[data-visible="true"] {
    display: block;
}

.operator__slide-editor {
    display: flex;
    flex-direction: column;
    gap: 0.65rem;
}

.operator__slide-editor label {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    font-size: 0.8rem;
    color: var(--operator-muted);
}

.operator__slide-editor textarea,
.operator__slide-editor input {
    border-radius: 8px;
    border: 1px solid rgba(15, 23, 42, 0.16);
    padding: 0.4rem 0.55rem;
    font-family: inherit;
    font-size: 0.9rem;
    width: min(100%, calc(var(--operator-line-limit-ch, 32) * 1ch + 1.75rem));
    margin-inline: auto;
}

.operator__slide-editor input::placeholder {
    font-style: italic;
    color: rgba(15, 23, 42, 0.45);
}

.operator__slide-editor textarea {
    line-height: var(--operator-line-line-height, 1.35);
    min-height: calc(var(--operator-line-line-height, 1.35) * 2em + 0.6rem);
    max-height: calc(var(--operator-line-line-height, 1.35) * 2em + 0.6rem);
    height: calc(var(--operator-line-line-height, 1.35) * 2em + 0.6rem);
    overflow-y: auto;
    resize: none;
}



body.operator[data-mode="edit"] .operator__slide-editor textarea,
body.operator[data-mode="edit"] .operator__slide-editor input {
    text-align: center;
}

.operator__slide-editor textarea[data-warning="true"] {
    border-color: #dc2626;
    background: #fef2f2;
}

body.operator[data-mode="live"] .operator__slide-editor {
    display: none;
}

body.operator[data-mode="edit"] .operator__slide-text {
    display: none;
}

body.operator[data-mode="live"] .operator__slide-controls {
    display: none;
}

body.operator[data-mode="edit"] .operator__slide-group {
    display: none;
}

.operator__slide-group {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    font-size: 0.75rem;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    background: rgba(59, 124, 255, 0.16);
    color: var(--operator-accent-dark);
    border-radius: 999px;
    padding: 0.15rem 0.6rem;
    align-self: center;
}

.operator__slide-group.is-inherited {
    background: rgba(15, 23, 42, 0.1);
    color: var(--operator-muted);
}

.operator__panel {
    position: absolute;
    inset: 0;
    background: var(--operator-bg);
    display: none;
    padding: 1.5rem;
}

body.operator[data-view="worship"] [data-view-panel="worship"] {
    display: flex;
}

body.operator[data-view="bible"] [data-view-panel="bible"],
body.operator[data-view="timers"] [data-view-panel="timers"] {
    display: block;
}

body.operator[data-view="settings"] [data-view-panel="settings"] {
    display: block;
}

.operator__panel--settings {
    padding: 0;
}

.operator__settings-frame {
    width: 100%;
    height: 100%;
    border: none;
    border-radius: var(--operator-radius);
    box-shadow: var(--shadow-soft);
    background: #ffffff;
}

.operator__panel--bible iframe {
    width: 100%;
    height: 100%;
    border: none;
    border-radius: var(--operator-radius);
    background: #ffffff;
    box-shadow: var(--shadow-soft);
}

.operator__timers {
    display: flex;
    flex-wrap: wrap;
    gap: 1rem;
    margin-bottom: 1.25rem;
}

.operator__timer-card {
    flex: 1 1 220px;
    background: var(--operator-panel);
    border-radius: var(--operator-radius);
    padding: 1rem 1.2rem;
    box-shadow: var(--shadow-soft);
}

.operator__timer-state {
    display: inline-block;
    font-size: 0.75rem;
    color: var(--operator-muted);
    margin-left: 0.5rem;
    padding: 0.125rem 0.5rem;
    border-radius: 999px;
    background: rgba(59, 124, 255, 0.12);
}

.operator__timer-primary {
    margin: 0.35rem 0 0.1rem;
    font-size: 1.75rem;
    font-variant-numeric: tabular-nums;
}

.operator__timer-actions {
    display: flex;
    gap: 1.5rem;
}

.operator__timer-group {
    background: var(--operator-panel);
    border-radius: var(--operator-radius);
    padding: 1rem;
    flex: 1 1 240px;
    box-shadow: var(--shadow-soft);
}

.operator__timer-field {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    margin-bottom: 0.75rem;
}

.operator__timer-field input {
    border-radius: 8px;
    border: 1px solid rgba(15, 23, 42, 0.12);
    padding: 0.5rem 0.6rem;
    font-size: 0.9rem;
    max-width: 160px;
}

.operator__timer-help {
    margin: -0.35rem 0 0.85rem;
    font-size: 0.75rem;
    color: var(--operator-muted);
}

.operator__timer-buttons {
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
}

.operator__timer-buttons button {
    flex: 1;
    border-radius: 8px;
    border: none;
    background: rgba(59, 124, 255, 0.1);
    color: var(--operator-accent-dark);
    padding: 0.45rem 0.5rem;
    cursor: pointer;
}

.operator__timer-links {
    display: flex;
    gap: 0.5rem;
    margin-top: 0.75rem;
    flex-wrap: wrap;
}

.operator__timer-links button {
    flex: 1;
    border-radius: 8px;
    border: 1px solid rgba(59, 124, 255, 0.4);
    background: rgba(59, 124, 255, 0.08);
    color: var(--operator-accent-dark);
    padding: 0.45rem 0.5rem;
    cursor: pointer;
}

.operator__toast {
    position: fixed;
    bottom: 1.5rem;
    right: 1.5rem;
    background: var(--operator-text);
    color: #ffffff;
    padding: 0.75rem 1rem;
    border-radius: 10px;
    box-shadow: var(--shadow-soft);
    opacity: 0;
    transform: translateY(8px);
    transition: opacity 0.2s ease, transform 0.2s ease;
    pointer-events: none;
}

.operator__toast[data-visible="true"] {
    opacity: 1;
    transform: translateY(0);
}

.operator__library-modal,
.operator__playlist-modal {
    position: fixed;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(15, 23, 42, 0.65);
    opacity: 0;
    pointer-events: none;
    transition: opacity 0.2s ease;
    padding: 1.5rem;
    z-index: 1200;
}

.operator__library-modal[data-open="true"],
.operator__playlist-modal[data-open="true"] {
    opacity: 1;
    pointer-events: auto;
}

.operator__library-modal-panel,
.operator__playlist-modal-panel {
    width: min(520px, 90vw);
    max-height: 80vh;
    background: var(--operator-panel);
    border-radius: var(--operator-radius);
    border: 1px solid var(--operator-border);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    box-shadow: var(--shadow-elevated);
}

.operator__library-modal-header,
.operator__playlist-modal-header {
    padding: 1rem 1.25rem;
    display: flex;
    justify-content: space-between;
    align-items: center;
    border-bottom: 1px solid var(--operator-border);
}

.operator__library-modal-header h3,
.operator__playlist-modal-header h3 {
    margin: 0;
    font-size: 1.05rem;
}

.operator__library-modal-close,
.operator__playlist-modal-close {
    border: none;
    background: transparent;
    color: var(--operator-muted);
    font-size: 1.3rem;
    cursor: pointer;
}

.operator__library-modal-body,
.operator__playlist-modal-body {
    padding: 1rem 1.25rem;
    overflow-y: auto;
}

.operator__library-edit {
    position: fixed;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(15, 23, 42, 0.65);
    opacity: 0;
    pointer-events: none;
    transition: opacity 0.25s ease;
    padding: 1.5rem;
    z-index: 1300;
}

.operator__library-edit[data-open="true"] {
    opacity: 1;
    pointer-events: auto;
}

.operator__library-edit-panel {
    width: min(420px, 92vw);
    background: var(--operator-panel);
    border-radius: var(--operator-radius);
    border: 1px solid var(--operator-border);
    box-shadow: var(--shadow-elevated);
}

.operator__library-edit-form {
    display: flex;
    flex-direction: column;
    gap: 1.25rem;
    padding: 1.5rem;
}

.operator__library-edit-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
}

.operator__library-edit-header h3 {
    margin: 0;
    font-size: 1.15rem;
}

.operator__library-edit-body {
    display: flex;
    flex-direction: column;
    gap: 1rem;
}

.operator__library-edit-body label {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    font-size: 0.9rem;
    color: var(--operator-muted);
}

.operator__library-edit-body input[type="text"] {
    border-radius: 10px;
    border: 1px solid rgba(15, 23, 42, 0.12);
    padding: 0.6rem 0.7rem;
    font-size: 1rem;
    color: var(--operator-text);
    background: rgba(255, 255, 255, 0.95);
}

.operator__library-edit-body input[type="text"]:focus {
    outline: none;
    border-color: rgba(59, 124, 255, 0.65);
    box-shadow: 0 0 0 3px rgba(59, 124, 255, 0.15);
}

.operator__library-edit-favorite {
    flex-direction: row;
    align-items: center;
    gap: 0.6rem;
    cursor: pointer;
    color: var(--operator-text);
}

.operator__library-edit-favorite input {
    width: 1.15rem;
    height: 1.15rem;
}

.operator__library-edit-body select {
    border-radius: 10px;
    border: 1px solid rgba(15, 23, 42, 0.12);
    padding: 0.55rem 0.7rem;
    font-size: 1rem;
    color: var(--operator-text);
    background: rgba(255, 255, 255, 0.95);
}

.operator__library-edit-body select:focus {
    outline: none;
    border-color: rgba(59, 124, 255, 0.65);
    box-shadow: 0 0 0 3px rgba(59, 124, 255, 0.15);
}

.operator__library-edit-footer {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 1rem;
}

.operator__library-edit[data-mode="create"] [data-role="library-edit-delete"] {
    display: none;
}

.operator__library-edit-delete {
    border: 1px solid rgba(239, 68, 68, 0.4);
    background: rgba(239, 68, 68, 0.12);
    color: rgb(239, 68, 68);
    border-radius: 8px;
    padding: 0.5rem 0.85rem;
    cursor: pointer;
}

.operator__library-edit-delete:hover {
    background: rgba(239, 68, 68, 0.22);
}

.operator__library-edit-actions {
    display: flex;
    gap: 0.75rem;
}

.operator__library-edit-actions button {
    border: none;
    border-radius: 8px;
    padding: 0.5rem 0.85rem;
    font-weight: 600;
    cursor: pointer;
}

.operator__library-edit-actions [data-role="library-edit-cancel"] {
    background: rgba(148, 163, 184, 0.18);
    color: var(--operator-muted);
}

.operator__library-edit-actions [data-role="library-edit-save"] {
    background: rgba(59, 124, 255, 0.16);
    color: var(--operator-accent-dark);
}

.operator__library-edit-form[data-submitting="true"] button {
    pointer-events: none;
    opacity: 0.6;
}

.operator__presentation-list .empty,
.operator__slides .empty {
    color: var(--operator-muted);
    font-size: 0.9rem;
}


.operator__catalog {
    --catalog-top-size: 320px;
    flex: 0 0 320px;
    display: flex;
    flex-direction: column;
    background: var(--operator-panel);
    border: 1px solid var(--operator-border);
    border-radius: var(--operator-radius);
    padding: 1rem 1.25rem;
    gap: 0;
    max-height: calc(100vh - 5.5rem);
    position: sticky;
    top: calc(4.75rem);
}

.operator__catalog-top {
    display: flex;
    flex-direction: column;
    gap: 1rem;
    overflow-y: auto;
    padding-right: 0.35rem;
    flex: 0 0 var(--catalog-top-size);
    min-height: 0;
}

.operator__catalog-resizer {
    flex: 0 0 10px;
    cursor: row-resize;
    margin: 0 -1.25rem;
    background: linear-gradient(90deg, rgba(15, 23, 42, 0) 0%, rgba(15, 23, 42, 0.12) 50%, rgba(15, 23, 42, 0) 100%);
    border-radius: 999px;
}

.operator__catalog-bottom {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
    border-top: 1px solid rgba(15, 23, 42, 0.08);
    padding-top: 0.85rem;
    overflow-y: auto;
    padding-right: 0.35rem;
    margin-top: 0.85rem;
}

.operator__presentations-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
    margin-bottom: 0.75rem;
}

.operator__presentations-heading h2 {
    margin: 0;
    font-size: 0.95rem;
}

.operator__presentations-count {
    font-size: 0.75rem;
    color: var(--operator-muted);
}

.operator__presentations-actions {
    display: inline-flex;
    gap: 0.45rem;
}

.operator__presentations-actions button {
    font-size: 0.75rem;
    padding: 0.3rem 0.75rem;
    border-radius: 8px;
    border: 1px solid rgba(59, 124, 255, 0.3);
    background: rgba(59, 124, 255, 0.12);
    color: var(--operator-accent-dark);
    cursor: pointer;
    transition: background 0.2s ease, color 0.2s ease, border 0.2s ease;
}

.operator__presentations-actions button:hover:enabled {
    background: rgba(59, 124, 255, 0.24);
    color: #ffffff;
}

.operator__presentations-actions button:disabled {
    cursor: not-allowed;
    opacity: 0.45;
    border-color: rgba(107, 111, 123, 0.24);
    background: rgba(107, 111, 123, 0.12);
    color: var(--operator-muted);
}

.operator__slides-column {
    flex: 1;
    display: flex;
    flex-direction: column;
    background: var(--operator-panel);
    border: 1px solid var(--operator-border);
    border-radius: var(--operator-radius);
    padding: 1.2rem 1.4rem;
    gap: 1rem;
    min-height: 0;
}

.operator__slides-heading {
    display: flex;
    align-items: stretch;
    justify-content: space-between;
    gap: 1rem;
    width: 100%;
}

.operator__slides {
    flex: 1;
    overflow-y: auto;
    padding: 0.35rem;
    display: grid;
    grid-template-columns: repeat(var(--operator-slide-columns, 3), minmax(0, 1fr));
    gap: 0.9rem;
    align-content: start;
}

.operator__slides[data-size="compact"] {
    --operator-slide-columns: 4;
}

.operator__slides[data-size="medium"] {
    --operator-slide-columns: 3;
}

.operator__slides[data-size="expanded"] {
    --operator-slide-columns: 2;
}

.operator__slide-card {
    padding: 0.85rem;
}

.operator__list-button {
    font-size: 0.85rem;
    padding: 0.5rem 0.7rem;
}

.operator__presentation-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
}

.operator__presentation-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    border: 1px solid rgba(15, 23, 42, 0.08);
    border-radius: 10px;
    padding: 0.55rem 0.75rem;
    background: #ffffff;
    font-size: 0.84rem;
    cursor: pointer;
    transition: border 0.2s ease, box-shadow 0.2s ease, background 0.2s ease;
}

.operator__presentation-item[data-drop-position] {
    position: relative;
}

.operator__presentation-item[data-drop-position="before"]::before,
.operator__presentation-item[data-drop-position="after"]::after {
    content: '';
    position: absolute;
    left: 10px;
    right: 10px;
    border-top: 3px solid rgba(59, 124, 255, 0.85);
    border-radius: 999px;
    pointer-events: none;
}

.operator__presentation-item[data-drop-position="before"]::before {
    top: -6px;
}

.operator__presentation-item[data-drop-position="after"]::after {
    bottom: -6px;
}

.operator__presentation-item.is-active {
    border-color: rgba(59, 124, 255, 0.55);
    box-shadow: 0 0 0 2px rgba(59, 124, 255, 0.2);
}

.operator__presentation-item.is-stage-active {
    background: rgba(59, 124, 255, 0.1);
}

.operator__presentation-item[data-type="separator"] {
    background: rgba(15, 23, 42, 0.06);
    border-style: dashed;
    font-style: italic;
    cursor: default;
}

.operator__presentation-item[data-type="separator"] span {
    opacity: 0.85;
}
.settings__form--osc {
    margin-bottom: 1.5rem;
}

.settings__osc-status {
    border-top: 1px solid rgba(15, 23, 42, 0.08);
    padding-top: 1rem;
}

.settings__status-line {
    display: flex;
    align-items: center;
    margin-bottom: 0.75rem;
}

.settings__status-list {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(160px, 1fr));
    gap: 0.8rem 1.2rem;
    margin: 0 0 0.75rem 0;
    padding: 0;
}

.settings__status-list dt {
    font-size: 0.75rem;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: rgba(255, 255, 255, 0.65);
    margin: 0 0 0.2rem 0;
}

.settings__status-list dd {
    margin: 0;
    font-size: 0.95rem;
    font-weight: 500;
}

"#;

const TABLET_STYLES: &str = r#"
body.tablet {
    margin: 0;
    min-height: 100vh;
    display: flex;
    flex-direction: column;
    font-family: "Inter", "Segoe UI", system-ui, sans-serif;
    background: linear-gradient(180deg, #0f172a 0%, #1e293b 100%);
    color: #f8fafc;
}

.tablet-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 1.25rem 1.75rem;
    background: rgba(12, 20, 35, 0.9);
    box-shadow: 0 14px 32px rgba(15, 23, 42, 0.55);
}

.tablet-header h1 {
    margin: 0;
    font-size: 1.35rem;
}

.tablet-header__subtitle {
    margin: 0.4rem 0 0;
    font-size: 0.9rem;
    color: #cbd5f5;
}

.tablet-header__modes {
    display: inline-flex;
    gap: 0.4rem;
    background: rgba(148, 163, 184, 0.18);
    padding: 0.25rem;
    border-radius: 999px;
}

.tablet-header__modes button {
    border: none;
    border-radius: 999px;
    padding: 0.45rem 0.9rem;
    background: transparent;
    color: inherit;
    cursor: pointer;
    font-size: 0.85rem;
}

.tablet-header__modes button[data-active="true"] {
    background: #38bdf8;
    color: #0f172a;
    box-shadow: 0 10px 22px rgba(56, 189, 248, 0.4);
}

.tablet-layout {
    flex: 1;
    display: flex;
    overflow: hidden;
}

.tablet-sidebar {
    width: 260px;
    padding: 1.25rem;
    background: rgba(15, 23, 42, 0.72);
    border-right: 1px solid rgba(148, 163, 184, 0.25);
    display: flex;
    flex-direction: column;
    gap: 1.1rem;
}

.tablet-main {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
}

.tablet-main__header {
    padding: 1.2rem 1.6rem 0.75rem;
    display: flex;
    align-items: center;
    justify-content: space-between;
}

.tablet-main__header h2 {
    margin: 0;
    font-size: 1.05rem;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: #a5b4fc;
}

.tablet-panel h2 {
    margin: 0 0 0.8rem;
    font-size: 0.95rem;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: #94a3b8;
}

.tablet-list {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
}

.tablet-list-item {
    display: flex;
    align-items: center;
    gap: 0.45rem;
}

.tablet-list-actions {
    display: flex;
    gap: 0.3rem;
}

.tablet-list-action {
    border: 1px solid transparent;
    border-radius: 10px;
    background: rgba(148, 163, 184, 0.22);
    color: #e2e8f0;
    font-size: 0.78rem;
    padding: 0.35rem 0.55rem;
    cursor: pointer;
    transition: background 0.2s ease, color 0.2s ease;
}

.tablet-list-action:hover {
    background: rgba(56, 189, 248, 0.28);
    color: #0f172a;
}

.tablet-list-action--danger {
    background: rgba(239, 68, 68, 0.24);
    color: #fecaca;
}

.tablet-list-action--danger:hover {
    background: rgba(239, 68, 68, 0.38);
    color: #0f172a;
}

.tablet-button {
    border: none;
    text-align: left;
    background: rgba(148, 163, 184, 0.18);
    color: #f8fafc;
    padding: 0.55rem 0.75rem;
    border-radius: 10px;
    font-size: 0.95rem;
    cursor: pointer;
    transition: transform 0.2s ease, background 0.2s ease;
    display: flex;
    align-items: center;
    gap: 0.55rem;
}

.tablet-button:hover {
    transform: translateY(-1px);
}

.tablet-button[data-active="true"] {
    background: rgba(56, 189, 248, 0.3);
    box-shadow: 0 12px 26px rgba(56, 189, 248, 0.35);
}

.tablet-button__label {
    flex: 1;
}

.tablet-button__meta {
    font-size: 0.78rem;
    color: #cbd5f5;
    background: rgba(56, 189, 248, 0.25);
    border-radius: 999px;
    padding: 0.05rem 0.45rem;
}

.tablet-slides {
    flex: 1;
    padding: 1.5rem;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: 1.25rem;
    overflow-y: auto;
}

.tablet-slides__empty {
    color: #94a3b8;
    font-size: 0.95rem;
}

.tablet-slide {
    background: rgba(15, 23, 42, 0.8);
    border-radius: 16px;
    padding: 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    border: 1px solid transparent;
    cursor: pointer;
    transition: border-color 0.2s ease, box-shadow 0.2s ease, transform 0.2s ease;
}

.tablet-slide:hover {
    transform: translateY(-2px);
}

.tablet-slide.is-active {
    border-color: rgba(56, 189, 248, 0.8);
    box-shadow: 0 14px 30px rgba(56, 189, 248, 0.38);
}

.tablet-slide header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    color: #cbd5f5;
    font-size: 0.85rem;
}

.tablet-slide__group {
    background: rgba(56, 189, 248, 0.18);
    padding: 0.1rem 0.45rem;
    border-radius: 999px;
}

.tablet-slide__body p {
    margin: 0;
    white-space: pre-wrap;
    line-height: 1.45;
}

.tablet-slide__translation {
    color: #93c5fd;
    font-size: 0.9rem;
}

.tablet-editor {
    position: fixed;
    inset: 0;
    background: rgba(12, 20, 35, 0.7);
    display: flex;
    align-items: center;
    justify-content: center;
    opacity: 0;
    pointer-events: none;
    transition: opacity 0.2s ease;
}

.tablet-editor[data-open="true"] {
    opacity: 1;
    pointer-events: auto;
}

.tablet-editor__content {
    background: #0f172a;
    border-radius: 18px;
    width: min(520px, 92vw);
    padding: 1.5rem;
    display: flex;
    flex-direction: column;
    gap: 1rem;
    box-shadow: 0 30px 60px rgba(15, 23, 42, 0.6);
}

.tablet-editor__content textarea,
.tablet-editor__content input {
    border-radius: 10px;
    border: 1px solid rgba(148, 163, 184, 0.2);
    padding: 0.7rem 0.8rem;
    font-family: inherit;
    font-size: 0.95rem;
    background: rgba(15, 23, 42, 0.6);
    color: #f8fafc;
}

.tablet-editor__content textarea {
    min-height: 110px;
    resize: vertical;
}

.tablet-editor__error {
    margin: 0;
    color: #fca5a5;
    font-size: 0.85rem;
    display: none;
}

.tablet-editor__error[data-visible="true"] {
    display: block;
}

.tablet-editor__actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.75rem;
}

.tablet-editor__actions button {
    border: none;
    border-radius: 10px;
    padding: 0.5rem 1.1rem;
    font-size: 0.9rem;
    cursor: pointer;
}

.tablet-editor__actions button[data-role="editor-save"] {
    background: #38bdf8;
    color: #0f172a;
}

.tablet-editor__actions button[data-role="editor-cancel"] {
    background: rgba(148, 163, 184, 0.28);
    color: #f8fafc;
}

.tablet-toast {
    position: fixed;
    bottom: 1.5rem;
    right: 1.5rem;
    background: rgba(15, 23, 42, 0.88);
    color: #f8fafc;
    padding: 0.7rem 1rem;
    border-radius: 12px;
    box-shadow: 0 12px 26px rgba(15, 23, 42, 0.55);
    opacity: 0;
    transform: translateY(8px);
    transition: opacity 0.2s ease, transform 0.2s ease;
    pointer-events: none;
}

.tablet-toast[data-visible="true"] {
    opacity: 1;
    transform: translateY(0);
}
"#;

const BIBLE_STYLES: &str = r#"
body.bible {
    margin: 0;
    min-height: 100vh;
    display: flex;
    flex-direction: column;
    font-family: "Inter", "Segoe UI", system-ui, sans-serif;
    background: #f8fafc;
    color: #0f172a;
}

.bible__header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 1.5rem 2rem;
    background: #0f172a;
    color: #f8fafc;
    box-shadow: 0 14px 24px rgba(15, 23, 42, 0.25);
}

.bible__header h1 {
    margin: 0;
    font-size: 1.4rem;
}

.bible__header p {
    margin: 0.4rem 0 0;
    color: #cbd5f5;
}

.bible__clear {
    border: none;
    background: rgba(99, 102, 241, 0.2);
    color: #eef2ff;
    padding: 0.6rem 1.2rem;
    border-radius: 10px;
    cursor: pointer;
    font-size: 0.95rem;
}

.bible__search {
    display: grid;
    gap: 1rem;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    padding: 1.5rem 2rem;
    background: #eef2ff;
}

.bible__search label {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    font-size: 0.85rem;
    color: #4c51bf;
}

.bible__search select,
.bible__search input {
    border-radius: 10px;
    border: 1px solid rgba(79, 70, 229, 0.35);
    padding: 0.65rem 0.75rem;
    font-size: 0.95rem;
    font-family: inherit;
    background: #ffffff;
}

.bible__search-button {
    align-self: end;
    border: none;
    border-radius: 10px;
    background: #4f46e5;
    color: #eef2ff;
    padding: 0.65rem 1.25rem;
    font-size: 0.95rem;
    cursor: pointer;
    box-shadow: 0 12px 24px rgba(79, 70, 229, 0.3);
}

.bible__active {
    padding: 1.5rem 2rem;
}

.bible__active-card {
    background: #ffffff;
    border-radius: 16px;
    padding: 1.25rem 1.4rem;
    box-shadow: 0 14px 30px rgba(15, 23, 42, 0.12);
}

.bible__active-card--empty {
    background: rgba(248, 250, 252, 0.6);
    border: 1px dashed rgba(148, 163, 184, 0.45);
    box-shadow: none;
}

.bible__active-card header {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 1rem;
    margin-bottom: 0.8rem;
}

.bible__active-translation {
    font-size: 0.85rem;
    color: #6366f1;
}

.bible__active-card p {
    margin: 0;
    white-space: pre-wrap;
    line-height: 1.6;
}

.bible__results {
    padding: 0 2rem 2.5rem;
    display: grid;
    gap: 1rem;
}

.bible__result {
    background: #ffffff;
    border-radius: 14px;
    padding: 1rem 1.2rem;
    border: 1px solid rgba(148, 163, 184, 0.25);
    box-shadow: 0 8px 20px rgba(15, 23, 42, 0.08);
}

.bible__result header {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 1rem;
    margin-bottom: 0.6rem;
}

.bible__result-actions button {
    border: none;
    background: #38bdf8;
    color: #0f172a;
    border-radius: 8px;
    padding: 0.45rem 0.85rem;
    font-size: 0.85rem;
    cursor: pointer;
}

.bible__result p {
    margin: 0;
    white-space: pre-wrap;
    line-height: 1.5;
}

.bible__empty {
    color: #64748b;
    font-size: 0.95rem;
}

.bible__toast {
    position: fixed;
    bottom: 1.5rem;
    right: 1.5rem;
    background: #0f172a;
    color: #f8fafc;
    padding: 0.7rem 1rem;
    border-radius: 10px;
    box-shadow: 0 12px 24px rgba(15, 23, 42, 0.28);
    opacity: 0;
    transform: translateY(6px);
    transition: opacity 0.2s ease, transform 0.2s ease;
    pointer-events: none;
}

.bible__toast[data-visible="true"] {
    opacity: 1;
    transform: translateY(0);
}

@media (max-width: 720px) {
    .bible__header {
        flex-direction: column;
        align-items: flex-start;
        gap: 0.75rem;
    }

    .bible__search {
        grid-template-columns: 1fr;
    }
}
"#;

const SETTINGS_STYLES: &str = r#"
:root {
    color-scheme: light;
}

body.settings {
    margin: 0;
    background: #f8fafc;
    color: #0f172a;
    font-family: 'Inter', system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
}

.settings__header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 24px 40px;
    background: #ffffff;
    border-bottom: 1px solid #e2e8f0;
}

.settings__header-title h1 {
    margin: 0;
    font-size: 1.75rem;
    font-weight: 600;
}

.settings__header-title p {
    margin: 8px 0 0;
    color: #475569;
}

.settings__header-nav .settings__link {
    text-decoration: none;
    color: #3b82f6;
    font-weight: 600;
}

.settings__header-nav .settings__link:hover {
    text-decoration: underline;
}

.settings__main {
    max-width: 1000px;
    margin: 32px auto;
    padding: 0 32px 48px;
    display: flex;
    flex-direction: column;
    gap: 32px;
}

.settings__card {
    background: #ffffff;
    border-radius: 20px;
    box-shadow: 0 12px 40px rgba(15, 23, 42, 0.08);
    padding: 32px;
    display: flex;
    flex-direction: column;
    gap: 24px;
}

.settings__card-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 24px;
}

.settings__card-header h2 {
    margin: 0;
    font-size: 1.5rem;
    font-weight: 600;
}

.settings__card-header p {
    margin: 8px 0 0;
    color: #475569;
    max-width: 460px;
}

.settings__badge-group {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 4px;
}

.settings__badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 48px;
    padding: 6px 12px;
    border-radius: 999px;
    background: #eef2ff;
    color: #312e81;
    font-weight: 600;
    font-size: 0.95rem;
}

.settings__badge-label {
    font-size: 0.8rem;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: #64748b;
}

.settings__form {
    display: flex;
    flex-direction: column;
    gap: 20px;
    background: #f8fafc;
    border: 1px solid #e2e8f0;
    border-radius: 16px;
    padding: 24px;
}

.settings__form-header h3 {
    margin: 0;
    font-size: 1.2rem;
    font-weight: 600;
}

.settings__form-header p {
    margin: 6px 0 0;
    color: #64748b;
}

.settings__form-row {
    display: flex;
    flex-wrap: wrap;
    gap: 16px;
}

.settings__form-row--single {
    justify-content: flex-start;
}

.settings__form-row label {
    display: flex;
    flex-direction: column;
    gap: 8px;
    flex: 1 1 220px;
    font-weight: 600;
    color: #0f172a;
}

.settings__form-row label span {
    font-size: 0.9rem;
}

.settings__form-row input[type="text"],
.settings__form-row input[type="number"] {
    padding: 10px 12px;
    border: 1px solid #cbd5f5;
    border-radius: 10px;
    font-size: 1rem;
    background: #ffffff;
    color: #0f172a;
    transition: border-color 0.2s ease, box-shadow 0.2s ease;
}

.settings__form-row input:focus {
    outline: none;
    border-color: #6366f1;
    box-shadow: 0 0 0 3px rgba(99, 102, 241, 0.12);
}

.settings__form-control--small {
    flex: 0 1 120px;
}

.settings__form-checkbox {
    flex: 0 1 auto;
    flex-direction: row;
    align-items: center;
    gap: 10px;
    padding-top: 28px;
    font-weight: 600;
}

.settings__form-checkbox--block {
    padding-top: 0;
}

.settings__form-checkbox input {
    width: 18px;
    height: 18px;
}

.settings__form-actions {
    display: flex;
    gap: 12px;
    align-items: center;
}

.settings__form-checkbox--inline {
    flex: 0 0 auto;
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
    font-weight: 600;
    color: #0f172a;
}

.settings__form-checkbox--inline input {
    width: 18px;
    height: 18px;
}

.settings__button {
    border: none;
    border-radius: 10px;
    font-size: 0.95rem;
    font-weight: 600;
    padding: 10px 18px;
    cursor: pointer;
    transition: transform 0.15s ease, box-shadow 0.15s ease;
}

.settings__button:disabled {
    opacity: 0.6;
    cursor: wait;
}

.settings__button--primary {
    background: #4f46e5;
    color: #ffffff;
    box-shadow: 0 12px 24px rgba(79, 70, 229, 0.22);
}

.settings__button--primary:hover:not(:disabled) {
    transform: translateY(-1px);
    box-shadow: 0 14px 28px rgba(79, 70, 229, 0.26);
}

.settings__button--ghost {
    background: transparent;
    color: #475569;
    border: 1px solid #cbd5f5;
}

.settings__button--ghost:hover {
    background: #e2e8f0;
}

.settings__button--danger {
    background: #dc2626;
    color: #ffffff;
    box-shadow: 0 10px 24px rgba(220, 38, 38, 0.25);
}

.settings__button--danger:hover {
    transform: translateY(-1px);
}

body.settings[data-mode="create"] [data-role="host-reset"] {
    display: none;
}

.settings__form-status {
    min-height: 1.2rem;
    font-size: 0.9rem;
    margin: 0;
}

.settings__form-status[data-state="error"] {
    color: #dc2626;
}

.settings__form-status[data-state="success"] {
    color: #16a34a;
}

.settings__list {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 16px;
}

.settings__list-item {
    display: flex;
    justify-content: space-between;
    gap: 24px;
    padding: 20px 24px;
    border: 1px solid #e2e8f0;
    border-radius: 16px;
    background: #ffffff;
    box-shadow: 0 10px 24px rgba(15, 23, 42, 0.04);
}

.settings__list-item[data-enabled="false"] {
    opacity: 0.75;
}

.settings__list-primary {
    display: flex;
    flex-direction: column;
    gap: 8px;
}

.settings__list-title {
    display: flex;
    align-items: center;
    gap: 12px;
}

.settings__host-label {
    font-size: 1.1rem;
    font-weight: 600;
}

.settings__status {
    font-size: 0.8rem;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    padding: 4px 10px;
    border-radius: 999px;
}

.settings__status--enabled {
    background: #dcfcef;
    color: #047857;
}

.settings__status--connecting {
    background: #bfdbfe;
    color: #1d4ed8;
}

.settings__status--disabled {
    background: #fee2e2;
    color: #b91c1c;
}

.settings__status--error {
    background: #fee2e2;
    color: #b91c1c;
}

.settings__list-line {
    margin: 0;
    font-family: 'JetBrains Mono', 'Fira Mono', monospace;
    font-size: 0.95rem;
    color: #0f172a;
}

.settings__list-meta {
    margin: 0;
    color: #64748b;
    font-size: 0.85rem;
}

.settings__list-meta--warning {
    color: #b91c1c;
    font-weight: 600;
}

.settings__list-actions {
    display: flex;
    gap: 10px;
    align-items: flex-start;
}

.settings__list-empty {
    padding: 32px;
    border: 1px dashed #cbd5f5;
    border-radius: 16px;
    text-align: center;
    color: #64748b;
    background: #f8fafc;
    font-weight: 500;
}

.settings__toast {
    position: fixed;
    right: 28px;
    bottom: 28px;
    padding: 14px 20px;
    background: #1e293b;
    color: #f8fafc;
    border-radius: 12px;
    box-shadow: 0 18px 40px rgba(15, 23, 42, 0.35);
    opacity: 0;
    pointer-events: none;
    transform: translateY(20px);
    transition: opacity 0.2s ease, transform 0.2s ease;
}

.settings__toast[data-visible="true"] {
    opacity: 1;
    pointer-events: auto;
    transform: translateY(0);
}

.settings__toast[data-state="success"] {
    background: #0f766e;
}

.settings__toast[data-state="error"] {
    background: #b91c1c;
}

body.settings[data-embedded="true"] {
    background: transparent;
}

body.settings[data-embedded="true"] .settings__header {
    display: none;
}

body.settings[data-embedded="true"] .settings__main {
    margin: 0;
    padding: 16px 24px 32px;
}

body.settings[data-embedded="true"] .settings__card {
    box-shadow: none;
}

.settings__legend {
    margin-top: 1.75rem;
    background: rgba(148, 163, 184, 0.08);
    border: 1px solid rgba(148, 163, 184, 0.2);
    border-radius: 14px;
    padding: 1.25rem;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
}

.settings__legend-note {
    margin: 0;
    color: #cbd5f5;
    font-size: 0.85rem;
    line-height: 1.4;
}

.settings__legend h3 {
    margin: 0;
    font-size: 1.05rem;
    font-weight: 600;
}

.settings__legend dl {
    margin: 0;
    display: grid;
    gap: 0.25rem 1.25rem;
    grid-template-columns: minmax(160px, auto) 1fr;
}

.settings__legend dt {
    font-weight: 600;
    color: #cbd5f5;
}

.settings__legend dd {
    margin: 0;
    color: #e2e8f0;
}

@media (max-width: 840px) {
    .settings__card {
        padding: 24px;
    }

    .settings__card-header {
        flex-direction: column;
        align-items: flex-start;
    }

    .settings__badge-group {
        flex-direction: row;
        align-items: center;
        gap: 12px;
    }

    .settings__list-item {
        flex-direction: column;
    }

    .settings__list-actions {
        align-self: flex-end;
    }
}
"#;

const HOME_STYLES: &str = r#"
body.home {
    margin: 0;
    min-height: 100vh;
    font-family: "Inter", "Segoe UI", system-ui, sans-serif;
    background: linear-gradient(180deg, #111827 0%, #1f2937 100%);
    color: #f8fafc;
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding: 4rem 1.5rem;
}

.home__container {
    width: min(960px, 100%);
    display: flex;
    flex-direction: column;
    gap: 2rem;
}

.home__cta-row {
    display: flex;
    justify-content: flex-start;
}

.home__cta-button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0.9rem 1.6rem;
    border-radius: 999px;
    background: #3b82f6;
    color: #0f172a;
    font-weight: 600;
    font-size: 1rem;
    text-decoration: none;
    box-shadow: 0 18px 36px rgba(59, 130, 246, 0.35);
    transition: transform 0.2s ease, box-shadow 0.2s ease;
}

.home__cta-button:hover {
    transform: translateY(-2px);
    box-shadow: 0 24px 42px rgba(59, 130, 246, 0.45);
}

.home__header h1 {
    margin: 0 0 0.5rem;
    font-size: 2rem;
}

.home__header p {
    margin: 0;
    color: #cbd5f5;
}

.home__section h2 {
    margin: 0 0 0.6rem;
    font-size: 1.15rem;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: #93c5fd;
}

.home__links {
    list-style: none;
    display: flex;
    flex-wrap: wrap;
    gap: 1rem;
    margin: 0;
    padding: 0;
}

.home__links a {
    display: inline-flex;
    align-items: center;
    background: rgba(148, 163, 184, 0.18);
    color: #f8fafc;
    padding: 0.75rem 1.2rem;
    border-radius: 12px;
    text-decoration: none;
    font-size: 0.95rem;
    transition: background 0.2s ease, transform 0.2s ease;
}

.home__links a:hover {
    background: rgba(59, 130, 246, 0.3);
    transform: translateY(-2px);
}
"#;

const TIMER_OVERLAY_STYLES: &str = r#"
body.overlay {
    margin: 0;
    min-height: 100vh;
    background: transparent;
    display: flex;
    align-items: center;
    justify-content: center;
    font-family: "Inter", "Segoe UI", system-ui, sans-serif;
    color: #f8fafc;
}

.overlay__timer {
    font-size: 12vw;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-align: center;
    text-shadow: 0 12px 40px rgba(15, 23, 42, 0.55);
    font-variant-numeric: tabular-nums;
    font-feature-settings: 'tnum' 1;
    text-rendering: optimizeLegibility;
    -webkit-font-smoothing: antialiased;
    font-smooth: always;
}

@media (max-width: 720px) {
    .overlay__timer {
        font-size: 18vw;
        letter-spacing: 0.06em;
    }
}
"#;

#[component]
fn TabletDocument(
    library_json: String,
    playlist_json: String,
    stage_json: String,
) -> impl IntoView {
    let library_json_safe = library_json.replace("</script>", r"<\/script>");
    let playlist_json_safe = playlist_json.replace("</script>", r"<\/script>");
    let stage_json_safe = stage_json.replace("</script>", r"<\/script>");
    let script = TABLET_SCRIPT_TEMPLATE
        .replace("__LIBRARIES__", &library_json_safe)
        .replace("__PLAYLISTS__", &playlist_json_safe)
        .replace("__STAGE__", &stage_json_safe);

    view! {
        <html lang="en">
            <head>
                <meta charset="utf-8" />
                <meta name="viewport" content="width=device-width, initial-scale=1.0" />
                <title>"Presenter Tablet"</title>
                <style>{TABLET_STYLES}</style>
            </head>
            <body class="tablet" data-mode="live">
                <header class="tablet-header">
                    <div>
                        <h1>"Presenter Tablet"</h1>
                        <p class="tablet-header__subtitle" data-role="mode-status">
                            "Live mode — tap slides to trigger stage output."
                        </p>
                    </div>
                    <div class="tablet-header__mode">
                        <div class="tablet-header__modes">
                            <button type="button" data-role="mode-toggle" data-mode="live" data-active="true">
                                "Live Mode"
                            </button>
                            <button type="button" data-role="mode-toggle" data-mode="edit" data-active="false">
                                "Edit Mode"
                            </button>
                        </div>
                    </div>
                </header>
                <main class="tablet-layout">
                    <aside class="tablet-sidebar">
                        <section class="tablet-panel">
                            <h2>"Libraries"</h2>
                            <div class="tablet-list" data-role="library-list">
                                <p class="tablet-slides__empty">"Loading libraries…"</p>
                            </div>
                        </section>
                        <section class="tablet-panel">
                            <h2>"Playlists"</h2>
                            <div class="tablet-list" data-role="playlist-list">
                                <p class="tablet-slides__empty">"No playlists configured."</p>
                            </div>
                        </section>
                        <section class="tablet-panel">
                            <h2>"Presentations"</h2>
                            <div class="tablet-list" data-role="presentation-list">
                                <p class="tablet-slides__empty">"Select a library or playlist."</p>
                            </div>
                        </section>
                    </aside>
                    <section class="tablet-main">
                        <header class="tablet-main__header">
                            <h2 data-role="context-title">"Presentations"</h2>
                        </header>
                        <div class="tablet-slides" data-role="slides">
                            <p class="tablet-slides__empty">"Select a presentation to load slides."</p>
                        </div>
                    </section>
                </main>
                <div class="tablet-editor" data-role="editor" data-open="false">
                    <div class="tablet-editor__content">
                        <header>
                            <h2>"Edit Slide"</h2>
                        </header>
                        <label>
                            <span>"Main"</span>
                            <textarea data-role="editor-main" placeholder="Main lyrics or text"></textarea>
                        </label>
                        <label>
                            <span>"Translation"</span>
                            <textarea data-role="editor-translation" placeholder="Translation or secondary language"></textarea>
                        </label>
                        <label>
                            <span>"Stage"</span>
                            <textarea data-role="editor-stage" placeholder="Stage display text"></textarea>
                        </label>
                        <label>
                            <span>"Group"</span>
                            <input data-role="editor-group" type="text" placeholder="Optional group or section name" />
                        </label>
                        <p class="tablet-editor__error" data-role="editor-error"></p>
                        <div class="tablet-editor__actions">
                            <button type="button" data-role="editor-cancel">"Cancel"</button>
                            <button type="button" data-role="editor-save">"Save"</button>
                        </div>
                    </div>
                </div>
                <div class="tablet-toast" data-role="toast" data-visible="false"></div>
                <script>{script}</script>
            </body>
        </html>
    }
}

pub async fn render_timer_overlay(state: &AppState) -> anyhow::Result<Html<String>> {
    let overview = state.timers_overview().await?;
    let initial_seconds = overview.countdown_to_start.seconds_remaining;
    let initial_display = format_seconds_compact(initial_seconds);
    let timers_json = to_string(&overview).unwrap_or_else(|_| "{}".to_string());
    let timers_json = timers_json.replace("</script>", r"<\/script>");

    let script = format!(
        r#"(function() {{
  const initial = {timers_json};
  let overview = initial || {{}};
  let countdown = overview.countdown_to_start || overview.countdownToStart || {{}};
  let remaining = Number(countdown.seconds_remaining ?? countdown.secondsRemaining ?? 0);
  let state = String(countdown.state ?? 'idle').toLowerCase();
  const valueEl = document.getElementById('timer-value');

  const coerceTargetEpoch = (value) => {{
    if (typeof value !== 'string') return null;
    const parsed = Date.parse(value);
    return Number.isNaN(parsed) ? null : parsed;
  }};

  let targetEpochMs = coerceTargetEpoch(
    countdown.target ?? countdown.targetUtc ?? countdown.targetUTC ?? null
  );

  const clampNumber = (value) => (Number.isFinite(value) ? value : 0);
  const format = (seconds) => {{
    const total = Math.max(0, Math.floor(clampNumber(seconds)));
    if (total < 60) {{
      return String(total);
    }}
    const minutes = Math.floor(total / 60);
    const secs = total % 60;
    return `${{String(minutes).padStart(2, '0')}}:${{String(secs).padStart(2, '0')}}`;
  }};

  const remainingFromTarget = () => {{
    if (!Number.isFinite(targetEpochMs)) return null;
    return Math.max(0, Math.round((targetEpochMs - Date.now()) / 1000));
  }};

  const publishState = () => {{
    window.__presenterTimerOverlayState = {{ remaining, state }};
  }};

  const render = () => {{
    if (valueEl) {{
      valueEl.textContent = format(remaining);
    }}
    publishState();
  }};

  const applyOverview = (nextOverview) => {{
    if (!nextOverview) return;
    const nextCountdown =
      nextOverview.countdown_to_start ||
      nextOverview.countdownToStart ||
      {{}};
    const rawSeconds = Number(
      nextCountdown.seconds_remaining ??
        nextCountdown.secondsRemaining ??
        remaining
    );
    if (Number.isFinite(rawSeconds)) {{
      remaining = Math.floor(rawSeconds);
    }}
    if (typeof nextCountdown.state === 'string') {{
      state = nextCountdown.state.toLowerCase();
    }}
    const candidateTarget =
      nextCountdown.target ??
      nextCountdown.targetUtc ??
      nextCountdown.targetUTC ??
      null;
    const parsedTarget = coerceTargetEpoch(candidateTarget);
    if (parsedTarget !== null) {{
      targetEpochMs = parsedTarget;
    }}
    render();
  }};

  applyOverview(overview);

  window.setInterval(() => {{
    if (state === 'running') {{
      const derived = remainingFromTarget();
      if (derived !== null) {{
        remaining = derived;
      }} else {{
        remaining = Math.max(0, remaining - 1);
      }}
      render();
    }}
  }}, 1000);

  const connect = () => {{
    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const socket = new WebSocket(`${{protocol}}//${{window.location.host}}/live/ws`);
    window.__presenterTimerOverlaySocket = socket;

    socket.addEventListener('message', (event) => {{
      try {{
        const data = JSON.parse(event.data);
        if (data.type === 'timers') {{
          overview = data.overview || overview;
          applyOverview(overview);
        }}
      }} catch (error) {{
        console.warn('timer overlay parse error', error);
      }}
    }});

    const scheduleReconnect = () => {{
      window.setTimeout(connect, 1500);
    }};

    socket.addEventListener('close', scheduleReconnect);
    socket.addEventListener('error', () => {{
      try {{ socket.close(); }} catch (_) {{}}
    }});
  }};

  connect();
}})();"#,
        timers_json = timers_json
    );

    let owner = Owner::new_root(None);
    let html = owner.with(|| {
        view! {
            <html lang="en">
                <head>
                    <meta charset="utf-8" />
                    <title>"Presenter Timer Overlay"</title>
                    <style>{TIMER_OVERLAY_STYLES}</style>
                </head>
                <body class="overlay overlay--timer">
                    <div class="overlay__timer" id="timer-value">{initial_display}</div>
                    <script>{script}</script>
                </body>
            </html>
        }
        .into_view()
        .to_html()
    });

    Ok(Html(format!("<!DOCTYPE html>{html}")))
}

pub async fn render_tablet_ui(state: &AppState) -> anyhow::Result<Html<String>> {
    let library_summaries = state.library_summaries(None).await?;
    let playlists = state.playlists().await?;
    let stage_snapshot = state.stage_display_snapshot("worship-snv").await?;
    let favorite_ids: HashSet<_> = state
        .library_favorites()
        .await?
        .into_iter()
        .map(|id| id.to_string())
        .collect();

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

    let library_json = to_string(&library_rows).unwrap_or_else(|_| "[]".to_string());
    let playlist_json = to_string(&playlist_rows).unwrap_or_else(|_| "[]".to_string());
    let stage_json = stage_snapshot
        .map(|snapshot| to_string(&snapshot).unwrap_or_else(|_| "null".to_string()))
        .unwrap_or_else(|| "null".to_string());

    let owner = Owner::new_root(None);
    let html = owner.with(|| {
        view! { <TabletDocument library_json=library_json.clone() playlist_json=playlist_json.clone() stage_json=stage_json.clone() /> }
            .into_view()
            .to_html()
    });

    Ok(Html(format!("<!DOCTYPE html>{html}")))
}

#[component]
fn BibleDocument(
    translations: Vec<BibleTranslation>,
    active: Option<BibleBroadcast>,
    translations_json: String,
    active_json: String,
) -> impl IntoView {
    let translations_json_safe = translations_json.replace("</script>", r"<\/script>");
    let active_json_safe = active_json.replace("</script>", r"<\/script>");
    let script = BIBLE_SCRIPT_TEMPLATE
        .replace("__TRANSLATIONS__", &translations_json_safe)
        .replace("__ACTIVE__", &active_json_safe);

    view! {
        <html lang="en">
            <head>
                <meta charset="utf-8" />
                <title>"Presenter Bible Control"</title>
                <style>{BIBLE_STYLES}</style>
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

pub async fn render_settings_ui(state: &AppState) -> anyhow::Result<Html<String>> {
    let hosts = state.list_resolume_hosts().await?;
    let statuses = state.resolume_status_snapshot().await;
    let android_displays = state.list_android_stage_displays().await?;
    let android_statuses = state.android_stage_status_snapshot().await;
    let osc_settings = state.osc_settings().await?;
    let osc_status = state.osc_status_snapshot().await;
    let ableset_settings = state.ableset_settings().await?;
    let ableset_status = state.ableset_status_snapshot().await;
    let feature_flags = state.feature_flags();

    let host_rows: Vec<SettingsHostRow> = hosts
        .into_iter()
        .map(|host| {
            let created_display = format_settings_timestamp(host.created_at);
            let updated_display = format_settings_timestamp(host.updated_at);
            let status = statuses
                .get(&host.id)
                .cloned()
                .unwrap_or_else(ResolumeConnectionSnapshot::disabled);
            let status_state = match status.state {
                ResolumeConnectionState::Disabled => "Disabled".to_string(),
                ResolumeConnectionState::Connecting => "Connecting".to_string(),
                ResolumeConnectionState::Connected => "Connected".to_string(),
                ResolumeConnectionState::Error => "Error".to_string(),
            };
            SettingsHostRow {
                id: host.id.to_string(),
                label: host.label,
                host: host.host,
                port: host.port,
                is_enabled: host.is_enabled,
                created_at: host.created_at.to_rfc3339(),
                created_at_display: created_display,
                updated_at: host.updated_at.to_rfc3339(),
                updated_at_display: updated_display,
                status_state,
                status_message: status.last_error.clone(),
                last_latency_ms: status.last_latency_ms,
                status: Some(status),
            }
        })
        .collect();

    let android_rows: Vec<SettingsAndroidDisplayRow> = android_displays
        .into_iter()
        .map(|display| {
            let status = android_statuses
                .get(&display.id)
                .cloned()
                .unwrap_or_else(AndroidStageDisplayStatusSnapshot::disabled);
            let status_state = match status.state {
                crate::android_stage::AndroidStageDisplayState::Disabled => "Disabled".to_string(),
                crate::android_stage::AndroidStageDisplayState::Connecting => {
                    "Connecting".to_string()
                }
                crate::android_stage::AndroidStageDisplayState::Launching => {
                    "Launching".to_string()
                }
                crate::android_stage::AndroidStageDisplayState::Running => "Running".to_string(),
                crate::android_stage::AndroidStageDisplayState::Error => "Error".to_string(),
            };
            let created_display = format_settings_timestamp(display.created_at);
            let updated_display = format_settings_timestamp(display.updated_at);
            let last_attempt_display = status
                .last_attempt
                .map(format_settings_timestamp)
                .unwrap_or_else(|| "\u{2014}".to_string());
            let last_success_display = status
                .last_success
                .map(format_settings_timestamp)
                .unwrap_or_else(|| "\u{2014}".to_string());
            SettingsAndroidDisplayRow {
                id: display.id.to_string(),
                label: display.label,
                host: display.host,
                port: display.port,
                launch_component: display.launch_component,
                is_enabled: display.is_enabled,
                created_at: display.created_at.to_rfc3339(),
                created_at_display: created_display,
                updated_at: display.updated_at.to_rfc3339(),
                updated_at_display: updated_display,
                status_state,
                last_attempt_display,
                last_success_display,
                status_message: status.last_error.clone(),
                status: Some(status),
            }
        })
        .collect();

    let hosts_json = to_string(&host_rows).unwrap_or_else(|_| "[]".to_string());
    let hosts_json = hosts_json.replace("</script>", r"<\/script>");
    let android_json = to_string(&android_rows).unwrap_or_else(|_| "[]".to_string());
    let android_json = android_json.replace("</script>", r"<\/script>");

    let osc_config_json = json!({
        "enabled": osc_settings.enabled,
        "listenPort": osc_settings.listen_port,
        "addressPattern": osc_settings.address_pattern,
        "velocityMode": osc_settings.velocity_mode,
    });
    let osc_config_json = to_string(&osc_config_json)
        .unwrap_or_else(|_| "{}".to_string())
        .replace("</script>", r"<\/script>");
    let osc_status_json = to_string(&osc_status)
        .unwrap_or_else(|_| "{}".to_string())
        .replace("</script>", r"<\/script>");
    let ableset_config_json = json!({
        "enabled": ableset_settings.enabled,
        "host": ableset_settings.host,
        "httpPort": ableset_settings.http_port,
        "oscPort": ableset_settings.osc_port,
        "libraryName": ableset_settings.library_name,
        "songPrefixLength": ableset_settings.song_prefix_length,
    });
    let ableset_config_json = to_string(&ableset_config_json)
        .unwrap_or_else(|_| "{}".to_string())
        .replace("</script>", r"<\/script>");
    let ableset_status_json = to_string(&ableset_status)
        .unwrap_or_else(|_| "{}".to_string())
        .replace("</script>", r"<\/script>");
    let feature_json = to_string(&feature_flags)
        .unwrap_or_else(|_| "{}".to_string())
        .replace("</script>", r"<\/script>");

    let script = SETTINGS_SCRIPT_TEMPLATE
        .replace("__RESOLUME_HOSTS__", &hosts_json)
        .replace("__ANDROID_STAGE_DISPLAYS__", &android_json)
        .replace("__OSC_CONFIG__", &osc_config_json)
        .replace("__OSC_STATUS__", &osc_status_json)
        .replace("__ABLESET_CONFIG__", &ableset_config_json)
        .replace("__ABLESET_STATUS__", &ableset_status_json)
        .replace("__FEATURE_FLAGS__", &feature_json);

    let owner = Owner::new_root(None);
    let html = owner.with(|| {
        view! {
            <SettingsDocument
                hosts=host_rows.clone()
                android_displays=android_rows.clone()
                osc_settings=osc_settings.clone()
                osc_status=osc_status.clone()
                ableset_settings=ableset_settings.clone()
                ableset_status=ableset_status.clone()
                features=feature_flags.clone()
                script=script.clone()
            />
        }
        .into_view()
        .to_html()
    });

    Ok(Html(format!("<!DOCTYPE html>{html}")))
}

#[component]
fn SettingsDocument(
    hosts: Vec<SettingsHostRow>,
    android_displays: Vec<SettingsAndroidDisplayRow>,
    osc_settings: OscSettings,
    osc_status: OscStatusSnapshot,
    ableset_settings: AbleSetSettings,
    ableset_status: AbleSetStatusSnapshot,
    features: FeatureFlags,
    script: String,
) -> impl IntoView {
    let hosts = Arc::new(hosts);
    let host_count_text = hosts.len().to_string();
    let android_displays = Arc::new(android_displays);
    let android_count_text = android_displays.len().to_string();
    let companion_enabled = features.companion_enabled;
    let companion_port_text = features.companion_port.to_string();
    let osc_port_value = osc_settings.listen_port.to_string();
    let osc_address_value = osc_settings.address_pattern.clone();
    let osc_mode_value = match osc_settings.velocity_mode {
        presenter_core::VelocityMode::ZeroBased => "zero_based",
        presenter_core::VelocityMode::OneBased => "one_based",
    };
    let osc_mode_value_string = osc_mode_value.to_string();
    let osc_host_port_display = osc_status.host_port.unwrap_or(osc_settings.listen_port);
    let osc_status_state = if !osc_status.enabled {
        "disabled".to_string()
    } else if osc_status.listening {
        "listening".to_string()
    } else {
        "enabled".to_string()
    };
    let osc_status_label = format!(
        "{}{}",
        osc_status_state
            .chars()
            .next()
            .map(|c| c.to_uppercase().collect::<String>())
            .unwrap_or_else(String::new),
        osc_status_state.chars().skip(1).collect::<String>()
    );
    let osc_last_message_display = osc_status
        .last_message_at
        .map(format_settings_timestamp)
        .unwrap_or_else(|| "\u{2014}".to_string());
    let osc_last_note_display = osc_status
        .last_note
        .map(|note| {
            if let Some(velocity) = osc_status.last_velocity {
                format!("note {note} (vel {velocity})")
            } else {
                format!("note {note}")
            }
        })
        .unwrap_or_else(|| "\u{2014}".to_string());
    let osc_last_error = osc_status.last_error.clone();
    let ableset_host_value = ableset_settings.host.clone();
    let ableset_http_port_value = ableset_settings.http_port.to_string();
    let ableset_osc_port_value = ableset_settings.osc_port.to_string();
    let ableset_library_value = ableset_settings.library_name.clone();
    let ableset_prefix_value = ableset_settings.song_prefix_length.to_string();
    let ableset_enabled = ableset_settings.enabled;
    let ableset_status_state = if !ableset_status.enabled {
        "disabled"
    } else if ableset_status.tracking {
        "tracking"
    } else {
        "enabled"
    };
    let ableset_status_label = format!(
        "{}{}",
        ableset_status_state
            .chars()
            .next()
            .map(|c| c.to_uppercase().collect::<String>())
            .unwrap_or_else(String::new),
        ableset_status_state.chars().skip(1).collect::<String>()
    );
    let ableset_last_song_name = ableset_status
        .last_song
        .as_ref()
        .map(|song| song.name.clone())
        .unwrap_or_else(|| "\u{2014}".to_string());
    let ableset_last_song_prefix = ableset_status
        .last_song
        .as_ref()
        .map(|song| song.prefix.clone())
        .unwrap_or_else(|| "\u{2014}".to_string());
    let ableset_last_song_seen = ableset_status
        .last_song
        .as_ref()
        .and_then(|song| song.last_seen_at)
        .map(format_settings_timestamp)
        .unwrap_or_else(|| "\u{2014}".to_string());
    let ableset_last_error = ableset_status.last_error.clone();

    view! {
        <html lang="en">
            <head>
                <meta charset="utf-8" />
                <title>"Presenter Settings"</title>
                <meta name="viewport" content="width=device-width, initial-scale=1" />
                <style>{SETTINGS_STYLES}</style>
            </head>
            <body class="settings" data-mode="create">
                <header class="settings__header">
                    <div class="settings__header-title">
                        <h1>"Presenter Settings"</h1>
                        <p>"Configure integrations and controller connections."</p>
                    </div>
                    <nav class="settings__header-nav">
                        <a href="/" class="settings__link">"← Back to hub"</a>
                    </nav>
                </header>
                <main class="settings__main">
                    <section class="settings__card settings__card--feature">
                        <header class="settings__card-header">
                            <div>
                                <h2>"Companion"</h2>
                            </div>
                        </header>
                        <form class="settings__form settings__form--compact" data-role="feature-companion-form" autocomplete="off">
                            <div class="settings__form-row settings__form-row--compact settings__form-row--inline">
                                <label class="settings__form-checkbox settings__form-checkbox--inline">
                                    <input
                                        type="checkbox"
                                        data-role="feature-companion-toggle"
                                        checked={companion_enabled}
                                    />
                                    <span>"Enable"</span>
                                </label>
                                <label class="settings__form-control--tiny">
                                    <span>"Port"</span>
                                    <input
                                        type="number"
                                        min="1"
                                        max="65535"
                                        value={companion_port_text.clone()}
                                        data-role="feature-companion-port"
                                        required
                                    />
                                </label>
                                <button
                                    type="submit"
                                    class="settings__button settings__button--primary settings__button--compact"
                                    data-role="feature-submit"
                                >"Save"</button>
                            </div>
                            <p class="settings__form-status" data-role="feature-status" data-state="idle"></p>
                        </form>
                    </section>
                    <section class="settings__card">
                        <header class="settings__card-header">
                            <div>
                                <h2>"Resolume Arena Connections"</h2>
                                <p>
                                    "Define Resolume web servers Presenter should control."
                                </p>
                            </div>
                            <div class="settings__badge-group">
                                <span class="settings__badge" data-role="host-count">{host_count_text.clone()}</span>
                                <span class="settings__badge-label">"Hosts"</span>
                            </div>
                        </header>
                        <form class="settings__form" data-role="host-form" autocomplete="off">
                            <input type="hidden" data-role="host-id" />
                            <div class="settings__form-header">
                                <div>
                                    <h3 data-role="form-title">"Add Resolume Connection"</h3>
                                    <p data-role="form-subtitle">"Specify hostname, port, and availability."</p>
                                </div>
                            </div>
                            <div class="settings__form-row">
                                <label>
                                    <span>"Label"</span>
                                    <input
                                        type="text"
                                        name="label"
                                        data-role="host-label"
                                        placeholder="Main Arena"
                                        required
                                    />
                                </label>
                                <label>
                                    <span>"Hostname or DNS"</span>
                                    <input
                                        type="text"
                                        name="host"
                                        data-role="host-host"
                                        placeholder="resolume.lan"
                                        required
                                    />
                                </label>
                                <label class="settings__form-control--small">
                                    <span>"Port"</span>
                                    <input
                                        type="number"
                                        name="port"
                                        data-role="host-port"
                                        min="1"
                                        max="65535"
                                        value="8090"
                                        required
                                    />
                                </label>
                            </div>
                            <div class="settings__form-row settings__form-row--single">
                                <label class="settings__form-checkbox settings__form-checkbox--block">
                                    <input type="checkbox" name="isEnabled" data-role="host-enabled" checked />
                                    <span>"Enabled"</span>
                                </label>
                            </div>
                            <div class="settings__form-actions">
                                <button
                                    type="submit"
                                    class="settings__button settings__button--primary"
                                    data-role="host-submit"
                                >"Add Connection"</button>
                                <button
                                    type="button"
                                    class="settings__button settings__button--ghost"
                                    data-role="host-reset"
                                >"Cancel"</button>
                            </div>
                            <p class="settings__form-status" data-role="form-status" data-state="idle"></p>
                        </form>
                        <ul class="settings__list" data-role="resolume-host-list">
                            <Show
                                when={
                                    let hosts = Arc::clone(&hosts);
                                    move || !hosts.is_empty()
                                }
                                fallback={move || view! {
                                    <li class="settings__list-empty" data-role="host-empty">"No Resolume connections defined yet."</li>
                                }}
                            >
                                <For
                                    each={
                                        let hosts = Arc::clone(&hosts);
                                        move || (*hosts).clone()
                                    }
                                    key=|host: &SettingsHostRow| host.id.clone()
                                    children={|host: SettingsHostRow| {
                                        let raw_state = if host.status_state.is_empty() {
                                            "disabled".to_string()
                                        } else {
                                            host.status_state.to_lowercase()
                                        };
                                        let status_class =
                                            format!("settings__status settings__status--{}", raw_state);
                                        let status_label = format!(
                                            "{}{}",
                                            raw_state
                                                .chars()
                                                .next()
                                                .map(|c| c.to_uppercase().collect::<String>())
                                                .unwrap_or_else(String::new),
                                            raw_state.chars().skip(1).collect::<String>()
                                        );
                                        let latency_text = host
                                            .last_latency_ms
                                            .map(|ms| format!("{ms:.1} ms"))
                                            .unwrap_or_else(|| "—".to_string());
                                        let warning_text = host.status_message.clone().unwrap_or_default();
                                        let warning_view = (!warning_text.is_empty()).then(|| {
                                            view! { <p class="settings__list-meta settings__list-meta--warning">{format!("⚠ {warning_text}")}</p> }
                                        });
                                        let host_id_edit = host.id.clone();
                                        let host_id_delete = host.id.clone();
                                        view! {
                                            <li
                                                class="settings__list-item"
                                                data-id={host.id.clone()}
                                                data-enabled={host.is_enabled.to_string()}
                                            >
                                                <div class="settings__list-primary">
                                                    <div class="settings__list-title">
                                                        <span class="settings__host-label">{host.label.clone()}</span>
                                                        <span class={status_class}>{status_label.clone()}</span>
                                                    </div>
                                                    <p class="settings__list-line">
                                                        <code>{host.host.clone()}</code>
                                                        <span class="settings__host-port">{format!(":{}", host.port)}</span>
                                                    </p>
                                                    <p class="settings__list-meta">{"Updated "}{host.updated_at_display.clone()}</p>
                                                    <p class="settings__list-meta">{"Created "}{host.created_at_display.clone()}</p>
                                                    <p class="settings__list-meta">{"Latency "}{latency_text}</p>
                                                    {warning_view}
                                                </div>
                                                <div class="settings__list-actions">
                                                    <button
                                                        type="button"
                                                        class="settings__button settings__button--ghost"
                                                        data-role="host-edit"
                                                        data-id={host_id_edit}
                                                    >"Edit"</button>
                                                    <button
                                                        type="button"
                                                        class="settings__button settings__button--danger"
                                                        data-role="host-delete"
                                                        data-id={host_id_delete}
                                                    >"Delete"</button>
                                                </div>
                                            </li>
                                        }
                                    }}
                                />
                            </Show>
                        </ul>
                        <section class="settings__legend">
                            <h3>"Clip Tokens"</h3>
                            <p class="settings__legend-note">
                                "Presenter updates every clip whose name contains these tokens (for example, #main-a or #main-a-2) and alternates between A/B lanes so the next look is always preloaded."
                            </p>
                            <dl>
                                <div>
                                    <dt>"#main-a / #main-b"</dt>
                                    <dd>"Main lyric text, alternating between A and B for seamless cuts."</dd>
                                </div>
                                <div>
                                    <dt>"#translate-a / #translate-b"</dt>
                                    <dd>"Translation lyric text matched to each lane."</dd>
                                </div>
                                <div>
                                    <dt>"#bible-a / #bible-b"</dt>
                                    <dd>"Bible verse text for scripture cues."</dd>
                                </div>
                                <div>
                                    <dt>"#bible-translate-a / #bible-translate-b"</dt>
                                    <dd>"Bible translation label accompanying the verse."</dd>
                                </div>
                                <div>
                                    <dt>"#bible-clear"</dt>
                                    <dd>"Clears the Bible layer when triggered."</dd>
                                </div>
                                <div>
                                    <dt>"#song-name"</dt>
                                    <dd>"Displays the active song title (numeric prefixes like '001 ' are removed automatically)."</dd>
                                </div>
                                <div>
                                    <dt>"#band-name"</dt>
                                    <dd>"Displays the library/band the current song belongs to."</dd>
                                </div>
                                <div>
                                    <dt>"Suffixes: -u / -re"</dt>
                                    <dd>"Append -u to force uppercase and -re to collapse multi-line text into a single space-delimited line. Combine them (e.g., #translate-b-u-re) for stacked transforms."</dd>
                                </div>
                            </dl>
                        </section>
                    </section>
                    <section class="settings__card">
                        <header class="settings__card-header">
                            <div>
                                <h2>"Android Stage Launchers"</h2>
                                <p>"Keep each Android TV pinned to the Fully Kiosk stage display."</p>
                            </div>
                            <div class="settings__badge-group">
                                <span class="settings__badge" data-role="android-count">{android_count_text.clone()}</span>
                                <span class="settings__badge-label">"Displays"</span>
                            </div>
                        </header>
                        <form class="settings__form" data-role="android-form" autocomplete="off">
                            <input type="hidden" data-role="android-id" />
                            <div class="settings__form-header">
                                <div>
                                    <h3 data-role="android-form-title">"Add Android Stage Display"</h3>
                                    <p data-role="android-form-subtitle">"Presenter reconnects and relaunches Fully Kiosk whenever the device appears."</p>
                                </div>
                            </div>
                            <div class="settings__form-row">
                                <label>
                                    <span>"Label"</span>
                                    <input type="text" name="label" data-role="android-label" placeholder="Stage Left" required />
                                </label>
                                <label>
                                    <span>"Hostname or DNS"</span>
                                    <input type="text" name="host" data-role="android-host" placeholder="sd1l.lan" required />
                                </label>
                                <label class="settings__form-control--small">
                                    <span>"Port"</span>
                                    <input type="number" name="port" data-role="android-port" min="1" max="65535" value="5555" required />
                                </label>
                            </div>
                            <div class="settings__form-row settings__form-row--single">
                                <label>
                                    <span>"Launch Component"</span>
                                    <input
                                        type="text"
                                        name="launchComponent"
                                        data-role="android-component"
                                        placeholder="com.fullykiosk.videokiosk/de.ozerov.fully.MainActivity"
                                        required
                                    />
                                </label>
                            </div>
                            <div class="settings__form-row settings__form-row--single">
                                <label class="settings__form-checkbox settings__form-checkbox--block">
                                    <input type="checkbox" name="isEnabled" data-role="android-enabled" checked />
                                    <span>"Enabled"</span>
                                </label>
                            </div>
                            <div class="settings__form-actions">
                                <button
                                    type="submit"
                                    class="settings__button settings__button--primary"
                                    data-role="android-submit"
                                >"Add Android Display"</button>
                                <button
                                    type="button"
                                    class="settings__button settings__button--ghost"
                                    data-role="android-reset"
                                >"Cancel"</button>
                            </div>
                            <p class="settings__form-status" data-role="android-form-status" data-state="idle"></p>
                        </form>
                        <ul class="settings__list" data-role="android-display-list">
                            <Show
                                when={
                                    let displays = Arc::clone(&android_displays);
                                    move || !displays.is_empty()
                                }
                                fallback={move || view! {
                                    <li class="settings__list-empty" data-role="android-empty">"No Android stage displays configured yet."</li>
                                }}
                            >
                                <For
                                    each={
                                        let displays = Arc::clone(&android_displays);
                                        move || (*displays).clone()
                                    }
                                    key=|display: &SettingsAndroidDisplayRow| display.id.clone()
                                    children={|display: SettingsAndroidDisplayRow| {
                                        let raw_state = if display.status_state.is_empty() {
                                            "disabled".to_string()
                                        } else {
                                            display.status_state.to_lowercase().replace(' ', "-")
                                        };
                                        let status_class =
                                            format!("settings__status settings__status--{}", raw_state);
                                        let status_label = display.status_state.clone();
                                        let warning_text = display.status_message.clone().unwrap_or_default();
                                        let warning_view = (!warning_text.is_empty()).then(|| {
                                            view! { <p class="settings__list-meta settings__list-meta--warning">{format!("⚠ {}", warning_text)}</p> }
                                        });
                                        let display_id_edit = display.id.clone();
                                        let display_id_delete = display.id.clone();
                                        view! {
                                            <li
                                                class="settings__list-item"
                                                data-id={display.id.clone()}
                                                data-enabled={display.is_enabled.to_string()}
                                            >
                                                <div class="settings__list-primary">
                                                    <div class="settings__list-title">
                                                        <span class="settings__host-label">{display.label.clone()}</span>
                                                        <span class={status_class}>{status_label}</span>
                                                    </div>
                                                    <p class="settings__list-line">
                                                        <code>{display.host.clone()}</code>
                                                        <span class="settings__host-port">{format!(":{}", display.port)}</span>
                                                    </p>
                                                    <p class="settings__list-meta">{"Component "}{display.launch_component.clone()}</p>
                                                    <p class="settings__list-meta">{"Last attempt "}{display.last_attempt_display.clone()}</p>
                                                    <p class="settings__list-meta">{"Last success "}{display.last_success_display.clone()}</p>
                                                    <p class="settings__list-meta">{"Updated "}{display.updated_at_display.clone()}</p>
                                                    <p class="settings__list-meta">{"Created "}{display.created_at_display.clone()}</p>
                                                    {warning_view}
                                                </div>
                                                <div class="settings__list-actions">
                                                    <button
                                                        type="button"
                                                        class="settings__button settings__button--ghost"
                                                        data-role="android-edit"
                                                        data-id={display_id_edit}
                                                    >"Edit"</button>
                                                    <button
                                                        type="button"
                                                        class="settings__button settings__button--danger"
                                                        data-role="android-delete"
                                                        data-id={display_id_delete}
                                                    >"Delete"</button>
                                                </div>
                                            </li>
                                        }
                                    }}
                                />
                            </Show>
                        </ul>
                    </section>
                    <section class="settings__card settings__card--osc">
                        <header class="settings__card-header">
                            <div>
                                <h2>"OSC Bridge"</h2>
                                <p>"Receive Ableton cues via the OSC MIDI Send Max for Live device."</p>
                            </div>
                        </header>
                        <form
                            class="settings__form settings__form--osc"
                            data-role="osc-form"
                            autocomplete="off"
                            data-mode={if osc_settings.enabled { "enabled" } else { "disabled" }}
                        >
                            <div class="settings__form-row settings__form-row--single">
                                <label class="settings__form-checkbox settings__form-checkbox--block">
                                    <input type="checkbox" data-role="osc-enabled" checked={osc_settings.enabled} />
                                    <span>"Enabled"</span>
                                </label>
                            </div>
                            <div class="settings__form-row">
                                <label class="settings__form-control settings__form-control--small">
                                    <span>"Listener Port"</span>
                                    <input
                                        type="number"
                                        data-role="osc-port"
                                        min="1"
                                        max="65535"
                                        value={osc_port_value.clone()}
                                        required
                                    />
                                </label>
                                <label>
                                    <span>"Address Pattern"</span>
                                    <input
                                        type="text"
                                        data-role="osc-address"
                                        value={osc_address_value.clone()}
                                        placeholder="/note"
                                        required
                                    />
                                </label>
                                <label>
                                    <span>"Velocity Mapping"</span>
                                    <select data-role="osc-mode" prop:value={osc_mode_value_string.clone()}>
                                        <option value="zero_based" selected={osc_mode_value == "zero_based"}>"Zero-based (0 = first item)"</option>
                                        <option value="one_based" selected={osc_mode_value == "one_based"}>"One-based (1 = first item)"</option>
                                    </select>
                                </label>
                            </div>
                            <div class="settings__form-actions">
                                <button
                                    type="submit"
                                    class="settings__button settings__button--primary"
                                    data-role="osc-submit"
                                >"Save OSC Settings"</button>
                            </div>
                        </form>
                        <section class="settings__osc-status">
                            <div class="settings__status-line">
                                <span
                                    class="settings__status"
                                    data-role="osc-status-indicator"
                                    data-state={osc_status_state.clone()}
                                >{osc_status_label.clone()}</span>
                            </div>
                            <dl class="settings__status-list">
                                <div>
                                    <dt>"Host Port"</dt>
                                    <dd data-role="osc-status-host-port">{osc_host_port_display}</dd>
                                </div>
                                <div>
                                    <dt>"Last event"</dt>
                                    <dd data-role="osc-status-last-message">{osc_last_message_display.clone()}</dd>
                                </div>
                                <div>
                                    <dt>"Last note"</dt>
                                    <dd data-role="osc-status-last-note">{osc_last_note_display.clone()}</dd>
                                </div>
                            </dl>
                            <p
                                class="settings__list-meta settings__list-meta--warning"
                                data-role="osc-status-error"
                                data-visible={if osc_last_error.is_some() { "true" } else { "false" }}
                            >{osc_last_error.clone().map(|err| format!("⚠ {}", err)).unwrap_or_default()}</p>
                        </section>
                    </section>
                    <section class="settings__card settings__card--ableset">
                        <header class="settings__card-header">
                            <div>
                                <h2>"AbleSet Bridge"</h2>
                                <p>"Map Ableton cues to the NEWLEVEL library using AbleSet."</p>
                            </div>
                        </header>
                        <form
                            class="settings__form settings__form--ableset"
                            data-role="ableset-form"
                            autocomplete="off"
                            data-mode={if ableset_enabled { "enabled" } else { "disabled" }}
                        >
                            <div class="settings__form-row settings__form-row--single">
                                <label class="settings__form-checkbox settings__form-checkbox--block">
                                    <input type="checkbox" data-role="ableset-enabled" checked={ableset_enabled} />
                                    <span>"Enable AbleSet automation"</span>
                                </label>
                            </div>
                            <div class="settings__form-row">
                                <label>
                                    <span>"AbleSet Host"</span>
                                    <input
                                        type="text"
                                        data-role="ableset-host"
                                        value={ableset_host_value.clone()}
                                        required
                                    />
                                </label>
                                <label class="settings__form-control settings__form-control--small">
                                    <span>"HTTP Port"</span>
                                    <input
                                        type="number"
                                        data-role="ableset-http-port"
                                        min="1"
                                        max="65535"
                                        value={ableset_http_port_value.clone()}
                                        required
                                    />
                                </label>
                                <label class="settings__form-control settings__form-control--small">
                                    <span>"OSC Port"</span>
                                    <input
                                        type="number"
                                        data-role="ableset-osc-port"
                                        min="1"
                                        max="65535"
                                        value={ableset_osc_port_value.clone()}
                                        required
                                    />
                                </label>
                            </div>
                            <div class="settings__form-row">
                                <label>
                                    <span>"Library Name"</span>
                                    <input
                                        type="text"
                                        data-role="ableset-library"
                                        value={ableset_library_value.clone()}
                                        required
                                    />
                                </label>
                                <label class="settings__form-control settings__form-control--small">
                                    <span>"Song Prefix Length"</span>
                                    <input
                                        type="number"
                                        data-role="ableset-prefix"
                                        min="1"
                                        max="6"
                                        value={ableset_prefix_value.clone()}
                                        required
                                    />
                                </label>
                            </div>
                            <div class="settings__form-actions">
                                <button
                                    type="submit"
                                    class="settings__button settings__button--primary"
                                    data-role="ableset-submit"
                                >"Save AbleSet Settings"</button>
                            </div>
                            <p class="settings__form-status" data-role="ableset-form-status" data-state="idle"></p>
                        </form>
                        <div class="settings__status-panel">
                            <span
                                class={format!("settings__status settings__status--{}", ableset_status_state)}
                                data-role="ableset-status-indicator"
                            >{ableset_status_label.clone()}</span>
                            <dl class="settings__status-list">
                                <div>
                                    <dt>"Current Song"</dt>
                                    <dd data-role="ableset-status-song">{ableset_last_song_name.clone()}</dd>
                                </div>
                                <div>
                                    <dt>"Prefix"</dt>
                                    <dd data-role="ableset-status-prefix">{ableset_last_song_prefix.clone()}</dd>
                                </div>
                                <div>
                                    <dt>"Last Update"</dt>
                                    <dd data-role="ableset-status-updated">{ableset_last_song_seen.clone()}</dd>
                                </div>
                            </dl>
                            <p class="settings__list-meta settings__list-meta--warning" data-role="ableset-status-error">
                                {ableset_last_error.clone().unwrap_or_default()}
                            </p>
                            <button type="button" class="settings__button settings__button--ghost" data-role="ableset-refresh">"Refresh"</button>
                        </div>
                    </section>

                </main>
                <div class="settings__toast" data-role="toast" data-visible="false"></div>
                <script>{script}</script>
            </body>
        </html>
    }
}

fn format_settings_timestamp(value: DateTime<Utc>) -> String {
    value.format("%Y-%m-%d %H:%M UTC").to_string()
}

#[component]
fn HomeDocument() -> impl IntoView {
    view! {
        <html lang="en">
            <head>
                <meta charset="utf-8" />
                <title>"Presenter surfaces"</title>
                <style>{HOME_STYLES}</style>
            </head>
            <body class="home">
                <main class="home__container">
                    <header class="home__header">
                        <h1>"Presenter Demo Environment"</h1>
                        <p>"Quick links to control surfaces and stage displays for live verification."</p>
                    </header>
                    <div class="home__cta-row">
                        <a
                            class="home__cta-button"
                            href="/ui/settings"
                            target="_blank"
                            rel="noopener"
                        >"Open Settings"</a>
                    </div>
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
