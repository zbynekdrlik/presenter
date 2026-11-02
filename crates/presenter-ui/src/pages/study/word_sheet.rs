//! The KJV word sheet (spec §6.6): a tapped word's Strong's entry, over the
//! reader or compare. Its URL is `w=G4102&wv=1.3`; a place naming no word on the
//! page closes the sheet. ◀ ▶ and the number tabs replace the entry, a
//! derivation link adds one, so Back returns through the links.

use std::collections::HashMap;

use leptos::prelude::*;
use presenter_core::bible::resolver::Step;

use super::compare::{self, NumberedText};
use super::scroll::ScrollPlan;
use super::sheet::Sheet;
use super::{nav, reader, study_error, Chapter, Study};
use crate::api::sermons::is_not_found;
use crate::api::study::{load_word_entry, WordEntry};
use crate::state::study::{StudyRoute, StudyVerse, StudyView};
use crate::state::study_word::{
    find_word, first_sentence, linked_parts, neighbour_word, WordInfo, WordPlace, SCOPE_ALL,
};

/// A Strong's entry in memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Lexicon {
    Loading,
    Ready(Box<WordEntry>),
    /// No such entry here: the dictionary was not imported.
    Missing,
    Failed(String),
}

pub(super) type LexiconEntries = HashMap<String, Lexicon>;

/// What the page holds of the word on the route.
#[derive(Debug, Clone, PartialEq, Eq)]
enum WordOnPage {
    /// The text is on its way.
    Waiting,
    /// The text's read failed: its retry may still bring the word.
    TextFailed,
    /// No such word here: nothing to show a sheet about.
    Gone,
    Here(WordInfo),
}

fn among(verses: &[StudyVerse], word: &WordPlace, only: Option<u16>) -> WordOnPage {
    find_word(verses, word, only).map_or(WordOnPage::Gone, WordOnPage::Here)
}

fn in_reader(s: Study, word: &WordPlace) -> WordOnPage {
    let Some(wanted) = s.place() else {
        return WordOnPage::Waiting;
    };
    if s.pending.with(Option::is_some) {
        return WordOnPage::Waiting;
    }
    s.chapter.with(|chapter| match chapter {
        Chapter::Ready(place, text) if *place == wanted => among(&text.verses, word, None),
        Chapter::Missing(place) if *place == wanted => WordOnPage::Gone,
        Chapter::Failed(place, _) if *place == wanted => WordOnPage::TextFailed,
        _ => WordOnPage::Waiting,
    })
}

fn word_on_page(s: Study) -> Option<WordOnPage> {
    let word = s.route.with(|route| route.word.clone())?;
    let compared = s
        .route
        .with(|route| matches!(route.view, StudyView::Compare { .. }));
    if !compared {
        return Some(in_reader(s, &word));
    }
    Some(match compare::numbered_text(s) {
        NumberedText::Waiting => WordOnPage::Waiting,
        NumberedText::Absent => WordOnPage::Gone,
        NumberedText::Failed => WordOnPage::TextFailed,
        NumberedText::Here { verses, only } => among(&verses, &word, only),
    })
}

fn fetch_entry(s: Study, number: String) {
    s.lexicon.update(|entries| {
        entries.insert(number.clone(), Lexicon::Loading);
    });
    leptos::task::spawn_local(async move {
        let entry = match load_word_entry(&number).await {
            Ok(entry) => Lexicon::Ready(Box::new(entry)),
            Err(err) if is_not_found(&err) => Lexicon::Missing,
            Err(err) => Lexicon::Failed(study_error(&err)),
        };
        s.lexicon.update(|entries| {
            entries.insert(number, entry);
        });
    });
}

/// The number the sheet or the Strong's view shows.
fn wanted_number(s: Study) -> Option<String> {
    s.route.with(|route| match &route.view {
        StudyView::Strongs { number, .. } => Some(number.clone()),
        _ => route.word.as_ref().map(|word| word.strongs.clone()),
    })
}

