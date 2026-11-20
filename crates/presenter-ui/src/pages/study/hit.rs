//! One search hit (#983): its place, its text, the favourites' short codes
//! (a tap shows the verse in that translation in the row, a tap on the hit's
//! own code goes back), "Porovnať" and "+".

use leptos::prelude::*;
use presenter_core::bible::sermon::translation_short_code;

use super::scroll::ScrollPlan;
use super::{add_to_sermon, compare, nav, study_error, Study};
use crate::api::bible::BibleSearchHit;
use crate::api::sermons::{is_not_found, SermonPassage};
use crate::api::study::load_chapter;
use crate::state::study::{chapter_key, mark_matches, StudyRoute};
use crate::state::study_search::{hit_translations, passage_text};

/// What the row shows: the hit itself, or its verses in another translation.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Shown {
    Hit,
    Loading(String),
    Other(String, String),
    Missing(String),
    Failed(String, String),
}

impl Shown {
    /// The translation whose chip is pressed and whose code the label names.
    fn code<'a>(&'a self, original: &'a str) -> &'a str {
        match self {
            Self::Hit => original,
            Self::Loading(code)
            | Self::Other(code, _)
            | Self::Missing(code)
            | Self::Failed(code, _) => code,
        }
    }

    /// The translation a tap on the row, "Porovnať" and "+" act in: the
    /// other one only while its text is on screen.
    fn acting<'a>(&'a self, original: &'a str) -> &'a str {
        match self {
            Self::Other(code, _) => code,
            _ => original,
        }
    }

    /// A tap on `code` asks again unless that answer is on its way or here
    /// (a failed read can be retried).
    fn wants_load(&self, code: &str) -> bool {
        !matches!(
            self,
            Self::Loading(held) | Self::Other(held, _) | Self::Missing(held)
                if held.as_str() == code
        )
    }
}

fn passage_shown(
    code: String,
    verses: &[crate::state::study::StudyVerse],
    (first, last): (u16, u16),
) -> Shown {
    match passage_text(verses, first, last) {
        Some(passage) => Shown::Other(code, passage),
        None => Shown::Missing(code),
    }
}

/// The verses `first..=last` of the hit's chapter in `code`, through the
/// page's chapter cache. Only the newest pick lands; a row gone is no error.
fn show_in(
    s: Study,
    shown: RwSignal<Shown>,
    code: String,
    (book, chapter, first, last): (String, u16, u16, u16),
) {
    let key = chapter_key(&code, &book, chapter);
    if let Some(text) = s.cache.try_update_value(|cache| cache.get(&key)).flatten() {
        shown.set(passage_shown(code, &text.verses, (first, last)));
        return;
    }
    shown.set(Shown::Loading(code.clone()));
    leptos::task::spawn_local(async move {
        let answer = load_chapter(&code, &book, chapter).await;
        // Kept even when this row has moved on (the NLT's daily limit). Never
        // an empty chapter: the reader reads a cached chapter as Ready.
        if let Ok(text) = &answer {
            if !text.verses.is_empty() {
                s.cache
                    .try_update_value(|cache| cache.put(key.clone(), text.clone()));
            }
        }
        let waiting = shown.try_with_untracked(|now| *now == Shown::Loading(code.clone()));
        if waiting != Some(true) {
            return;
        }
        let next = match answer {
            Ok(text) => passage_shown(code, &text.verses, (first, last)),
            // An unknown book is a place this translation lacks.
            Err(err) if is_not_found(&err) => Shown::Missing(code),
            Err(err) => {
                leptos::logging::log!("study hit: {code} {book} {chapter} not read");
                Shown::Failed(code, study_error(&err))
            }
        };
        shown.try_set(next);
    });
}

fn hit_text(hit: &BibleSearchHit, query: &str) -> AnyView {
    mark_matches(&hit.text, query)
        .into_iter()
        .map(|(piece, marked)| {
            if marked {
                view! { <mark>{piece}</mark> }.into_any()
            } else {
                view! { <span>{piece}</span> }.into_any()
            }
        })
        .collect_view()
        .into_any()
}

fn shown_text(shown: &Shown, hit: &BibleSearchHit, query: &str) -> AnyView {
    match shown {
        Shown::Hit => hit_text(hit, query),
        Shown::Loading(_) => "Načítavam…".into_any(),
        Shown::Other(_, text) => text.clone().into_any(),
        Shown::Missing(_) => "Tento preklad tento verš nemá.".into_any(),
        Shown::Failed(_, message) => message.clone().into_any(),
    }
}

