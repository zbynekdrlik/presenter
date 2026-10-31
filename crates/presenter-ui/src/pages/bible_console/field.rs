//! The console field (spec §7.1 item 2): any way of writing a reference.
//! Enter / "Go" closes the phone keyboard (and retries a failed preview); on
//! a desktop it moves focus to ZOBRAZIŤ, so Enter again shows. Escape clears,
//! and blurs when already empty. It never autofocuses on a phone (the
//! keyboard would cover the screen).

use leptos::prelude::*;

use super::Console;
use crate::state::AppContext;

#[component]
pub(super) fn ConsoleField(console: Console) -> impl IntoView {
    let c = console;
    let phone = use_ctx!(AppContext).phone_shell;

    let on_keydown = move |ev: web_sys::KeyboardEvent| match ev.key().as_str() {
        "Enter" => {
            ev.prevent_default();
            c.retry.update(|n| *n = n.wrapping_add(1));
            if phone.get_untracked() {
                if let Some(field) = c.field.get_untracked() {
                    let _ = field.blur();
                }
            } else if c.target_key.with_untracked(Option::is_some) {
                c.focus_show.set(true);
            }
        }
        "Escape" => {
            ev.prevent_default();
            if c.input.with_untracked(String::is_empty) {
                if let Some(field) = c.field.get_untracked() {
                    let _ = field.blur();
                }
            } else {
                c.input.set(String::new());
            }
        }
        _ => {}
    };
    // iOS autocorrect turns "zid" into "kid"; tachys has no typed
    // `autocorrect` attribute, so it is set on the element itself.
    Effect::new(move |_| {
        if let Some(field) = c.field.get() {
            let _ = field.set_attribute("autocorrect", "off");
        }
    });
    let on_clear = move |_| {
        c.input.set(String::new());
        if let Some(field) = c.field.get_untracked() {
            let _ = field.focus();
        }
    };

    view! {
        <div class="console-field">
            <input
                type="text"
                class="console-field__input"
                data-role="console-field"
                node_ref=c.field
                placeholder="Odkaz alebo slová\u{2026}"
                aria-label="Odkaz alebo slová"
                enterkeyhint="go"
                autocapitalize="off"
                autocomplete="off"
                spellcheck="false"
                prop:value=move || c.input.get()
                on:input=move |ev| c.input.set(event_target_value(&ev))
                on:keydown=on_keydown
                on:focus=move |_| c.field_focused.set(true)
                on:blur=move |_| c.field_focused.set(false)
            />
            <Show when=move || c.input.with(|text| !text.is_empty())>
                <button
                    type="button"
                    class="console-field__clear"
                    data-role="console-clear-field"
                    aria-label="Vymazať"
                    on:click=on_clear
                >
                    "\u{2715}"
                </button>
            </Show>
        </div>
    }
}
