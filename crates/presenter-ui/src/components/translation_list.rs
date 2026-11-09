//! The ▾ list of every installed translation (#921): short code + full name,
//! grouped by language (`state::bible_translations::translation_groups`).
//! The console's bar opens it as a popover under a row, Study as its
//! `s=preklady` sheet. A pick is handed to `on_pick`; the caller selects the
//! translation and closes the list. Scrolls when long, so a site with a
//! hundred translations still opens one compact list.

use leptos::prelude::*;
use presenter_core::bible::sermon::translation_short_code;
use presenter_core::BibleTranslation;

use crate::state::bible_translations::TranslationGroup;

#[component]
pub fn TranslationList(
    /// The groups to list, already filtered for this row.
    #[prop(into)]
    groups: Signal<Vec<TranslationGroup>>,
    /// Every installed translation, for the full names.
    #[prop(into)]
    translations: Signal<Vec<BibleTranslation>>,
    /// The translation that is on (pressed).
    #[prop(into)]
    selected: Signal<Option<String>>,
    /// `data-kind` of every option (`main`, `secondary`, `study`).
    kind: &'static str,
    on_pick: Callback<String>,
) -> impl IntoView {
    let name_of = move |code: &str| {
        translations.with_untracked(|list| {
            list.iter()
                .find(|t| t.code == code)
                .map(|t| t.name.clone())
                .unwrap_or_else(|| code.to_string())
        })
    };
    view! {
        <div class="translation-list" data-role="translation-list" data-kind=kind>
            <For
                each=move || groups.get()
                key=|group| (group.label, group.codes.clone())
                children=move |group| {
                    let codes = group.codes.clone();
                    view! {
                        <div class="translation-list__group" role="group" aria-label=group.label>
                            <div class="translation-list__heading">{group.label}</div>
                            {codes
                                .into_iter()
                                .map(|code| {
                                    let name = name_of(&code);
                                    option(code, name, kind, selected, on_pick)
                                })
                                .collect_view()}
                        </div>
                    }
                }
            />
        </div>
    }
}

fn option(
    code: String,
    name: String,
    kind: &'static str,
    selected: Signal<Option<String>>,
    on_pick: Callback<String>,
) -> impl IntoView {
    let label = translation_short_code(&code);
    let mine = code.clone();
    let pressed =
        Memo::new(move |_| selected.with(|current| current.as_deref() == Some(mine.as_str())));
    view! {
        <button
            type="button"
            class="translation-list__option"
            data-role="translation-option"
            data-kind=kind
            data-translation=code.clone()
            aria-pressed=move || if pressed.get() { "true" } else { "false" }
            on:click=move |_| on_pick.run(code.clone())
        >
            <span class="translation-list__code">{label}</span>
            <span class="translation-list__name">{name}</span>
        </button>
    }
}