fn translation_chips(
    s: Study,
    shown: RwSignal<Shown>,
    (favourites, original): (Memo<Vec<(usize, String)>>, String),
    place: (String, u16, u16, u16),
) -> impl IntoView {
    let codes = {
        let original = original.clone();
        move || favourites.with(|slots| hit_translations(slots, &original))
    };
    view! {
        <nav class="study-hit-translations" data-role="hit-translations" aria-label="Iný preklad">
            <For
                each=codes
                key=|code| code.clone()
                children=move |code| {
                    let mine = code.clone();
                    let original = original.clone();
                    let place = place.clone();
                    let pressed = {
                        let original = original.clone();
                        move || shown.with(|now| now.code(&original) == mine)
                    };
                    let target = code.clone();
                    let on_click = move |_| {
                        if target == original {
                            shown.set(Shown::Hit);
                        } else if shown.with_untracked(|now| now.wants_load(&target)) {
                            show_in(s, shown, target.clone(), place.clone());
                        }
                    };
                    view! {
                        <button
                            type="button"
                            class="study-chip study-hit-translation"
                            data-role="hit-translation"
                            data-translation=code.clone()
                            aria-pressed=move || if pressed() { "true" } else { "false" }
                            on:click=on_click
                        >
                            {translation_short_code(&code)}
                        </button>
                    }
                }
            />
        </nav>
    }
}

/// `favourites` = the reader's visible favourites, computed once for the
/// whole list (up to 1000 rows).
#[component]
pub(super) fn Hit(
    s: Study,
    hit: BibleSearchHit,
    query: String,
    favourites: Memo<Vec<(usize, String)>>,
) -> impl IntoView {
    let reference = hit.reference.clone();
    let original = hit.translation.code.clone();
    let shown = RwSignal::new(Shown::Hit);
    let book = reference.book_code.clone();
    let first = reference.verse_start;
    let last = reference.verse_end.max(first);
    let chapter = reference.chapter;
    let acting = {
        let original = original.clone();
        move || shown.with(|now| now.acting(&original).to_string())
    };
    let label = {
        let place = reference.to_human_readable();
        let original = original.clone();
        move || {
            let code = shown.with(|now| now.code(&original).to_string());
            format!("{place} · {}", translation_short_code(&code))
        }
    };
    let on_add = {
        let book = book.clone();
        let acting = acting.clone();
        move |_| {
            let Some(book) = book.clone() else {
                return;
            };
            add_to_sermon::add(
                s,
                SermonPassage {
                    translation: acting(),
                    book_code: book,
                    chapter,
                    verse_start: first,
                    verse_end: last,
                },
            );
        }
    };
    let on_compare = {
        let book = book.clone();
        let acting = acting.clone();
        move |_| {
            if let Some(book) = book.as_deref() {
                compare::open_at(s, &acting(), (book, chapter, first));
            }
        }
    };
    let on_open = {
        let book = book.clone();
        let acting = acting.clone();
        move |_| {
            let Some(book) = book.as_deref() else {
                return;
            };
            let route =
                StudyRoute::reader(&acting(), book, chapter).with_verses(Some((first, last)));
            nav::go(s, route, ScrollPlan::Verse(first));
        }
    };
    let text = move || shown.with(|now| shown_text(now, &hit, &query));
    let chips = book.clone().map(|book| {
        translation_chips(
            s,
            shown,
            (favourites, original.clone()),
            (book, chapter, first, last),
        )
    });

    view! {
        <div class="study-hit-row" data-role="search-hit-row">
            <button
                type="button"
                class="study-hit"
                data-role="search-hit"
                data-book-code=book.clone().unwrap_or_default()
                data-book-number=reference.book_number.unwrap_or_default()
                data-chapter=chapter
                data-verse=first
                data-translation=acting.clone()
                on:click=on_open
            >
                <span class="study-hit__label">{label}</span>
                <span class="study-hit__text" data-role="search-hit-text">{text}</span>
            </button>
            <div class="study-hit-tools">
                {chips}
                {book.is_some().then(|| view! {
                    <button
                        type="button"
                        class="study-icon study-icon--wide"
                        data-role="search-hit-compare"
                        on:click=on_compare
                    >
                        "Porovnať"
                    </button>
                })}
                <button
                    type="button"
                    class="study-icon"
                    data-role="search-hit-add"
                    aria-label="Pridať do kázne"
                    on:click=on_add
                >
                    "+"
                </button>
            </div>
        </div>
    }
}
