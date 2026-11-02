//! Cross-references (spec §6.7): "Odkazy (N)" on a verse opens the most voted
//! ten, each with its text, and "Ďalšie (N)" the rest. The whole list comes
//! once, when the verse sheet opens, so the button can name N. A multi-verse
//! selection asks about its first verse. The URL is the verse sheet's
//! (`v=1&s=odkazy`), so the sheet survives a reload like the others.

use leptos::prelude::*;

use super::scroll::ScrollPlan;
use super::sheet::Sheet;
use super::{add_to_sermon, nav, study_error, Chapter, Study, REMOTE_TRANSLATION};
use crate::api::sermons::SermonPassage;
use crate::api::study::{load_cross_references, CrossReference};
use crate::state::study::{
    crossref_target, hidden_crossrefs, verse_reference, StudyRoute, StudySheet, FIRST_CROSSREFS,
};

/// `translation`, book, chapter, verse.
type RefKey = (String, String, u16, u16);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) enum CrossRefs {
    #[default]
    Idle,
    Ready {
        key: RefKey,
        items: Vec<CrossReference>,
    },
    Failed {
        key: RefKey,
        message: String,
    },
}

/// The verse the open sheets are about.
fn wanted(s: Study) -> Option<RefKey> {
    let place = s.place()?;
    s.route.with(|route| {
        let (verse, _) = route.verses?;
        matches!(
            route.sheet,
            Some(StudySheet::Actions | StudySheet::CrossRefs)
        )
        .then_some((
            place.translation.clone(),
            place.book.clone(),
            place.chapter,
            verse,
        ))
    })
}

fn holds(s: Study, wanted: &RefKey) -> bool {
    s.crossrefs.with_untracked(|held| match held {
        CrossRefs::Ready { key, .. } => key == wanted,
        _ => false,
    })
}

fn fetch(s: Study, key: RefKey) {
    leptos::task::spawn_local(async move {
        let (translation, book, chapter, verse) = key.clone();
        let answer = load_cross_references(&translation, &book, (chapter, verse)).await;
        if untrack(|| wanted(s)).as_ref() != Some(&key) {
            return;
        }
        s.crossrefs.set(match answer {
            Ok(list) => CrossRefs::Ready {
                key,
                items: list.items,
            },
            Err(err) => CrossRefs::Failed {
                key,
                message: study_error(&err),
            },
        });
    });
}

pub(super) fn install(s: Study) {
    let key = Memo::new(move |_| wanted(s));
    Effect::new(move |_| {
        s.retry.track();
        let Some(key) = key.get() else {
            return;
        };
        if !holds(s, &key) {
            s.crossrefs.set(CrossRefs::Idle);
            fetch(s, key);
        }
    });
}

/// `Odkazy (23)`, or `Odkazy` until the answer is here.
pub(super) fn button_label(s: Study) -> String {
    s.crossrefs.with(|held| match held {
        CrossRefs::Ready { items, .. } => format!("Odkazy ({})", items.len()),
        _ => "Odkazy".to_string(),
    })
}

/// It takes the verse sheet's place: Back never lands on a sheet behind it.
pub(super) fn open(s: Study) {
    nav::replace(s, nav::sheet_route(s, StudySheet::CrossRefs));
}

/// The translation whose text the row shows. The NLT reads the KJV's text but
/// stays the reader's translation, so a tap does not change the Bible in hand.
fn read_in(reader: &str, item: &CrossReference) -> String {
    match &item.fallback_translation {
        Some(code) if reader != REMOTE_TRANSLATION => code.clone(),
        _ => reader.to_string(),
    }
}

fn open_reference(s: Study, reader: &str, item: &CrossReference) {
    nav::go(
        s,
        StudyRoute::reader(&read_in(reader, item), &item.book_code, item.chapter).with_verses(
            Some(crossref_target(
                item.chapter,
                item.verse,
                item.chapter_end,
                item.verse_end,
            )),
        ),
        ScrollPlan::Verse(item.verse),
    );
}

