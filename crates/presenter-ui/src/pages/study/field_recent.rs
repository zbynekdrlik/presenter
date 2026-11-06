//! The field's ▾: the earlier searches of this browser (owner, 2026-11-06:
//! "šípku dole aby som si mohol predplniť niečo z histórie čo som už
//! hľadal"). A pick only fills the field, so it can still be changed; Enter
//! opens it as if typed.

use leptos::prelude::*;

use super::field::Field;
use crate::state::session;
use crate::state::study_recent::{parse, RECENT_KEY};

/// The stored list, or none.
pub(super) fn load_recent() -> Vec<String> {
    parse(session::get_persistent(RECENT_KEY).as_deref())
}

/// ▾ at the field's right end. A press keeps the field's focus (the list
/// shows only while it has it, and the iOS keyboard stays up).
#[component]
pub(super) fn RecentButton(f: Field) -> impl IntoView {
    let on_click = move |_| {
        f.recent_open.update(|open| *open = !*open);
        if let Some(element) = f.element.get_untracked() {
            let _ = element.focus();
        }
    };
    view! {
        <button
            type="button"
            class="study-field__recent"
            data-role="study-field-recent"
            aria-label="Predtým hľadané"
            aria-expanded=move || f.recent_open.get().to_string()
            on:mousedown=|ev| ev.prevent_default()
            on:click=on_click
        >
            "\u{25BE}"
        </button>
    }
}

/// The list under the field, newest first.
#[component]
pub(super) fn RecentList(f: Field) -> impl IntoView {
    let pick = move |text: String| {
        f.recent_open.set(false);
        f.complete(&text);
    };
    view! {
        <div
            class="study-suggest"
            data-role="study-recent"
            on:mousedown=|ev| ev.prevent_default()
        >
            <p class="study-suggest__label">"Predtým hľadané"</p>
            <Show
                when=move || f.recent.with(|list| !list.is_empty())
                fallback=|| {
                    view! {
                        <p class="study-hint" data-role="study-recent-empty">
                            "Zatiaľ nič. Čo otvoríš alebo vyhľadáš, bude tu."
                        </p>
                    }
                }
            >
                <For
                    each=move || f.recent.get()
                    key=|text| text.clone()
                    children=move |text| {
                        let picked = text.clone();
                        view! {
                            <button
                                type="button"
                                class="study-row"
                                data-role="study-recent-item"
                                on:click=move |_| pick(picked.clone())
                            >
                                <span class="study-row__label">{text}</span>
                            </button>
                        }
                    }
                />
            </Show>
        </div>
    }
}
