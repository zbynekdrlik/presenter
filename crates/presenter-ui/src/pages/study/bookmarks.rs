//! Named bookmarks (#915, the owner's option 2 of 2026-11-08): "Pridať záložku"
//! in the verse sheet adds the first selected verse at once under its place's
//! name (#932: no naming sheet), the header's Záložky sheet
//! (`s=zalozky`) lists them. A row opens the reader at its verse; "Presunúť sem"
//! moves it to the place being read, "Premenovať" names it again, "Zmazať"
//! deletes it with Späť. The server keeps the list (the phone and the PC share
//! it); lookups and `study.last` never touch it.

use leptos::prelude::*;

use super::reader::chapter_settled_untracked;
use super::scroll::{self, ScrollPlan};
use super::sheet::Sheet;
use super::{nav, sermon_error, Chapter, Study, Undo};
use crate::api::bookmarks::{self as api, Bookmark, BookmarkPlace};
use crate::state::study_bookmarks::{
    offered_name, open_route, place_label, reading_place, ReadingPlace,
};

/// What the Záložky sheet shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) enum BookmarkList {
    #[default]
    Loading,
    Ready(Vec<Bookmark>),
    Failed(String),
}

fn to_api(place: ReadingPlace) -> BookmarkPlace {
    BookmarkPlace {
        translation: place.translation,
        book_code: place.book,
        chapter: place.chapter,
        verse: place.verse,
    }
}

fn of(bookmark: &Bookmark) -> BookmarkPlace {
    BookmarkPlace {
        translation: bookmark.translation.clone(),
        book_code: bookmark.book_code.clone(),
        chapter: bookmark.chapter,
        verse: bookmark.verse,
    }
}

fn load(s: Study) {
    leptos::task::spawn_local(async move {
        s.bookmarks.set(match api::list().await {
            Ok(list) => BookmarkList::Ready(list),
            Err(err) => BookmarkList::Failed(sermon_error(&err)),
        });
    });
}

/// A row the server answered replaces the shown one.
fn put_row(s: Study, changed: Bookmark) {
    s.bookmarks.update(|list| {
        if let BookmarkList::Ready(rows) = list {
            if let Some(row) = rows.iter_mut().find(|row| row.id == changed.id) {
                *row = changed;
            }
        }
    });
}

/// The verse being read; in the reader only once its chapter is on screen.
fn here(s: Study) -> Option<ReadingPlace> {
    let route = s.route.get_untracked();
    let in_view = chapter_settled_untracked(s)
        .then(|| scroll::verse_in_view(s))
        .flatten();
    reading_place(&route, s.translation().as_deref(), in_view)
}

/// "Pridať záložku" in the verse sheet (#932, owner 2026-11-11: "miesto aby sa
/// rovno pridala tak vyskočí ďalšie okno … Načo je to dobré?!"): one tap adds
/// the first selected verse under the name the old sheet offered ("Hebrejom
/// 11:4"); Premenovať in Záložky names it later. The verse sheet closes now, the
/// answer is a notice.
pub(super) fn add_here(s: Study) {
    let route = s.route.get_untracked();
    let (Some(place), Some(name)) = (
        reading_place(&route, s.translation().as_deref(), None),
        untrack(|| offered(s)),
    ) else {
        return;
    };
    nav::close_sheet(s);
    leptos::task::spawn_local(async move {
        match api::create(&name, &to_api(place), None).await {
            Ok(made) => s.tell(format!("Záložka pridaná: {}", made.name)),
            Err(err) => s.tell(sermon_error(&err)),
        }
    });
}

/// Späť on "Záložka zmazaná": it comes back where it stood.
pub(super) fn undo_delete(s: Study, bookmark: Bookmark, index: usize) {
    leptos::task::spawn_local(async move {
        match api::create(&bookmark.name, &of(&bookmark), Some(index)).await {
            Ok(_) => {
                s.tell("Záložka je späť".to_string());
                load(s);
            }
            Err(err) => s.tell_undo(
                sermon_error(&err),
                Undo::Bookmark {
                    bookmark: Box::new(bookmark),
                    index,
                },
            ),
        }
    });
}

