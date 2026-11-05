//! Text-search rows (#850 spec §5.11): when the field holds words rather
//! than a reference, the main translation's text is searched 300 ms after the
//! last keystroke (`GET /bible/search`, 10 hits). Each hit is labelled with
//! the translation; a tap previews it — it never goes to the wall directly.

use std::cell::RefCell;
use std::rc::Rc;

use gloo_timers::callback::Timeout;
use leptos::prelude::*;
use presenter_core::bible::sermon::translation_short_code;

use super::{Console, Picked};
use crate::api::bible::{self as api, BibleSearchHit};
use crate::state::bible_console_search::{hit_target, wants_text_search};

const SEARCH_DELAY_MS: u32 = 300;
const SEARCH_LIMIT: u32 = 10;
const SNIPPET_CHARS: usize = 90;

/// The hits for one input in one main translation; `None` = the search failed.
#[derive(Clone)]
struct Found {
    input: String,
    main: String,
    hits: Option<Vec<BibleSearchHit>>,
}

#[component]
pub(super) fn TextSearch(console: Console) -> impl IntoView {
    let c = console;
    let found = RwSignal::new(None::<Found>);
    let pending: Rc<RefCell<Option<Timeout>>> = Rc::new(RefCell::new(None));
    Effect::new(move |_| {
        let input = c.input.get();
        let main = c.main.get().unwrap_or_default();
        let wants = c.resolution.with(|r| wants_text_search(&input, r));
        pending.borrow_mut().take();
        if !wants {
            found.set(None);
            return;
        }
        *pending.borrow_mut() = Some(Timeout::new(SEARCH_DELAY_MS, move || {
            leptos::task::spawn_local(async move {
                let answer = api::search(&input, &main, Some(SEARCH_LIMIT), None).await;
                if answer.is_err() {
                    leptos::logging::log!("console text search failed for {input:?}");
                }
                // `try_`: the console may have unmounted while this was on its way.
                let current = c.input.try_with_untracked(|now| *now == input) == Some(true)
                    && c.main.try_get_untracked().flatten().as_deref() == Some(main.as_str());
                if current {
                    found.set(Some(Found {
                        input,
                        main,
                        hits: answer.ok(),
                    }));
                }
            });
        }));
    });
    let current = move || {
        found.get().filter(|f| {
            c.input.with(|now| *now == f.input)
                && c.main.with(|now| now.as_deref() == Some(f.main.as_str()))
        })
    };
    let hits = move || current().and_then(|f| f.hits).unwrap_or_default();
    let failed = move || current().is_some_and(|f| f.hits.is_none());

    view! {
        <div class="console-text-results" data-role="console-text-results">
            <For
                each=hits
                key=|hit| {
                    let r = &hit.reference;
                    format!("{}:{}:{}-{}", r.book, r.chapter, r.verse_start, r.verse_end)
                }
                children=move |hit| search_row(c, hit)
            />
            <Show when=failed>
                <p class="console-preview__error" data-role="console-search-error" role="alert">
                    "Hľadanie v texte zlyhalo"
                </p>
            </Show>
        </div>
    }
}

fn search_row(c: Console, hit: BibleSearchHit) -> impl IntoView {
    let target = hit_target(&hit);
    let label = format!(
        "{} \u{00B7} {}",
        hit.reference.to_human_readable(),
        translation_short_code(&hit.translation.code)
    );
    let snippet: String = hit.text.chars().take(SNIPPET_CHARS).collect();
    let code = target
        .as_ref()
        .map(|t| t.book.code.clone())
        .unwrap_or_default();
    let on_tap = move |_| {
        if let Some(target) = target.clone() {
            c.picked.set(Some(Picked {
                input: c.input.get_untracked(),
                main: c.main.get_untracked(),
                target,
            }));
        }
    };
    view! {
        <button
            type="button"
            class="console-row console-row--text"
            data-role="console-text-search"
            data-book-code=code
            data-chapter=hit.reference.chapter.to_string()
            data-verse-start=hit.reference.verse_start.to_string()
            data-verse-end=hit.reference.verse_end.to_string()
            on:click=on_tap
        >
            <span class="console-row__label">{label}</span>
            <small class="console-row__detail console-row__detail--snippet">{snippet}</small>
        </button>
    }
}
