//! A verse's text (spec §5.2, §6.3), the ONE renderer of the reader and compare:
//! headings, italics, notes and the tagged words of the KJV. A verse with no
//! markup is its plain text. Words are spans inside the line, so a tap is read
//! here, once, for the whole verse: a chapter has hundreds of them.

use leptos::prelude::*;
use presenter_core::bible::{MarkupSegment, SegmentKind};
use wasm_bindgen::JsCast;

use super::{word_sheet, Study};
use crate::state::session;
use crate::state::study::StudyVerse;
use crate::state::study_word::{WordPlace, KEY_NUMBERS};

const WORD: &str = "[data-role=\"study-word\"]";
const BADGE: &str = "[data-role=\"study-strongs-badge\"]";

/// The word an event landed on; on a badge, that badge's number.
fn tapped_word(ev: &web_sys::Event) -> Option<WordPlace> {
    let target = ev.target()?.dyn_into::<web_sys::Element>().ok()?;
    let word = target.closest(WORD).ok()??;
    let badge = target.closest(BADGE).ok()?;
    let strongs = badge
        .unwrap_or_else(|| word.clone())
        .get_attribute("data-strongs")?;
    Some(WordPlace {
        strongs,
        verse: word.get_attribute("data-verse")?.parse().ok()?,
        token: word.get_attribute("data-token")?.parse().ok()?,
    })
}

/// Text dragged over with a mouse: the click that ends it is no tap.
fn text_selected() -> bool {
    crate::utils::window::window()
        .get_selection()
        .ok()
        .flatten()
        .is_some_and(|selection| !selection.is_collapsed())
}

fn word_view(
    verse: u16,
    token: usize,
    segment: &MarkupSegment,
    numbers: bool,
    active: Memo<Option<usize>>,
) -> AnyView {
    let text = segment.text.clone();
    let printed = if segment.italic {
        view! { <i>{text}</i> }.into_any()
    } else {
        text.into_any()
    };
    let badges = numbers.then(|| {
        segment
            .strongs
            .iter()
            .map(|number| {
                view! {
                    <sup
                        class="study-strongs-badge"
                        data-role="study-strongs-badge"
                        data-strongs=number.clone()
                    >
                        {number.clone()}
                    </sup>
                }
            })
            .collect_view()
    });
    view! {
        <span
            class="study-word"
            data-role="study-word"
            role="button"
            tabindex="0"
            data-strongs=segment.strongs.first().cloned()
            data-verse=verse
            data-token=token
            data-active=move || (active.get() == Some(token)).then_some("true")
        >
            {printed}
            {badges}
        </span>
    }
    .into_any()
}

/// A † in the line, its text under it when opened: no state worth a URL.
#[component]
fn Note(text: String) -> impl IntoView {
    let open = RwSignal::new(false);
    view! {
        <button
            type="button"
            class="study-note"
            data-role="study-note"
            aria-label="Poznámka"
            aria-expanded=move || if open.get() { "true" } else { "false" }
            on:click=move |_| open.update(|open| *open = !*open)
        >
            "\u{2020}"
        </button>
        <Show when=move || open.get()>
            <span class="study-note__text" data-role="study-note-text" role="note">
                {text.clone()}
            </span>
        </Show>
    }
}

fn inline_view(
    verse: u16,
    token: usize,
    segment: &MarkupSegment,
    numbers: bool,
    active: Memo<Option<usize>>,
) -> AnyView {
    match segment.kind {
        SegmentKind::Heading => ().into_any(),
        SegmentKind::Note => view! { <Note text=segment.text.clone() /> }.into_any(),
        SegmentKind::Text if !segment.strongs.is_empty() => {
            word_view(verse, token, segment, numbers, active)
        }
        SegmentKind::Text if segment.italic => view! { <i>{segment.text.clone()}</i> }.into_any(),
        SegmentKind::Text => segment.text.clone().into_any(),
    }
}

/// `class` = the reader's or compare's; `kind` = compare's `data-kind`;
/// `on_tap` = the reader's: a tap on the text, told whether it hit a word
/// (that word's sheet opens after it).
#[component]
pub(super) fn VerseBody(
    s: Study,
    verse: StudyVerse,
    class: &'static str,
    #[prop(optional)] kind: Option<&'static str>,
    #[prop(optional)] on_tap: Option<Callback<bool>>,
) -> impl IntoView {
    let number = verse.verse;
    let active = Memo::new(move |_| {
        s.route.with(|route| {
            let word = route.word.as_ref()?;
            (word.verse == number).then_some(word.token)
        })
    });
    let segments = verse.markup.map(|markup| markup.0).unwrap_or_default();
    let headings = segments
        .iter()
        .filter(|segment| segment.kind == SegmentKind::Heading)
        .map(|segment| {
            view! {
                <h3 class="study-heading" data-role="study-heading">{segment.text.clone()}</h3>
            }
        })
        .collect_view();
    let text = verse.text;
    // A verse renders again when the chip turns: one closure per verse.
    let line = move || {
        if segments.is_empty() {
            return text.clone().into_any();
        }
        let numbers = s.numbers.get();
        segments
            .iter()
            .enumerate()
            .map(|(token, segment)| inline_view(number, token, segment, numbers, active))
            .collect_view()
            .into_any()
    };
    let on_click = move |ev: web_sys::MouseEvent| {
        if text_selected() {
            return;
        }
        let word = tapped_word(&ev);
        if let Some(on_tap) = on_tap {
            on_tap.run(word.is_some());
        }
        if let Some(word) = word {
            ev.stop_propagation();
            word_sheet::open(s, word);
        }
    };
    let on_key = move |ev: web_sys::KeyboardEvent| {
        if !matches!(ev.key().as_str(), "Enter" | " ") {
            return;
        }
        if let Some(word) = tapped_word(&ev) {
            ev.prevent_default();
            word_sheet::open(s, word);
        }
    };
    view! {
        <div class=class data-kind=kind on:click=on_click on:keydown=on_key>
            {headings}
            <p class="study-line">{line}</p>
        </div>
    }
}

/// "Čísla": the numbers after the words, kept per device. Only while the text
/// on screen has some.
#[component]
pub(super) fn NumbersChip(s: Study, shown: Memo<bool>) -> impl IntoView {
    let toggle = move |_| {
        let on = !s.numbers.get_untracked();
        session::set_persistent(KEY_NUMBERS, if on { "1" } else { "0" });
        s.numbers.set(on);
    };
    view! {
        <Show when=move || shown.get()>
            <button
                type="button"
                class="study-chip"
                data-role="study-numbers"
                aria-pressed=move || if s.numbers.get() { "true" } else { "false" }
                on:click=toggle
            >
                "Čísla"
            </button>
        </Show>
    }
}