fn open(s: Study, bookmark: &Bookmark) {
    let route = open_route(
        &bookmark.translation,
        &bookmark.book_code,
        bookmark.chapter,
        bookmark.verse,
    );
    nav::go(s, route, ScrollPlan::Verse(bookmark.verse));
}

fn move_here(s: Study, busy: RwSignal<bool>, id: i64) {
    let Some(place) = here(s) else {
        return;
    };
    busy.set(true);
    leptos::task::spawn_local(async move {
        match api::move_to(id, &to_api(place)).await {
            Ok(moved) => {
                s.tell(format!(
                    "Presunuté: {}",
                    offered_name(&moved.book, moved.chapter, moved.verse)
                ));
                put_row(s, moved);
            }
            Err(err) => s.tell(sermon_error(&err)),
        }
        busy.set(false);
    });
}

fn delete(s: Study, busy: RwSignal<bool>, id: i64) {
    busy.set(true);
    leptos::task::spawn_local(async move {
        match api::delete(id).await {
            Ok(deleted) => {
                s.bookmarks.update(|list| {
                    if let BookmarkList::Ready(rows) = list {
                        rows.retain(|row| row.id != id);
                    }
                });
                s.tell_undo(
                    format!("Zmazané: {}", deleted.bookmark.name),
                    Undo::Bookmark {
                        bookmark: Box::new(deleted.bookmark),
                        index: deleted.index,
                    },
                );
            }
            Err(err) => s.tell(sermon_error(&err)),
        }
        busy.set(false);
    });
}

fn rename(s: Study, busy: RwSignal<bool>, editing: RwSignal<Option<i64>>, id: i64, name: String) {
    busy.set(true);
    leptos::task::spawn_local(async move {
        match api::rename(id, &name).await {
            Ok(renamed) => {
                put_row(s, renamed);
                editing.set(None);
            }
            Err(err) => s.tell(sermon_error(&err)),
        }
        busy.set(false);
    });
}

#[component]
fn RenameForm(
    s: Study,
    busy: RwSignal<bool>,
    editing: RwSignal<Option<i64>>,
    bookmark: Bookmark,
) -> impl IntoView {
    let draft = RwSignal::new(bookmark.name.clone());
    let id = bookmark.id;
    view! {
        <form
            class="study-bookmark__rename"
            on:submit=move |ev| {
                ev.prevent_default();
                let name = draft.get_untracked();
                if !busy.get_untracked() && !name.trim().is_empty() {
                    rename(s, busy, editing, id, name);
                }
            }
        >
            <input
                class="study-input"
                data-role="bookmark-name"
                type="text"
                maxlength="80"
                autocomplete="off"
                prop:value=move || draft.get()
                on:input=move |ev| draft.set(event_target_value(&ev))
            />
            <button
                type="submit"
                class="study-button study-button--primary"
                data-role="bookmark-rename-save"
                disabled=move || busy.get() || draft.with(|name| name.trim().is_empty())
            >
                "Uložiť"
            </button>
        </form>
    }
}

#[component]
fn BookmarkRow(
    s: Study,
    busy: RwSignal<bool>,
    editing: RwSignal<Option<i64>>,
    can_move: Memo<bool>,
    bookmark: Bookmark,
) -> impl IntoView {
    let id = bookmark.id;
    let shown = bookmark.clone();
    let place = place_label(
        &bookmark.book,
        bookmark.chapter,
        bookmark.verse,
        &bookmark.translation,
    );
    let head = move || {
        if editing.get() == Some(id) {
            view! { <RenameForm s=s busy=busy editing=editing bookmark=shown.clone() /> }.into_any()
        } else {
            let opened = shown.clone();
            view! {
                <button
                    type="button"
                    class="study-row"
                    data-role="bookmark-open"
                    on:click=move |_| open(s, &opened)
                >
                    <span class="study-row__label">{shown.name.clone()}</span>
                    <span class="study-row__detail">{place.clone()}</span>
                </button>
            }
            .into_any()
        }
    };
    view! {
        <div class="study-bookmark" data-role="bookmark-row" data-bookmark-id=id.to_string()>
            {head}
            <div class="study-bookmark__actions">
                <Show when=move || can_move.get()>
                    <button
                        type="button"
                        class="study-button"
                        data-role="bookmark-move"
                        disabled=move || busy.get()
                        on:click=move |_| move_here(s, busy, id)
                    >
                        "Presunúť sem"
                    </button>
                </Show>
                <button
                    type="button"
                    class="study-button"
                    data-role="bookmark-rename"
                    disabled=move || busy.get()
                    on:click=move |_| {
                        editing.update(|open| *open = if *open == Some(id) { None } else { Some(id) })
                    }
                >
                    "Premenovať"
                </button>
                <button
                    type="button"
                    class="study-button"
                    data-role="bookmark-delete"
                    disabled=move || busy.get()
                    on:click=move |_| delete(s, busy, id)
                >
                    "Zmazať"
                </button>
            </div>
        </div>
    }
}

