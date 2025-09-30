use crate::state::AppState;
use axum::response::Html;
use leptos::prelude::*;
use presenter_core::{
    playlist::PlaylistEntryKind, BibleBroadcast, BibleTranslation, TimerState, TimersOverview,
};
use reactive_graph::owner::Owner;
use serde::Serialize;
use serde_json::to_string;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

const OPERATOR_SCRIPT_TEMPLATE: &str = include_str!("operator_script.js");
const TABLET_SCRIPT_TEMPLATE: &str = include_str!("tablet_script.js");
const BIBLE_SCRIPT_TEMPLATE: &str = include_str!("bible_script.js");

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

#[component]
pub fn OperatorDocument(
    libraries: Vec<LibraryRow>,
    playlists: Vec<PlaylistRow>,
    timers: TimersOverview,
    libraries_json: String,
    playlists_json: String,
) -> impl IntoView {
    let initial_library_id = libraries.first().map(|library| library.id.clone());
    let initial_playlist_id = playlists.first().map(|playlist| playlist.id.clone());
    let libraries = Arc::new(libraries);
    let playlists = Arc::new(playlists);
    let timers = Arc::new(timers);

    let libraries_json = libraries_json.replace("</script>", r"<\/script>");
    let playlists_json = playlists_json.replace("</script>", r"<\/script>");
    let timers_json = to_string(&*timers).unwrap_or_else(|_| "{}".to_string());
    let timers_json = timers_json.replace("</script>", r"<\/script>");

    let operator_script = OPERATOR_SCRIPT_TEMPLATE
        .replace("__LIBRARIES__", &libraries_json)
        .replace("__PLAYLISTS__", &playlists_json)
        .replace("__TIMERS__", &timers_json);

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
                        </div>
                        <div class="operator__header-right">
                            <div class="operator__stage-preview" data-role="stage-status" data-active="false">
                                <div class="operator__stage-preview-panel" data-role="stage-next">"—"</div>
                                <div class="operator__stage-preview-panel operator__stage-preview-panel--current" data-role="stage-current">"—"</div>
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
                                <div class="operator__catalog-bottom" data-role="catalog-bottom">
                                    <header class="operator__group-header operator__presentations-header">
                                        <div class="operator__presentations-heading">
                                            <h2 data-role="context-title">"Presentations"</h2>
                                            <span class="operator__group-count operator__group-count--static" data-role="presentation-count">"—"</span>
                                        </div>
                                        <div class="operator__group-controls">
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
                                            <span class="operator__timer-state" id="countdown-state">
                                                {format_timer_state(overview.countdown_to_start.state)}
                                            </span>
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
                                        <span>"Target"</span>
                                        <input type="datetime-local" data-role="countdown-target-input" />
                                    </label>
                                    <div class="operator__timer-buttons">
                                        <button type="button" data-command="set_countdown_target">"Set Target"</button>
                                        <button type="button" data-command="start_countdown">"Start"</button>
                                        <button type="button" data-command="pause_countdown">"Pause"</button>
                                        <button type="button" data-command="reset_countdown">"Reset"</button>
                                    </div>
                                </div>
                                <div class="operator__timer-group">
                                    <h3>"Preach"</h3>
                                    <div class="operator__timer-buttons">
                                        <button type="button" data-command="start_preach">"Start"</button>
                                        <button type="button" data-command="pause_preach">"Pause"</button>
                                        <button type="button" data-command="reset_preach">"Reset"</button>
                                    </div>
                                </div>
                            </div>
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

    let owner = Owner::new_root(None);
    let html = owner.with(|| {
        view! {
            <OperatorDocument
                libraries=library_rows.clone()
                playlists=playlist_rows.clone()
                timers=timers.clone()
                libraries_json=libraries_json.clone()
                playlists_json=playlists_json.clone()
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
    padding: 0.5rem 0.65rem;
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

.operator__view-nav,
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
    gap: 1rem;
}

.operator__stage-preview {
    position: relative;
    display: inline-flex;
    align-items: stretch;
    gap: 0.75rem;
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

.operator__stage-preview-panel {
    width: 180px;
    height: 70px;
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
    justify-content: space-between;
    align-items: center;
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
    padding: 0.5rem 0.65rem;
    font-family: inherit;
    font-size: 0.9rem;
    width: 100%;
}

.operator__slide-editor input::placeholder {
    font-style: italic;
    color: rgba(15, 23, 42, 0.45);
}

.operator__slide-editor textarea {
    line-height: var(--operator-line-line-height, 1.35);
    min-height: calc(var(--operator-line-line-height, 1.35) * 2em + 1.2rem);
    max-height: calc(var(--operator-line-line-height, 1.35) * 2em + 1.2rem);
    height: calc(var(--operator-line-line-height, 1.35) * 2em + 1.2rem);
    max-width: calc(var(--operator-line-limit-ch, 32) * 1ch + 1.75rem);
    overflow-y: auto;
    resize: none;
}

.operator__slide-editor input {
    max-width: calc(var(--operator-line-limit-ch, 32) * 1ch + 1.75rem);
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
}

.operator__timer-buttons {
    display: flex;
    gap: 0.5rem;
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
                    <section class="home__section">
                        <h2>"Control Surfaces"</h2>
                        <ul class="home__links">
                            <li><a href="/ui/operator">"Operator UI"</a></li>
                            <li><a href="/ui/tablet">"Tablet UI"</a></li>
                            <li><a href="/ui/bible">"Bible Control"</a></li>
                        </ul>
                    </section>
                    <section class="home__section">
                        <h2>"Stage Displays"</h2>
                        <ul class="home__links">
                            <li><a href="/stage/worship-snv">"WORSHIP SNV"</a></li>
                            <li><a href="/stage/worship-pp">"WORSHIP PP"</a></li>
                            <li><a href="/stage/timer">"Timer"</a></li>
                            <li><a href="/stage/preach">"Preach"</a></li>
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
