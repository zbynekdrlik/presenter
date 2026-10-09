//! The chapter / verse inputs of the Bible live tab (out of `bible.rs` since
//! #825): the #257 keyboard flow (chapter → Enter → verse start → Enter →
//! verse end), the #702 start → end mirror, and the #825 range bound — a typed
//! chapter or verse past the selected book's range is clamped to the last one,
//! the boxes show the maximum ("/ N", `max=`), and an inline note says why
//! (`data-role="bible-range-hint"`). The decision lives in the host-tested
//! `state::bible_range`.

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use super::bible::BibleFocusRefs;
use crate::state::bible::BibleState;
use crate::state::bible_range::{
    bound_chapter, bound_verse, next_hint, BoundedInput, RangeField, RangeHint,
};

/// The input element an event fired on.
fn event_input(ev: &web_sys::Event) -> Option<web_sys::HtmlInputElement> {
    ev.target()
        .and_then(|target| target.dyn_into::<web_sys::HtmlInputElement>().ok())
}

/// Put the bounded value back into the box when the typed one was clamped —
/// the signal may already hold that value, so a re-render alone would leave
/// the over-range text visible.
fn show_bounded(input: &web_sys::HtmlInputElement, bounded: &BoundedInput) {
    if bounded.hint.is_some() {
        input.set_value(&bounded.value.to_string());
    }
}

