//! The verse sheet (spec §6.3): what can be done with the selected verses.
//! It leaves the text tappable: another verse extends the selection, and the
//! range row says how many verses are selected and steps the end with − / +
//! (#936: the sheet may cover the verses below). A new action is one
//! [`VerseAction`] and its arm in [`run`].

use leptos::prelude::*;

use super::sheet::Sheet;
use super::{add_to_sermon, bookmarks, compare, crossrefs, nav, Chapter, Study};
use crate::api::sermons::SermonPassage;
use crate::components::timer_panel::copy_to_clipboard;
use crate::state::study::{copy_text, verse_reference};
use crate::state::study_range::{range_len, step_range_end, verse_count_text};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VerseAction {
    AddToSermon,
    Compare,
    CrossRefs,
    SaveBookmark,
    Copy,
}

impl VerseAction {
    fn label(self, s: Study) -> String {
        match self {
            Self::AddToSermon => match selected_len(s) {
                Some(count) if count > 1 => {
                    format!("Pridať do kázne ({})", verse_count_text(count))
                }
                _ => "Pridať do kázne".to_string(),
            },
            Self::Compare => "Porovnať".to_string(),
            Self::CrossRefs => crossrefs::button_label(s),
            Self::SaveBookmark => "Pridať záložku".to_string(),
            Self::Copy => "Kopírovať".to_string(),
        }
    }

    fn role(self) -> &'static str {
        match self {
            Self::AddToSermon => "verse-add-to-sermon",
            Self::Compare => "verse-compare",
            Self::CrossRefs => "verse-crossrefs",
            Self::SaveBookmark => "verse-save-bookmark",
            Self::Copy => "verse-copy",
        }
    }
}

/// The order of use (#932): the owner's most used one first, then the SNV
/// counters of the phone (`GET /ui/usage`, 90 days: compare 5, add to sermon 2,
/// cross-references 1, copy 0).
const VERSE_ACTIONS: [VerseAction; 5] = [
    VerseAction::SaveBookmark,
    VerseAction::Compare,
    VerseAction::AddToSermon,
    VerseAction::CrossRefs,
    VerseAction::Copy,
];

/// The selection's reference, and its text for the clipboard.
fn selection(s: Study) -> Option<(String, String)> {
    let verses = s.route.with(|route| route.verses)?;
    let wanted = s.place()?;
    s.chapter.with(|chapter| {
        let Chapter::Ready(place, text) = chapter else {
            return None;
        };
        if *place != wanted {
            return None;
        }
        let reference = verse_reference(&text.book, place.chapter, verses, &place.translation);
        let chosen = text
            .verses
            .iter()
            .filter(|verse| (verses.0..=verses.1).contains(&verse.verse))
            .map(|verse| (verse.verse, verse.text.as_str()));
        let copied = copy_text(chosen, &reference);
        Some((reference, copied))
    })
}

/// How many verses are selected.
fn selected_len(s: Study) -> Option<u16> {
    s.route.with(|route| route.verses).map(range_len)
}

/// The last verse of the chapter on screen.
fn last_verse(s: Study) -> Option<u16> {
    s.chapter.with(|chapter| match chapter {
        Chapter::Ready(_, text) => text.verses.last().map(|verse| verse.verse),
        _ => None,
    })
}

/// − / +: the selection's end, one verse back or on.
fn step_end(s: Study, delta: i32) {
    let route = s.route.get_untracked();
    let Some(verses) = route.verses else {
        return;
    };
    let Some(last) = leptos::prelude::untrack(|| last_verse(s)) else {
        return;
    };
    if let Some(range) = step_range_end(verses, delta, last) {
        nav::replace(s, route.with_verses(Some(range)));
    }
}

/// "3 verše" and the − / + of the end (#936).
#[component]
fn RangeRow(s: Study) -> impl IntoView {
    let count = move || selected_len(s).map(verse_count_text).unwrap_or_default();
    let at_start = move || {
        s.route
            .with(|route| route.verses.is_none_or(|(a, b)| a == b))
    };
    let at_end = move || {
        let end = s.route.with(|route| route.verses.map(|(_, end)| end));
        end.is_none() || end == last_verse(s)
    };
    view! {
        <div class="study-range" data-role="verse-range">
            <span class="study-range__count" data-role="verse-range-count">{count}</span>
            <button
                type="button"
                class="study-button"
                data-role="verse-range-shrink"
                aria-label="O verš menej"
                disabled=at_start
                on:click=move |_| step_end(s, -1)
            >
                "− verš"
            </button>
            <button
                type="button"
                class="study-button"
                data-role="verse-range-grow"
                aria-label="O verš viac"
                disabled=at_end
                on:click=move |_| step_end(s, 1)
            >
                "+ verš"
            </button>
        </div>
    }
}

/// The selection as a sermon passage. The route's chapter is the one on screen.
fn passage(s: Study) -> Option<SermonPassage> {
    let (verse_start, verse_end) = s.route.with_untracked(|route| route.verses)?;
    let place = s.place()?;
    Some(SermonPassage {
        translation: place.translation,
        book_code: place.book,
        chapter: place.chapter,
        verse_start,
        verse_end,
    })
}

/// Runs the action; `true` = it closed the sheet (a history step, so the
/// sheet stays on screen a moment longer and must not run it twice).
fn run(s: Study, action: VerseAction) -> bool {
    match action {
        VerseAction::AddToSermon => {
            if let Some(passage) = passage(s) {
                add_to_sermon::add(s, passage);
            }
            false
        }
        VerseAction::Compare => {
            compare::open_from_selection(s);
            false
        }
        VerseAction::CrossRefs => {
            crossrefs::open(s);
            false
        }
        VerseAction::SaveBookmark => bookmarks::add_here(s),
        VerseAction::Copy => {
            let Some((_, text)) = selection(s) else {
                return false;
            };
            s.notice.set(Some(
                if copy_to_clipboard(&text) {
                    "Skopírované"
                } else {
                    "Kopírovanie zlyhalo"
                }
                .to_string(),
            ));
            nav::close_sheet(s);
            true
        }
    }
}

#[component]
pub(super) fn VerseActionsSheet(s: Study) -> impl IntoView {
    // An action that closes the sheet runs once: the close is a history step,
    // so a quick second tap would add a second bookmark and step back twice.
    let closing = StoredValue::new(false);
    let title = Signal::derive(move || selection(s).map(|(label, _)| label).unwrap_or_default());
    view! {
        <Sheet s=s role="verse-actions" title=title>
            <RangeRow s=s />
            <p class="study-sheet__hint">
                "Ťukni na ďalší verš v texte — výber sa po neho rozšíri."
            </p>
            <div class="study-sheet__actions">
                {VERSE_ACTIONS
                    .into_iter()
                    .map(|action| {
                        view! {
                            <button
                                type="button"
                                class="study-button study-button--primary"
                                data-role=action.role()
                                on:click=move |_| {
                                    if !closing.get_value() && run(s, action) {
                                        closing.set_value(true);
                                    }
                                }
                            >
                                {move || action.label(s)}
                            </button>
                        }
                    })
                    .collect_view()}
            </div>
        </Sheet>
    }
}
