//! The one field (spec §6.4): a reference in any form opens the reader, a book
//! offers its chapters, words search the text. The resolver runs in the
//! browser on every keystroke, through the verse console's row builder. An
//! empty field offers all the books. Nothing here sends to a wall.

use leptos::html::Input;
use leptos::prelude::*;

use super::scroll::ScrollPlan;
use super::{nav, Study};
use crate::state::bible_console::{ConsoleRow, RowTarget};
use crate::state::study::{
    chapter_grid, preview_target, suggest, ChapterGrid, StudyRoute, StudySuggest, StudyTarget,
    StudyView,
};

const MIN_TEXT_CHARS: usize = 2;

#[derive(Clone, Copy)]
struct Field {
    s: Study,
    input: RwSignal<String>,
    focused: RwSignal<bool>,
    element: NodeRef<Input>,
}

impl Field {
    fn leave(self) {
        if let Some(element) = self.element.get_untracked() {
            let _ = element.blur();
        }
    }

    fn open(self, target: StudyTarget) {
        let Some(code) = self
            .s
            .route
            .with_untracked(|route| route.translation.clone())
        else {
            return;
        };
        let route =
            StudyRoute::reader(&code, &target.book, target.chapter).with_verses(target.verses);
        let plan = target
            .verses
            .map_or(ScrollPlan::Top, |(start, _)| ScrollPlan::Verse(start));
        self.input.set(String::new());
        self.leave();
        nav::go(self.s, route, plan);
    }

    fn search(self, text: &str) {
        let Some(code) = self
            .s
            .route
            .with_untracked(|route| route.translation.clone())
        else {
            return;
        };
        self.leave();
        nav::go(self.s, StudyRoute::search(&code, text), ScrollPlan::Keep);
    }

    fn complete(self, text: &str) {
        self.input.set(text.to_string());
        if let Some(element) = self.element.get_untracked() {
            let _ = element.focus();
        }
    }
}

#[component]
pub(super) fn StudyField(s: Study) -> impl IntoView {
    let f = Field {
        s,
        input: RwSignal::new(String::new()),
        focused: RwSignal::new(false),
        element: NodeRef::new(),
    };
    let suggestions = Memo::new(move |_| {
        let input = f.input.get();
        let translation = s.translation().unwrap_or_default();
        let found = s.with_structure(|structure| {
            structure.map(|structure| suggest(&input, structure, &translation))
        });
        match found.flatten() {
            Some(suggestions) => suggestions,
            // No structure yet: words can still be searched.
            None => StudySuggest {
                text_search: (input.trim().chars().count() >= MIN_TEXT_CHARS)
                    .then(|| input.trim().to_string()),
                ..StudySuggest::default()
            },
        }
    });
    // The search view's words stay in the field; leaving them empties it.
    let shown_query = Memo::new(move |_| {
        s.route.with(|route| match &route.view {
            StudyView::Search { query } => Some(query.clone()),
            StudyView::Reader { .. } => None,
        })
    });
    Effect::new(move |previous: Option<bool>| {
        let query = shown_query.get();
        let searching = query.is_some();
        if searching || previous == Some(true) {
            f.input.set(query.unwrap_or_default());
        }
        searching
    });
    // iOS autocorrect turns "zid" into "kid" (console rule).
    Effect::new(move |_| {
        if let Some(element) = f.element.get() {
            let _ = element.set_attribute("autocorrect", "off");
        }
    });

    let on_keydown = move |ev: web_sys::KeyboardEvent| match ev.key().as_str() {
        "Enter" => {
            ev.prevent_default();
            let found = suggestions.get_untracked();
            if let Some(target) = found.enter_target() {
                f.open(target);
            } else if let Some(text) = found.text_search {
                f.search(&text);
            }
        }
        "Escape" => {
            ev.prevent_default();
            if f.input.with_untracked(String::is_empty) {
                f.leave();
            } else {
                f.input.set(String::new());
            }
        }
        _ => {}
    };

    view! {
        <div class="study-field">
            <input
                type="text"
                class="study-field__input"
                data-role="study-field"
                node_ref=f.element
                placeholder="Odkaz alebo slová\u{2026}"
                aria-label="Odkaz alebo slová"
                enterkeyhint="search"
                autocapitalize="off"
                autocomplete="off"
                spellcheck="false"
                prop:value=move || f.input.get()
                on:input=move |ev| f.input.set(event_target_value(&ev))
                on:keydown=on_keydown
                on:focus=move |_| f.focused.set(true)
                on:blur=move |_| f.focused.set(false)
            />
            <Show when=move || f.input.with(|text| !text.is_empty())>
                <button
                    type="button"
                    class="study-field__clear"
                    data-role="study-field-clear"
                    aria-label="Vymazať"
                    on:mousedown=|ev| ev.prevent_default()
                    on:click=move |_| f.complete("")
                >
                    "\u{2715}"
                </button>
            </Show>
            <Show when=move || f.focused.get()>
                <Suggestions f=f suggestions=suggestions />
            </Show>
        </div>
    }
}