/// A sermon passage stays in one chapter, so a reference across chapters has
/// no add button. The text added is the text the row shows.
fn passage(reader: &str, item: &CrossReference) -> Option<SermonPassage> {
    (item.chapter == item.chapter_end).then(|| SermonPassage {
        translation: item
            .fallback_translation
            .clone()
            .unwrap_or_else(|| reader.to_string()),
        book_code: item.book_code.clone(),
        chapter: item.chapter,
        verse_start: item.verse,
        verse_end: item.verse_end,
    })
}

fn item_view(s: Study, reader: String, item: CrossReference) -> impl IntoView {
    let add = passage(&reader, &item);
    let target = item.clone();
    let code = item.fallback_translation.as_deref().map(|code| {
        format!(
            " ({})",
            presenter_core::bible::sermon::translation_short_code(code)
        )
    });
    view! {
        <article class="study-crossref" data-role="crossref-item" data-votes=item.votes>
            <button
                type="button"
                class="study-crossref__open"
                data-role="crossref-open"
                on:click=move |_| open_reference(s, &reader, &target)
            >
                {item.label}
            </button>
            <p class="study-crossref__text">
                {item.text}
                {code.map(|code| view! { <span class="study-crossref__code">{code}</span> })}
            </p>
            {add.map(|passage| {
                view! {
                    <button
                        type="button"
                        class="study-button"
                        data-role="crossref-add"
                        on:click=move |_| add_to_sermon::add(s, passage.clone())
                    >
                        "Pridať do kázne"
                    </button>
                }
            })}
        </article>
    }
}

fn sheet_title(s: Study) -> String {
    let Some((_, _, _, verse)) = wanted(s) else {
        return "Krížové odkazy".to_string();
    };
    s.chapter.with(|chapter| match chapter {
        Chapter::Ready(place, text) => format!(
            "Odkazy · {}",
            verse_reference(
                &text.book,
                place.chapter,
                (verse, verse),
                &place.translation
            )
        ),
        _ => "Krížové odkazy".to_string(),
    })
}

#[component]
pub(super) fn CrossRefSheet(s: Study) -> impl IntoView {
    let everything = RwSignal::new(false);
    let title = Signal::derive(move || sheet_title(s));
    let reader = move || {
        s.route
            .with(|route| route.translation.clone())
            .unwrap_or_default()
    };
    let list =
        move || {
            s.crossrefs.with(|held| {
                match held {
            CrossRefs::Idle => view! {
                <p class="study-sheet__hint" data-role="crossref-loading">"Načítava sa\u{2026}"</p>
            }
            .into_any(),
            CrossRefs::Failed { message, .. } => {
                let message = message.clone();
                view! {
                    <div class="study-error" data-role="crossref-error" role="alert">
                        <p>{message}</p>
                        <button
                            type="button"
                            class="study-button"
                            data-role="study-retry"
                            on:click=move |_| s.retry.update(|n| *n = n.wrapping_add(1))
                        >
                            "Skúsiť znova"
                        </button>
                    </div>
                }
                .into_any()
            }
            CrossRefs::Ready { items, .. } if items.is_empty() => view! {
                <p class="study-sheet__hint" data-role="crossref-empty">
                    "Tento verš nemá krížové odkazy."
                </p>
            }
            .into_any(),
            CrossRefs::Ready { items, .. } => {
                let shown = if everything.get() { items.len() } else { FIRST_CROSSREFS };
                let hidden = hidden_crossrefs(items.len(), shown);
                let reader = reader();
                let rows = items
                    .iter()
                    .take(shown)
                    .cloned()
                    .map(|item| item_view(s, reader.clone(), item))
                    .collect_view();
                let more = (hidden > 0).then(|| {
                    view! {
                        <button
                            type="button"
                            class="study-button"
                            data-role="crossref-more"
                            on:click=move |_| everything.set(true)
                        >
                            {format!("Ďalšie ({hidden})")}
                        </button>
                    }
                });
                view! {
                    {rows}
                    {more}
                }
                .into_any()
            }
        }
            })
        };
    view! {
        <Sheet s=s role="crossref-sheet" title=title modal=true>
            <div class="study-crossrefs" data-role="crossref-list">{list}</div>
            <p class="study-sheet__hint" data-role="crossref-source">
                "Krížové odkazy: OpenBible.info (CC BY)"
            </p>
        </Sheet>
    }
}
