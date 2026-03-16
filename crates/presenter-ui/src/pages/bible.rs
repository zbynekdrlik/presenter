use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::api::bible;
use crate::state::bible::{BibleState, SelectedBook};
use crate::state::AppContext;

use super::bible_slides::BibleSlidesColumn;

/// Bible page — 2-column layout matching the legacy Bible UI.
/// Rendered inside the operator shell's `<section data-view-panel="bible">`.
#[component]
pub fn BiblePage() -> impl IntoView {
    let ctx = use_context::<AppContext>().expect("AppContext");
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
    let bs = use_context::<BibleState>().expect("BibleState");
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
                attr:data-active=move || if bible_tab.get() == "live" { "true" } else { "false" }
                on:click=make_tab_click("live")
            >"Live"</button>
            <button
                type="button"
                data-role="bible-tab"
                data-tab="prepared"
                attr:data-active=move || if bible_tab.get() == "prepared" { "true" } else { "false" }
                on:click=make_tab_click("prepared")
            >"Prepared"</button>
            <button
                type="button"
                data-role="bible-tab"
                data-tab="settings"
                attr:data-active=move || if bible_tab.get() == "settings" { "true" } else { "false" }
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
    let bs = use_context::<BibleState>().expect("BibleState");
    let bible_tab = bs.bible_tab;

    view! {
        <div
            class="bible__tab-panel"
            data-bible-panel="live"
            attr:data-visible=move || if bible_tab.get() == "live" { "true" } else { "false" }
        >
            <TranslationSelectors />
            <BookFilter />
            <BookList />
            <ReferenceInputs />
            <LoadButton />
            <hr class="operator__divider" />
            <SelectionControls />
        </div>
    }
}

#[component]
fn TranslationSelectors() -> impl IntoView {
    let bs = use_context::<BibleState>().expect("BibleState");
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
    let bs = use_context::<BibleState>().expect("BibleState");
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
    let bs = use_context::<BibleState>().expect("BibleState");

    view! {
        <div class="operator__list operator__list--tight" data-role="book-list">
            {move || {
                let filtered = bs.filtered_books();
                let selected_book = bs.selected_book;
                let selected_chapter = bs.selected_chapter;
                let verse_start = bs.verse_start;
                let verse_end = bs.verse_end;

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
                            <button
                                type="button"
                                class="operator__list-item"
                                data-role="book-item"
                                data-book-code=code
                                attr:data-active=move || if is_active() { "true" } else { "false" }
                                on:click=on_click
                            >
                                {display_name}
                            </button>
                        }
                    }).collect_view().into_any()
                }
            }}
        </div>
    }
}

