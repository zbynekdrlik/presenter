//! Biblia → Nastavenie → „Preferované preklady" (#921): which translations
//! get a chip in the console's two rows. One checkbox per installed
//! translation and row; a tick adds the translation at the end of the row's
//! list, an untick removes it. Saved at once with the Bible preferences on
//! the server (`BiblePreferencesDraft::preferred_*`), the shared default every
//! console reads on its next start.

use leptos::prelude::*;
use presenter_core::bible::sermon::translation_short_code;

use crate::api::bible;
use crate::state::bible::BibleState;
use crate::state::bible_translations::{
    console_chip_cap, offered_as_main, translation_groups, TranslationGroup,
};
use crate::state::AppContext;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Row {
    Main,
    Secondary,
}

/// The row's list after a tick (`on`) or an untick of `code`.
fn toggled(list: &[String], code: &str, on: bool) -> Vec<String> {
    let mut next: Vec<String> = list.iter().filter(|c| *c != code).cloned().collect();
    if on {
        next.push(code.to_string());
    }
    next
}

#[component]
pub fn PreferredTranslations() -> impl IntoView {
    view! {
        <div class="bible-preferred" data-role="preferred-translations">
            <h3 class="bible-preferred__title">"Preferované preklady"</h3>
            <p class="bible-preferred__hint">
                {format!(
                    "Tlačidlá v riadku prekladov na karte Naživo. Zmestí sa ich {} (na telefóne {}), \
                     v druhom riadku o jedno menej; všetky ostatné preklady sú v zozname \u{25BE}.",
                    console_chip_cap(false, false),
                    console_chip_cap(true, false),
                )}
            </p>
            <RowChoices row=Row::Main />
            <RowChoices row=Row::Secondary />
        </div>
    }
}

#[component]
fn RowChoices(row: Row) -> impl IntoView {
    let bs = use_ctx!(BibleState);
    let ctx = use_ctx!(AppContext);
    let translations = bs.translations;
    let (preferred, kind, legend) = match row {
        Row::Main => (bs.preferred_main, "main", "Hlavný preklad"),
        Row::Secondary => (bs.preferred_secondary, "secondary", "Druhý preklad"),
    };
    let groups = move || {
        translations.with(|installed| {
            translation_groups(installed, |code| match row {
                // NLT is never a main translation: no main chip for it either.
                Row::Main => offered_as_main(code, None),
                Row::Secondary => true,
            })
        })
    };
    let on_toggle = move |code: String, on: bool| {
        let next = toggled(&preferred.get_untracked(), &code, on);
        preferred.set(next.clone());
        let draft = match row {
            Row::Main => presenter_core::BiblePreferencesDraft {
                preferred_main: Some(next),
                ..Default::default()
            },
            Row::Secondary => presenter_core::BiblePreferencesDraft {
                preferred_secondary: Some(next),
                ..Default::default()
            },
        };
        let ctx = ctx.clone();
        leptos::task::spawn_local(async move {
            if let Err(e) = bible::update_preferences(&draft).await {
                ctx.show_toast(&format!("Uloženie zlyhalo: {e}"), "error");
            }
        });
    };

    view! {
        <fieldset class="bible-preferred__row" data-row=kind>
            <legend>{legend}</legend>
            <For
                each=groups
                key=|group: &TranslationGroup| (group.label, group.codes.clone())
                children=move |group| {
                    let on_toggle = on_toggle.clone();
                    view! {
                        <div class="bible-preferred__group">
                            <span class="bible-preferred__group-label">{group.label}</span>
                            {group
                                .codes
                                .into_iter()
                                .map(|code| {
                                    let name = translations
                                        .with_untracked(|list| {
                                            list.iter()
                                                .find(|t| t.code == code)
                                                .map(|t| t.name.clone())
                                                .unwrap_or_default()
                                        });
                                    let label = translation_short_code(&code);
                                    let mine = code.clone();
                                    let checked = Memo::new(move |_| {
                                        preferred.with(|list| list.iter().any(|c| *c == mine))
                                    });
                                    let on_toggle = on_toggle.clone();
                                    let toggle_code = code.clone();
                                    view! {
                                        <label class="bible-preferred__option">
                                            <input
                                                type="checkbox"
                                                data-role="preferred-translation"
                                                data-kind=kind
                                                data-translation=code.clone()
                                                prop:checked=move || checked.get()
                                                on:change=move |ev| {
                                                    on_toggle(toggle_code.clone(), event_target_checked(&ev))
                                                }
                                            />
                                            <span class="bible-preferred__code">{label}</span>
                                            <span class="bible-preferred__name">{name}</span>
                                        </label>
                                    }
                                })
                                .collect_view()}
                        </div>
                    }
                }
            />
        </fieldset>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(list: &[&str]) -> Vec<String> {
        list.iter().map(|c| c.to_string()).collect()
    }

    #[test]
    fn a_tick_adds_at_the_end_and_an_untick_removes() {
        let list = codes(&["slk-seb", "slk-roh"]);
        assert_eq!(
            toggled(&list, "ces-csp", true),
            codes(&["slk-seb", "slk-roh", "ces-csp"])
        );
        assert_eq!(toggled(&list, "slk-seb", false), codes(&["slk-roh"]));
        // A repeated tick never duplicates.
        assert_eq!(
            toggled(&list, "slk-roh", true),
            codes(&["slk-seb", "slk-roh"])
        );
    }
}
