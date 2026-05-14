use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::api::bible;
use crate::state::bible::{BibleState, LoadedPassage, SelectedBook};
use crate::state::AppContext;

use super::bible_controls::SelectionControls;
use super::bible_prepared::{BiblePreparedTab, BiblePresentationModal, BibleSettingsTab};
use super::bible_slides::BibleSlidesColumn;

/// Shared `NodeRef` handles for the four inputs that participate in the
/// keyboard-navigation chain on the Bible live tab. Provided via context so
/// each input's `on:keydown` handler can step focus to the next input.
#[derive(Copy, Clone)]
struct BibleFocusRefs {
    book_filter: NodeRef<leptos::html::Input>,
    chapter: NodeRef<leptos::html::Input>,
    verse_start: NodeRef<leptos::html::Input>,
    verse_end: NodeRef<leptos::html::Input>,
}

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

    // Load translations + preferences on mount
    {
        let translations = bs.translations;
        let selected_translation = bs.selected_translation;
        let secondary_translation = bs.secondary_translation;
        let character_limit = bs.character_limit;
        leptos::task::spawn_local(async move {
            // Load preferences first to set saved translation choices
            if let Ok(prefs) = bible::get_preferences().await {
                if let Some(ref main) = prefs.main_translation {
                    selected_translation.set(Some(main.clone()));
                }
                if let Some(ref sec) = prefs.secondary_translation {
                    secondary_translation.set(Some(sec.clone()));
                }
                character_limit.set(prefs.character_limit);
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
        let presentations = bs.presentations;
        let version = ctx.bible_presentations_version;
        leptos::task::spawn_local(async move {
            if let Ok(pres) = bible::list_presentations().await {
                presentations.set(pres);
            }
        });
        Effect::new(move || {
            let _v = version.get(); // track the signal
            leptos::task::spawn_local(async move {
                if let Ok(pres) = bible::list_presentations().await {
                    presentations.set(pres);
                }
            });
        });
    }

    // Load books when translation changes.
    // If the currently-selected book exists in the new translation (matched
    // by book code), preserve the selection and clamp chapter/verse against
    // the new book's structure. Otherwise clear the selection.
    {
        let selected_translation = bs.selected_translation;
        let books = bs.books;
        let selected_book = bs.selected_book;
        let selected_chapter = bs.selected_chapter;
        let verse_start = bs.verse_start;
        let verse_end = bs.verse_end;
        Effect::new(move || {
            let trans = selected_translation.get();
            if let Some(code) = trans {
                leptos::task::spawn_local(async move {
                    let Ok(book_list) = bible::list_books(&code).await else {
                        return;
                    };
                    let current = selected_book.get_untracked();
                    let current_chapter = selected_chapter.get_untracked();
                    let current_v_start = verse_start.get_untracked();
                    let current_v_end = verse_end.get_untracked();
                    books.set(book_list.clone());

                    let Some(current) = current else {
                        return;
                    };
                    // Find the same book (by code) in the new translation
                    let Some(new_book) = book_list.iter().find(|b| b.code == current.code) else {
                        // Book doesn't exist in new translation - clear selection
                        selected_book.set(None);
                        return;
                    };
                    let chapter_count = new_book.chapters.len() as u16;
                    let verse_counts: Vec<u16> =
                        new_book.chapters.iter().map(|c| c.verse_count).collect();
                    let clamped = crate::state::bible::clamp_selection(
                        chapter_count,
                        &verse_counts,
                        current_chapter,
                        current_v_start,
                        current_v_end,
                    );
                    selected_book.set(Some(SelectedBook {
                        book: new_book.book.clone(),
                        code: new_book.code.clone(),
                        number: new_book.number,
                        chapter_count,
                        verse_counts,
                    }));
                    selected_chapter.set(clamped.chapter);
                    verse_start.set(clamped.verse_start);
                    verse_end.set(clamped.verse_end);
                });
            }
        });
    }

    // Debounced auto-load: when chapter / verse_start / verse_end change, wait
    // 300ms then resolve the passage. Rapid typing only fires one request when
    // the user stops.
    {
        use std::cell::RefCell;
        use std::rc::Rc;
        let bs_inner = bs.clone();
        let ctx_inner = ctx.clone();
        let chapter_sig = bs.selected_chapter;
        let v_start_sig = bs.verse_start;
        let v_end_sig = bs.verse_end;
        let pending: Rc<RefCell<Option<gloo_timers::callback::Timeout>>> =
            Rc::new(RefCell::new(None));
        Effect::new(move |prev: Option<()>| {
            // Track the three signals.
            let _c = chapter_sig.get();
            let _vs = v_start_sig.get();
            let _ve = v_end_sig.get();
            // Skip the very first run (initial signal reads, not a user change).
            if prev.is_none() {
                return;
            }
            // Replace any pending timer with a new one.
            let bs_for_timer = bs_inner.clone();
            let ctx_for_timer = ctx_inner.clone();
            let new_timer = gloo_timers::callback::Timeout::new(300, move || {
                load_passage(&bs_for_timer, &ctx_for_timer, false);
            });
            *pending.borrow_mut() = Some(new_timer);
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
            >"Live"</button>
            <button
                type="button"
                data-role="bible-tab"
                data-tab="prepared"
                data-active=move || if bible_tab.get() == "prepared" { "true" } else { "false" }
                on:click=make_tab_click("prepared")
            >"Prepared"</button>
            <button
                type="button"
                data-role="bible-tab"
                data-tab="settings"
                data-active=move || if bible_tab.get() == "settings" { "true" } else { "false" }
                on:click=make_tab_click("settings")
            >"Settings"</button>
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

    // Shared focus chain for keyboard navigation (#257).
    let refs = BibleFocusRefs {
        book_filter: NodeRef::<leptos::html::Input>::new(),
        chapter: NodeRef::<leptos::html::Input>::new(),
        verse_start: NodeRef::<leptos::html::Input>::new(),
        verse_end: NodeRef::<leptos::html::Input>::new(),
    };
    provide_context(refs);

    // Auto-focus the book-filter on first mount so the operator can start
    // typing immediately without clicking. `NodeRef::on_load` runs once when
    // the element first appears in the DOM.
    refs.book_filter.on_load(|el| {
        let _ = el.focus();
    });

    view! {
        <div
            class="bible__tab-panel"
            data-bible-panel="live"
            data-visible=move || if bible_tab.get() == "live" { "true" } else { "false" }
        >
            <TranslationSelectors />
            <BookFilter />
            <BookList />
            <ReferenceInputs />
            <LoadButton />
            <SelectionControls />
        </div>
    }
}

#[component]
fn TranslationSelectors() -> impl IntoView {
    let bs = use_ctx!(BibleState);
    let translations = bs.translations;
    let selected_translation = bs.selected_translation;
    let secondary_translation = bs.secondary_translation;

    let on_main_change = move |ev: web_sys::Event| {
        let target = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlSelectElement>().ok());
        if let Some(select) = target {
            let val = select.value();
            if !val.is_empty() {
                selected_translation.set(Some(val));
            }
        }
    };

    let on_secondary_change = move |ev: web_sys::Event| {
        let target = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlSelectElement>().ok());
        if let Some(select) = target {
            let val = select.value();
            secondary_translation.set(if val.is_empty() { None } else { Some(val) });
        }
    };

    view! {
        <div class="bible__translation-row">
            <label class="operator__field operator__field--compact">
                <span>"Main"</span>
                <select data-role="main-translation" on:change=on_main_change>
                    {move || {
                        let current = selected_translation.get();
                        translations.get().into_iter().map(|t| {
                            let code = t.code.clone();
                            let is_selected = current.as_ref() == Some(&code);
                            let label = if t.language.is_empty() {
                                t.name.clone()
                            } else {
                                format!("{} ({})", t.name, t.language)
                            };
                            view! {
                                <option value=code selected=is_selected>{label}</option>
                            }
                        }).collect_view()
                    }}
                </select>
            </label>
            <label class="operator__field operator__field--compact">
                <span>"Secondary"</span>
                <select data-role="secondary-translation" on:change=on_secondary_change>
                    <option value="">"None"</option>
                    {move || {
                        let current = secondary_translation.get();
                        translations.get().into_iter().map(|t| {
                            let code = t.code.clone();
                            let is_selected = current.as_ref() == Some(&code);
                            let label = if t.language.is_empty() {
                                t.name.clone()
                            } else {
                                format!("{} ({})", t.name, t.language)
                            };
                            view! {
                                <option value=code selected=is_selected>{label}</option>
                            }
                        }).collect_view()
                    }}
                </select>
            </label>
        </div>
    }
}

#[component]
fn BookFilter() -> impl IntoView {
    let bs = use_ctx!(BibleState);
    let refs = expect_context::<BibleFocusRefs>();
    let book_filter = bs.book_filter;

    let on_input = move |ev: web_sys::Event| {
        let target = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok());
        if let Some(input) = target {
            book_filter.set(input.value());
        }
    };

    let on_keydown = {
        let bs = bs.clone();
        move |ev: web_sys::KeyboardEvent| {
            if ev.key() != "Enter" {
                return;
            }
            ev.prevent_default();
            // Pick the first filtered book only when the filter is non-empty;
            // an empty filter is the post-chain return to book-filter, where
            // pressing Enter should not stomp the currently selected book.
            let filter_text = book_filter.get_untracked();
            if !filter_text.trim().is_empty() {
                let books = bs.filtered_books();
                if let Some(book) = books.first().cloned() {
                    let chapter_count = book.chapters.len() as u16;
                    let verse_counts: Vec<u16> =
                        book.chapters.iter().map(|c| c.verse_count).collect();
                    let current_chapter = bs.selected_chapter.get_untracked();
                    let current_v_start = bs.verse_start.get_untracked();
                    let current_v_end = bs.verse_end.get_untracked();
                    let clamped = crate::state::bible::clamp_selection(
                        chapter_count,
                        &verse_counts,
                        current_chapter,
                        current_v_start,
                        current_v_end,
                    );
                    bs.selected_book.set(Some(SelectedBook {
                        book: book.book.clone(),
                        code: book.code.clone(),
                        number: book.number,
                        chapter_count,
                        verse_counts,
                    }));
                    bs.selected_chapter.set(clamped.chapter);
                    bs.verse_start.set(clamped.verse_start);
                    bs.verse_end.set(clamped.verse_end);
                    book_filter.set(String::new());
                }
            }
            if let Some(el) = refs.chapter.get() {
                let _ = el.focus();
                el.select();
            }
        }
    };

    view! {
        <label class="operator__field">
            <span>"Find book"</span>
            <input
                type="search"
                data-role="book-filter"
                placeholder="Start typing\u{2026}"
                node_ref=refs.book_filter
                prop:value=move || book_filter.get()
                on:input=on_input
                on:keydown=on_keydown
            />
        </label>
    }
}

#[component]
fn BookList() -> impl IntoView {
    let bs = use_ctx!(BibleState);

    view! {
        <div class="operator__list operator__list--tight" data-role="book-list">
            {move || {
                let filtered = bs.filtered_books();
                let selected_book = bs.selected_book;
                let selected_chapter = bs.selected_chapter;
                let verse_start = bs.verse_start;
                let verse_end = bs.verse_end;
                let book_filter = bs.book_filter;

                // When a book is selected AND the filter is empty, collapse
                // the list to show just the selected book. Typing in the
                // filter expands the list again with matching books.
                let collapsed = selected_book
                    .get()
                    .filter(|_| book_filter.get().is_empty());
                if let Some(selected) = collapsed {
                    return view! {
                        <div class="operator__list-item">
                            <div
                                class="operator__list-button"
                                data-role="book-item"
                                data-active="true"
                            >
                                <span class="operator__list-label">{selected.book.clone()}</span>
                            </div>
                        </div>
                    }.into_any();
                }

                if filtered.is_empty() {
                    view! {
                        <p class="operator__slides-empty">"No books found."</p>
                    }.into_any()
                } else {
                    filtered.into_iter().map(|book| {
                        let book_name = book.book.clone();
                        let code = book.code.clone();
                        let number = book.number;
                        let chapter_count = book.chapters.len() as u16;
                        let verse_counts: Vec<u16> = book.chapters.iter().map(|c| c.verse_count).collect();
                        let display_name = book_name.clone();

                        let is_active = {
                            let code = code.clone();
                            move || {
                                selected_book.get()
                                    .as_ref()
                                    .map(|sb| sb.code == code)
                                    .unwrap_or(false)
                            }
                        };

                        let on_click = {
                            let book_name = book_name.clone();
                            let code = code.clone();
                            let verse_counts = verse_counts.clone();
                            move |_| {
                                // Preserve chapter/verse if they fit the new book.
                                let current_chapter = selected_chapter.get_untracked();
                                let current_v_start = verse_start.get_untracked();
                                let current_v_end = verse_end.get_untracked();
                                let clamped = crate::state::bible::clamp_selection(
                                    chapter_count,
                                    &verse_counts,
                                    current_chapter,
                                    current_v_start,
                                    current_v_end,
                                );
                                selected_book.set(Some(SelectedBook {
                                    book: book_name.clone(),
                                    code: code.clone(),
                                    number,
                                    chapter_count,
                                    verse_counts: verse_counts.clone(),
                                }));
                                selected_chapter.set(clamped.chapter);
                                verse_start.set(clamped.verse_start);
                                verse_end.set(clamped.verse_end);
                                // Clear the filter so the list collapses and
                                // is ready for the next search.
                                book_filter.set(String::new());
                            }
                        };

                        view! {
                            <div class="operator__list-item">
                                <button
                                    type="button"
                                    class="operator__list-button"
                                    data-role="book-item"
                                    data-book-code=code
                                    data-active=move || if is_active() { "true" } else { "false" }
                                    on:click=on_click
                                >
                                    <span class="operator__list-label">{display_name}</span>
                                    <span class="operator__list-meta">{chapter_count}" ch."</span>
                                </button>
                            </div>
                        }
                    }).collect_view().into_any()
                }
            }}
        </div>
    }
}

#[component]
fn ReferenceInputs() -> impl IntoView {
    let bs = use_ctx!(BibleState);
    let refs = expect_context::<BibleFocusRefs>();
    let book_filter = bs.book_filter;
    let selected_chapter = bs.selected_chapter;
    let verse_start_signal = bs.verse_start;
    let verse_end_signal = bs.verse_end;
    let on_chapter = move |ev: web_sys::Event| {
        let target = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok());
        if let Some(input) = target {
            if let Ok(val) = input.value().parse::<u16>() {
                selected_chapter.set(val.max(1));
                verse_start_signal.set(1);
                verse_end_signal.set(None);
            }
        }
    };

    let on_verse_start = move |ev: web_sys::Event| {
        let target = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok());
        if let Some(input) = target {
            if let Ok(val) = input.value().parse::<u16>() {
                verse_start_signal.set(val.max(1));
            }
        }
    };

    let on_verse_end = move |ev: web_sys::Event| {
        let target = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok());
        if let Some(input) = target {
            let val_str = input.value();
            if val_str.is_empty() {
                verse_end_signal.set(None);
            } else if let Ok(val) = val_str.parse::<u16>() {
                verse_end_signal.set(Some(val.max(1)));
            }
        }
    };

    // Enter on chapter → commit chapter value, jump to verse-start.
    let on_chapter_keydown = move |ev: web_sys::KeyboardEvent| {
        if ev.key() != "Enter" {
            return;
        }
        ev.prevent_default();
        if let Some(input) = refs.chapter.get() {
            if let Ok(val) = input.value().parse::<u16>() {
                selected_chapter.set(val.max(1));
                verse_start_signal.set(1);
                verse_end_signal.set(None);
            }
        }
        if let Some(el) = refs.verse_start.get() {
            let _ = el.focus();
            el.select();
        }
    };

    // Enter on verse-start → commit value, jump to verse-end.
    let on_verse_start_keydown = move |ev: web_sys::KeyboardEvent| {
        if ev.key() != "Enter" {
            return;
        }
        ev.prevent_default();
        if let Some(input) = refs.verse_start.get() {
            if let Ok(val) = input.value().parse::<u16>() {
                verse_start_signal.set(val.max(1));
            }
        }
        if let Some(el) = refs.verse_end.get() {
            let _ = el.focus();
            el.select();
        }
    };

    // Enter on verse-end → commit (or clear) value, return to book-filter.
    // The debounced auto-load effect (`bible.rs` mount-time) already fires
    // a passage fetch 300ms after the signal updates, so no explicit load
    // click is needed here. Clearing the filter collapses the book list so
    // the operator can immediately start typing the next book.
    let on_verse_end_keydown = move |ev: web_sys::KeyboardEvent| {
        if ev.key() != "Enter" {
            return;
        }
        ev.prevent_default();
        if let Some(input) = refs.verse_end.get() {
            let val_str = input.value();
            if val_str.is_empty() {
                verse_end_signal.set(None);
            } else if let Ok(val) = val_str.parse::<u16>() {
                verse_end_signal.set(Some(val.max(1)));
            }
            let _ = input.blur();
        }
        book_filter.set(String::new());
        if let Some(el) = refs.book_filter.get() {
            let _ = el.focus();
        }
    };

    view! {
        <div class="operator__reference-grid">
            <label class="operator__field">
                <span>"Chapter"</span>
                <input
                    type="number"
                    data-role="chapter-input"
                    min="1"
                    node_ref=refs.chapter
                    prop:value=move || selected_chapter.get().to_string()
                    on:change=on_chapter
                    on:keydown=on_chapter_keydown
                />
            </label>
            <label class="operator__field">
                <span>"Verse start"</span>
                <input
                    type="number"
                    data-role="verse-start"
                    min="1"
                    node_ref=refs.verse_start
                    prop:value=move || verse_start_signal.get().to_string()
                    on:change=on_verse_start
                    on:keydown=on_verse_start_keydown
                />
            </label>
            <label class="operator__field">
                <span>"Verse end"</span>
                <input
                    type="number"
                    data-role="verse-end"
                    min="1"
                    node_ref=refs.verse_end
                    prop:value=move || verse_end_signal.get().map(|v| v.to_string()).unwrap_or_default()
                    placeholder="All"
                    on:change=on_verse_end
                    on:keydown=on_verse_end_keydown
                />
            </label>
        </div>
    }
}

