//! The verse sheet (spec §6.3): what can be done with the selected verses.
//! It leaves the text tappable: another verse number extends the selection. A
//! new action is one [`VerseAction`] and its arm in [`run`] (E1 adds "Pridať do
//! kázne", S4 "Porovnať" and "Odkazy").

use leptos::prelude::*;

use super::sheet::Sheet;
use super::{nav, Chapter, Study};
use crate::components::timer_panel::copy_to_clipboard;
use crate::state::study::{copy_text, verse_reference};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VerseAction {
    Copy,
}

impl VerseAction {
    fn label(self) -> &'static str {
        match self {
            Self::Copy => "Kopírovať",
        }
    }

    fn role(self) -> &'static str {
        match self {
            Self::Copy => "verse-copy",
        }
    }
}

const VERSE_ACTIONS: [VerseAction; 1] = [VerseAction::Copy];

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

fn run(s: Study, action: VerseAction) {
    match action {
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
                                {action.label()}
                            </button>
                        }
                    })
                    .collect_view()}
            </div>
        </Sheet>
    }
}
