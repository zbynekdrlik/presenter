//! Bookmarks (#915, reshaped by #951 — owner, 2026-11-14: "ja len potrebujem
//! vediet kde som skoncil citanie"). The header's Záložky sheet (`s=zalozky`)
//! adds the place being read with ONE button at its top (#996, as in MySword:
//! "Pridať Hebrejom 11:4", the highlighted verse, else the one at the top of
//! the reader) and lists them NEWEST FIRST, each row the place ("Hebrejom 11:4", its
//! translation as a small tag) and the start of the verse's text, the whole row
//! opening the reader there. Nothing else per row: a drag on the handle reorders
//! (pointer events on a `touch-action: none` handle, as `sermon_reorder.rs`; the
//! decisions are `nameplates::order`'s), and "Vybrať" in the head turns the rows
//! into choices for ONE "Zmazať (N)" under the list, with Späť. No rename, no
//! "Presunúť sem" (0 presses in 30 days on SNV). The server keeps the list (the
//! phone and the PC share it); lookups and `study.last` never touch it.
//!
//! #952 (owner 2026-11-20): the list is one FOLDER's. The default one, "Kde som
//! skončil", is where the sheet opens; a folder chip, then the add button, saves
//! into one of the reader's own folders. The sheet's folder row and its menu are
//! `bookmark_folders.rs`; the folder is in the URL (`s=zalozky<id>`), so a
//! switch remounts this sheet and every list call names its folder.

use leptos::prelude::*;
use web_sys::PointerEvent;

use super::bookmark_folders::{
    folder_by_id, folder_ids, fresh, load_folders, show, FolderMenu, FolderRow,
};
use super::scroll::{self, ScrollPlan};
use super::sheet::Sheet;
use super::{nav, reader, sermon_error, Chapter, Study, Undo};
use crate::api::bookmarks::{self as api, Bookmark, BookmarkPlace};
use crate::api::ApiError;
use crate::components::drag_rows;
use crate::components::nameplates::order::{apply_order, drop_marker, reorder_to_slot};
use crate::state::study::{StudySheet, StudyView};
use crate::state::study_bookmarks::{
    add_disabled_reason, add_label, added_notice, chosen_in_order, delete_label, deleted_notice,
    folder_gone, offered_name, open_route, reading_place, restore_order, toggle_chosen,
    translation_tag, ReadingPlace,
};

/// What the Záložky sheet shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) enum BookmarkList {
    #[default]
    Loading,
    Ready(Vec<Bookmark>),
    Failed(String),
}

/// `MouseEvent.button` of the primary button (a finger, the left mouse button).
const PRIMARY_BUTTON: i16 = 0;

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

/// Read the folder's list. An answer for a folder no longer on screen (a
/// quick switch, a closed sheet) is dropped: only the folder shown may land.
pub(super) fn load(s: Study, folder: Option<i64>) {
    leptos::task::spawn_local(async move {
        let answer = api::list(folder).await;
        let shown = s.route.try_with_untracked(|route| route.sheet);
        if shown != Some(Some(StudySheet::Bookmarks(folder))) {
            return;
        }
        s.bookmarks.set(match answer {
            Ok(list) => BookmarkList::Ready(list),
            Err(err) => BookmarkList::Failed(sermon_error(&err)),
        });
    });
}

/// The place the add button adds (#996, owner 2026-11-24: "kliknem na vers …
/// potom na zalozku a potom zistim ze tam neviem pridat aktualny vers"): the
/// highlighted verse (`v=`, a tap on the text or on the number), else the verse
/// at the top of the reader, else the chapter's first — with its name
/// ("Hebrejom 11:4"). `None` off a chapter, or while the chapter on screen is
/// not the route's yet (its verses would name the wrong place). Tracks the
/// route and the chapter; the verse at the top is read from the page then.
fn here(s: Study) -> Option<(ReadingPlace, String)> {
    if !reader::chapter_settled(s) {
        return None;
    }
    let route = s.route.get();
    let in_view = if route.verses.is_none() {
        scroll::verse_in_view(s)
    } else {
        None
    };
    let place = reading_place(&route, s.translation().as_deref(), in_view)?;
    let book = s.chapter.with(|chapter| match chapter {
        Chapter::Ready(_, text) => Some(text.book.clone()),
        _ => None,
    })?;
    let name = offered_name(&book, place.chapter, place.verse);
    Some((place, name))
}

