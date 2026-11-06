//! The translation bar (spec §7.1 item 1b, owner decision 2026-10-13): one
//! tap switches the main translation or the line below it. It writes the same
//! signals as the old selects, so the #701 behaviour stays: the browser keeps
//! its choice (localStorage) and the server preference is the default every
//! new console starts from. The preview re-resolves at once; what is on the
//! wall changes only on the next show. Phone and desktop alike since #850 B2c
//! (it replaced the desktop selects).

use leptos::prelude::*;
use presenter_core::bible::sermon::translation_short_code;

use crate::state::bible::BibleState;
use crate::state::bible_translations::translation_chips;
use crate::state::AppContext;

#[component]
pub(super) fn TranslationBar() -> impl IntoView {
    let bs = use_ctx!(BibleState);
    let translations = bs.translations;
    let main_signal = bs.selected_translation;
    let secondary_signal = bs.secondary_translation;
    let chips = Memo::new(move |_| {
        let codes: Vec<String> =
            translations.with(|list| list.iter().map(|t| t.code.clone()).collect());
        translation_chips(
            &codes,
            main_signal.read().as_deref(),
            secondary_signal.read().as_deref(),
        )
    });
    let show_more = RwSignal::new(false);
    let main = move || chips.with(|c| c.main.clone());
    let more = move || chips.with(|c| c.more.clone());
    let secondary = move || chips.with(|c| c.secondary.clone());
    // A desktop names the rows like the mockup v3 Biblia panel (a heading
    // over each row); a phone keeps the short inline labels of mockup v2.
    let phone = use_ctx!(AppContext).phone_shell;
    let main_label = move || {
        if phone.get() {
            "Preklad"
        } else {
            "Hlavný preklad"
        }
    };
    let secondary_label = move || {
        if phone.get() {
            "Pod ním"
        } else {
            "Druhý preklad"
        }
    };

    view! {
        <div class="console-translations" data-role="console-translations">
            <div class="console-translations__row">
                <span class="console-translations__label">{main_label}</span>
                <For
                    each=main
                    key=|code| code.clone()
                    children=move |code| chip(code, "main", main_signal)
                />
                <Show when=move || chips.with(|c| !c.more.is_empty())>
                    <button
                        type="button"
                        class="console-translation"
                        data-role="console-translation-more"
                        aria-pressed=move || if show_more.get() { "true" } else { "false" }
                        on:click=move |_| show_more.update(|open| *open = !*open)
                    >
                        "\u{2026}"
                    </button>
                </Show>
                <Show when=move || show_more.get()>
                    <For
                        each=more
                        key=|code| code.clone()
                        children=move |code| chip(code, "main", main_signal)
                    />
                </Show>
            </div>
            <div class="console-translations__row">
                <span class="console-translations__label">{secondary_label}</span>
                <For
                    each=secondary
                    key=|code| code.clone()
                    children=move |code| chip(code, "secondary", secondary_signal)
                />
                <button
                    type="button"
                    class="console-translation"
                    data-role="console-translation"
                    data-kind="secondary"
                    data-translation=""
                    aria-pressed=move || {
                        if secondary_signal.with(Option::is_none) { "true" } else { "false" }
                    }
                    on:click=move |_| secondary_signal.set(None)
                >
                    "žiadny"
                </button>
            </div>
        </div>
    }
}

/// One translation chip; `selected` is the main or the secondary signal.
fn chip(code: String, kind: &'static str, selected: RwSignal<Option<String>>) -> impl IntoView {
    let label = translation_short_code(&code);
    let mine = code.clone();
    let pressed =
        Memo::new(move |_| selected.with(|current| current.as_deref() == Some(mine.as_str())));
    view! {
        <button
            type="button"
            class="console-translation"
            data-role="console-translation"
            data-kind=kind
            data-translation=code.clone()
            aria-pressed=move || if pressed.get() { "true" } else { "false" }
            on:click=move |_| selected.set(Some(code.clone()))
        >
            {label}
        </button>
    }
}
