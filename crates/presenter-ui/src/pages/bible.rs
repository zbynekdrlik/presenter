use leptos::prelude::*;

use crate::api::bible;
use crate::components::stage_preview::{PreviewPlace, StagePreview};
use crate::state::bible::BibleState;
use crate::state::AppContext;

use super::bible_console::BibleConsole;
use super::bible_prepared::{BiblePreparedTab, BiblePresentationModal, BibleSettingsTab};
use super::bible_slides::BibleSlidesColumn;

/// Bible page — 2-column layout matching the legacy Bible UI.
/// Rendered inside the operator shell's `<section data-view-panel="bible">`.
#[component]
pub fn BiblePage() -> impl IntoView {
    let ctx = use_ctx!(AppContext);
    let bs = BibleState::new();
    provide_context(bs.clone());

    // Flag: set to true after initial preferences + translations are loaded.
    // Prevents the auto-save effect from firing during initial hydration.
    let translations_loaded = RwSignal::new(false);
    // The books that came with the console start, until the effect takes them.
    let prepared_books = StoredValue::new(None::<(String, Vec<bible::BibleBookDto>)>);

    // Load translations + preferences on mount
    {
        let translations = bs.translations;
        let selected_translation = bs.selected_translation;
        let secondary_translation = bs.secondary_translation;
        let character_limit = bs.character_limit;
        leptos::task::spawn_local(async move {
            let own_main = crate::state::session::get_persistent("bibleMainTranslation")
                .filter(|main| !main.is_empty());
            // Load preferences first to set saved translation choices. The
            // same answer brings the books (#850 S8), kept before the
            // translation is set so the books effect below finds them.
            if let Ok(start) = bible::console_start(own_main.as_deref()).await {
                if let (Some(code), Some(list)) = (start.books_translation, start.books) {
                    prepared_books.set_value(Some((code, list)));
                }
                let prefs = start.preferences;
                if let Some(ref main) = prefs.main_translation {
                    selected_translation.set(Some(main.clone()));
                }
                if let Some(ref sec) = prefs.secondary_translation {
                    secondary_translation.set(Some(sec.clone()));
                }
                character_limit.set(prefs.character_limit);
            }
            // #701: per-browser localStorage override wins over the server-global
            // default, so concurrent operators keep independent selections and an
            // explicitly-cleared secondary survives a refresh. Applied AFTER the
            // server-pref load (so it takes precedence) and BEFORE the
            // "default main to first translation" step below.
            if let Some(main) = own_main {
                selected_translation.set(Some(main));
            }
            match crate::state::session::get_persistent("bibleSecondaryTranslation") {
                // Empty "" sentinel = the user explicitly cleared the secondary.
                Some(sec) if sec.is_empty() => secondary_translation.set(None),
                Some(sec) => secondary_translation.set(Some(sec)),
                // Absent key = never chosen for this browser; keep server/default.
                None => {}
            }
            if let Ok(trans) = bible::list_translations().await {
                // Set default if preferences didn't set one
                if selected_translation.get_untracked().is_none() {
                    if let Some(first) = trans.first() {
                        selected_translation.set(Some(first.code.clone()));
                    }
                }
                translations.set(trans);
            }
            // Mark loaded AFTER all initial sets are done
            translations_loaded.set(true);
        });
    }

    // Auto-save translation preferences when the user changes them
    {
        let selected_translation = bs.selected_translation;
        let secondary_translation = bs.secondary_translation;
        let character_limit = bs.character_limit;
        Effect::new(move || {
            let main = selected_translation.get();
            let sec = secondary_translation.get();
            // Skip if translations haven't loaded yet (initial hydration)
            if !translations_loaded.get_untracked() {
                return;
            }
            // #701: persist the selection PER BROWSER (localStorage) so it wins
            // over the server-global default on the next load. Secondary uses an
            // empty "" sentinel to record an EXPLICITLY cleared secondary (vs an
            // absent key = never chosen). Written from references before `main`/
            // `sec` are moved into the server-preferences POST below.
            if let Some(m) = main.as_deref() {
                crate::state::session::set_persistent("bibleMainTranslation", m);
            }
            match sec.as_deref() {
                Some(code) => {
                    crate::state::session::set_persistent("bibleSecondaryTranslation", code)
                }
                None => crate::state::session::set_persistent("bibleSecondaryTranslation", ""),
            }
            let limit = character_limit.get_untracked();
            leptos::task::spawn_local(async move {
                let draft = presenter_core::BiblePreferencesDraft {
                    main_translation: main,
                    secondary_translation: sec,
                    character_limit: Some(limit),
                };
                if let Err(e) = bible::update_preferences(&draft).await {
                    web_sys::console::warn_1(&format!("auto-save preferences failed: {e}").into());
                }
            });
        });
    }

    // Load current broadcast state
    {
        let active_broadcast = ctx.active_bible_broadcast;
        leptos::task::spawn_local(async move {
            if let Ok(broadcast) = bible::get_broadcast().await {
                active_broadcast.set(broadcast);
            }
        });
    }

    // Load presentations (and re-fetch when BibleSlidesChanged arrives)
    {
        let version = ctx.bible_presentations_version;
        // The effect's first run is the initial load.
        Effect::new(move || {
            let _v = version.get(); // track the signal
            leptos::task::spawn_local(
                crate::pages::bible_prepared_list::reload_presentation_lists(bs),
            );
        });
    }

    // Load the main translation's books (the console's versification).
    {
        let selected_translation = bs.selected_translation;
        let books = bs.books;
        let books_translation = bs.books_translation;
        Effect::new(move || {
            let Some(code) = selected_translation.get() else {
                return;
            };
            leptos::task::spawn_local(async move {
                let prepared = prepared_books
                    .try_update_value(|prepared| prepared.take())
                    .flatten()
                    .filter(|(prepared_for, _)| *prepared_for == code);
                let book_list = match prepared {
                    Some((_, list)) => list,
                    None => match bible::list_books(&code).await {
                        Ok(list) => list,
                        Err(_) => return,
                    },
                };
                // A newer translation switch may have started while this
                // request was in flight: the last-SELECTED translation wins,
                // and the settle marker never names a code that is not
                // selected any more.
                if selected_translation.get_untracked().as_deref() != Some(code.as_str()) {
                    return;
                }
                books.set(book_list);
                // Settle marker (LAST write): E2E awaits it instead of racing
                // the re-render (bible-page rule, #727).
                books_translation.set(Some(code));
            });
        });
    }

    // Sync data-bible-tab on body for CSS
    {
        let bible_tab = bs.bible_tab;
        let view = ctx.view;
        Effect::new(move || {
            let tab = bible_tab.get();
            if view.get() == "bible" {
                if let Some(body) = crate::utils::window::document_body() {
                    let _ = body.set_attribute("data-bible-tab", &tab);
                }
            }
        });
    }

    // Sync data-mode on body for Bible page (same as operator.rs)
    {
        let mode = ctx.mode;
        let view = ctx.view;
        Effect::new(move || {
            let m = mode.get();
            let v = view.get();
            if v == "bible" {
                if let Some(body) = crate::utils::window::document_body() {
                    let _ = body.set_attribute("data-mode", &m);
                }
            }
        });
    }

    view! {
        <aside class="operator__catalog operator__catalog--bible" data-role="catalog">
            <div class="operator__catalog-top">
                <BibleTabNav />
                <BibleLiveTab />
                <BiblePreparedTab />
                <BibleSettingsTab />
            </div>
            // #875 H2: the verse on the wall, the counter and the broom at the
            // bottom of the left column (desktop; a phone keeps them in the
            // header). Mounted only while Biblia is shown: one copy at a time.
            <Show when=move || !ctx.phone_shell.get() && ctx.view.get() == "bible">
                <StagePreview place=PreviewPlace::Bible />
            </Show>
        </aside>
        <BibleSlidesColumn />
        <BiblePresentationModal />
    }
}