/// The rows on screen: none while the list is not `Ready`. One keyed `<For>`
/// outside the state match, so a reload (after Späť) keeps unchanged rows,
/// an open rename field and what was typed in it.
fn ready_rows(s: Study) -> Vec<Bookmark> {
    s.bookmarks.with(|list| match list {
        BookmarkList::Ready(rows) => rows.clone(),
        _ => Vec::new(),
    })
}

#[component]
fn ListState(s: Study) -> impl IntoView {
    move || match s.bookmarks.get() {
        BookmarkList::Loading => view! {
            <p class="study-empty" data-role="bookmarks-loading">"Načítavam…"</p>
        }
        .into_any(),
        BookmarkList::Failed(message) => view! {
            <div class="study-error" data-role="bookmarks-error" role="alert">
                <p>{message}</p>
                <button
                    type="button"
                    class="study-button"
                    data-role="bookmarks-retry"
                    on:click=move |_| load(s)
                >
                    "Skúsiť znova"
                </button>
            </div>
        }
        .into_any(),
        BookmarkList::Ready(rows) if rows.is_empty() => view! {
            <p class="study-empty" data-role="bookmarks-empty">
                "Zatiaľ žiadne záložky. Ťukni na verš a zvoľ „Pridať záložku“."
            </p>
        }
        .into_any(),
        BookmarkList::Ready(_) => ().into_any(),
    }
}

#[component]
pub(super) fn BookmarksSheet(s: Study) -> impl IntoView {
    // A list from an earlier visit is never shown as this one's answer.
    s.bookmarks.set(BookmarkList::Loading);
    load(s);
    let busy = RwSignal::new(false);
    let editing = RwSignal::new(None::<i64>);
    let can_move = Memo::new(move |_| {
        let route = s.route.get();
        reading_place(&route, s.translation().as_deref(), None).is_some()
    });
    // The settle marker of the specs: `ready` once this visit's list answered.
    let state = Signal::derive(move || match s.bookmarks.with(|list| list.clone()) {
        BookmarkList::Loading => "loading",
        BookmarkList::Ready(_) => "ready",
        BookmarkList::Failed(_) => "error",
    });
    view! {
        <Sheet
            s=s
            role="bookmarks"
            title=Signal::derive(|| "Záložky".to_string())
            modal=true
            state=state
        >
            <ListState s=s />
            <div class="study-list" data-role="bookmarks-list">
                <For
                    each=move || ready_rows(s)
                    key=|row| {
                        (
                            row.id,
                            row.name.clone(),
                            row.translation.clone(),
                            row.book_code.clone(),
                            row.chapter,
                            row.verse,
                        )
                    }
                    children=move |row| {
                        view! {
                            <BookmarkRow
                                s=s
                                busy=busy
                                editing=editing
                                can_move=can_move
                                bookmark=row
                            />
                        }
                    }
                />
            </div>
        </Sheet>
    }
}

/// The name offered for the first selected verse, once its chapter is here.
fn offered(s: Study) -> Option<String> {
    let verse = s.route.with(|route| route.verses.map(|(first, _)| first))?;
    let wanted = s.place()?;
    s.chapter.with(|chapter| match chapter {
        Chapter::Ready(place, text) if *place == wanted => {
            Some(offered_name(&text.book, place.chapter, verse))
        }
        _ => None,
    })
}