/// Over the page while the field has focus. A press here keeps that focus, or
/// the blur would remove the row before its `click`.
#[component]
fn Suggestions(f: Field, suggestions: Memo<StudySuggest>) -> impl IntoView {
    // A book's row would only repeat the grid under it.
    let rows = move || {
        suggestions.with(|found| {
            if found.grid.is_some() {
                Vec::new()
            } else {
                found.view.rows.clone()
            }
        })
    };
    let chips = move || suggestions.with(|found| found.view.chips.clone());
    let has_chips = move || suggestions.with(|found| !found.view.chips.is_empty());
    let hint = move || suggestions.with(|found| found.view.hint.clone());
    let grid = move || suggestions.with(|found| found.grid.clone());
    let text = move || suggestions.with(|found| found.text_search.clone());
    let books = Memo::new(move |_| {
        if !f.input.with(|input| input.trim().is_empty()) {
            return Vec::new();
        }
        f.s.with_structure(|structure| {
            structure.map_or_else(Vec::new, |found| {
                found
                    .books()
                    .iter()
                    .map(|book| (book.code.clone(), book.name.clone()))
                    .collect()
            })
        })
        .unwrap_or_default()
    });
    let book_grid = move |code: String| {
        f.s.with_structure(|structure| chapter_grid(structure?, &code))
            .flatten()
    };

    view! {
        <div
            class="study-suggest"
            data-role="study-suggest"
            on:mousedown=|ev| ev.prevent_default()
        >
            <For
                each=rows
                key=|row| row.key.clone()
                children=move |row| view! { <Row f=f row=row /> }
            />
            {move || {
                text()
                    .map(|words| {
                        let search = words.clone();
                        view! {
                            <button
                                type="button"
                                class="study-row"
                                data-role="study-text-search"
                                on:click=move |_| f.search(&search)
                            >
                                <span class="study-row__label">"Hľadať v texte"</span>
                                <small class="study-row__detail">{format!("„{words}“")}</small>
                            </button>
                        }
                    })
            }}
            {move || {
                hint()
                    .map(|hint| {
                        view! {
                            <p class="study-hint" data-role="study-hint" role="status">
                                {hint}
                            </p>
                        }
                    })
            }}
            <Show when=has_chips>
                <div class="study-chips" data-role="study-chips">
                    <For
                        each=chips
                        key=|row| row.key.clone()
                        children=move |row| view! { <Chip f=f row=row /> }
                    />
                </div>
            </Show>
            {move || grid().map(|grid| view! { <Chapters f=f grid=grid /> })}
            <Show when=move || books.with(|list| !list.is_empty())>
                <p class="study-suggest__label">"Knihy"</p>
                <div class="study-chips" data-role="study-books">
                    <For
                        each=move || books.get()
                        key=|book| book.0.clone()
                        children=move |(code, name)| {
                            let of_book = code.clone();
                            view! {
                                <button
                                    type="button"
                                    class="study-chip"
                                    data-role="study-book"
                                    data-book-code=code
                                    on:click=move |_| {
                                        if let Some(found) = book_grid(of_book.clone()) {
                                            f.complete(&format!("{} ", found.name));
                                        }
                                    }
                                >
                                    {name}
                                </button>
                            }
                        }
                    />
                </div>
            </Show>
        </div>
    }
}

fn tap(f: Field, target: &RowTarget) {
    match target {
        RowTarget::Preview(preview) => f.open(preview_target(preview)),
        RowTarget::Complete(text) => f.complete(text),
    }
}

#[component]
fn Row(f: Field, row: ConsoleRow) -> impl IntoView {
    let target = row.target.clone();
    view! {
        <button
            type="button"
            class="study-row"
            data-role="study-row"
            data-kind=row.kind
            on:click=move |_| tap(f, &target)
        >
            <span class="study-row__label">{row.label.clone()}</span>
            {(!row.detail.is_empty())
                .then(|| view! { <small class="study-row__detail">{row.detail.clone()}</small> })}
        </button>
    }
}

#[component]
fn Chip(f: Field, row: ConsoleRow) -> impl IntoView {
    let target = row.target.clone();
    view! {
        <button
            type="button"
            class="study-chip"
            data-role="study-chip"
            data-kind=row.kind
            on:click=move |_| tap(f, &target)
        >
            {row.label.clone()}
        </button>
    }
}

#[component]
fn Chapters(f: Field, grid: ChapterGrid) -> impl IntoView {
    let code = grid.code.clone();
    view! {
        <p class="study-suggest__label" data-role="study-grid-title">{grid.name.clone()}</p>
        <div class="study-grid" data-role="study-chapter-grid" data-book-code=grid.code.clone()>
            {grid
                .chapters
                .iter()
                .map(|chapter| {
                    let chapter = *chapter;
                    let book = code.clone();
                    view! {
                        <button
                            type="button"
                            class="study-grid__cell"
                            data-role="study-chapter-cell"
                            data-chapter=chapter
                            on:click=move |_| {
                                f.open(StudyTarget {
                                    book: book.clone(),
                                    chapter,
                                    verses: None,
                                })
                            }
                        >
                            {chapter}
                        </button>
                    }
                })
                .collect_view()}
        </div>
    }
}