pub(super) fn install(s: Study) {
    let number = Memo::new(move |_| wanted_number(s));
    Effect::new(move |_| {
        s.retry.track();
        let Some(number) = number.get() else {
            return;
        };
        let needed = s.lexicon.with_untracked(|entries| {
            matches!(entries.get(&number), None | Some(Lexicon::Failed(_)))
        });
        if needed {
            fetch_entry(s, number);
        }
    });
    let on_page = Memo::new(move |_| word_on_page(s));
    Effect::new(move |_| {
        if on_page.get() == Some(WordOnPage::Gone) {
            nav::replace(s, s.route.get_untracked().without_sheet());
        }
    });
}

/// A tap on a word. The sheet in the way goes, or Back would land on it.
pub(super) fn open(s: Study, word: WordPlace) {
    let reading = untrack(|| s.place().is_some());
    if reading && !reader::chapter_settled_untracked(s) {
        return;
    }
    let route = s.route.get_untracked();
    if route.sheet.is_some() {
        nav::replace(s, route.with_verses(None).with_word(word));
    } else {
        nav::open_sheet(s, route.with_word(word));
    }
}

fn replace_word(s: Study, word: WordPlace) {
    nav::replace(s, s.route.get_untracked().with_word(word));
}

/// A number of a derivation: Back returns to this entry.
fn follow(s: Study, number: &str) {
    let route = s.route.get_untracked();
    if let Some(word) = route.word.clone() {
        nav::open_sheet(s, route.with_word(word.with_strongs(number)));
    }
}

fn open_occurrences(s: Study, number: &str) {
    let translation = s.route.with_untracked(|route| route.translation.clone());
    nav::leave_word_sheet(
        s,
        StudyRoute::strongs(translation, number, SCOPE_ALL),
        ScrollPlan::Top,
    );
}

/// The derivation with every number a link.
fn derivation_view(s: Study, text: &str) -> impl IntoView {
    linked_parts(text)
        .into_iter()
        .map(|(piece, link)| match link {
            Some(number) => {
                let target = number.clone();
                view! {
                    <button
                        type="button"
                        class="study-word-link"
                        data-role="word-sheet-link"
                        data-strongs=number
                        on:click=move |_| follow(s, &target)
                    >
                        {piece}
                    </button>
                }
                .into_any()
            }
            None => piece.into_any(),
        })
        .collect_view()
}

fn entry_view(s: Study, entry: &WordEntry, full: RwSignal<bool>) -> impl IntoView {
    let rtl = entry.strongs.starts_with('H').then_some("rtl");
    let whole = entry.definition.clone();
    let peek = first_sentence(&entry.definition).to_string();
    let definition = move || {
        if full.get() {
            whole.clone()
        } else {
            peek.clone()
        }
    };
    let derivation = entry.derivation.clone();
    let linked = move || {
        (full.get() && !derivation.is_empty()).then(|| {
            view! {
                <p class="study-word-entry__derivation" data-role="word-sheet-derivation">
                    {derivation_view(s, &derivation)}
                </p>
            }
        })
    };
    let number = entry.strongs.clone();
    view! {
        <p class="study-word-entry__lemma">
            <span class="study-word-entry__greek" data-role="word-sheet-lemma" dir=rtl lang=rtl.map(|_| "he")>
                {entry.lemma.clone()}
            </span>
            <span>{entry.transliteration.clone()}</span>
            <span class="study-word-entry__sound">{entry.pronunciation.clone()}</span>
        </p>
        <p class="study-word-entry__definition" data-role="word-sheet-definition">{definition}</p>
        {linked}
        <p class="study-word-entry__kjv" data-role="word-sheet-kjv">
            {format!("V KJV: {}", entry.kjv_renderings)}
        </p>
        <div class="study-word-entry__actions">
            <button
                type="button"
                class="study-button study-button--primary"
                data-role="word-sheet-occurrences"
                on:click=move |_| open_occurrences(s, &number)
            >
                {format!("Všetky výskyty ({})", entry.occurrence_count)}
            </button>
            <button
                type="button"
                class="study-button"
                data-role="word-sheet-full"
                aria-pressed=move || if full.get() { "true" } else { "false" }
                on:click=move |_| full.update(|full| *full = !*full)
            >
                "Celé heslo"
            </button>
        </div>
    }
}

