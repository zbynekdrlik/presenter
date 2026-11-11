//! The verse sheet (spec §6.3): what can be done with the selected verses.
//! It leaves the text tappable: another verse number extends the selection. A
//! new action is one [`VerseAction`] and its arm in [`run`].

use leptos::prelude::*;

use super::sheet::Sheet;
use super::{add_to_sermon, bookmarks, compare, crossrefs, nav, Chapter, Study};
use crate::api::sermons::SermonPassage;
use crate::components::timer_panel::copy_to_clipboard;
use crate::state::study::{copy_text, verse_reference};

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
            Self::AddToSermon => "Pridať do kázne".to_string(),
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
            <p class="study-sheet__hint">"Ťuknutím na ďalší verš vyberieš rozsah."</p>
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
