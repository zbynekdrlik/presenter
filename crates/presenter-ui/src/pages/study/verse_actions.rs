//! The verse sheet (spec §6.3): what can be done with the selected verses.
//! It leaves the text tappable: another verse number extends the selection. A
//! new action is one [`VerseAction`] and its arm in [`run`].

use leptos::prelude::*;

use super::sheet::Sheet;
use super::{add_to_sermon, compare, crossrefs, nav, Chapter, Study};
use crate::api::sermons::SermonPassage;
use crate::components::timer_panel::copy_to_clipboard;
use crate::state::study::{copy_text, verse_reference};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VerseAction {
    AddToSermon,
    Compare,
    CrossRefs,
    Copy,
}

impl VerseAction {
    fn label(self, s: Study) -> String {
        match self {
            Self::AddToSermon => "Pridať do kázne".to_string(),
            Self::Compare => "Porovnať".to_string(),
            Self::CrossRefs => crossrefs::button_label(s),
            Self::Copy => "Kopírovať".to_string(),
        }
    }

    fn role(self) -> &'static str {
        match self {
            Self::AddToSermon => "verse-add-to-sermon",
            Self::Compare => "verse-compare",
            Self::CrossRefs => "verse-crossrefs",
            Self::Copy => "verse-copy",
        }
    }
}

const VERSE_ACTIONS: [VerseAction; 4] = [
    VerseAction::AddToSermon,
    VerseAction::Compare,
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

fn run(s: Study, action: VerseAction) {
    match action {
        VerseAction::AddToSermon => {
            if let Some(passage) = passage(s) {
                add_to_sermon::add(s, passage);
            }
        }
        VerseAction::Compare => compare::open_from_selection(s),
        VerseAction::CrossRefs => crossrefs::open(s),
        VerseAction::Copy => {
            let Some((_, text)) = selection(s) else {
                return;
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
        }
    }
}

#[component]
pub(super) fn VerseActionsSheet(s: Study) -> impl IntoView {
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
                                on:click=move |_| run(s, action)
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
