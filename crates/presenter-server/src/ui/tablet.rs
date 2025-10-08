use crate::state::AppState;
use axum::response::Html;
use leptos::prelude::*;
use presenter_core::playlist::PlaylistEntryKind;
use reactive_graph::owner::Owner;
use serde_json::to_string;
use std::collections::{HashMap, HashSet};

use super::models::{LibraryRow, PlaylistEntryRow, PlaylistRow, PresentationRow};
use super::scripts::TABLET as TABLET_SCRIPT_TEMPLATE;
use super::styles::TABLET as TABLET_STYLES;

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
