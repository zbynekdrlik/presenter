//! `+ Pridať verš` (spec §7.6): the editor's field, with the reference resolver
//! of the reader's.

use leptos::prelude::*;

use super::add_to_sermon::{begin_add, set_active};
use super::nav;
use super::scroll::ScrollPlan;
use super::sermon_editor::{run_write, Editor, WriteEnd};
use crate::api::sermons::{self, SermonPassage};
use crate::state::study::{suggest, StudyRoute};

/// `+ Pridať verš`: a reference is added here, words go to the text search,
/// whose hits add to this (active) sermon. The field keeps what was typed until
/// the verse is in.
fn add_from_field(e: Editor, text: RwSignal<String>) {
    let input = text.get_untracked();
    let Some(translation) = e
        .detail
        .with_untracked(|detail| detail.as_ref().map(|held| held.default_translation.clone()))
    else {
        return;
    };
    let suggestion = e.s.structures.with_untracked(|map| {
        map.get(&translation)
            .map(|structure| (suggest(&input, structure, &translation), structure.clone()))
    });
    let Some((suggestion, structure)) = suggestion else {
        e.s.tell("Zoznam kníh sa ešte načítava".to_string());
        return;
    };
    // A word naming a book is a search here, never a whole chapter added.
    let target = suggestion
        .enter_target()
        .filter(|target| target.verses.is_some() || suggestion.text_search.is_none());
    if let Some(target) = target {
        let verses = target.verses.or_else(|| {
            let last = structure.book(&target.book)?.verse_count(target.chapter)?;
            Some((1, last))
        });
        let Some((verse_start, verse_end)) = verses else {
            return;
        };
        let passage = SermonPassage {
            translation,
            book_code: target.book,
            chapter: target.chapter,
            verse_start,
            verse_end,
        };
        // The field keeps its text meanwhile: a second Enter must not add again.
        if !begin_add(e.s, &passage) {
            return;
        }
        leptos::task::spawn_local(async move {
            let end = run_write(
                e,
                "Kázeň sa medzitým zmenila — skús znova",
                move |id, revision| async move {
                    sermons::add_item(&id, revision, None, &passage).await
                },
            )
            .await;
            e.s.adding.set_value(None);
            if end == WriteEnd::Landed {
                text.update(|shown| {
                    if *shown == input {
                        shown.clear();
                    }
                });
            }
        });
        return;
    }
    if let Some(query) = suggestion.text_search {
        text.set(String::new());
        set_active(e.s, Some(e.id()));
        nav::go(
            e.s,
            StudyRoute::search(&translation, &query),
            ScrollPlan::Top,
        );
        return;
    }
    e.s.tell(
        suggestion
            .view
            .hint
            .unwrap_or_else(|| "Odkaz sa nepodarilo prečítať".to_string()),
    );
}

#[component]
pub(super) fn AddVerse(e: Editor) -> impl IntoView {
    let text = RwSignal::new(String::new());
    view! {
        <form
            class="sermon-add"
            on:submit=move |ev| {
                ev.prevent_default();
                add_from_field(e, text);
            }
        >
            <input
                class="study-input"
                data-role="sermon-add-field"
                type="text"
                placeholder="+ Pridať verš: gal 3 13-14"
                autocomplete="off"
                autocapitalize="off"
                prop:value=move || text.get()
                on:input=move |ev| text.set(event_target_value(&ev))
            />
        </form>
    }
}
