//! The translation bar (spec §7.1 item 1b, owner decision 2026-10-13): one
//! tap switches the main translation or the line below it. It writes the same
//! signals as the old selects, so the #701 behaviour stays: the browser keeps
//! its choice (localStorage) and the server preference is the default every
//! new console starts from. The preview re-resolves at once; what is on the
//! wall changes only on the next show. Phone and desktop alike since #850 B2c
//! (it replaced the desktop selects).
//!
//! #921 (owner 2026-11-09): each row is ONE line that never grows. It holds
//! the preferred translations (Biblia → Nastavenie) up to the row's cap, the
//! selected one in the last place when it is not preferred, and a ▾ that opens
//! the list of every installed translation (`components::translation_list`).

use leptos::prelude::*;
use presenter_core::bible::sermon::translation_short_code;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

use crate::components::translation_list::TranslationList;
use crate::state::bible::BibleState;
use crate::state::bible_translations::{
    console_chip_cap, offered_as_main, row_chips, translation_groups, translation_name,
    TranslationRow as Row,
};
use crate::state::AppContext;

#[component]
pub(super) fn TranslationBar() -> impl IntoView {
    let bs = use_ctx!(BibleState);
    let ctx = use_ctx!(AppContext);
    let phone = ctx.phone_shell;
    // A desktop names the rows like the mockup v3 Biblia panel (a heading
    // over each row); a phone keeps the short inline labels of mockup v2.
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
    let open = RwSignal::new(None::<Row>);
    let secondary = bs.secondary_translation;
    // Leaving Biblia's Live tab closes an open list: a list kept in a hidden
    // panel would keep the window keys switched off (`operator.rs`).
    let bible_tab = bs.bible_tab;
    let view = ctx.view;
    Effect::new(move || {
        let shown = view.get() == "bible" && bible_tab.get() == "live";
        if !shown && open.get_untracked().is_some() {
            open.set(None);
        }
    });

    view! {
        <div class="console-translations" data-role="console-translations">
            <div class="console-translations__row" data-row="main">
                <span class="console-translations__label">{main_label}</span>
                <div class="console-translations__chips">
                    <RowChips row=Row::Main open=open />
                </div>
            </div>
            <div class="console-translations__row" data-row="secondary">
                <span class="console-translations__label">{secondary_label}</span>
                <div class="console-translations__chips">
                    <button
                        type="button"
                        class="console-translation"
                        data-role="console-translation"
                        data-kind="secondary"
                        data-translation=""
                        title="Bez druhého prekladu"
                        aria-pressed=move || {
                            if secondary.with(Option::is_none) { "true" } else { "false" }
                        }
                        on:click=move |_| secondary.set(None)
                    >
                        "žiadny"
                    </button>
                    <RowChips row=Row::Secondary open=open />
                </div>
            </div>
            <Show when=move || open.get().is_some()>
                <ListCloser open=open />
            </Show>
        </div>
    }
}

/// While a list is open: the transparent backdrop (a press beside the list
/// closes it) and Escape anywhere on the page. The Escape listener sits on
/// the window in the CAPTURE phase, before the operator's own window keys
/// (which would close the header search instead), and lives only as long as
/// the list, so a closed list takes no key.
#[component]
fn ListCloser(open: RwSignal<Option<Row>>) -> impl IntoView {
    let on_key =
        Closure::<dyn Fn(web_sys::KeyboardEvent)>::new(move |ev: web_sys::KeyboardEvent| {
            if ev.key() == "Escape" {
                ev.stop_propagation();
                ev.prevent_default();
                open.set(None);
            }
        });
    let window = crate::utils::window::window();
    let _ = window.add_event_listener_with_callback_and_bool(
        "keydown",
        on_key.as_ref().unchecked_ref(),
        true,
    );
    let listener = StoredValue::new_local(on_key);
    on_cleanup(move || {
        listener.with_value(|on_key| {
            let _ = crate::utils::window::window().remove_event_listener_with_callback_and_bool(
                "keydown",
                on_key.as_ref().unchecked_ref(),
                true,
            );
        });
    });
    view! { <div class="console-translations__backdrop" on:click=move |_| open.set(None)></div> }
}

/// The chips of one row, its ▾ and (while open) its list.
#[component]
fn RowChips(row: Row, open: RwSignal<Option<Row>>) -> impl IntoView {
    let bs = use_ctx!(BibleState);
    let phone = use_ctx!(AppContext).phone_shell;
    let translations = bs.translations;
    let main = bs.selected_translation;
    let (selected, preferred) = match row {
        Row::Main => (bs.selected_translation, bs.preferred_main),
        Row::Secondary => (bs.secondary_translation, bs.preferred_secondary),
    };
    let offered = move |code: &str| match row {
        Row::Main => main.with(|m| offered_as_main(code, m.as_deref())),
        Row::Secondary => true,
    };
    let chips = Memo::new(move |_| {
        let cap = console_chip_cap(phone.get(), row == Row::Secondary);
        preferred.with(|preferred| {
            translations.with(|installed| {
                selected.with(|current| {
                    row_chips(preferred, installed, current.as_deref(), cap, offered)
                })
            })
        })
    });
    let groups = Signal::derive(move || translations.with(|t| translation_groups(t, offered)));
    let is_open = move || open.get() == Some(row);
    let on_pick = Callback::new(move |code: String| {
        selected.set(Some(code));
        open.set(None);
    });

    view! {
        <For
            each=move || chips.get()
            key=|code| code.clone()
            children=move |code| {
                let name = translations.with_untracked(|list| translation_name(list, &code));
                chip(code, name, row.kind(), selected)
            }
        />
        <button
            type="button"
            class="console-translation console-translation--menu"
            data-role="console-translation-menu"
            data-kind=row.kind()
            title="Všetky preklady"
            aria-label="Všetky preklady"
            aria-haspopup="dialog"
            aria-expanded=move || if is_open() { "true" } else { "false" }
            on:click=move |_| open.update(|o| *o = if *o == Some(row) { None } else { Some(row) })
        >
            "\u{25BE}"
        </button>
        <Show when=is_open>
            <div class="console-translations__popover" role="dialog" aria-label="Všetky preklady">
                <TranslationList
                    groups=groups
                    translations=translations
                    selected=selected
                    kind=row.kind()
                    on_pick=on_pick
                />
            </div>
        </Show>
    }
}

/// One translation chip; `selected` is the main or the secondary signal. The
/// full name is the tooltip and, after the visible code, the accessible name.
fn chip(
    code: String,
    name: String,
    kind: &'static str,
    selected: RwSignal<Option<String>>,
) -> impl IntoView {
    let label = translation_short_code(&code);
    let aria = format!("{label} — {name}");
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
            title=name
            aria-label=aria
            aria-pressed=move || if pressed.get() { "true" } else { "false" }
            on:click=move |_| selected.set(Some(code.clone()))
        >
            {label}
        </button>
    }
}