/// The add button: the place goes on TOP of the folder shown (`None` = the
/// default one). The sheet stays open and its list is read again, so the new
/// row shows at once; the notice names the place (and an own folder).
fn add_here(s: Study, folder: Option<i64>, adding: RwSignal<bool>) {
    if adding.get_untracked() {
        return;
    }
    let Some((place, name)) = untrack(|| here(s)) else {
        let reading = s
            .route
            .with_untracked(|route| matches!(route.view, StudyView::Reader { .. }));
        s.tell(add_disabled_reason(reading).to_string());
        return;
    };
    let folder_name = folder
        .and_then(|id| untrack(|| folder_by_id(s, id)))
        .map(|found| found.name);
    adding.set(true);
    leptos::task::spawn_local(async move {
        let answer = api::create(folder, &name, &to_api(place), None).await;
        // The sheet may be closed by now.
        adding.try_set(false);
        match answer {
            Ok(made) => {
                s.tell(added_notice(&made.name, folder_name.as_deref()));
                load(s, folder);
            }
            Err(err) => s.tell(sermon_error(&err)),
        }
    });
}

/// "Pridať Hebrejom 11:4" at the top of the sheet, under the folder chips;
/// off, with the reason under it, while there is no place to add.
#[component]
fn AddHere(s: Study, folder: Option<i64>) -> impl IntoView {
    let adding = RwSignal::new(false);
    let place = Memo::new(move |_| here(s).map(|(_, name)| name));
    let reading = move || {
        s.route
            .with(|route| matches!(route.view, StudyView::Reader { .. }))
    };
    view! {
        <div class="study-bookmarks__add">
            <button
                type="button"
                class="study-button study-button--primary"
                data-role="bookmarks-add"
                disabled=move || place.with(Option::is_none) || adding.get()
                on:click=move |_| add_here(s, folder, adding)
            >
                {move || add_label(place.get().as_deref())}
            </button>
            <Show when=move || place.with(Option::is_none)>
                <p class="study-sheet__hint" data-role="bookmarks-add-why">
                    {move || add_disabled_reason(reading())}
                </p>
            </Show>
        </div>
    }
}

