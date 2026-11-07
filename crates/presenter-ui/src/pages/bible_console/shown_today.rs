//! "Táto bohoslužba" (#850 B2d, spec §7.4): the verses shown today, shared by
//! every device. A tap previews, only ZOBRAZIŤ sends. The list is read on
//! mount, whenever the wall changes (whoever sent it: `active_bible_slide` is
//! fed by the live events) and when the local date changes. Hidden while empty
//! and while the field has text.

use leptos::prelude::*;

use super::prepared::local_date_signal;
use super::{Console, Picked};
use crate::api::bible::{self as api, ShownVerse};
use crate::state::bible_console_shown::shown_target;
use crate::state::AppContext;

#[component]
pub(super) fn ShownTodaySection(console: Console) -> impl IntoView {
    let c = console;
    let on_air = use_ctx!(AppContext).active_bible_slide;
    let date = local_date_signal();
    let verses = RwSignal::new(Vec::<ShownVerse>::new());
    let asked = StoredValue::new(0u64);
    Effect::new(move |_| {
        on_air.track();
        date.track();
        let mine = asked.get_value().wrapping_add(1);
        asked.set_value(mine);
        leptos::task::spawn_local(async move {
            let answer = api::shown_today().await;
            // An older answer never replaces a newer one; `try_`: the console
            // may have unmounted while this was on its way.
            if asked.try_get_value() != Some(mine) {
                return;
            }
            match answer {
                Ok(list) => verses.set(list),
                // A reload during the read: the next page reads again.
                Err(err) if err.is_cut_off() => leptos::logging::log!("shown today: {err}"),
                Err(err) => leptos::logging::warn!("shown today: {err}"),
            }
        });
    });
    let visible = move || c.input.with(String::is_empty) && verses.with(|v| !v.is_empty());
    view! {
        <Show when=visible>
            <section class="console-shown" data-role="console-shown">
                <h3 class="console-shown__heading">"Táto bohoslužba"</h3>
                <div class="console-shown__chips">
                    <For
                        each=move || verses.get()
                        key=|verse| format!("{}:{}", verse.label, verse.translation_code)
                        children=move |verse| shown_chip(c, verse)
                    />
                </div>
            </section>
        </Show>
    }
}

fn shown_chip(c: Console, verse: ShownVerse) -> impl IntoView {
    let target = shown_target(&verse);
    let code = target.book.code.clone();
    let segment = target.segment.clone();
    let selected = Memo::new(move |_| {
        c.prepared.with(Option::is_none)
            && c.target.with(|now| {
                now.as_ref()
                    .is_some_and(|now| now.book.code == code && now.segment == segment)
            })
    });
    let translation = verse.translation_code;
    let on_tap = move |_| {
        c.prepared.set(None);
        c.after_show.set(false);
        c.at_end.set(false);
        c.picked.set(Some(Picked {
            input: c.input.get_untracked(),
            main: Some(translation.clone()),
            target: target.clone(),
        }));
    };
    view! {
        <button
            type="button"
            class="console-chip"
            data-role="console-shown-chip"
            data-book-code=verse.book_code
            data-chapter=verse.chapter.to_string()
            aria-pressed=move || if selected.get() { "true" } else { "false" }
            on:click=on_tap
        >
            {verse.label}
        </button>
    }
}