/// One tab per number of a word that has several.
fn tabs_view(s: Study, numbers: Vec<String>, current: Memo<String>) -> impl IntoView {
    (numbers.len() > 1).then(|| {
        let tabs = numbers
            .into_iter()
            .map(|number| {
                let mine = number.clone();
                let target = number.clone();
                let label = number.clone();
                let on_tab = move |_| {
                    let word = s.route.with_untracked(|route| route.word.clone());
                    if let Some(word) = word {
                        replace_word(s, word.with_strongs(&target));
                    }
                };
                view! {
                    <button
                        type="button"
                        class="study-chip"
                        data-role="word-sheet-tab"
                        data-strongs=number.clone()
                        aria-pressed=move || if current.get() == mine { "true" } else { "false" }
                        on:click=on_tab
                    >
                        {label}
                    </button>
                }
            })
            .collect_view();
        view! { <div class="study-chips" data-role="word-sheet-tabs">{tabs}</div> }
    })
}

#[component]
pub(super) fn WordSheet(s: Study) -> impl IntoView {
    let full = RwSignal::new(false);
    let info = Memo::new(move |_| match word_on_page(s) {
        Some(WordOnPage::Here(info)) => Some(info),
        _ => None,
    });
    let number = Memo::new(move |_| {
        s.route
            .with(|route| route.word.as_ref().map(|word| word.strongs.clone()))
            .unwrap_or_default()
    });
    let title = Signal::derive(move || match info.with(|word| word.clone()) {
        Some(word) => format!("{} · {}", word.text, number.get()),
        None => number.get(),
    });
    let entry = Memo::new(move |_| {
        s.lexicon
            .with(|entries| entries.get(&number.get()).cloned())
    });
    let body = move || match entry.get() {
        Some(Lexicon::Ready(entry)) => entry_view(s, &entry, full).into_any(),
        Some(Lexicon::Missing) => view! {
            <p class="study-sheet__hint" data-role="word-sheet-missing">
                "Slovník nie je nainštalovaný"
            </p>
        }
        .into_any(),
        Some(Lexicon::Failed(message)) => view! {
            <div class="study-error" data-role="word-sheet-error" role="alert">
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
        .into_any(),
        Some(Lexicon::Loading) | None => view! {
            <p class="study-sheet__hint" data-role="word-sheet-loading">"Načítava sa\u{2026}"</p>
        }
        .into_any(),
    };
    let tabs = move || info.get().map(|word| tabs_view(s, word.numbers, number));
    // A phone's backdrop covers the text's own retry.
    let text_failed = move || word_on_page(s) == Some(WordOnPage::TextFailed);
    let target = move |dir| {
        info.with(|word| {
            word.as_ref()
                .and_then(|word| neighbour_word(word, dir).cloned())
        })
    };
    let step = move |dir| {
        if let Some(word) = target(dir) {
            replace_word(s, word);
        }
    };
    view! {
        <Sheet
            s=s
            role="word-sheet"
            title=title
            modal=true
            state=Signal::derive(move || if full.get() { "full" } else { "peek" })
            strongs=Signal::derive(move || number.get())
        >
            <Show when=text_failed>
                <div class="study-error" data-role="word-sheet-text-error" role="alert">
                    <p>"Text sa nepodarilo načítať."</p>
                    <button
                        type="button"
                        class="study-button"
                        data-role="study-retry"
                        on:click=move |_| s.retry.update(|n| *n = n.wrapping_add(1))
                    >
                        "Skúsiť znova"
                    </button>
                </div>
            </Show>
            {tabs}
            <div class="study-word-entry">{body}</div>
            <div class="study-word-steps">
                <button
                    type="button"
                    class="study-button"
                    data-role="word-sheet-prev"
                    aria-label="Predchádzajúce slovo"
                    disabled=move || target(Step::Prev).is_none()
                    on:click=move |_| step(Step::Prev)
                >
                    "\u{25C0} slovo"
                </button>
                <button
                    type="button"
                    class="study-button"
                    data-role="word-sheet-next"
                    aria-label="Nasledujúce slovo"
                    disabled=move || target(Step::Next).is_none()
                    on:click=move |_| step(Step::Next)
                >
                    "slovo \u{25B6}"
                </button>
            </div>
        </Sheet>
    }
}