// ---------------------------------------------------------------------------
// Tab navigation
// ---------------------------------------------------------------------------

#[component]
fn BibleTabNav() -> impl IntoView {
    let bs = use_ctx!(BibleState);
    let bible_tab = bs.bible_tab;

    let make_tab_click = move |tab: &'static str| {
        move |_| {
            bible_tab.set(tab.to_string());
        }
    };

    view! {
        <nav class="bible__tab-nav" data-role="bible-tab-nav">
            <button
                type="button"
                data-role="bible-tab"
                data-tab="live"
                data-active=move || if bible_tab.get() == "live" { "true" } else { "false" }
                on:click=make_tab_click("live")
            >"Naživo"</button>
            <button
                type="button"
                data-role="bible-tab"
                data-tab="prepared"
                data-active=move || if bible_tab.get() == "prepared" { "true" } else { "false" }
                on:click=make_tab_click("prepared")
            >"Pripravené"</button>
            <button
                type="button"
                data-role="bible-tab"
                data-tab="settings"
                data-active=move || if bible_tab.get() == "settings" { "true" } else { "false" }
                on:click=make_tab_click("settings")
            >"Nastavenie"</button>
        </nav>
    }
}

// ---------------------------------------------------------------------------
// Live tab
// ---------------------------------------------------------------------------

#[component]
fn BibleLiveTab() -> impl IntoView {
    let bs = use_ctx!(BibleState);
    let bible_tab = bs.bible_tab;

    view! {
        <div
            class="bible__tab-panel"
            data-bible-panel="live"
            data-visible=move || if bible_tab.get() == "live" { "true" } else { "false" }
        >
            <BibleConsole />
        </div>
    }
}

// SelectionControls (over the slide grid): bible_controls.rs; the Prepared tab: bible_prepared.rs