#[component]
pub(super) fn ReferenceInputs() -> impl IntoView {
    let bs = use_ctx!(BibleState);
    let refs = expect_context::<BibleFocusRefs>();
    let book_filter = bs.book_filter;
    let selected_book = bs.selected_book;
    let selected_chapter = bs.selected_chapter;
    let verse_start_signal = bs.verse_start;
    let verse_end_signal = bs.verse_end;
    let range_hint = RwSignal::new(None::<RangeHint>);

    // A book change clears the note (#825).
    Effect::new(move || {
        selected_book.with(|_| ());
        range_hint.set(None);
    });

    // The selected book's chapter count + per-chapter verse counts; (0, [])
    // without a book, when only the lower bound 1 applies.
    let counts = move || {
        selected_book.with_untracked(|book| {
            book.as_ref()
                .map(|book| (book.chapter_count, book.verse_counts.clone()))
                .unwrap_or_default()
        })
    };
    // The note after a commit to `field` whose value was `previous` — kept
    // when the box only re-commits the value the clamp wrote (`next_hint`).
    let update_hint = move |field: RangeField, previous: u16, bounded: &BoundedInput| {
        let next = range_hint
            .with_untracked(|current| next_hint(current.as_ref(), field, previous, bounded));
        range_hint.set(next);
    };
    let apply_chapter = move |typed: u16| -> BoundedInput {
        let (chapter_count, verse_counts) = counts();
        let previous = selected_chapter.get_untracked();
        let bounded = bound_chapter(typed, chapter_count, &verse_counts);
        update_hint(RangeField::Chapter, previous, &bounded);
        // A re-commit of the same chapter must not reset the verses typed
        // meanwhile (the `change` can arrive after the focus moved on).
        if bounded.value != previous {
            verse_start_signal.set(1);
            verse_end_signal.set(None);
        }
        selected_chapter.set(bounded.value);
        bounded
    };
    let apply_verse_start = move |typed: u16| -> BoundedInput {
        let (chapter_count, verse_counts) = counts();
        let chapter = selected_chapter.get_untracked();
        let previous = verse_start_signal.get_untracked();
        let bounded = bound_verse(typed, chapter, chapter_count, &verse_counts);
        update_hint(RangeField::VerseStart, previous, &bounded);
        verse_start_signal.set(bounded.value);
        // #702: mirror the start into the end — the dominant case is a
        // single verse, so entering a start auto-fills the end with the same
        // number. A later explicit end edit persists (nothing re-mirrors
        // until the start changes again), so a range / to-end is still one
        // edit away.
        verse_end_signal.set(Some(bounded.value));
        bounded
    };
    let apply_verse_end = move |typed: u16| -> BoundedInput {
        let (chapter_count, verse_counts) = counts();
        let chapter = selected_chapter.get_untracked();
        let previous = verse_end_signal.get_untracked().unwrap_or(0);
        let bounded = bound_verse(typed, chapter, chapter_count, &verse_counts);
        update_hint(RangeField::VerseEnd, previous, &bounded);
        verse_end_signal.set(Some(bounded.value));
        bounded
    };
    // An emptied end box means "to the end of the chapter".
    let commit_verse_end = move |input: &web_sys::HtmlInputElement| {
        let val_str = input.value();
        if val_str.is_empty() {
            verse_end_signal.set(None);
            range_hint.set(None);
        } else if let Ok(val) = val_str.parse::<u16>() {
            show_bounded(input, &apply_verse_end(val));
        }
    };

    let on_chapter = move |ev: web_sys::Event| {
        if let Some(input) = event_input(&ev) {
            if let Ok(val) = input.value().parse::<u16>() {
                show_bounded(&input, &apply_chapter(val));
            }
        }
    };

    let on_verse_start = move |ev: web_sys::Event| {
        if let Some(input) = event_input(&ev) {
            if let Ok(val) = input.value().parse::<u16>() {
                show_bounded(&input, &apply_verse_start(val));
            }
        }
    };

    let on_verse_end = move |ev: web_sys::Event| {
        if let Some(input) = event_input(&ev) {
            commit_verse_end(&input);
        }
    };

    // Enter on chapter → commit (bounded) chapter value, jump to verse-start.
    let on_chapter_keydown = move |ev: web_sys::KeyboardEvent| {
        if ev.key() != "Enter" {
            return;
        }
        ev.prevent_default();
        if let Some(input) = refs.chapter.get() {
            if let Ok(val) = input.value().parse::<u16>() {
                show_bounded(&input, &apply_chapter(val));
            }
        }
        if let Some(el) = refs.verse_start.get() {
            let _ = el.focus();
            el.select();
        }
    };

    // Enter on verse-start → commit (bounded) value, jump to verse-end. The
    // end input is focused + selected below, so a range is one type away.
    let on_verse_start_keydown = move |ev: web_sys::KeyboardEvent| {
        if ev.key() != "Enter" {
            return;
        }
        ev.prevent_default();
        if let Some(input) = refs.verse_start.get() {
            if let Ok(val) = input.value().parse::<u16>() {
                show_bounded(&input, &apply_verse_start(val));
            }
        }
        if let Some(el) = refs.verse_end.get() {
            let _ = el.focus();
            el.select();
        }
    };

    // Enter on verse-end → commit (or clear) value, return to book-filter.
    // The debounced auto-load effect (`bible.rs` mount-time) already fires
    // a passage fetch 300ms after the signal updates, so no explicit load
    // click is needed here. Clearing the filter collapses the book list so
    // the operator can immediately start typing the next book.
    let on_verse_end_keydown = move |ev: web_sys::KeyboardEvent| {
        if ev.key() != "Enter" {
            return;
        }
        ev.prevent_default();
        if let Some(input) = refs.verse_end.get() {
            commit_verse_end(&input);
            let _ = input.blur();
        }
        book_filter.set(String::new());
        if let Some(el) = refs.book_filter.get() {
            let _ = el.focus();
        }
    };

    // The maximum chapter of the book and verse of the selected chapter.
    let chapter_max = move || selected_book.with(|book| book.as_ref().map(|b| b.chapter_count));
    let verse_max = move || {
        let chapter = usize::from(selected_chapter.get().max(1));
        selected_book.with(|book| {
            book.as_ref()
                .and_then(|b| b.verse_counts.get(chapter - 1).copied())
        })
    };

    view! {
        <div class="operator__reference-grid">
            <label class="operator__field">
                <span>
                    "Chapter"
                    {move || chapter_max().map(|n| view! {
                        <span class="operator__field-max" data-role="chapter-max">{format!("/ {n}")}</span>
                    })}
                </span>
                <input
                    type="number"
                    data-role="chapter-input"
                    min="1"
                    max=move || chapter_max().map(|n| n.to_string())
                    node_ref=refs.chapter
                    prop:value=move || selected_chapter.get().to_string()
                    on:change=on_chapter
                    on:keydown=on_chapter_keydown
                />
            </label>
            <label class="operator__field">
                <span>
                    "Verse start"
                    {move || verse_max().map(|n| view! {
                        <span class="operator__field-max" data-role="verse-max">{format!("/ {n}")}</span>
                    })}
                </span>
                <input
                    type="number"
                    data-role="verse-start"
                    min="1"
                    max=move || verse_max().map(|n| n.to_string())
                    node_ref=refs.verse_start
                    prop:value=move || verse_start_signal.get().to_string()
                    on:input=on_verse_start
                    on:keydown=on_verse_start_keydown
                />
            </label>
            <label class="operator__field">
                <span>
                    "Verse end"
                    {move || verse_max().map(|n| view! {
                        <span class="operator__field-max" data-role="verse-end-max">{format!("/ {n}")}</span>
                    })}
                </span>
                <input
                    type="number"
                    data-role="verse-end"
                    min="1"
                    max=move || verse_max().map(|n| n.to_string())
                    node_ref=refs.verse_end
                    prop:value=move || verse_end_signal.get().map(|v| v.to_string()).unwrap_or_default()
                    placeholder="All"
                    on:change=on_verse_end
                    on:keydown=on_verse_end_keydown
                />
            </label>
        </div>
        {move || range_hint.get().map(|hint| view! {
            <p class="operator__range-hint" data-role="bible-range-hint" role="status">{hint.text}</p>
        })}
    }
}