/// Resolve the currently-selected passage and update state. Called by both
/// the manual Load button and the debounced auto-load effect. Silently no-ops
/// when the selection is incomplete. `show_errors` controls whether missing-
/// selection and resolve errors surface as toasts (true for button, false for
/// debounced auto-load to avoid toast spam while the user is still typing).
fn load_passage(bs: &BibleState, ctx: &AppContext, show_errors: bool) {
    let Some(book) = bs.selected_book.get_untracked() else {
        if show_errors {
            ctx.show_toast("Select a book first", "error");
        }
        return;
    };
    let Some(main_code) = bs.selected_translation.get_untracked() else {
        if show_errors {
            ctx.show_toast("Select a translation first", "error");
        }
        return;
    };
    let secondary = bs.secondary_translation.get_untracked();
    let chapter = bs.selected_chapter.get_untracked();
    let v_start = bs.verse_start.get_untracked();
    let v_end = bs.verse_end.get_untracked();
    let char_limit = bs.character_limit.get_untracked();

    let slides = bs.slides;
    let loading = bs.loading_slides;
    let selected_ids = bs.selected_slide_ids;
    let toast_message = ctx.toast_message;
    let toast_variant = ctx.toast_variant;

    loading.set(true);
    selected_ids.set(std::collections::HashSet::new());

    let label = if let Some(ve) = v_end {
        format!("{} {}:{}-{}", book.book, chapter, v_start, ve)
    } else {
        format!("{} {}:{}", book.book, chapter, v_start)
    };
    let history_entry = LoadedPassage {
        book: book.book.clone(),
        book_code: book.code.clone(),
        book_number: book.number,
        chapter,
        verse_start: v_start,
        verse_end: v_end,
        translation_code: main_code.clone(),
        label,
    };

    let req = bible::ResolveRequest {
        main_translation: main_code,
        secondary_translation: secondary.filter(|s| !s.is_empty()),
        book: book.book,
        book_code: Some(book.code),
        chapter,
        verse_start: v_start,
        verse_end: v_end,
        character_limit: Some(char_limit),
    };

    let history_signal = bs.loaded_passages_history;
    leptos::task::spawn_local(async move {
        match bible::resolve_slides(&req).await {
            Ok(resp) => {
                slides.set(resp.slides);
                history_signal.update(|history| {
                    history.retain(|p| p.label != history_entry.label);
                    history.insert(0, history_entry);
                    history.truncate(12);
                });
            }
            Err(e) => {
                if show_errors {
                    toast_variant.set("error".to_string());
                    toast_message.set(Some(format!("Failed to load passage: {e}")));
                }
            }
        }
        loading.set(false);
    });
}

#[component]
fn LoadButton() -> impl IntoView {
    let bs = use_ctx!(BibleState);
    let ctx = use_ctx!(AppContext);

    let on_load = {
        let bs = bs.clone();
        let ctx = ctx.clone();
        move |_| {
            load_passage(&bs, &ctx, true);
        }
    };

    let is_disabled = move || {
        bs.selected_book.get().is_none()
            || bs.selected_translation.get().is_none()
            || bs.loading_slides.get()
    };

    view! {
        <button
            type="button"
            class="operator__list-action operator__list-action--primary"
            data-role="load-button"
            on:click=on_load
            disabled=is_disabled
        >
            {move || if bs.loading_slides.get() { "Loading\u{2026}" } else { "Load passage" }}
        </button>
    }
}

// SelectionControls and AddToPresentationButtons are in bible_controls.rs
// BiblePreparedTab, PresentationCard, BiblePresentationModal, BibleSettingsTab are in bible_prepared.rs