/// Späť on "Zmazané …": each comes back where it stood, the lowest index first
/// (`restore_order`), so every row lands on its old place.
pub(super) fn undo_delete(s: Study, folder: Option<i64>, items: Vec<(Bookmark, usize)>) {
    leptos::task::spawn_local(async move {
        let items = restore_order(items);
        for (at, (bookmark, index)) in items.iter().enumerate() {
            let back = api::create(folder, &bookmark.name, &of(bookmark), Some(*index)).await;
            if let Err(err) = back {
                if matches!(err, ApiError::Refused(404, _) | ApiError::Status(404, _)) {
                    // The folder itself is gone (deleted meanwhile): another
                    // Späť would fail the same way.
                    s.tell(sermon_error(&err));
                    return;
                }
                // The ones not back yet keep their Späť.
                s.tell_undo(
                    sermon_error(&err),
                    Undo::Bookmarks {
                        folder,
                        items: items[at..].to_vec(),
                    },
                );
                load(s, folder);
                return;
            }
        }
        s.tell(if items.len() == 1 {
            "Záložka je späť".to_string()
        } else {
            "Záložky sú späť".to_string()
        });
        load(s, folder);
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

/// The rows on screen: none while the list is not `Ready`.
fn ready_rows(s: Study) -> Vec<Bookmark> {
    s.bookmarks.with(|list| match list {
        BookmarkList::Ready(rows) => rows.clone(),
        _ => Vec::new(),
    })
}

fn ids_now(s: Study) -> Vec<i64> {
    s.bookmarks.with_untracked(|list| match list {
        BookmarkList::Ready(rows) => rows.iter().map(|row| row.id).collect(),
        _ => Vec::new(),
    })
}

/// The page's bookmark writes (#951): ONE at a time, an order before a delete,
/// owned by the page, so a write still waiting when the sheet closes is sent,
/// and a waiting order never lands after a delete as a stale 409 whose toast
/// would replace the delete's Späť.
#[derive(Default)]
pub(super) struct BookmarkWrites {
    /// A write on its way.
    sending: bool,
    /// The newest order chosen and not sent yet, with its folder.
    order: Option<(Option<i64>, Vec<i64>)>,
    /// The chosen ids of "Zmazať (N)", with the sheet that asked (and so its
    /// folder).
    delete: Option<(Vec<i64>, ListUi)>,
}

/// Send the next waiting write, if none is on its way.
fn pump(s: Study) {
    let next = s.bookmark_writes.try_update_value(|writes| {
        if writes.sending {
            return None;
        }
        let next = match writes.order.take() {
            Some((folder, ids)) => Some(Write::Order(folder, ids)),
            None => writes.delete.take().map(|(ids, ui)| Write::Delete(ids, ui)),
        };
        writes.sending = next.is_some();
        next
    });
    let Some(next) = next.flatten() else {
        return;
    };
    leptos::task::spawn_local(async move {
        match next {
            Write::Order(folder, ids) => {
                if let Err(err) = api::reorder(folder, &ids).await {
                    // Another device changed the list: show it as stored.
                    s.tell(sermon_error(&err));
                    load(s, folder);
                }
            }
            Write::Delete(ids, ui) => {
                let answer = api::delete_many(ui.folder, &ids).await;
                deleted(s, ui, &ids, answer);
            }
        }
        s.bookmark_writes
            .update_value(|writes| writes.sending = false);
        pump(s);
    });
}

enum Write {
    Order(Option<i64>, Vec<i64>),
    Delete(Vec<i64>, ListUi),
}

/// A delete answered. The sheet may be gone by now: its signals are written
/// with `try_*` only.
fn deleted(s: Study, ui: ListUi, ids: &[i64], answer: Result<Vec<api::Deleted>, ApiError>) {
    ui.busy.try_set(false);
    match answer {
        Ok(deleted) => {
            s.bookmarks.update(|list| {
                if let BookmarkList::Ready(rows) = list {
                    rows.retain(|row| !ids.contains(&row.id));
                }
            });
            ui.choosing.try_set(false);
            ui.chosen.try_set(Vec::new());
            s.tell_undo(
                deleted_notice(deleted.len()),
                Undo::Bookmarks {
                    folder: ui.folder,
                    items: deleted
                        .into_iter()
                        .map(|item| (item.bookmark, item.index))
                        .collect(),
                },
            );
        }
        Err(err) => {
            s.tell(sermon_error(&err));
            load(s, ui.folder);
        }
    }
}

/// The sheet's own state, `Copy` for the rows.
#[derive(Clone, Copy)]
pub(super) struct ListUi {
    s: Study,
    /// The folder on screen (`None` = the default one).
    folder: Option<i64>,
    /// "Vybrať" is on: a row tap chooses instead of opening.
    choosing: RwSignal<bool>,
    chosen: RwSignal<Vec<i64>>,
    /// The dragged id and the slot it would drop into now.
    drag: RwSignal<Option<(i64, usize)>>,
    /// A delete waiting or on its way.
    busy: RwSignal<bool>,
}

impl ListUi {
    fn new(s: Study, folder: Option<i64>) -> Self {
        Self {
            s,
            folder,
            choosing: RwSignal::new(false),
            chosen: RwSignal::new(Vec::new()),
            drag: RwSignal::new(None),
            busy: RwSignal::new(false),
        }
    }

    /// The chosen ids still listed, in the list's order.
    fn chosen_now(self) -> Vec<i64> {
        let listed: Vec<i64> = ready_rows(self.s).iter().map(|row| row.id).collect();
        self.chosen.with(|chosen| chosen_in_order(&listed, chosen))
    }

    fn set_choosing(self, on: bool) {
        self.choosing.set(on);
        self.chosen.set(Vec::new());
    }

    /// A drop: the rows move at once, the order goes to the server (only the
    /// newest waiting order is sent next, so the server ends on the last one).
    fn drop_at(self, id: i64, slot: usize) {
        let Some(ids) = reorder_to_slot(&ids_now(self.s), &id, slot) else {
            return;
        };
        self.s.bookmarks.update(|list| {
            if let BookmarkList::Ready(rows) = list {
                apply_order(rows, &ids, |row| row.id);
            }
        });
        self.s
            .bookmark_writes
            .update_value(|writes| writes.order = Some((self.folder, ids)));
        pump(self.s);
    }

    /// "Zmazať (N)": the chosen ones at once (after any order still on its
    /// way), then a toast with Späť.
    fn delete_chosen(self) {
        let ids = self.chosen_now();
        if ids.is_empty() || self.busy.get_untracked() {
            return;
        }
        self.busy.set(true);
        self.s
            .bookmark_writes
            .update_value(|writes| writes.delete = Some((ids, self)));
        pump(self.s);
    }
}

const HANDLE: &str = "[data-role=\"bookmark-drag\"]";

/// The drop slot under the pointer, measured on the rows on screen.
fn slot_under(ev: &PointerEvent) -> Option<usize> {
    drag_rows::slot_under(
        ev,
        HANDLE,
        "[data-role=\"bookmarks-list\"]",
        "[data-role=\"bookmark-row\"]",
    )
}

/// The handle: press, move, release. A press captures the pointer, so the
/// moves and the release reach the handle anywhere on the screen.
#[component]
fn DragHandle(ui: ListUi, id: i64, position: Memo<Option<usize>>) -> impl IntoView {
    let drag = ui.drag;
    let on_down = move |ev: PointerEvent| {
        if ev.button() != PRIMARY_BUTTON {
            return;
        }
        let Some(handle) = drag_rows::handle_of(&ev, HANDLE) else {
            return;
        };
        ev.prevent_default();
        let _ = handle.set_pointer_capture(ev.pointer_id());
        drag.set(Some((id, position.get_untracked().unwrap_or(0))));
    };
    let on_move = move |ev: PointerEvent| {
        if drag.with_untracked(Option::is_none) {
            return;
        }
        if ev.buttons() == 0 {
            drag.set(None);
            return;
        }
        if let Some(slot) = slot_under(&ev) {
            drag.update(|held| {
                if let Some(held) = held {
                    held.1 = slot;
                }
            });
        }
    };
    let on_up = move |_: PointerEvent| {
        let Some((id, slot)) = drag.get_untracked() else {
            return;
        };
        drag.set(None);
        ui.drop_at(id, slot);
    };
    view! {
        <button
            type="button"
            class="study-bookmark__handle"
            data-role="bookmark-drag"
            aria-label="Presunúť záložku"
            on:pointerdown=on_down
            on:pointermove=on_move
            on:pointerup=on_up
            on:pointercancel=move |_| drag.set(None)
            on:lostpointercapture=move |_| drag.set(None)
        >
            "\u{2261}"
        </button>
    }
}

#[component]
fn BookmarkRow(ui: ListUi, bookmark: Bookmark) -> impl IntoView {
    let s = ui.s;
    let id = bookmark.id;
    let place = offered_name(&bookmark.book, bookmark.chapter, bookmark.verse);
    let tag = translation_tag(&bookmark.translation);
    let text = bookmark.text.clone();
    // The row's place follows the list, never captured: a drop moves it.
    let position = Memo::new(move |_| {
        s.bookmarks.with(|list| match list {
            BookmarkList::Ready(rows) => rows.iter().position(|row| row.id == id),
            _ => None,
        })
    });
    let count = Memo::new(move |_| {
        s.bookmarks.with(|list| match list {
            BookmarkList::Ready(rows) => rows.len(),
            _ => 0,
        })
    });
    let marker = move || {
        let (dragged, slot) = ui.drag.get()?;
        let from = ids_now(s).iter().position(|held| *held == dragged)?;
        drop_marker(from, slot, position.get()?, count.get())
    };
    let chosen = Memo::new(move |_| ui.chosen.with(|chosen| chosen.contains(&id)));
    let opened = bookmark.clone();
    let on_tap = move |_| {
        if ui.choosing.get_untracked() {
            ui.chosen.update(|chosen| toggle_chosen(chosen, id));
        } else {
            open(s, &opened);
        }
    };
    view! {
        <div
            class="study-bookmark"
            data-role="bookmark-row"
            data-bookmark-id=id.to_string()
            data-chosen=move || chosen.get().then_some("true")
            data-drop=marker
            data-dragging=move || {
                ui.drag.with(|held| held.is_some_and(|(dragged, _)| dragged == id)).then_some("true")
            }
        >
            <Show when=move || ui.choosing.get()>
                <span class="study-bookmark__check" aria-hidden="true">
                    {move || if chosen.get() { "\u{2713}" } else { "" }}
                </span>
            </Show>
            <button
                type="button"
                class="study-bookmark__open"
                data-role=move || if ui.choosing.get() { "bookmark-pick" } else { "bookmark-open" }
                aria-pressed=move || ui.choosing.get().then(|| chosen.get().to_string())
                on:click=on_tap
            >
                <span class="study-bookmark__place">
                    <span class="study-bookmark__label">{place}</span>
                    <span class="study-bookmark__tag">{tag}</span>
                </span>
                {text
                    .map(|text| {
                        view! {
                            <span class="study-bookmark__text" data-role="bookmark-text">
                                {text}
                            </span>
                        }
                    })}
            </button>
            <Show when=move || !ui.choosing.get()>
                <DragHandle ui=ui id=id position=position />
            </Show>
        </div>
    }
}

const EMPTY_DEFAULT: &str =
    "Zatiaľ žiadne záložky. Miesto, kde čítaš, pridáš tlačidlom „Pridať …“ hore.";
const EMPTY_FOLDER: &str =
    "Priečinok je prázdny. Miesto, kde čítaš, sem pridáš tlačidlom „Pridať …“ hore.";

#[component]
fn ListState(s: Study, folder: Option<i64>) -> impl IntoView {
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
                    on:click=move |_| load(s, folder)
                >
                    "Skúsiť znova"
                </button>
            </div>
        }
        .into_any(),
        BookmarkList::Ready(rows) if rows.is_empty() => view! {
            <p class="study-empty" data-role="bookmarks-empty">
                {if folder.is_some() { EMPTY_FOLDER } else { EMPTY_DEFAULT }}
            </p>
        }
        .into_any(),
        BookmarkList::Ready(_) => ().into_any(),
    }
}

/// "Vybrať" / "Hotovo" in the head: only with rows to choose from.
fn choose_toggle(ui: ListUi) -> impl IntoView {
    let any = Memo::new(move |_| {
        ui.s.bookmarks
            .with(|list| matches!(list, BookmarkList::Ready(rows) if !rows.is_empty()))
    });
    move || {
        (any.get() || ui.choosing.get()).then(|| {
            view! {
                <button
                    type="button"
                    class="study-button study-sheet__action"
                    data-role="bookmarks-choose"
                    aria-pressed=move || ui.choosing.get().to_string()
                    on:click=move |_| ui.set_choosing(!ui.choosing.get_untracked())
                >
                    {move || if ui.choosing.get() { "Hotovo" } else { "Vybrať" }}
                </button>
            }
        })
    }
}

/// The ONE delete under the list while choosing.
fn delete_bar(ui: ListUi) -> impl IntoView {
    let count = Memo::new(move |_| ui.chosen_now().len());
    move || {
        ui.choosing.get().then(|| {
            view! {
                <button
                    type="button"
                    class="study-button study-button--danger"
                    data-role="bookmarks-delete"
                    disabled=move || count.get() == 0 || ui.busy.get()
                    on:click=move |_| ui.delete_chosen()
                >
                    {move || delete_label(count.get())}
                </button>
            }
        })
    }
}

#[component]
pub(super) fn BookmarksSheet(s: Study, folder: Option<i64>) -> impl IntoView {
    // A list from an earlier visit (or folder) is never shown as this one's answer.
    s.bookmarks.set(BookmarkList::Loading);
    let read = load_folders(s);
    match folder {
        None => load(s, None),
        // An own folder is read once the reader's folders confirm it is still
        // theirs; a folder gone (deleted elsewhere, a stale link) shows the
        // default one instead, asking nothing for it.
        Some(_) => {
            Effect::new(move |done: Option<bool>| {
                if done == Some(true) || !fresh(s, read) {
                    return done.unwrap_or(false);
                }
                if folder_gone(folder, &folder_ids(s)) {
                    show(s, None);
                } else {
                    load(s, folder);
                }
                true
            });
        }
    }
    let ui = ListUi::new(s, folder);
    // The settle marker of the specs: `ready` once this visit's list answered.
    let state = Signal::derive(move || match s.bookmarks.with(|list| list.clone()) {
        BookmarkList::Loading => "loading",
        BookmarkList::Ready(_) => "ready",
        BookmarkList::Failed(_) => "error",
    });
    // A drag whose row is gone (a reload after a refused order) never ends on
    // its own: its handle, and so its release, is gone.
    Effect::new(move |_| {
        let gone = ui
            .drag
            .get()
            .is_some_and(|(id, _)| !ready_rows(s).iter().any(|row| row.id == id));
        if gone {
            ui.drag.set(None);
        }
    });
    view! {
        <Sheet
            s=s
            role="bookmarks"
            title=Signal::derive(|| "Záložky".to_string())
            modal=true
            state=state
            action=ViewFn::from(move || choose_toggle(ui))
            footer=ViewFn::from(move || delete_bar(ui))
        >
            <FolderRow s=s folder=folder read=read />
            <AddHere s=s folder=folder />
            <FolderMenu s=s folder=folder />
            <ListState s=s folder=folder />
            <div
                class="study-list study-bookmarks"
                data-role="bookmarks-list"
                data-choosing=move || ui.choosing.get().then_some("true")
            >
                <For
                    each=move || ready_rows(s)
                    key=|row| (row.id, row.text.clone())
                    children=move |row| view! { <BookmarkRow ui=ui bookmark=row /> }
                />
            </div>
        </Sheet>
    }
}
