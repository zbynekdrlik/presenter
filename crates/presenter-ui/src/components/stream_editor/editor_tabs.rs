//! Scény | Menovky | Písma tabs of the stream editor (#829).
//!
//! The editor used to render every area in one long column — the scene
//! columns, the open-scene workspace, then the nameplates (Menovky) and the
//! fonts at the very bottom, so during a service the nameplates were
//! effectively hidden. A tab bar under the header now shows one area at a time.
//!
//! The panels stay MOUNTED: an inactive panel is only hidden
//! (`data-active="false"` + CSS), so an unsaved element draft or a half-typed
//! nameplate survives a tab switch and no "discard changes?" question is
//! needed. The active tab is remembered in localStorage (`stream-editor-tab`)
//! and in the `?tab=` URL param, which `replace_url_param` merges with
//! `?output=` so neither drops the other.

use leptos::prelude::*;

use super::output_paths::{read_stored, write_stored};
use super::{bool_attr, StreamEditorCtx};

/// `localStorage` key for the last active editor tab.
const TAB_STORAGE_KEY: &str = "stream-editor-tab";

/// One area of the editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorTab {
    /// The scene columns + the open-scene workspace (element panel + preview).
    Scenes,
    /// The nameplate (lower third) list.
    Nameplates,
    /// The uploaded web fonts.
    Fonts,
}

impl EditorTab {
    /// Every tab, in tab-bar order.
    pub const ALL: [EditorTab; 3] = [EditorTab::Scenes, EditorTab::Nameplates, EditorTab::Fonts];

    /// The id used in `?tab=`, localStorage and the `data-tab` attribute.
    pub fn id(self) -> &'static str {
        match self {
            EditorTab::Scenes => "scenes",
            EditorTab::Nameplates => "nameplates",
            EditorTab::Fonts => "fonts",
        }
    }

    /// The tab-bar label.
    pub fn label(self) -> &'static str {
        match self {
            EditorTab::Scenes => "Scény",
            EditorTab::Nameplates => "Menovky",
            EditorTab::Fonts => "Písma",
        }
    }

    /// The tab with this id (`None` for an unknown id).
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.id() == id)
    }
}

/// The tab to open: a valid `?tab=` → a valid remembered tab → Scény.
pub(super) fn resolve_tab(url: Option<&str>, stored: Option<&str>) -> EditorTab {
    url.and_then(EditorTab::from_id)
        .or_else(|| stored.and_then(EditorTab::from_id))
        .unwrap_or(EditorTab::Scenes)
}

/// The tab to open on load (URL → localStorage → Scény).
pub fn initial_tab() -> EditorTab {
    let url = crate::utils::window::url_param("tab");
    let stored = read_stored(TAB_STORAGE_KEY);
    resolve_tab(url.as_deref(), stored.as_deref())
}

impl StreamEditorCtx {
    /// Show another tab and remember it (localStorage + `?tab=`). Only the
    /// panels' visibility changes — nothing is unmounted, so no unsaved work is
    /// lost and the dirty-draft guard is not involved.
    pub fn select_tab(self, tab: EditorTab) {
        if self.tab.get_untracked() == tab {
            return;
        }
        self.tab.set(tab);
        write_stored(TAB_STORAGE_KEY, tab.id());
        crate::utils::window::replace_url_param("tab", tab.id());
    }
}

/// The tab bar under the editor header. Its buttons wrap at phone width.
#[component]
pub fn EditorTabs(ctx: StreamEditorCtx) -> impl IntoView {
    let buttons = EditorTab::ALL
        .into_iter()
        .map(|t| {
            let active = move || bool_attr(ctx.tab.get() == t);
            view! {
                <button
                    type="button"
                    role="tab"
                    class="stream-editor__tab"
                    data-role="stream-editor-tab"
                    data-tab=t.id()
                    data-active=active
                    aria-selected=active
                    on:click=move |_| ctx.select_tab(t)
                >
                    {t.label()}
                </button>
            }
        })
        .collect_view();
    view! {
        <nav
            class="stream-editor__tabs"
            data-role="stream-editor-tabs"
            role="tablist"
            aria-label="Časti editora"
        >
            {buttons}
        </nav>
    }
}

/// One tab's panel: always mounted, hidden while another tab is active.
#[component]
pub fn TabPanel(ctx: StreamEditorCtx, tab: EditorTab, children: Children) -> impl IntoView {
    view! {
        <div
            class="stream-editor__tab-panel"
            data-role="stream-tab-panel"
            data-tab=tab.id()
            data-active=move || bool_attr(ctx.tab.get() == tab)
            role="tabpanel"
        >
            {children()}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tab_id_round_trips() {
        for t in EditorTab::ALL {
            assert_eq!(EditorTab::from_id(t.id()), Some(t));
        }
        assert_eq!(EditorTab::from_id("menovky"), None);
        assert_eq!(EditorTab::from_id(""), None);
    }

    #[test]
    fn the_url_wins_over_the_remembered_tab() {
        assert_eq!(
            resolve_tab(Some("fonts"), Some("nameplates")),
            EditorTab::Fonts
        );
    }

    #[test]
    fn the_remembered_tab_is_used_without_a_url_param() {
        assert_eq!(resolve_tab(None, Some("nameplates")), EditorTab::Nameplates);
        // An unknown URL value falls through to the remembered tab.
        assert_eq!(resolve_tab(Some("bogus"), Some("fonts")), EditorTab::Fonts);
    }

    #[test]
    fn scenes_is_the_default() {
        assert_eq!(resolve_tab(None, None), EditorTab::Scenes);
        assert_eq!(resolve_tab(Some("bogus"), Some("bogus")), EditorTab::Scenes);
    }
}
