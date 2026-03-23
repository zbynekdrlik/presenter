use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::api::bible;
use crate::state::bible::{BibleState, LoadedPassage, SelectedBook};
use crate::state::AppContext;

use super::bible_controls::{BibleSearch, SelectionControls};
use super::bible_slides::BibleSlidesColumn;

/// Bible page — 2-column layout matching the legacy Bible UI.
/// Rendered inside the operator shell's `<section data-view-panel="bible">`.
#[component]
pub fn BiblePage() -> impl IntoView {
    let ctx = use_ctx!(AppContext);
    let bs = BibleState::new();
    provide_context(bs.clone());

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

    // Load presentations
    {
        let presentations = bs.presentations;
        leptos::task::spawn_local(async move {
            if let Ok(pres) = bible::list_presentations().await {
                presentations.set(pres);
            }
        });
    }

    // Load books when translation changes
    {
        let selected_translation = bs.selected_translation;
        let books = bs.books;
        let selected_book = bs.selected_book;
        Effect::new(move || {
            let trans = selected_translation.get();
            if let Some(code) = trans {
                let books = books;
                let selected_book = selected_book;
                leptos::task::spawn_local(async move {
                    if let Ok(book_list) = bible::list_books(&code).await {
                        books.set(book_list);
                        selected_book.set(None);
                    }
                });
            }
        });
    }

    // Sync data-bible-tab on body for CSS
    {
        let bible_tab = bs.bible_tab;
        let view = ctx.view;
        Effect::new(move || {
            let tab = bible_tab.get();
            let v = view.get();
            if v == "bible" {
                if let Some(body) = crate::utils::window::document_body() {
                    let _ = body.set_attribute("data-bible-tab", &tab);
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

    view! {
        <div
            class="bible__tab-panel"
            data-bible-panel="live"
            data-visible=move || if bible_tab.get() == "live" { "true" } else { "false" }
        >
            <TranslationSelectors />
            <BibleSearch />
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
    let book_filter = bs.book_filter;

    let on_input = move |ev: web_sys::Event| {
        let target = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok());
        if let Some(input) = target {
            book_filter.set(input.value());
        }
    };

    view! {
        <label class="operator__field">
            <span>"Find book"</span>
            <input
                type="search"
                data-role="book-filter"
                placeholder="Start typing\u{2026}"
                prop:value=move || book_filter.get()
                on:input=on_input
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

                // If a book is already selected, collapse the list to just that book
                if let Some(selected) = selected_book.get() {
                    return view! {
                        <div class="operator__list-item">
                            <button
                                type="button"
                                class="operator__list-button"
                                data-role="book-item"
                                data-active="true"
                                on:click=move |_| { selected_book.set(None); }
                            >
                                <span class="operator__list-label">{selected.book.clone()}</span>
                                <span class="operator__list-meta">"Change"</span>
                            </button>
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
                                selected_book.set(Some(SelectedBook {
                                    book: book_name.clone(),
                                    code: code.clone(),
                                    number,
                                    chapter_count,
                                    verse_counts: verse_counts.clone(),
                                }));
                                selected_chapter.set(1);
                                verse_start.set(1);
                                verse_end.set(None);
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

    view! {
        <div class="operator__reference-grid">
            <label class="operator__field">
                <span>"Chapter"</span>
                <input
                    type="number"
                    data-role="chapter-input"
                    min="1"
                    prop:value=move || selected_chapter.get().to_string()
                    on:change=on_chapter
                />
            </label>
            <label class="operator__field">
                <span>"Verse start"</span>
                <input
                    type="number"
                    data-role="verse-start"
                    min="1"
                    prop:value=move || verse_start_signal.get().to_string()
                    on:change=on_verse_start
                />
            </label>
            <label class="operator__field">
                <span>"Verse end"</span>
                <input
                    type="number"
                    data-role="verse-end"
                    min="1"
                    prop:value=move || verse_end_signal.get().map(|v| v.to_string()).unwrap_or_default()
                    placeholder="All"
                    on:change=on_verse_end
                />
            </label>
        </div>
    }
}

#[component]
fn LoadButton() -> impl IntoView {
    let bs = use_ctx!(BibleState);
    let ctx = use_ctx!(AppContext);

    let on_load = move |_| {
        let selected_book = bs.selected_book.get_untracked();
        let Some(book) = selected_book else {
            ctx.show_toast("Select a book first", "error");
            return;
        };
        let main_trans = bs.selected_translation.get_untracked();
        let Some(main_code) = main_trans else {
            ctx.show_toast("Select a translation first", "error");
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

        // Build history label
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
                    // Push to history on successful load
                    history_signal.update(|history| {
                        history.retain(|p| p.label != history_entry.label);
                        history.insert(0, history_entry);
                        history.truncate(12);
                    });
                }
                Err(e) => {
                    toast_variant.set("error".to_string());
                    toast_message.set(Some(format!("Failed to load passage: {e}")));
                }
            }
            loading.set(false);
        });
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

// ---------------------------------------------------------------------------
// Prepared tab
// ---------------------------------------------------------------------------

#[component]
fn BiblePreparedTab() -> impl IntoView {
    let bs = use_ctx!(BibleState);
    let ctx = use_ctx!(AppContext);
    let bible_tab = bs.bible_tab;
    let presentations = bs.presentations;
    let active_presentation_id = bs.active_presentation_id;
    let edit_mode = bs.edit_mode;

    let on_create = {
        let ctx = ctx.clone();
        move |_| {
            let window = crate::utils::window::window();
            let name = match window.prompt_with_message("Presentation name:") {
                Ok(Some(n)) if !n.trim().is_empty() => n.trim().to_string(),
                _ => return,
            };
            let toast_message = ctx.toast_message;
            let toast_variant = ctx.toast_variant;
            leptos::task::spawn_local(async move {
                match bible::create_presentation(&name).await {
                    Ok(detail) => {
                        let new_id = detail.id.clone();
                        toast_variant.set("success".to_string());
                        toast_message.set(Some(format!("Created \"{}\"", detail.name)));
                        if let Ok(pres) = bible::list_presentations().await {
                            presentations.set(pres);
                        }
                        active_presentation_id.set(Some(new_id));
                    }
                    Err(e) => {
                        toast_variant.set("error".to_string());
                        toast_message.set(Some(format!("Failed to create: {e}")));
                    }
                }
            });
        }
    };

    view! {
        <div
            class="bible__tab-panel"
            data-bible-panel="prepared"
            data-visible=move || if bible_tab.get() == "prepared" { "true" } else { "false" }
        >
            <div class="bible__mode-toggle">
                <button
                    type="button"
                    data-role="bible-mode-live"
                    data-active=move || if !edit_mode.get() { "true" } else { "false" }
                    on:click=move |_| edit_mode.set(false)
                >"Live"</button>
                <button
                    type="button"
                    data-role="bible-mode-edit"
                    data-active=move || if edit_mode.get() { "true" } else { "false" }
                    on:click=move |_| edit_mode.set(true)
                >"Edit"</button>
            </div>
            <div class="bible__prepared-header">
                <h3>"Presentations"</h3>
                <button
                    type="button"
                    class="operator__list-action"
                    data-role="presentation-create"
                    aria-label="Create presentation"
                    on:click=on_create
                >"+"</button>
            </div>
            <div class="bible__prepared-list" data-role="presentations-list">
                {move || {
                    let pres_list = presentations.get();
                    if pres_list.is_empty() {
                        view! {
                            <p class="operator__slides-empty">"No Bible presentations yet."</p>
                        }.into_any()
                    } else {
                        pres_list.into_iter().map(|p| {
                            view! { <PresentationCard presentation=p /> }
                        }).collect_view().into_any()
                    }
                }}
            </div>
            <BiblePresentationModal />
        </div>
    }
}

#[component]
fn PresentationCard(presentation: bible::BiblePresentationSummary) -> impl IntoView {
    let bs = use_ctx!(BibleState);
    let active_presentation_id = bs.active_presentation_id;
    let active_presentation_slides = bs.active_presentation_slides;

    let id = presentation.id.clone();
    let name = presentation.name.clone();
    let count = presentation.slide_count;
    let id_for_click = id.clone();
    let id_for_edit = id.clone();
    let name_for_edit = name.clone();

    let is_active = {
        let id = id.clone();
        move || active_presentation_id.get().as_ref() == Some(&id)
    };

    let on_click = move |_| {
        let id = id_for_click.clone();
        active_presentation_id.set(Some(id.clone()));
        leptos::task::spawn_local(async move {
            if let Ok(detail) = bible::get_presentation(&id).await {
                active_presentation_slides.set(detail.slides);
            }
        });
    };

    let on_edit = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        bs.modal_presentation_id.set(Some(id_for_edit.clone()));
        bs.modal_presentation_name.set(name_for_edit.clone());
    };

    view! {
        <div
            class="operator__presentation-card"
            class:is-active=is_active
            data-presentation-id=id.clone()
            data-role="presentation-card"
            on:click=on_click
        >
            <div style="display:flex;justify-content:space-between;align-items:center;">
                <strong>{name.clone()}</strong>
                <button
                    type="button"
                    class="operator__presentation-action"
                    data-role="presentation-edit"
                    on:click=on_edit
                    title="Edit presentation"
                >"\u{270F}\u{FE0F}"</button>
            </div>
            <p>{format!("{} slide(s)", count)}</p>
        </div>
    }
}

#[component]
fn BiblePresentationModal() -> impl IntoView {
    let bs = use_ctx!(BibleState);
    let ctx = use_ctx!(AppContext);
    let modal_id = bs.modal_presentation_id;
    let modal_name = bs.modal_presentation_name;

    let is_open = move || modal_id.get().is_some();

    let on_close = move |_: web_sys::MouseEvent| {
        modal_id.set(None);
        modal_name.set(String::new());
    };

    let on_name_input = move |ev: web_sys::Event| {
        let target = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok());
        if let Some(input) = target {
            modal_name.set(input.value());
        }
    };

    let on_save = {
        let ctx = ctx.clone();
        move |ev: web_sys::SubmitEvent| {
            ev.prevent_default();
            let Some(id) = modal_id.get_untracked() else {
                return;
            };
            let new_name = modal_name.get_untracked().trim().to_string();
            if new_name.is_empty() {
                return;
            }
            let presentations = bs.presentations;
            let toast_message = ctx.toast_message;
            let toast_variant = ctx.toast_variant;
            leptos::task::spawn_local(async move {
                match bible::rename_presentation(&id, &new_name).await {
                    Ok(()) => {
                        toast_variant.set("success".to_string());
                        toast_message.set(Some("Renamed".to_string()));
                        if let Ok(pres) = bible::list_presentations().await {
                            presentations.set(pres);
                        }
                        modal_id.set(None);
                        modal_name.set(String::new());
                    }
                    Err(e) => {
                        toast_variant.set("error".to_string());
                        toast_message.set(Some(format!("Rename failed: {e}")));
                    }
                }
            });
        }
    };

    let on_delete = {
        let ctx = ctx.clone();
        move |_: web_sys::MouseEvent| {
            let Some(id) = modal_id.get_untracked() else {
                return;
            };
            let window = crate::utils::window::window();
            if let Ok(confirmed) = window.confirm_with_message("Delete this presentation?") {
                if !confirmed {
                    return;
                }
            }
            let presentations = bs.presentations;
            let active_id = bs.active_presentation_id;
            let active_slides = bs.active_presentation_slides;
            let toast_message = ctx.toast_message;
            let toast_variant = ctx.toast_variant;
            leptos::task::spawn_local(async move {
                match bible::delete_presentation(&id).await {
                    Ok(()) => {
                        active_id.set(None);
                        active_slides.set(Vec::new());
                        toast_variant.set("success".to_string());
                        toast_message.set(Some("Deleted".to_string()));
                        if let Ok(pres) = bible::list_presentations().await {
                            presentations.set(pres);
                        }
                        modal_id.set(None);
                        modal_name.set(String::new());
                    }
                    Err(e) => {
                        toast_variant.set("error".to_string());
                        toast_message.set(Some(format!("Delete failed: {e}")));
                    }
                }
            });
        }
    };

    let on_backdrop = move |ev: web_sys::MouseEvent| {
        // Close only if clicking the backdrop itself
        let target = ev.target();
        let current = ev.current_target();
        if target == current {
            modal_id.set(None);
            modal_name.set(String::new());
        }
    };

    view! {
        <div
            class="bible__modal-overlay"
            data-role="presentation-modal"
            style:display=move || if is_open() { "flex" } else { "none" }
            on:click=on_backdrop
        >
            <form class="bible__modal" on:submit=on_save>
                <h3>"Edit presentation"</h3>
                    <label class="operator__field">
                        <span>"Name"</span>
                        <input
                            type="text"
                            data-role="modal-presentation-name"
                            prop:value=move || modal_name.get()
                            on:input=on_name_input
                        />
                    </label>
                    <div class="bible__modal-actions">
                        <button
                            type="button"
                            class="bible__modal-btn bible__modal-btn--danger"
                            data-role="modal-delete"
                            on:click=on_delete
                        >"Delete"</button>
                        <div class="bible__modal-actions-right">
                            <button
                                type="button"
                                class="bible__modal-btn"
                                data-role="modal-cancel"
                                on:click=on_close
                            >"Cancel"</button>
                            <button
                                type="submit"
                                class="bible__modal-btn bible__modal-btn--primary"
                                data-role="modal-save"
                            >"Save"</button>
                        </div>
                    </div>
            </form>
        </div>
    }
}
// ---------------------------------------------------------------------------
// Settings tab
// ---------------------------------------------------------------------------

#[component]
fn BibleSettingsTab() -> impl IntoView {
    let bs = use_ctx!(BibleState);
    let ctx = use_ctx!(AppContext);
    let bible_tab = bs.bible_tab;
    let character_limit = bs.character_limit;

    let on_char_limit_change = move |ev: web_sys::Event| {
        let target = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok());
        if let Some(input) = target {
            if let Ok(val) = input.value().parse::<u32>() {
                character_limit.set(val.clamp(1, 4000));
            }
        }
    };

    let on_save = {
        let ctx = ctx.clone();
        move |_| {
            let limit = character_limit.get_untracked();
            let main = bs.selected_translation.get_untracked();
            let sec = bs.secondary_translation.get_untracked();
            let toast_message = ctx.toast_message;
            let toast_variant = ctx.toast_variant;

            let draft = presenter_core::BiblePreferencesDraft {
                main_translation: main,
                secondary_translation: sec,
                character_limit: Some(limit),
            };

            leptos::task::spawn_local(async move {
                match bible::update_preferences(&draft).await {
                    Ok(()) => {
                        toast_variant.set("success".to_string());
                        toast_message.set(Some("Preferences saved".to_string()));
                    }
                    Err(e) => {
                        toast_variant.set("error".to_string());
                        toast_message.set(Some(format!("Save failed: {e}")));
                    }
                }
            });
        }
    };

    view! {
        <div
            class="bible__tab-panel"
            data-bible-panel="settings"
            data-visible=move || if bible_tab.get() == "settings" { "true" } else { "false" }
        >
            <div class="operator__form-group">
                <label class="operator__field">
                    <span>"Character limit"</span>
                    <input
                        type="number"
                        data-role="char-limit"
                        min="1"
                        max="4000"
                        prop:value=move || character_limit.get().to_string()
                        on:change=on_char_limit_change
                    />
                </label>
                <button
                    type="button"
                    class="operator__list-action operator__list-action--primary"
                    data-role="save-preferences"
                    on:click=on_save
                >"Save preferences"</button>
            </div>
        </div>
    }
}