#[component]
fn ReferenceInputs() -> impl IntoView {
    let bs = use_context::<BibleState>().expect("BibleState");
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
    let bs = use_context::<BibleState>().expect("BibleState");
    let ctx = use_context::<AppContext>().expect("AppContext");

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

        leptos::task::spawn_local(async move {
            match bible::resolve_slides(&req).await {
                Ok(resp) => {
                    slides.set(resp.slides);
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

#[component]
fn SelectionControls() -> impl IntoView {
    let bs = use_context::<BibleState>().expect("BibleState");
    let ctx = use_context::<AppContext>().expect("AppContext");

    let selected_count = move || bs.selected_slide_ids.get().len();

    let on_select_all = move |_| {
        let all_ids: std::collections::HashSet<String> =
            bs.slides.get().iter().map(|s| s.id.clone()).collect();
        bs.selected_slide_ids.set(all_ids);
    };

    // Presentation selector for "Add to..."
    let presentations = bs.presentations;

    let on_pres_change = move |ev: web_sys::Event| {
        let target = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlSelectElement>().ok());
        if let Some(select) = target {
            let val = select.value();
            bs.active_presentation_id
                .set(if val.is_empty() { None } else { Some(val) });
        }
    };

    let on_add_selected = {
        let bs = bs.clone();
        let ctx = ctx.clone();
        move |_| {
            let pres_id = bs.active_presentation_id.get_untracked();
            let Some(pres_id) = pres_id else {
                ctx.show_toast("Select a presentation first", "error");
                return;
            };
            let selected_ids = bs.selected_slide_ids.get_untracked();
            if selected_ids.is_empty() {
                ctx.show_toast("No slides selected", "error");
                return;
            }
            let slides = bs.slides.get_untracked();
            let inputs: Vec<bible::AppendSlideInput> = slides
                .iter()
                .filter(|s| selected_ids.contains(&s.id))
                .map(|s| bible::AppendSlideInput {
                    main: s.main.clone(),
                    translation: s.translation.clone(),
                    stage: s.stage.clone(),
                    group: s.group.clone(),
                })
                .collect();
            if inputs.is_empty() {
                return;
            }

            let bs_pres = bs.presentations;
            let toast_message = ctx.toast_message;
            let toast_variant = ctx.toast_variant;
            leptos::task::spawn_local(async move {
                match bible::append_presentation_slides(&pres_id, &inputs).await {
                    Ok(_) => {
                        toast_variant.set("success".to_string());
                        toast_message.set(Some(format!(
                            "Added {} slide(s) to presentation",
                            inputs.len()
                        )));
                        // Refresh presentations list to update slide count
                        if let Ok(pres) = bible::list_presentations().await {
                            bs_pres.set(pres);
                        }
                    }
                    Err(e) => {
                        toast_variant.set("error".to_string());
                        toast_message.set(Some(format!("Failed to add slides: {e}")));
                    }
                }
            });
        }
    };

    view! {
        <span data-role="selection-count" class="operator__slides-count">
            {move || format!("{} selected", selected_count())}
        </span>
        <button
            type="button"
            class="operator__list-action"
            data-role="select-all-slides"
            on:click=on_select_all
        >"Select all"</button>
        <label class="operator__field">
            <select data-role="presentation-select" on:change=on_pres_change>
                <option value="">"Add to\u{2026}"</option>
                {move || {
                    presentations.get().into_iter().map(|p| {
                        let id = p.id.clone();
                        let label = format!("{} ({} slides)", p.name, p.slide_count);
                        view! {
                            <option value=id>{label}</option>
                        }
                    }).collect_view()
                }}
            </select>
        </label>
        <button
            type="button"
            class="operator__list-action operator__list-action--primary"
            data-role="presentation-add"
            on:click=on_add_selected
        >"Add selected"</button>
    }
}

// ---------------------------------------------------------------------------
// Prepared tab
// ---------------------------------------------------------------------------

#[component]
fn BiblePreparedTab() -> impl IntoView {
    let bs = use_context::<BibleState>().expect("BibleState");
    let ctx = use_context::<AppContext>().expect("AppContext");
    let bible_tab = bs.bible_tab;
    let presentations = bs.presentations;
    let active_presentation_id = bs.active_presentation_id;
    let active_presentation_slides = bs.active_presentation_slides;

    let on_create = {
        let ctx = ctx.clone();
        move |_| {
            let presentations = presentations;
            let toast_message = ctx.toast_message;
            let toast_variant = ctx.toast_variant;
            let name = "New Presentation".to_string();
            leptos::task::spawn_local(async move {
                match bible::create_presentation(&name).await {
                    Ok(detail) => {
                        toast_variant.set("success".to_string());
                        toast_message.set(Some(format!("Created \"{}\"", detail.name)));
                        if let Ok(pres) = bible::list_presentations().await {
                            presentations.set(pres);
                        }
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
            attr:data-visible=move || if bible_tab.get() == "prepared" { "true" } else { "false" }
        >
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
                            let id = p.id.clone();
                            let name = p.name.clone();
                            let count = p.slide_count;
                            let id_for_click = id.clone();
                            let id_for_edit = id.clone();

                            let is_active = {
                                let id = id.clone();
                                move || active_presentation_id.get().as_ref() == Some(&id)
                            };

                            let on_click = {
                                let active_presentation_id = active_presentation_id;
                                let active_presentation_slides = active_presentation_slides;
                                move |_| {
                                    let id = id_for_click.clone();
                                    active_presentation_id.set(Some(id.clone()));
                                    let slides_signal = active_presentation_slides;
                                    leptos::task::spawn_local(async move {
                                        if let Ok(detail) = bible::get_presentation(&id).await {
                                            slides_signal.set(detail.slides);
                                        }
                                    });
                                }
                            };

                            let name_for_edit = name.clone();
                            let on_edit = {
                                let ctx = ctx.clone();
                                let presentations = presentations;
                                move |ev: web_sys::MouseEvent| {
                                    ev.stop_propagation();
                                    let id = id_for_edit.clone();
                                    let toast_message = ctx.toast_message;
                                    let toast_variant = ctx.toast_variant;
                                    let presentations = presentations;

                                    // Use JS prompt for rename (simple approach matching legacy)
                                    let window = crate::utils::window::window();
                                    if let Ok(Some(new_name)) = window.prompt_with_message_and_default(
                                        "Rename presentation:",
                                        &name_for_edit,
                                    ) {
                                        let new_name = new_name.trim().to_string();
                                        if !new_name.is_empty() {
                                            leptos::task::spawn_local(async move {
                                                match bible::rename_presentation(&id, &new_name).await {
                                                    Ok(()) => {
                                                        toast_variant.set("success".to_string());
                                                        toast_message.set(Some("Renamed".to_string()));
                                                        if let Ok(pres) = bible::list_presentations().await {
                                                            presentations.set(pres);
                                                        }
                                                    }
                                                    Err(e) => {
                                                        toast_variant.set("error".to_string());
                                                        toast_message.set(Some(format!("Rename failed: {e}")));
                                                    }
                                                }
                                            });
                                        }
                                    }
                                }
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
                        }).collect_view().into_any()
                    }
                }}
            </div>
            <PreparedDeleteButton />
        </div>
    }
}

#[component]
fn PreparedDeleteButton() -> impl IntoView {
    let bs = use_context::<BibleState>().expect("BibleState");
    let ctx = use_context::<AppContext>().expect("AppContext");

    let on_delete = {
        let bs = bs.clone();
        let ctx = ctx.clone();
        move |_| {
            let pres_id = bs.active_presentation_id.get_untracked();
            let Some(id) = pres_id else { return };
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
                    }
                    Err(e) => {
                        toast_variant.set("error".to_string());
                        toast_message.set(Some(format!("Delete failed: {e}")));
                    }
                }
            });
        }
    };

    let has_active = move || bs.active_presentation_id.get().is_some();

    view! {
        <div class="bible__prepared-actions">
            <button
                type="button"
                class="operator__list-action"
                data-role="presentation-delete"
                disabled=move || !has_active()
                on:click=on_delete
                style="color: #ef4444;"
            >"Delete presentation"</button>
        </div>
    }
}

// ---------------------------------------------------------------------------
// Settings tab
// ---------------------------------------------------------------------------

#[component]
fn BibleSettingsTab() -> impl IntoView {
    let bs = use_context::<BibleState>().expect("BibleState");
    let ctx = use_context::<AppContext>().expect("AppContext");
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
            attr:data-visible=move || if bible_tab.get() == "settings" { "true" } else { "false" }
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
